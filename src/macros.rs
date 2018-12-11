#[macro_export]
macro_rules! snoozy {
    (fn $name:ident ($ctx:ident: &mut Context $(,$arg:ident : &$argtype:ty)*) -> Result<$ret:tt> $body:expr) => {
		pub mod $name {
			use snoozy::*;

			pub mod op_struct {
				use snoozy::*;
				use super::super::*;

				#[allow(non_camel_case_types)]
				#[derive(Serialize, Debug)]
				pub struct $name {
					$(pub $arg: $argtype),*
				}

				impl Op for $name {
					type Res = $ret;
					fn run(&self, ctx: &mut Context) -> Result<$ret> {
						super::op_impl::$name(ctx, $(&self.$arg),*)
					}
				}
			}

			pub mod op_impl {
				use snoozy::*;
				use super::super::*;

				#[allow(clippy::ptr_arg)]
				pub fn $name($ctx: &mut Context, $($arg: &$argtype),*) -> Result<$ret> {
					$body
				}
			}

			pub mod def_initial {
				use snoozy::*;
				use super::super::*;

				#[allow(dead_code)]
				pub fn $name(def_name: &str, $($arg: $argtype),*) -> SnoozyRef<$ret> {
					def_initial(calculate_hash(&def_name), $name::op_struct::$name { $($arg: $arg),* })
				}
			}

			pub mod def_named {
				use snoozy::*;
				use super::super::*;

				#[allow(dead_code)]
				pub fn $name(identity_hash: u64, $($arg: $argtype),*) {
					def_named(identity_hash, $name::op_struct::$name { $($arg: $arg),* });
				}
			}
		}

		#[allow(dead_code)]
		pub fn $name($($arg: $argtype),*) -> SnoozyRef<$ret> {
			lazy_static! {
				static ref op_source_hash: u64 = calculate_hash(&stringify!($body));
			}

			use snoozy::*;
			def($name::op_struct::$name { $($arg: $arg),* }, *op_source_hash)
		}
	}
}

#[macro_export]
macro_rules! init_named {
    ($def_name:tt, $name:ident ($($arg:expr),*)) => {
		$name::def_initial::$name($def_name, $($arg),*)
	}
}

#[macro_export]
macro_rules! redef_named {
    ($asset_ref:ident, $name:ident ($($arg:expr),*)) => {
		$name::def_named::$name($asset_ref.identity_hash, $($arg),*)
	}
}
