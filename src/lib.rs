#![feature(specialization)]
#![feature(core_intrinsics)]

mod asset_reg;
mod cycle_detector;
mod iface;
mod maybe_serialize;
mod recipe_info;
mod refs;
mod whatever_hash;

pub use futures;
pub use iface::*;
pub use recipe_info::SnoozyRefDependency;
pub use refs::{OpaqueSnoozyRef, OpaqueSnoozyRefInner, SnoozyRef};
pub use whatever_hash::whatever_hash;

#[allow(unused_imports)]
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
use std::sync::{Arc, Mutex};

pub use failure::{err_msg, Error};
pub use twox_hash::XxHash as DefaultSnoozyHash;
pub type Result<T> = std::result::Result<T, Error>;

pub fn get_type_hash<T: 'static>() -> u64 {
    let mut s = DefaultSnoozyHash::default();
    std::any::TypeId::of::<T>().hash(&mut s);
    s.finish()
}

pub(crate) static mut RUNTIME: Option<Arc<Mutex<tokio::runtime::Runtime>>> = None;

pub fn initialize_runtime(runtime: Arc<Mutex<tokio::runtime::Runtime>>) {
    unsafe {
        RUNTIME = Some(runtime);
    }
}
