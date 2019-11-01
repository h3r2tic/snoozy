use serde::{ser::SerializeTuple, Serialize, Serializer};
use std::any::TypeId;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem::transmute;
use std::sync::{Arc, RwLock, Weak};

#[derive(Hash, Clone, Copy, Eq, PartialEq, Debug, Serialize)]
pub enum SnoozyIdentityHash {
    Recipe(u64),
    Named(u64),
}

impl SnoozyIdentityHash {
    pub fn is_named(&self) -> bool {
        if let SnoozyIdentityHash::Named(_) = self {
            true
        } else {
            false
        }
    }
}

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
    pub(crate) recipe_info: RwLock<crate::asset_reg::RecipeInfo>,
}

impl PartialEq for OpaqueSnoozyRefInner {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl Eq for OpaqueSnoozyRefInner {}

impl Hash for OpaqueSnoozyRefInner {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.addr.hash(state);
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

pub type OpaqueSnoozyRef = Arc<OpaqueSnoozyRefInner>;
pub type WeakOpaqueSnoozyRef = Weak<OpaqueSnoozyRefInner>;

#[derive(Hash, Serialize)]
pub struct SnoozyRef<Res> {
    pub opaque: OpaqueSnoozyRef,
    phantom: PhantomData<Res>,
}

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_UNIQUE_REF: AtomicU64 = AtomicU64::new(0);

impl<Res> SnoozyRef<Res> {
    pub fn new(opaque: OpaqueSnoozyRef) -> Self {
        Self {
            opaque,
            phantom: PhantomData,
        }
    }

    pub fn identity_hash(&self) -> SnoozyIdentityHash {
        self.opaque.addr.identity_hash
    }

    pub fn into_named(self) -> Self {
        let identity_hash =
            SnoozyIdentityHash::Named(NEXT_UNIQUE_REF.fetch_add(1, Ordering::Relaxed));

        Self {
            opaque: Arc::new(OpaqueSnoozyRefInner {
                addr: OpaqueSnoozyAddr {
                    identity_hash,
                    type_id: self.opaque.addr.type_id,
                },
                recipe_info: RwLock::new(self.opaque.recipe_info.read().unwrap().clone_desc()),
            }),
            phantom: PhantomData,
        }
    }

    pub fn rebind(&mut self, other: Self) {
        assert!(
            self.identity_hash().is_named(),
            "rebind() can only be used on named refs. Use into_named() first."
        );

        // Evaluate the recipe in case it's used recursively in its own definition
        crate::asset_reg::ASSET_REG.evaluate_recipe(&self.opaque);

        let mut entry = self.opaque.recipe_info.write().unwrap();

        let other_entry = other.opaque.recipe_info.read().unwrap();

        if entry.recipe_hash != other_entry.recipe_hash {
            let clone_desc = other_entry.clone_desc();
            entry.recipe_runner = clone_desc.recipe_runner;
            entry.recipe_meta = clone_desc.recipe_meta;
            entry.recipe_hash = clone_desc.recipe_hash;

            // Clear any pending rebuild of this asset, and instead schedule
            // a full rebuild including of all of its reverse dependencies.
            entry.rebuild_pending = false;
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
            unsafe { std::intrinsics::type_name::<Res>() },
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
