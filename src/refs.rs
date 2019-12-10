use futures::executor::block_on;
use serde::{ser::SerializeTuple, Serialize, Serializer};
use std::any::TypeId;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem::transmute;
use std::sync::{Arc, RwLock, Weak};

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
    pub recipe_info: RwLock<crate::asset_reg::RecipeInfo>,
    pub(crate) being_evaluated: futures::lock::Mutex<()>,
}

impl OpaqueSnoozyRefInner {
    fn clone_desc(&self) -> Self {
        Self {
            addr: self.addr.clone(),
            recipe_info: RwLock::new(self.recipe_info.read().unwrap().clone_desc()),
            being_evaluated: Default::default(),
        }
    }

    pub fn to_evaluation_path_node(&self) -> usize {
        self as *const Self as usize
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
pub struct OpaqueSnoozyRef(pub Arc<OpaqueSnoozyRefInner>);

impl OpaqueSnoozyRef {
    pub fn get_transient_op_id(&self) -> usize {
        let inner: &OpaqueSnoozyRefInner = &**self;
        inner as *const OpaqueSnoozyRefInner as usize
    }
}

impl std::ops::Deref for OpaqueSnoozyRef {
    type Target = Arc<OpaqueSnoozyRefInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq for OpaqueSnoozyRef {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(
            &*self.0 as *const OpaqueSnoozyRefInner,
            &*other.0 as *const OpaqueSnoozyRefInner,
        )
    }
}
impl Eq for OpaqueSnoozyRef {}

pub type WeakOpaqueSnoozyRef = Weak<OpaqueSnoozyRefInner>;

impl Hash for OpaqueSnoozyRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let ptr = &***self as *const OpaqueSnoozyRefInner as usize;
        ptr.hash(state);
    }
}

#[derive(Hash, Serialize)]
pub struct SnoozyRef<Res> {
    pub opaque: OpaqueSnoozyRef,
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
        self.opaque = OpaqueSnoozyRef(Arc::new((*self.opaque).clone_desc()));
        self.isolated = true;
        self
    }

    pub fn rebind(&mut self, other: Self) {
        assert!(
            self.isolated,
            "rebind() can only be used on isolated refs. Use isolate() first."
        );

        // Evaluate the recipe in case it's used recursively in its own definition
        block_on(crate::asset_reg::ASSET_REG.evaluate_recipe(&self.opaque, HashSet::new()));

        let hash_difference = {
            let entry = self.opaque.recipe_info.read().unwrap();
            let other_entry = other.opaque.recipe_info.read().unwrap();
            //dbg!((entry.recipe_hash, other_entry.recipe_hash));
            entry.recipe_hash != other_entry.recipe_hash
        };

        let mut info = self.opaque.recipe_info.write().unwrap();
        let other_info = Arc::try_unwrap(other.opaque.0)
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
