#![feature(specialization)]
#![feature(core_intrinsics)]

mod asset_reg;
mod iface;
mod maybe_serialize;
mod refs;
mod whatever_hash;

#[macro_use]
mod macros;

pub use iface::*;
pub use refs::{OpaqueSnoozyRef, SnoozyRef};
pub use whatever_hash::whatever_hash;

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate failure;
extern crate bincode;
extern crate serde;
#[macro_use]
extern crate lazy_static;

use std::default::Default;
use std::hash::{Hash, Hasher};

pub use failure::{err_msg, Error};
pub use twox_hash::XxHash as DefaultSnoozyHash;
pub type Result<T> = std::result::Result<T, Error>;

pub fn get_type_hash<T: 'static>() -> u64 {
    let mut s = DefaultSnoozyHash::default();
    std::any::TypeId::of::<T>().hash(&mut s);
    s.finish()
}

pub trait DerpySerialization: Sized {
    fn derpy_serialize(&self, s: &mut Vec<u8>) -> bool;
    fn derpy_deserialize(s: &[u8]) -> Option<Self>;
}
