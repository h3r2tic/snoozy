use serde::{ser::SerializeTuple, Serialize, Serializer};
use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::mem::transmute;
use std::sync::Arc;

#[derive(Hash, Clone, Eq, PartialEq, Debug)]
pub struct OpaqueSnoozyRefInner {
    pub(crate) identity_hash: u64,
    pub(crate) type_id: TypeId,
}

impl OpaqueSnoozyRefInner {
    pub(crate) fn new<Res: 'static>(identity_hash: u64) -> Self {
        Self {
            identity_hash,
            type_id: TypeId::of::<Res>(),
        }
    }
}

impl Serialize for OpaqueSnoozyRefInner {
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

#[derive(Hash, Clone, Eq, PartialEq, Debug)]
pub struct OpaqueSnoozyRef {
    pub(crate) inner: Arc<OpaqueSnoozyRefInner>,
}

impl Serialize for OpaqueSnoozyRef {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.inner.serialize(serializer)
    }
}

impl OpaqueSnoozyRef {
    pub(crate) fn new(inner: Arc<OpaqueSnoozyRefInner>) -> Self {
        Self { inner }
    }
}

impl std::ops::Deref for OpaqueSnoozyRef {
    type Target = OpaqueSnoozyRefInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

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
        self.opaque.inner.identity_hash
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
        write!(
            f,
            "SnoozyRef {{identity_hash: {}}}",
            self.opaque.identity_hash
        )
    }
}
