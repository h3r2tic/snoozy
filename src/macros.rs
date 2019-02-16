#[macro_export]
macro_rules! init_dynamic {
    ($name:ident ($($arg:expr),*)) => {
        $name::def_named_initial(calculate_hash(&(file!(), line!(), column!())), $name::new($($arg),*))
    }
}

#[macro_export]
macro_rules! redef_dynamic {
    ($asset_ref:ident, $name:ident ($($arg:expr),*)) => {
		$name::redef_named($asset_ref.identity_hash, $name::new($($arg),*))
	}
}
