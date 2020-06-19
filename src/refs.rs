use crate::cycle_detector::create_cycle_detector;
use crate::iface::EvalContext;
use crate::recipe_info::RecipeInfo;
use serde::{ser::SerializeTuple, Serialize, Serializer};
use std::any::TypeId;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem::transmute;
use std::sync::{
    atomic::{self, AtomicUsize},
    Arc, RwLock,
};

#[derive(Hash, Clone, Copy, Eq, PartialEq, Debug, Serialize)]
pub struct SnoozyIdentityHash(pub(crate) u64);

#[derive(Hash, Clone, Eq, PartialEq, Debug)]
pub struct OpaqueSnoozyAddr {
    pub(crate) identity_hash: SnoozyIdentityHash,
    pub(crate) type_id: TypeId,
}

impl OpaqueSnoozyAddr {
    pub(crate) fn new<Res: 'static>(identity_hash: SnoozyIdentityHash) -> Self {
        Self {
            identity_hash,
            type_id: TypeId::of::<Res>(),
        }
    }
}

impl Serialize for OpaqueSnoozyAddr {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tup = serializer.serialize_tuple(2)?;
        tup.serialize_element(&self.identity_hash)?;
        unsafe { tup.serialize_element(&transmute::<_, u64>(self.type_id))? };
        tup.end()
    }
}

pub struct OpaqueSnoozyRefInner {
    pub(crate) addr: OpaqueSnoozyAddr,
    pub recipe_info: RwLock<RecipeInfo>,
    pub rebuild_pending: AtomicUsize,
    pub(crate) being_evaluated: futures::lock::Mutex<()>,
}

#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct EvaluationPathNode(usize);

impl EvaluationPathNode {
    pub fn into_raw(self) -> usize {
        self.0
    }
}

impl OpaqueSnoozyRefInner {
    fn clone_desc(&self) -> Self {
        Self {
            addr: self.addr.clone(),
            recipe_info: RwLock::new(self.recipe_info.read().unwrap().clone_desc()),
            rebuild_pending: AtomicUsize::new(1),
            being_evaluated: Default::default(),
        }
    }

    pub fn to_evaluation_path_node(&self) -> EvaluationPathNode {
        EvaluationPathNode(self as *const Self as usize)
    }
}

impl fmt::Debug for OpaqueSnoozyRefInner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "OpaqueSnoozyRef {{identity_hash: {:?}}}",
            self.addr.identity_hash
        )
    }
}

impl Serialize for OpaqueSnoozyRefInner {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.addr.serialize(serializer)
    }
}

#[derive(Clone, Serialize)]
pub struct OpaqueSnoozyRef {
    pub inner: Arc<OpaqueSnoozyRefInner>,
    pub use_prev: bool,
}

impl OpaqueSnoozyRef {
    pub fn get_transient_op_id(&self) -> usize {
        let inner: &OpaqueSnoozyRefInner = &*self.inner;
        inner as *const OpaqueSnoozyRefInner as usize
    }
}

impl std::ops::Deref for OpaqueSnoozyRef {
    type Target = Arc<OpaqueSnoozyRefInner>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Eq for OpaqueSnoozyRef {}
impl PartialEq for OpaqueSnoozyRef {
    fn eq(&self, other: &Self) -> bool {
        self.use_prev == other.use_prev
            && std::ptr::eq(
                &*self.inner as *const OpaqueSnoozyRefInner,
                &*other.inner as *const OpaqueSnoozyRefInner,
            )
    }
}
impl Hash for OpaqueSnoozyRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let ptr = &*self.inner as *const OpaqueSnoozyRefInner as usize;
        ptr.hash(state);
        self.use_prev.hash(state);
    }
}

#[derive(Hash, Serialize)]
pub struct SnoozyRef<Res> {
    pub opaque: OpaqueSnoozyRef,
    // TODO: move `isolated` to `opaque`?
    isolated: bool,
    phantom: PhantomData<Res>,
}

