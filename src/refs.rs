use serde::{ser::SerializeTuple, Serialize, Serializer};
use std::any::TypeId;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem::transmute;
use std::sync::{Arc, RwLock, Weak};

#[derive(Hash, Clone, Eq, PartialEq, Debug)]
pub struct OpaqueSnoozyAddr {
    pub(crate) identity_hash: u64,
    pub(crate) type_id: TypeId,
}

impl OpaqueSnoozyAddr {
    pub(crate) fn new<Res: 'static>(identity_hash: u64) -> Self {
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
            "OpaqueSnoozyRef {{identity_hash: {}}}",
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

impl<Res> SnoozyRef<Res> {
    pub fn new(opaque: OpaqueSnoozyRef) -> Self {
        Self {
            opaque,
            phantom: PhantomData,
        }
    }

    pub fn identity_hash(&self) -> u64 {
        self.opaque.addr.identity_hash
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

//impl<Res> Copy for SnoozyRef<Res> {}
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
        write!(f, "SnoozyRef {{identity_hash: {}}}", self.identity_hash())
    }
}
