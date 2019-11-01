use std::any::Any;
use std::marker::PhantomData;

pub trait MaybeSerialize: Sized {
    fn does_support_serialization() -> bool;
    fn try_serialize(&self, s: &mut Vec<u8>) -> bool;
    fn try_deserialize(s: &mut [u8]) -> Option<Self>;
}

impl<T> MaybeSerialize for T
where
    T: Sized,
{
    default fn does_support_serialization() -> bool {
        false
    }
    default fn try_serialize(&self, _s: &mut Vec<u8>) -> bool {
        panic!();
    }
    default fn try_deserialize(_s: &mut [u8]) -> Option<Self> {
        panic!();
    }
}

/*impl<T> MaybeSerialize for T
where
    T: serde::Serialize + for<'a> serde::Deserialize<'a>,
{
    fn does_support_serialization() -> bool {
        true
    }
    fn try_serialize(&self, s: &mut Vec<u8>) -> bool {
        bincode::serialize_into(s, self).is_ok()
    }
    fn try_deserialize<'a>(s: &'a mut [u8]) -> Option<Self> {
        bincode::deserialize(s).ok()
    }
}*/

/*impl<T> MaybeSerialize for T
where
    T: for<'a> speedy::Readable<'a, speedy::Endianness>
        + for<'b> speedy::Writable<speedy::Endianness>,
{
    fn does_support_serialization() -> bool {
        true
    }
    fn try_serialize(&self, s: &mut Vec<u8>) -> bool {
        //<Self as serde::Serialize>::serialize(self, &mut serde_cbor::Serializer::new(s)).is_ok()
        let endian = speedy::Endianness::LittleEndian;
        <Self as speedy::Writable<_>>::write_to_stream(self, endian, s).is_ok()
        //bincode::serialize_into(s, self).is_ok()
    }
    fn try_deserialize<'a>(s: &'a mut [u8]) -> Option<Self> {
        /*<Self as serde::Deserialize>::deserialize(&mut serde_cbor::Deserializer::from_reader(s))
        .ok()*/
//bincode::deserialize(s).ok()
let endian = speedy::Endianness::LittleEndian;
<Self as speedy::Readable<_>>::read_from_buffer(endian, s).ok()
}
}
*/
impl<T> MaybeSerialize for T
where
    T: abomonation::Abomonation + Clone,
{
    fn does_support_serialization() -> bool {
        true
    }
    fn try_serialize(&self, s: &mut Vec<u8>) -> bool {
        unsafe { abomonation::encode(self, s).is_ok() }
    }
    fn try_deserialize(s: &mut [u8]) -> Option<Self> {
        if let Some((result, _)) = unsafe { abomonation::decode::<Self>(s) } {
            Some((*result).clone())
        } else {
            None
        }
    }
}

/*impl<T> MaybeSerialize for T
where
    T: DerpySerialization,
{
    fn does_support_serialization() -> bool {
        true
    }
    fn try_serialize(&self, s: &mut Vec<u8>) -> bool {
        self.derpy_serialize(s)
    }
    fn try_deserialize<'a>(s: &'a mut [u8]) -> Option<Self> {
        Self::derpy_deserialize(s)
    }
}*/

pub trait AnySerialize: Send + Sync {
    fn does_support_serialization(&self) -> bool;
    fn try_serialize(&self, obj: &(dyn Any + Send + Sync), w: &mut Vec<u8>) -> bool;
    fn try_deserialize<'a>(&self, s: &'a mut [u8]) -> Option<Box<dyn Any + Send + Sync>>;
    fn clone_boxed(&self) -> Box<dyn AnySerialize>;
}

pub struct AnySerializeProxy<T: Any + Sized> {
    phantom: PhantomData<fn(T)>,
}

impl<'a, T: Any + MaybeSerialize + Sized> AnySerializeProxy<T> {
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<T> AnySerialize for AnySerializeProxy<T>
where
    T: Any + Send + Sync + MaybeSerialize + Sized,
{
    fn does_support_serialization(&self) -> bool {
        <T as MaybeSerialize>::does_support_serialization()
    }

    fn try_serialize(&self, obj: &(dyn Any + Send + Sync), w: &mut Vec<u8>) -> bool {
        if let Some(ref obj) = obj.downcast_ref::<T>() {
            <T as MaybeSerialize>::try_serialize(obj, w)
        } else {
            false
        }
    }

    fn try_deserialize<'a>(&self, s: &'a mut [u8]) -> Option<Box<dyn Any + Send + Sync>> {
        if let Some(res) = <T as MaybeSerialize>::try_deserialize(s) {
            Some(Box::new(res))
        } else {
            None
        }
    }

    fn clone_boxed(&self) -> Box<dyn AnySerialize> {
        Box::new(Self {
            phantom: PhantomData,
        })
    }
}
