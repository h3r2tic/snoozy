use super::iface::Context;
use super::refs::OpaqueSnoozyRef;
use super::Result;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::sync::{Arc, Mutex, RwLock};

pub struct RecipeInfo {
    pub recipe_runner: Arc<(Fn(&mut Context) -> Result<Arc<Any + Send + Sync>>) + Send + Sync>,
    pub last_valid_build_result: Option<Arc<Any + Send + Sync>>,
    pub recipe_hash: u64,
    pub rebuild_pending: bool,
    // Assets this one requested during the last build
    pub dependencies: Vec<OpaqueSnoozyRef>,
    // Assets that requested this asset during their builds
    pub reverse_dependencies: HashSet<OpaqueSnoozyRef>,
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

    fn invalidate_asset_tree(&self, opaque_ref: OpaqueSnoozyRef) {
        //println!("invalidating {:?}", opaque_ref);

        let build_record_arc: Arc<_> = {
            let mut recipe_info_map = self.recipe_info.lock().unwrap();
            recipe_info_map.get_mut(&opaque_ref).unwrap().clone()
        };

        let mut rec = build_record_arc.write().unwrap();

        {
            rec.rebuild_pending = true;

            for dep in rec.reverse_dependencies.iter() {
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
            let recipe_info_lock = self
                .recipe_info
                .lock()
                .unwrap()
                .get(&opaque_ref)
                .unwrap()
                .clone();
            let recipe_info = recipe_info_lock.write().unwrap();

            (
                recipe_info.rebuild_pending,
                recipe_info.recipe_runner.clone(),
            )
        };

        if rebuild_pending {
            let mut ctx = Context {
                opaque_ref,
                dependencies: Vec::new(),
            };

            let res_or_err = (recipe_runner)(&mut ctx);

            let recipe_info_lock = self
                .recipe_info
                .lock()
                .unwrap()
                .get(&opaque_ref)
                .unwrap()
                .clone();
            let mut recipe_info = recipe_info_lock.write().unwrap();

            match res_or_err {
                Ok(res) => {
                    recipe_info.last_valid_build_result = Some(res);

                    //println!("Published build result for asset {:?}", opaque_ref);

                    let previous_dependencies: HashSet<OpaqueSnoozyRef> =
                        HashSet::from_iter(recipe_info.dependencies.iter().cloned());
                    let current_dependencies: HashSet<OpaqueSnoozyRef> =
                        HashSet::from_iter(ctx.dependencies.iter().cloned());

                    for dep in previous_dependencies.difference(&current_dependencies) {
                        // Don't keep circular dependencies
                        if self.being_evaluated.lock().unwrap()[&dep] {
                            continue;
                        }

                        let dep_recipe_info_lock =
                            self.recipe_info.lock().unwrap().get(&dep).unwrap().clone();
                        let mut dep_recipe_info = dep_recipe_info_lock.write().unwrap();

                        //println!("Unregistering dependency on {:?}", dep);

                        dep_recipe_info.reverse_dependencies.remove(&opaque_ref);
                    }

                    for dep in current_dependencies.difference(&previous_dependencies) {
                        // Don't keep circular dependencies
                        if self.being_evaluated.lock().unwrap()[&dep] {
                            continue;
                        }

                        let dep_recipe_info_lock =
                            self.recipe_info.lock().unwrap().get(&dep).unwrap().clone();
                        let mut dep_recipe_info = dep_recipe_info_lock.write().unwrap();

                        //println!("Registering dependency on {:?}", dep);

                        dep_recipe_info.reverse_dependencies.insert(opaque_ref);
                    }

                    recipe_info.rebuild_pending = false;
                    recipe_info.dependencies = ctx.dependencies;
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
