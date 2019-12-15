use crate::iface::{Context, ContextInner, EvalContext};
use crate::maybe_serialize::AnySerialize;
use crate::recipe_info::{RecipeBuildResult, RecipeDebugInfo, RecipeInfo};
use crate::refs::*;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::{atomic, Arc, Mutex, RwLock, Weak};

lazy_static! {
    pub(crate) static ref ASSET_REG: AssetReg = {
        AssetReg {
            refs: Default::default(),
            queued_asset_invalidations: Default::default(),
        }
    };
}

pub(crate) struct AssetReg {
    pub refs: RwLock<HashMap<OpaqueSnoozyAddr, Weak<OpaqueSnoozyRefInner>>>,
    pub queued_asset_invalidations: Arc<Mutex<Vec<OpaqueSnoozyRef>>>,
}

impl AssetReg {
    pub fn propagate_invalidations(&self, snapshot_idx: usize) {
        let mut queued = self.queued_asset_invalidations.lock().unwrap();

        for opaque_ref in queued.iter() {
            self.invalidate_asset_tree(opaque_ref, snapshot_idx);
        }

        queued.clear();
    }

    pub fn collect_garbage(&self) {
        self.refs
            .write()
            .unwrap()
            .retain(|_k, v| v.strong_count() > 0);
    }

    fn invalidate_asset_tree(&self, opaque_ref: &Arc<OpaqueSnoozyRefInner>, snapshot_idx: usize) {
        if opaque_ref
            .rebuild_pending
            .swap(snapshot_idx, atomic::Ordering::AcqRel)
            != snapshot_idx
        {
            let recipe_info = &opaque_ref.recipe_info;
            let recipe_info = recipe_info.read().unwrap();

            /*tracing::debug!(
                "invalidate asset tree of {}",
                recipe_info.recipe_meta.op_name
            );*/

            if let Some(ref build_record) = recipe_info.build_record {
                for dep in build_record
                    .reverse_dependencies
                    .iter()
                    .filter_map(|a| Some(Weak::upgrade(a)?))
                {
                    self.invalidate_asset_tree(&dep, snapshot_idx);
                }
            }
        }
    }

    fn cache_build_result(
        &self,
        opaque_ref: &OpaqueSnoozyAddr,
        asset: &(dyn Any + Send + Sync),
        recipe_info: &RecipeInfo,
    ) {
        let identity_hash = opaque_ref.identity_hash.0;
        let serialize_proxy = &*recipe_info.recipe_meta.serialize_proxy as &dyn AnySerialize;

        if !serialize_proxy.does_support_serialization() {
            panic!(
                "{} was marked with snoozy(cache), but does not support serialization",
                recipe_info.recipe_meta.result_type_name
            );
        }

        tracing::info!(
            "Caching {} on disk.",
            recipe_info.recipe_meta.result_type_name
        );

        let _ = std::fs::create_dir(".cache");

        let t0 = std::time::Instant::now();

        let mut data: Vec<u8> = Vec::new();
        serialize_proxy.try_serialize(asset, &mut data);

        tracing::info!(
            "Serialized into {} in {:?}",
            pretty_bytes::converter::convert(data.len() as f64),
            t0.elapsed()
        );

        let path = format!(".cache/{:x}.bin", identity_hash);
        let mut f = File::create(&path).expect("Unable to create file");
        if f.write_all(data.as_slice()).is_err() {
            let _ = std::fs::remove_file(&path);
        }
    }

    fn get_cached_build_result(
        &self,
        opaque_ref: &OpaqueSnoozyRef,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let identity_hash = opaque_ref.addr.identity_hash.0;
        let recipe_info = &opaque_ref.recipe_info;
        let recipe_info = recipe_info.read().unwrap();

        let serialize_proxy = &*recipe_info.recipe_meta.serialize_proxy as &dyn AnySerialize;

        if !serialize_proxy.does_support_serialization() {
            return None;
        }

        let t0 = std::time::Instant::now();

        let path = format!(".cache/{:x}.bin", identity_hash);

        let result = if let Ok(mut f) = File::open(&path) {
            let mut buffer = Vec::new();

            if f.read_to_end(&mut buffer).is_ok() {
                let result = serialize_proxy.try_deserialize(&mut buffer);
                tracing::info!(
                    "{} deserialized in {:?}",
                    recipe_info.recipe_meta.op_name,
                    t0.elapsed()
                );
                result
            } else {
                None
            }
        } else {
            None
        };

        result.map(Arc::from)
    }

