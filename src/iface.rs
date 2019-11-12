use super::asset_reg::{RecipeBuildRecord, RecipeInfo, RecipeMeta, RecipeRunner, ASSET_REG};
use super::maybe_serialize::MaybeSerialize;
use super::refs::{
    OpaqueSnoozyAddr, OpaqueSnoozyRef, OpaqueSnoozyRefInner, SnoozyIdentityHash, SnoozyRef,
};
use super::{DefaultSnoozyHash, Result};
use std::any::Any;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::{Arc, RwLock, Weak};

pub struct Context {
    pub(crate) opaque_ref: OpaqueSnoozyRef, // Handle for the asset that this Context was created for
    pub(crate) dependencies: HashSet<OpaqueSnoozyRef>,
    pub(crate) dependency_build_time: std::time::Duration,
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

        let t0 = std::time::Instant::now();
        ASSET_REG.evaluate_recipe(&opaque_ref);
        self.dependency_build_time += t0.elapsed();

        let recipe_info = &opaque_ref.recipe_info;
        let recipe_info = recipe_info.read().unwrap();

        match recipe_info.build_record {
            Some(RecipeBuildRecord {
                ref last_valid_build_result,
                ..
            }) => Ok(get_pinned_result(last_valid_build_result.clone())),
            _ => Err(format_err!(
                "Requested asset {:?} failed to build",
                *opaque_ref
            )),
        }
    }
}

pub trait Op: Send + Sync + 'static {
    type Res;

    fn run(&self, ctx: &mut Context) -> Result<Self::Res>;
    fn name() -> &'static str;
}

impl<T> RecipeRunner for T
where
    T: Op,
    T::Res: 'static + Send + Sync,
{
    fn run(&self, ctx: &mut Context) -> Result<Arc<dyn Any + Send + Sync>> {
        let build_result = self.run(ctx)?;
        Ok(Arc::new(build_result))
    }
}

pub fn snoozy_def_binding<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + Hash>(
    op: OpType,
) -> SnoozyRef<AssetType> {
    let mut s = DefaultSnoozyHash::default();
    <OpType as std::hash::Hash>::hash(&op, &mut s);
    def_binding(SnoozyIdentityHash(s.finish()), op)
}

fn def_binding<
    AssetType: 'static + Send + Sync + MaybeSerialize,
    OpType: Op<Res = AssetType> + Hash,
>(
    identity_hash: SnoozyIdentityHash,
    op: OpType,
) -> SnoozyRef<AssetType> {
    let mut s = DefaultSnoozyHash::default();
    <OpType as std::hash::Hash>::hash(&op, &mut s);
    let recipe_hash = s.finish();

    let mut refs = ASSET_REG.refs.write().unwrap();

    let opaque_addr = OpaqueSnoozyAddr::new::<AssetType>(identity_hash);

    match refs.get(&opaque_addr).and_then(Weak::upgrade) {
        // Definition doesn't exist. Create it
        None => {
            let recipe_info = RwLock::new(RecipeInfo {
                recipe_runner: Arc::new(op),
                recipe_meta: RecipeMeta::new::<AssetType>(<OpType as Op>::name()),
                rebuild_pending: true,
                build_record: None,
                recipe_hash,
            });

            let opaque_ref = Arc::new(OpaqueSnoozyRefInner {
                addr: opaque_addr.clone(),
                recipe_info,
            });

            refs.insert(opaque_addr, Arc::downgrade(&opaque_ref));

            SnoozyRef::new(OpaqueSnoozyRef(opaque_ref))
        }
        // Definition exists, so we can just return it.
        Some(opaque_ref) => {
            let entry = opaque_ref.recipe_info.read().unwrap();
            assert_eq!(entry.recipe_hash, recipe_hash);
            SnoozyRef::new(OpaqueSnoozyRef(opaque_ref.clone()))
        }
    }
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
            None => panic!("Requested asset {:?} failed to build", *opaque_ref),
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
