use super::asset_reg::{RecipeInfo, ASSET_REG};
use super::refs::{OpaqueSnoozyRef, SnoozyRef};
use super::{DefaultSnoozyHash, Result};
use std::any::Any;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock};

pub struct Context {
    pub(crate) opaque_ref: OpaqueSnoozyRef, // Handle for the asset that this Context was created for
    pub(crate) dependencies: Vec<OpaqueSnoozyRef>,
}

impl Context {
    pub fn get_invalidation_trigger(&self) -> impl Fn() {
        let queued_assets = ASSET_REG.queued_asset_invalidations.clone();
        let opaque_ref = self.opaque_ref;
        move || {
            queued_assets.lock().unwrap().push(opaque_ref);
        }
    }

    pub fn get<Res: 'static + Send + Sync, SnoozyT: Into<SnoozyRef<Res>>>(
        &mut self,
        asset_ref: SnoozyT,
    ) -> Result<Arc<Res>> {
        let asset_ref = asset_ref.into();
        let opaque_ref: OpaqueSnoozyRef = asset_ref.into();

        self.dependencies.push(opaque_ref);
        ASSET_REG.evaluate_recipe(opaque_ref);

        let recipe_info_lock = ASSET_REG
            .recipe_info
            .lock()
            .unwrap()
            .get(&opaque_ref)
            .unwrap()
            .clone();
        let recipe_info = recipe_info_lock.read().unwrap();

        match recipe_info.last_valid_build_result {
            Some(ref res) => Ok(res.clone().downcast::<Res>().unwrap()),
            None => Err(format_err!(
                "Requested asset {:?} failed to build",
                opaque_ref
            )),
        }
    }
}

pub trait Op: Send + 'static {
    type Res;

    fn run(&self, ctx: &mut Context) -> Result<Self::Res>;
}

pub trait SnoozyNamedOp {
    type Res;
    fn def_named_initial(identity_hash: u64, init_value: Self) -> SnoozyRef<Self::Res>;
    fn redef_named(identity_hash: u64, new_value: Self);
}

pub fn def<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + Hash>(
    op: OpType,
) -> SnoozyRef<AssetType> {
    let mut s = DefaultSnoozyHash::default();
    <OpType as std::hash::Hash>::hash(&op, &mut s);
    def_named(s.finish(), op)
}

pub fn def_named<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + Hash>(
    identity_hash: u64,
    op: OpType,
) -> SnoozyRef<AssetType> {
    let mut s = DefaultSnoozyHash::default();
    <OpType as std::hash::Hash>::hash(&op, &mut s);
    let recipe_hash = s.finish();

    let res = SnoozyRef::new(identity_hash);

    let make_recipe_runner = || -> Arc<(Fn(&mut Context) -> _) + Send + Sync> {
        let op_mutex = Mutex::new(op);
        Arc::new(move |mut ctx| -> Result<Arc<Any + Send + Sync>> {
            //println!("Running recipe {:?} ({})", &*op_mutex.lock().unwrap(), identity_hash);
            let build_result = op_mutex.lock().unwrap().run(&mut ctx)?;
            Ok(Arc::new(build_result))
        })
    };

    {
        let mut recipe_info = ASSET_REG.recipe_info.lock().unwrap();

        match recipe_info.get(&res.into()) {
            // Definition doesn't exist. Create it
            None => {
                recipe_info.insert(
                    res.into(),
                    Arc::new(RwLock::new(RecipeInfo {
                        recipe_runner: make_recipe_runner(),
                        last_valid_build_result: None,
                        rebuild_pending: true,
                        dependencies: Vec::new(),
                        reverse_dependencies: HashSet::new(),
                        recipe_hash,
                    })),
                );
            }
            // Definition exists. If the hash is the same, don't do anything
            Some(entry) => {
                let mut entry = entry.write().unwrap();
                if entry.recipe_hash != recipe_hash {
                    // Hash differs. Update the definition, but keep the last build result
                    entry.recipe_runner = make_recipe_runner();
                    entry.rebuild_pending = false;
                    entry.recipe_hash = recipe_hash;

                    ASSET_REG
                        .queued_asset_invalidations
                        .lock()
                        .unwrap()
                        .push(res.into());
                }
            }
        }
    }

    ASSET_REG
        .being_evaluated
        .lock()
        .unwrap()
        .entry(res.into())
        .or_insert(false);

    res
}

pub fn def_initial<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + Hash>(
    identity_hash: u64,
    op: OpType,
) -> SnoozyRef<AssetType> {
    let res = def_named(identity_hash, op);
    ASSET_REG.evaluate_recipe(res.into());
    res
}

pub struct Snapshot;
impl Snapshot {
    pub fn get<Res: 'static + Send + Sync>(&self, asset_ref: SnoozyRef<Res>) -> Arc<Res> {
        let opaque_ref: OpaqueSnoozyRef = asset_ref.into();

        ASSET_REG.evaluate_recipe(opaque_ref);

        let recipe_info_lock = ASSET_REG
            .recipe_info
            .lock()
            .unwrap()
            .get(&opaque_ref)
            .unwrap()
            .clone();
        let recipe_info = recipe_info_lock.read().unwrap();

        match recipe_info.last_valid_build_result {
            Some(ref res) => res.clone().downcast::<Res>().unwrap(),
            None => panic!("Requested asset {:?} failed to build", opaque_ref),
        }
    }
}

pub fn with_snapshot<F, Ret>(callback: F) -> Ret
where
    F: FnOnce(&mut Snapshot) -> Ret,
{
    ASSET_REG.propagate_invalidations();
    let mut snap = Snapshot;
    callback(&mut snap)
}