    pub async fn evaluate_recipe(
        &self,
        opaque_ref: &OpaqueSnoozyRef,
        evaluation_path: HashSet<EvaluationPathNode>,
        eval_context: EvalContext,
    ) {
        /*tracing::debug!(
            "evaluate_recipe({}): {:?}",
            opaque_ref.recipe_info.read().unwrap().recipe_meta.op_name,
            opaque_ref.to_evaluation_path_node()
        );*/

        let evaluation_path_node = opaque_ref.to_evaluation_path_node();
        if evaluation_path.contains(&evaluation_path_node) {
            //tracing::debug!("Recipe already being evaluated");
            return;
        }

        /*tracing::debug!(
            "proceeding with eval. node: {:?}; path: {:?}",
            evaluation_path_node,
            evaluation_path
        );*/

        let rebuild_pending = opaque_ref.rebuild_pending.load(atomic::Ordering::Acquire) != 0;
        if rebuild_pending {
            let eval_lock = opaque_ref.being_evaluated.lock().await;
            if opaque_ref.rebuild_pending.load(atomic::Ordering::Acquire) != 0 {
                let recipe_runner = {
                    let recipe_info = &opaque_ref.recipe_info;
                    let recipe_info = recipe_info.read().unwrap();
                    recipe_info.recipe_runner.clone()
                };

                let mut evaluation_path = evaluation_path;
                evaluation_path.insert(opaque_ref.to_evaluation_path_node());

                let ctx = Context {
                    inner: Arc::new(ContextInner {
                        opaque_ref: opaque_ref.clone(),
                        dependencies: Mutex::new(HashSet::new()),
                        evaluation_path: Mutex::new(evaluation_path),
                        debug_info: Mutex::new(RecipeDebugInfo::default()),
                    }),
                    eval_context: eval_context.clone(),
                };

                /*tracing::debug!(
                    "Running {}",
                    opaque_ref.recipe_info.read().unwrap().recipe_meta.op_name
                );*/

                let (res_or_err, should_cache) = {
                    if let Some(cached) = { self.get_cached_build_result(&opaque_ref) } {
                        (Ok(cached), false)
                    } else {
                        let res = recipe_runner.run(ctx.clone()).await;
                        (res, recipe_runner.should_cache_result())
                    }
                };

                match res_or_err {
                    Ok(res) => {
                        let recipe_info = &opaque_ref.recipe_info;
                        let mut recipe_info = recipe_info.write().unwrap();

                        if should_cache {
                            self.cache_build_result(&opaque_ref.addr, &*res, &recipe_info);
                        }

                        let ctx = Arc::try_unwrap(ctx.inner)
                            .ok()
                            .expect("Could not unwrap Context. Reference retained by user.");

                        let res = RecipeBuildResult {
                            artifact: res,
                            debug_info: ctx.debug_info.into_inner().unwrap(),
                        };

                        let build_record_diff = recipe_info.set_new_build_result(
                            res,
                            ctx.dependencies.into_inner().unwrap(),
                            eval_context.snapshot_idx,
                        );

                        for dep in &build_record_diff.removed_deps {
                            // Don't keep circular dependencies
                            if ctx
                                .evaluation_path
                                .lock()
                                .unwrap()
                                .contains(&dep.0.to_evaluation_path_node())
                            {
                                continue;
                            }

                            let dep = &dep.0.recipe_info;
                            let mut dep = dep.write().unwrap();
                            let to_remove: *const OpaqueSnoozyRefInner = &***opaque_ref;

                            if let Some(ref mut build_record) = dep.build_record {
                                build_record.reverse_dependencies.retain(|r| {
                                    let r = r.as_raw();
                                    !r.is_null() && !std::ptr::eq(r, to_remove)
                                });
                            }
                        }

                        for dep in &build_record_diff.added_deps {
                            // Don't keep circular dependencies
                            if ctx
                                .evaluation_path
                                .lock()
                                .unwrap()
                                .contains(&dep.0.to_evaluation_path_node())
                            {
                                continue;
                            }

                            let dep = &dep.0.recipe_info;
                            let mut dep = dep.write().unwrap();
                            let to_add: *const OpaqueSnoozyRefInner = &***opaque_ref;

                            if let Some(ref mut build_record) = dep.build_record {
                                let exists = build_record
                                    .reverse_dependencies
                                    .iter()
                                    .any(|r| std::ptr::eq(r.as_raw(), to_add));

                                if !exists {
                                    build_record
                                        .reverse_dependencies
                                        .push(Arc::downgrade(opaque_ref));
                                }
                            }
                        }

                        opaque_ref
                            .rebuild_pending
                            .store(0, atomic::Ordering::Release);
                    }
                    Err(err) => {
                        let recipe_info = &opaque_ref.recipe_info;
                        let recipe_info = recipe_info.read().unwrap();

                        // Build failed, but unless anything changes, this is what we're stuck with
                        opaque_ref
                            .rebuild_pending
                            .store(0, atomic::Ordering::Release);

                        //println!("Error building asset {:?}: {}", opaque_ref, err);

                        println!(
                            "Error running op {}: {}",
                            recipe_info.recipe_meta.op_name, err
                        );
                    }
                }
            }

            drop(eval_lock);
        }
    }
}
