#[macro_export]
macro_rules! snoozy {
    (fn $name:ident ($ctx:ident: &mut Context $(,$arg:ident : &$argtype:ty)*) -> Result<$ret:ty> $body:expr) => {
		pub mod $name {
			use snoozy::*;

			pub mod op_struct {
				use snoozy::*;
				use super::super::*;

				#[allow(non_camel_case_types)]
				pub struct $name {
					$(pub $arg: $argtype),*
				}

                impl RecipeHash for $name {
                    fn recipe_hash(&self) -> u64 {
                        0 $(^calculate_serialized_hash(&self.$arg))*
                    }
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
            static mut OP_SOURCE_HASH: u64 = 0;
            let op_source_hash = unsafe {
                if 0 == OP_SOURCE_HASH {
                    OP_SOURCE_HASH = calculate_hash(&stringify!($body));
                }

                OP_SOURCE_HASH
            };

			use snoozy::*;
			def($name::op_struct::$name { $($arg: $arg),* }, op_source_hash)
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
