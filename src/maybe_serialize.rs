use std::any::Any;
use std::marker::PhantomData;

pub trait MaybeSerialize<'a>: Sized {
    fn does_support_serialization() -> bool;
    fn try_serialize<S: serde::Serializer>(&self, s: S) -> bool;
    fn try_deserialize<S: serde::Deserializer<'a>>(s: S) -> Option<Self>;
}

impl<'a, T> MaybeSerialize<'a> for T
where
    T: Sized,
{
    default fn does_support_serialization() -> bool {
        false
    }
    default fn try_serialize<S: serde::Serializer>(&self, _: S) -> bool {
        panic!();
    }
    default fn try_deserialize<S: serde::Deserializer<'a>>(_: S) -> Option<Self> {
        panic!();
    }
}

impl<'a, T> MaybeSerialize<'a> for T
where
    T: serde::Serialize + serde::Deserialize<'a> + 'a,
{
    fn does_support_serialization() -> bool {
        true
    }
    default fn try_serialize<S: serde::Serializer>(&self, s: S) -> bool {
        <Self as serde::Serialize>::serialize(self, s).is_ok()
    }
    default fn try_deserialize<S: serde::Deserializer<'a>>(s: S) -> Option<Self> {
        <Self as serde::Deserialize>::deserialize(s).ok()
    }
}

pub trait AnySerialize<R: std::io::Read, W: std::io::Write>: Send + Sync {
    fn does_support_serialization(&self) -> bool;
    fn try_serialize(&self, obj: &dyn Any, w: &mut W) -> bool;
    fn try_deserialize(&self, s: &mut R) -> Option<Box<dyn Any>>;
}

pub struct AnySerializeProxy<T: Any + Sized> {
    phantom: PhantomData<fn(T)>,
}

impl<'a, T: Any + MaybeSerialize<'a> + Sized> AnySerializeProxy<T> {
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<'a, T, R: std::io::Read, W: std::io::Write> AnySerialize<R, W> for AnySerializeProxy<T>
where
    T: Any + MaybeSerialize<'a> + Sized,
{
    fn does_support_serialization(&self) -> bool {
        <T as MaybeSerialize>::does_support_serialization()
    }

    /*fn try_serialize(&self, obj: &dyn Any, s: S) -> bool {
        if let Some(ref obj) = obj.downcast_ref::<T>() {
            <T as MaybeSerialize>::try_serialize(obj, s)
        } else {
            false
        }
    }

    fn try_deserialize(&self, s: D) -> Option<Box<dyn Any>> {
        if let Some(res) = <T as MaybeSerialize>::try_deserialize(s) {
            Some(Box::new(res))
        } else {
            None
        }
    }*/

    fn try_serialize(&self, obj: &dyn Any, w: &mut W) -> bool {
        if let Some(ref obj) = obj.downcast_ref::<T>() {
            <T as MaybeSerialize>::try_serialize(obj, &mut serde_cbor::Serializer::new(w))
        } else {
            false
        }
    }
    fn try_deserialize(&self, s: &mut R) -> Option<Box<dyn Any>> {
        if let Some(res) =
            <T as MaybeSerialize>::try_deserialize(&mut serde_cbor::Deserializer::from_reader(s))
        {
            Some(Box::new(res))
        } else {
            None
        }
    }
}
