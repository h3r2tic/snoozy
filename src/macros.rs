#[macro_export]
macro_rules! init_dynamic {
    ($name:ident ($($arg:expr),*)) => {
        {
            let location = (file!(), line!(), column!());
            let location_hash = {
                let mut s = DefaultSnoozyHash::default();
                std::hash::Hash::hash(&location, &mut s);
                std::hash::Hasher::finish(&s)
            };

            $name::def_named_initial(location_hash, $name::new($($arg),*))
        }
    }
}

#[macro_export]
macro_rules! redef_dynamic {
    ($asset_ref:ident, $name:ident ($($arg:expr),*)) => {
		$name::redef_named($asset_ref.identity_hash, $name::new($($arg),*))
	}
}
