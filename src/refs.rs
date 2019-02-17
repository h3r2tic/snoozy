use serde::{ser::SerializeTuple, Serialize, Serializer};
use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::mem::transmute;

#[derive(Serialize, Hash)]
pub struct SnoozyRef<Res> {
    pub identity_hash: u64,
    phantom: PhantomData<Res>,
}

impl<Res> SnoozyRef<Res> {
    pub fn new(identity_hash: u64) -> Self {
        Self {
            identity_hash,
            phantom: PhantomData,
        }
    }
}

// So we can use Into<SnoozyRef<_>>, and accept either & or copy.
impl<Res> From<&SnoozyRef<Res>> for SnoozyRef<Res> {
    fn from(v: &SnoozyRef<Res>) -> Self {
        *v
    }
}

impl<Res> Copy for SnoozyRef<Res> {}
impl<Res> Clone for SnoozyRef<Res> {
    fn clone(&self) -> Self {
        SnoozyRef {
            identity_hash: self.identity_hash,
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
            self.identity_hash
        )
    }
}

#[cfg(not(core_intrinsics))]
impl<Res> fmt::Debug for SnoozyRef<Res> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SnoozyRef {{identity_hash: {}}}", self.identity_hash)
    }
}

#[derive(Hash, Clone, Copy, Eq, PartialEq, Debug)]
pub struct OpaqueSnoozyRef {
    pub(crate) identity_hash: u64,
    pub(crate) type_id: TypeId,
}

impl Serialize for OpaqueSnoozyRef {
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

impl<Res: 'static> Into<OpaqueSnoozyRef> for SnoozyRef<Res> {
    fn into(self) -> OpaqueSnoozyRef {
        OpaqueSnoozyRef {
            identity_hash: self.identity_hash,
            type_id: TypeId::of::<Res>(),
        }
    }
}