impl<Res> SnoozyRef<Res> {
    pub fn new(opaque: OpaqueSnoozyRef) -> Self {
        Self {
            opaque,
            isolated: false,
            phantom: PhantomData,
        }
    }

    pub fn identity_hash(&self) -> SnoozyIdentityHash {
        self.opaque.addr.identity_hash
    }

    pub fn isolate(mut self) -> Self {
        self.opaque = OpaqueSnoozyRef {
            inner: Arc::new((self.opaque.inner).clone_desc()),
            use_prev: self.opaque.use_prev,
        };
        self.isolated = true;
        self
    }

    pub fn prev(&self) -> Self {
        let mut res = self.clone();
        res.opaque.use_prev = true;
        res
    }

    pub fn rebind(&mut self, other: Self) {
        assert!(
            self.isolated,
            "rebind() can only be used on isolated refs. Use isolate() first."
        );

        let mut runtime = unsafe { crate::RUNTIME.as_ref().unwrap() }
            .try_lock()
            .expect("SnoozyRef::rebind cannot be called from an async context");

        let opaque_ref = self.opaque.clone();
        let task = runtime.spawn(async move {
            let (cycle_detector, _cycle_detector_backend) = create_cycle_detector();
            let eval_context = EvalContext {
                cycle_detector,
                snapshot_idx: 0,
            };

            // TODO
            /*std::thread::spawn(move || {
                cycle_detector_backend.run();
            });*/

            crate::asset_reg::ASSET_REG
                .evaluate_recipe(&opaque_ref, HashSet::new(), eval_context)
                .await
        });

        // Evaluate the recipe in case it's used recursively in its own definition
        runtime.block_on(task).unwrap();

        let hash_difference = {
            let entry = self.opaque.recipe_info.read().unwrap();
            let other_entry = other.opaque.recipe_info.read().unwrap();
            //dbg!((entry.recipe_hash, other_entry.recipe_hash));
            entry.recipe_hash != other_entry.recipe_hash
        };

        self.opaque.rebuild_pending.store(
            other.opaque.rebuild_pending.load(atomic::Ordering::Acquire),
            atomic::Ordering::Release,
        );

        let mut info = self.opaque.recipe_info.write().unwrap();
        let other_info = Arc::try_unwrap(other.opaque.inner)
            .expect("Rebind source must have only one reference")
            .recipe_info
            .into_inner()
            .unwrap();

        info.replace_desc(other_info);

        if hash_difference {
            // Schedule a full rebuild including of all of its reverse dependencies.
            crate::asset_reg::ASSET_REG
                .queued_asset_invalidations
                .lock()
                .unwrap()
                .push(self.opaque.clone());
        }
    }
}

impl<Res: 'static> Into<OpaqueSnoozyRef> for SnoozyRef<Res> {
    fn into(self) -> OpaqueSnoozyRef {
        self.opaque
    }
}

// So we can use Into<SnoozyRef<_>>, and accept either & or copy.
impl<Res> From<&SnoozyRef<Res>> for SnoozyRef<Res> {
    fn from(v: &SnoozyRef<Res>) -> Self {
        (*v).clone()
    }
}

impl<Res> Clone for SnoozyRef<Res> {
    fn clone(&self) -> Self {
        SnoozyRef {
            opaque: self.opaque.clone(),
            isolated: false, // Must call `isolate()` on the original handle
            phantom: PhantomData,
        }
    }
}

#[cfg(core_intrinsics)]
impl<Res> fmt::Debug for SnoozyRef<Res> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "SnoozyRef<{}> {{identity_hash: {}}}",
            std::any::type_name::<Res>(),
            self.opaque.identity_hash()
        )
    }
}

#[cfg(not(core_intrinsics))]
impl<Res> fmt::Debug for SnoozyRef<Res> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SnoozyRef {{identity_hash: {:?}}}", self.identity_hash())
    }
}
