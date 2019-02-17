use super::iface::Context;
use super::maybe_serialize::{AnySerialize, AnySerializeProxy, MaybeSerialize};
use super::refs::OpaqueSnoozyRef;
use super::Result;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, RwLock};

pub struct RecipeBuildRecord {
    pub last_valid_build_result: Arc<dyn Any + Send + Sync>,
    // Assets this one requested during the last build
    pub dependencies: HashSet<OpaqueSnoozyRef>,
    // Assets that requested this asset during their builds
    pub reverse_dependencies: HashSet<OpaqueSnoozyRef>,
}

pub(crate) struct RecipeMeta {
    op_name: &'static str,
    result_type_name: &'static str,
    serialize_proxy: Box<dyn AnySerialize>,
}

impl RecipeMeta {
    pub fn new<'a, T>(op_name: &'static str) -> Self
    where
        T: 'static + Send + Sync + MaybeSerialize,
    {
        let serialize_proxy: Box<AnySerializeProxy<T>> = Box::new(AnySerializeProxy::new());
        let serialize_proxy = serialize_proxy as Box<dyn AnySerialize>;

        Self {
            result_type_name: unsafe { std::intrinsics::type_name::<T>() },
            op_name: op_name,
            serialize_proxy,
        }
    }
}

pub(crate) struct RecipeInfo {
    pub recipe_runner: Arc<(Fn(&mut Context) -> Result<Arc<Any + Send + Sync>>) + Send + Sync>,
    pub recipe_meta: RecipeMeta,
    //pub any_serialize_proxy: Box<dyn AnySerialize<VecByteReader, VecByteWriter>>,
    pub recipe_hash: u64,

    pub rebuild_pending: bool,
    pub build_record: Option<RecipeBuildRecord>,
}

struct BuildRecordDiff {
    added_deps: Vec<OpaqueSnoozyRef>,
    removed_deps: Vec<OpaqueSnoozyRef>,
}

impl RecipeInfo {
    fn set_new_build_result(
        &mut self,
        build_result: Arc<dyn Any + Send + Sync>,
        dependencies: HashSet<OpaqueSnoozyRef>,
    ) -> BuildRecordDiff {
        let (prev_deps, prev_rev_deps) = self
            .build_record
            .take()
            .map(|r| (r.dependencies, r.reverse_dependencies))
            .unwrap_or_default();

        let added_deps = dependencies.difference(&prev_deps).cloned().collect();
        let removed_deps = prev_deps.difference(&dependencies).cloned().collect();

        self.build_record = Some(RecipeBuildRecord {
            last_valid_build_result: build_result,
            dependencies,
            // Keep the previous reverse dependencies. Until the dependent assets get rebuild,
            // we must assume they still depend on the current asset.
            reverse_dependencies: prev_rev_deps,
        });

        BuildRecordDiff {
            added_deps,
            removed_deps,
        }
    }
}

lazy_static! {
    pub(crate) static ref ASSET_REG: AssetReg = {
        AssetReg {
            recipe_info: Mutex::new(HashMap::new()),
            queued_asset_invalidations: Default::default(),
            being_evaluated: Default::default(),
        }
    };
}

pub(crate) struct AssetReg {
    being_evaluated: Mutex<HashSet<OpaqueSnoozyRef>>,
    pub recipe_info: Mutex<HashMap<OpaqueSnoozyRef, Arc<RwLock<RecipeInfo>>>>,
    pub queued_asset_invalidations: Arc<Mutex<Vec<OpaqueSnoozyRef>>>,
}

impl AssetReg {
    pub fn propagate_invalidations(&self) {
        let mut queued = self.queued_asset_invalidations.lock().unwrap();

        for opaque_ref in queued.iter() {
            self.invalidate_asset_tree(*opaque_ref);
        }

        queued.clear();
    }

    pub fn get_recipe_info_for_ref(&self, opaque_ref: OpaqueSnoozyRef) -> Arc<RwLock<RecipeInfo>> {
        let mut recipe_info_map = self.recipe_info.lock().unwrap();
        recipe_info_map.get_mut(&opaque_ref).unwrap().clone()
    }

    fn invalidate_asset_tree(&self, opaque_ref: OpaqueSnoozyRef) {
        let recipe_info = self.get_recipe_info_for_ref(opaque_ref);

        let mut recipe_info = recipe_info.write().unwrap();
        recipe_info.rebuild_pending = true;

        if let Some(ref build_record) = recipe_info.build_record {
            for dep in build_record.reverse_dependencies.iter() {
                self.invalidate_asset_tree(*dep);
            }
        }
    }

