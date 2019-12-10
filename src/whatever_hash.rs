use serde::Serialize;
use std::hash::{Hash, Hasher};

macro_rules! declare_optional_hash {
    ($tname:ident, $fname:ident, $impl_for:ty,  [$($req_traits:tt),*], $code:expr) => {
        pub trait $tname {
            fn $fname<H: Hasher>(&self, state: &mut H) -> bool;
        }

        impl<T> $tname for T {
            default fn $fname<H: Hasher>(&self, _state: &mut H) -> bool {
                false
            }
        }

        impl<T> $tname for $impl_for
        where
            T: $($req_traits+)*,
        {
            fn $fname<H: Hasher>(&self, state: &mut H) -> bool {
                $code(self, state);
                true
            }
        }
    };
}

declare_optional_hash!(
    PodVecHash,
    pod_vec_hash,
    Vec<T>,
    [Copy, Sized],
    |item: &Vec<T>, state| unsafe {
        let p = item.as_ptr();
        let item_sizeof = std::mem::size_of::<T>();
        let len = item.len() * item_sizeof;
        std::slice::from_raw_parts(p as *const u8, len).hash(state);
    }
);

declare_optional_hash!(
    BoxedPodVecHash,
    boxed_pod_vec_hash,
    Box<Vec<T>>,
    [Copy, Sized],
    |item: &Box<Vec<T>>, state| unsafe {
        let p = item.as_ptr();
        let item_sizeof = std::mem::size_of::<T>();
        let len = item.len() * item_sizeof;
        std::slice::from_raw_parts(p as *const u8, len).hash(state);
    }
);

declare_optional_hash!(
    SerdeSerializedHash,
    serde_serialized_hash,
    T,
    [Serialize],
    |item: &T, state| {
        let t0 = std::time::Instant::now();
        let encoded: Vec<u8> = bincode::serialize(item).unwrap();
        encoded.hash(state);
        let elapsed = t0.elapsed();
        if elapsed > std::time::Duration::from_millis(100) {
            tracing::warn!(
                "Serde-hash of {} took {:?} :(",
                std::any::type_name::<T>(),
                elapsed
            );
        }
    }
);

declare_optional_hash!(
    PlainOldDataHash,
    plain_old_data_hash,
    T,
    [Copy],
    |item: &T, state| {
        let p = item as *const T as *const u8;
        unsafe {
            std::slice::from_raw_parts(p, std::mem::size_of::<T>()).hash(state);
        }
    }
);

declare_optional_hash!(
    BuiltInRustHash,
    built_in_rust_hash,
    T,
    [Hash],
    <T as Hash>::hash
);

pub fn whatever_hash<T: 'static, H: Hasher>(t: &T, state: &mut H) {
    let hashed = BuiltInRustHash::built_in_rust_hash(t, state)
        || PodVecHash::pod_vec_hash(t, state)
        || BoxedPodVecHash::boxed_pod_vec_hash(t, state)
        || PlainOldDataHash::plain_old_data_hash(t, state)
        || SerdeSerializedHash::serde_serialized_hash(t, state);

    if !hashed {
        panic!(
            "No appropriate hash function found for {}",
            std::intrinsics::type_name::<T>()
        );
    }
}
