use super::iface::Context;
use super::refs::OpaqueSnoozyRef;
use super::Result;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

pub struct RecipeBuildRecord {
    pub last_valid_build_result: Arc<dyn Any + Send + Sync>,
    // Assets this one requested during the last build
    pub dependencies: HashSet<OpaqueSnoozyRef>,
    // Assets that requested this asset during their builds
    pub reverse_dependencies: HashSet<OpaqueSnoozyRef>,
}

pub struct RecipeInfo {
    pub recipe_runner: Arc<(Fn(&mut Context) -> Result<Arc<Any + Send + Sync>>) + Send + Sync>,
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
    pub static ref ASSET_REG: AssetReg = {
        AssetReg {
            recipe_info: Mutex::new(HashMap::new()),
            queued_asset_invalidations: Default::default(),
            being_evaluated: Default::default(),
        }
    };
}

pub struct AssetReg {
    pub being_evaluated: Mutex<HashMap<OpaqueSnoozyRef, bool>>,
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

    pub fn evaluate_recipe(&self, opaque_ref: OpaqueSnoozyRef) {
        //println!("evaluate recipe {:?}", opaque_ref);

        if self.being_evaluated.lock().unwrap()[&opaque_ref] {
            //println!("recipe already being evaluated");
            return;
        }

        self.being_evaluated
            .lock()
            .unwrap()
            .insert(opaque_ref, true);

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

            let res_or_err = (recipe_runner)(&mut ctx);

            let recipe_info = self.get_recipe_info_for_ref(opaque_ref);
            let mut recipe_info = recipe_info.write().unwrap();

            match res_or_err {
                Ok(res) => {
                    recipe_info.rebuild_pending = false;
                    let build_record_diff = recipe_info.set_new_build_result(res, ctx.dependencies);

                    for dep in &build_record_diff.removed_deps {
                        // Don't keep circular dependencies
                        if self.being_evaluated.lock().unwrap()[&dep] {
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
                        if self.being_evaluated.lock().unwrap()[&dep] {
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
                    // Build failed, but unless anything changes, this is what we're stuck with
                    recipe_info.rebuild_pending = false;

                    //println!("Error building asset {:?}: {}", opaque_ref, err);
                    println!("Error building asset: {}", err);
                }
            }
        }

        self.being_evaluated
            .lock()
            .unwrap()
            .insert(opaque_ref, false);
    }
}