    fn cache_build_result(
        &self,
        opaque_ref: OpaqueSnoozyRef,
        asset: &(dyn Any + Send + Sync),
        recipe_info: &RecipeInfo,
    ) {
        let serialize_proxy = &*recipe_info.recipe_meta.serialize_proxy as &dyn AnySerialize;

        if !serialize_proxy.does_support_serialization() {
            return;
        }

        println!(
            "The result ({}) supports serialization, and we'll cache it.",
            recipe_info.recipe_meta.result_type_name
        );

        let _ = std::fs::create_dir(".cache");

        let t0 = std::time::Instant::now();

        let mut data: Vec<u8> = Vec::new();
        serialize_proxy.try_serialize(asset, &mut data);

        println!(
            "Serialized into {} in {:?}",
            pretty_bytes::converter::convert(data.len() as f64),
            t0.elapsed()
        );

        let path = format!(".cache/{:x}.bin", opaque_ref.identity_hash);
        let mut f = File::create(&path).expect("Unable to create file");
        if !f.write_all(data.as_slice()).is_ok() {
            let _ = std::fs::remove_file(&path);
        }
    }

    fn get_cached_build_result(
        &self,
        opaque_ref: OpaqueSnoozyRef,
        recipe_info: &RecipeInfo,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let serialize_proxy = &*recipe_info.recipe_meta.serialize_proxy as &dyn AnySerialize;

        if !serialize_proxy.does_support_serialization() {
            return None;
        }

        let t0 = std::time::Instant::now();

        let path = format!(".cache/{:x}.bin", opaque_ref.identity_hash);

        let result = if let Ok(mut f) = File::open(&path) {
            let mut buffer = Vec::new();

            if f.read_to_end(&mut buffer).is_ok() {
                let result = serialize_proxy.try_deserialize(&mut buffer);
                println!("Deserialized in {:?}", t0.elapsed());
                result
            } else {
                None
            }
        } else {
            None
        };

        result.map(|x| Arc::from(x))
    }

    pub fn evaluate_recipe(&self, opaque_ref: OpaqueSnoozyRef) {
        //println!("evaluate recipe {:?}", opaque_ref);

        if !self.being_evaluated.lock().unwrap().insert(opaque_ref) {
            //println!("recipe already being evaluated");
            return;
        }

        self.being_evaluated.lock().unwrap().insert(opaque_ref);

        let (rebuild_pending, recipe_runner) = {
            let recipe_info = self.get_recipe_info_for_ref(opaque_ref);
            let recipe_info = recipe_info.write().unwrap();

            (
                recipe_info.rebuild_pending,
                recipe_info.recipe_runner.clone(),
            )
        };

        if rebuild_pending {
            let mut ctx = Context {
                opaque_ref,
                dependencies: HashSet::new(),
            };

            //println!("Running {}", recipe_info.recipe_meta.op_name);

            let (res_or_err, should_cache) = {
                if let Some(cached) = {
                    let recipe_info = self.get_recipe_info_for_ref(opaque_ref);
                    let recipe_info = recipe_info.write().unwrap();
                    self.get_cached_build_result(opaque_ref, &recipe_info)
                } {
                    (Ok(cached), false)
                } else {
                    let t0 = std::time::Instant::now();
                    let res = (recipe_runner)(&mut ctx);
                    let build_duration = t0.elapsed();

                    let should_cache = build_duration > std::time::Duration::from_millis(100);
                    if should_cache {
                        let recipe_info = self.get_recipe_info_for_ref(opaque_ref);
                        let recipe_info = recipe_info.write().unwrap();
                        println!(
                            "{} took {:?}",
                            recipe_info.recipe_meta.op_name, build_duration
                        );
                    }

                    (res, should_cache)
                }
            };

            match res_or_err {
                Ok(res) => {
                    let recipe_info = self.get_recipe_info_for_ref(opaque_ref);
                    let mut recipe_info = recipe_info.write().unwrap();

                    if should_cache {
                        self.cache_build_result(opaque_ref, &*res, &recipe_info);
                    }

                    recipe_info.rebuild_pending = false;
                    let build_record_diff = recipe_info.set_new_build_result(res, ctx.dependencies);

                    for dep in &build_record_diff.removed_deps {
                        // Don't keep circular dependencies
                        if self.being_evaluated.lock().unwrap().contains(&dep) {
                            continue;
                        }

                        let dep = self.get_recipe_info_for_ref(*dep);
                        let mut dep = dep.write().unwrap();
                        if let Some(ref mut build_record) = dep.build_record {
                            build_record.reverse_dependencies.remove(&opaque_ref);
                        }
                    }

                    for dep in &build_record_diff.added_deps {
                        // Don't keep circular dependencies
                        if self.being_evaluated.lock().unwrap().contains(&dep) {
                            continue;
                        }

                        let dep = self.get_recipe_info_for_ref(*dep);
                        let mut dep = dep.write().unwrap();
                        if let Some(ref mut build_record) = dep.build_record {
                            build_record.reverse_dependencies.insert(opaque_ref);
                        }
                    }
                }
                Err(err) => {
                    let recipe_info = self.get_recipe_info_for_ref(opaque_ref);
                    let mut recipe_info = recipe_info.write().unwrap();

                    // Build failed, but unless anything changes, this is what we're stuck with
                    recipe_info.rebuild_pending = false;

                    //println!("Error building asset {:?}: {}", opaque_ref, err);
                    println!("Error building asset: {}", err);
                }
            }
        }

        self.being_evaluated.lock().unwrap().remove(&opaque_ref);
    }
}
