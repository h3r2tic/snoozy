use super::asset_reg::{RecipeBuildRecord, RecipeInfo, RecipeMeta, ASSET_REG};
use super::maybe_serialize::MaybeSerialize;
use super::refs::{OpaqueSnoozyAddr, OpaqueSnoozyRef, OpaqueSnoozyRefInner, SnoozyRef};
use super::{DefaultSnoozyHash, Result};
use std::any::Any;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};

pub struct Context {
    pub(crate) opaque_ref: OpaqueSnoozyRef, // Handle for the asset that this Context was created for
    pub(crate) dependencies: HashSet<OpaqueSnoozyRef>,
}

fn get_pinned_result<T>(p: Arc<dyn Any + Send + Sync>) -> Pin<Arc<T>>
where
    T: Any + Send + Sync + 'static,
{
    let p: Arc<T> = p.downcast::<T>().unwrap();
    unsafe { Pin::new_unchecked(p) }
}

impl Context {
    pub fn get_invalidation_trigger(&self) -> impl Fn() {
        let queued_assets = ASSET_REG.queued_asset_invalidations.clone();
        let opaque_ref = self.opaque_ref.clone();
        move || {
            queued_assets.lock().unwrap().push(opaque_ref.clone());
        }
    }

    pub fn get<Res: 'static + Send + Sync, SnoozyT: Into<SnoozyRef<Res>>>(
        &mut self,
        asset_ref: SnoozyT,
    ) -> Result<Pin<Arc<Res>>> {
        let asset_ref = asset_ref.into();
        let opaque_ref: OpaqueSnoozyRef = asset_ref.opaque;

        self.dependencies.insert(opaque_ref.clone());
        ASSET_REG.evaluate_recipe(&opaque_ref);

        let recipe_info = &opaque_ref.recipe_info;
        let recipe_info = recipe_info.read().unwrap();

        match recipe_info.build_record {
            Some(RecipeBuildRecord {
                ref last_valid_build_result,
                ..
            }) => Ok(get_pinned_result(last_valid_build_result.clone())),
            _ => Err(format_err!(
                "Requested asset {:?} failed to build",
                opaque_ref
            )),
        }
    }
}

pub trait Op: Send + 'static {
    type Res;

    fn run(&self, ctx: &mut Context) -> Result<Self::Res>;
    fn name() -> &'static str;
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

pub fn def_named<
    'a,
    AssetType: 'static + Send + Sync + MaybeSerialize,
    OpType: Op<Res = AssetType> + Hash,
>(
    identity_hash: u64,
    op: OpType,
) -> SnoozyRef<AssetType> {
    let mut s = DefaultSnoozyHash::default();
    <OpType as std::hash::Hash>::hash(&op, &mut s);
    let recipe_hash = s.finish();

    let make_recipe_runner = || -> Arc<dyn (Fn(&mut Context) -> _) + Send + Sync> {
        let op_mutex = Mutex::new(op);
        Arc::new(move |mut ctx| -> Result<Arc<dyn Any + Send + Sync>> {
            //println!("Running recipe {:?} ({})", &*op_mutex.lock().unwrap(), identity_hash);
            let build_result = op_mutex.lock().unwrap().run(&mut ctx)?;
            Ok(Arc::new(build_result))
        })
    };

    let mut refs = ASSET_REG.refs.write().unwrap();

    //let opaque_addr: OpaqueSnoozyRefInner = res.clone().into();
    let opaque_addr = OpaqueSnoozyAddr::new::<AssetType>(identity_hash);
    //let res = SnoozyRef::new(identity_hash);

    match refs.get(&opaque_addr) {
        // Definition doesn't exist. Create it
        None => {
            let recipe_info = RwLock::new(RecipeInfo {
                recipe_runner: make_recipe_runner(),
                recipe_meta: RecipeMeta::new::<AssetType>(<OpType as Op>::name()),
                rebuild_pending: true,
                build_record: None,
                recipe_hash,
            });

            let opaque_ref = Arc::new(OpaqueSnoozyRefInner {
                addr: opaque_addr.clone(),
                recipe_info,
            });

            refs.insert(opaque_addr, opaque_ref.clone());

            SnoozyRef::new(opaque_ref)
        }
        // Definition exists. If the hash is the same, don't do anything
        Some(opaque_ref) => {
            // Always Some in this branch
            let mut entry = opaque_ref.recipe_info.write().unwrap();

            if entry.recipe_hash != recipe_hash {
                // Hash differs. Update the definition, but keep the last build record
                entry.recipe_runner = make_recipe_runner();
                entry.recipe_hash = recipe_hash;

                // Clear any pending rebuild of this asset, and instead schedule
                // a full rebuild including of all of its reverse dependencies.
                entry.rebuild_pending = false;
                ASSET_REG
                    .queued_asset_invalidations
                    .lock()
                    .unwrap()
                    .push(opaque_ref.clone());
            }

            SnoozyRef::new(opaque_ref.clone())
        }
    }
}

pub fn def_initial<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + Hash>(
    identity_hash: u64,
    op: OpType,
) -> SnoozyRef<AssetType> {
    let res = def_named(identity_hash, op);
    ASSET_REG.evaluate_recipe(&res.clone().into());
    res
}

pub struct Snapshot;
impl Snapshot {
    pub fn get<Res: 'static + Send + Sync>(&self, asset_ref: SnoozyRef<Res>) -> Pin<Arc<Res>> {
        let opaque_ref: OpaqueSnoozyRef = asset_ref.into();

        ASSET_REG.evaluate_recipe(&opaque_ref);

        let recipe_info = &opaque_ref.recipe_info;
        let recipe_info = recipe_info.read().unwrap();

        match recipe_info.build_record {
            Some(RecipeBuildRecord {
                ref last_valid_build_result,
                ..
            }) => get_pinned_result(last_valid_build_result.clone()),
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
