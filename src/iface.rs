use crate::asset_reg::{
    RecipeBuildRecord, RecipeDebugInfo, RecipeInfo, RecipeMeta, RecipeRunner, ASSET_REG,
};
use crate::cycle_detector::{create_cycle_detector, CycleDetector};
use crate::maybe_serialize::MaybeSerialize;
use crate::refs::{
    EvaluationPathNode, OpaqueSnoozyAddr, OpaqueSnoozyRef, OpaqueSnoozyRefInner,
    SnoozyIdentityHash, SnoozyRef,
};
use crate::{DefaultSnoozyHash, Result};
use async_trait::async_trait;
use std::any::Any;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock, Weak};

#[derive(Clone)]
pub struct EvalContext {
    pub cycle_detector: CycleDetector,
}

pub struct ContextInner {
    pub(crate) opaque_ref: OpaqueSnoozyRef, // Handle for the asset that this Context was created for
    pub(crate) dependencies: Mutex<HashSet<OpaqueSnoozyRef>>,
    pub(crate) evaluation_path: Mutex<HashSet<EvaluationPathNode>>,
    pub(crate) debug_info: Mutex<RecipeDebugInfo>,
}

#[derive(Clone)]
pub struct Context {
    pub inner: Arc<ContextInner>,
    pub eval_context: EvalContext,
}

impl Context {
    pub fn set_debug_name(&self, name: impl AsRef<str>) {
        self.inner.debug_info.lock().unwrap().debug_name = Some(name.as_ref().to_owned());
    }

    pub fn get_invalidation_trigger(&self) -> impl Fn() {
        let queued_assets = ASSET_REG.queued_asset_invalidations.clone();
        let opaque_ref = self.inner.opaque_ref.clone();
        move || {
            queued_assets.lock().unwrap().push(opaque_ref.clone());
        }
    }

    pub async fn get<Res: 'static + Send + Sync, SnoozyT: Into<SnoozyRef<Res>>>(
        &mut self,
        asset_ref: SnoozyT,
    ) -> Result<Arc<Res>> {
        let inner = &self.inner;
        let asset_ref = asset_ref.into();
        let opaque_ref: OpaqueSnoozyRef = asset_ref.opaque;

        self.eval_context.cycle_detector.add_edge(
            inner.opaque_ref.to_evaluation_path_node().into_raw(),
            opaque_ref.to_evaluation_path_node().into_raw(),
        );
        inner
            .dependencies
            .lock()
            .unwrap()
            .insert(opaque_ref.clone());

        let child_eval_path = inner.evaluation_path.lock().unwrap().clone();
        ASSET_REG
            .evaluate_recipe(&opaque_ref, child_eval_path, self.eval_context.clone())
            .await;

        let recipe_info = &opaque_ref.recipe_info;
        let recipe_info = recipe_info.read().unwrap();

        match recipe_info.build_record {
            Some(RecipeBuildRecord {
                ref last_valid_build_result,
                ..
            }) => Ok(last_valid_build_result.artifact.clone().downcast().unwrap()),
            _ => Err(format_err!(
                "Requested asset {:?} failed to build ({})",
                *opaque_ref,
                recipe_info.recipe_meta.op_name,
            )),
        }
    }

    pub fn get_transient_op_id(&self) -> usize {
        self.inner.opaque_ref.get_transient_op_id()
    }
}

pub trait Op: Send + Sync + 'static {
    type Res;

    fn run<'a>(
        &'a self,
        ctx: Context,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Self::Res>> + Send + 'a>>;
    fn name() -> &'static str;
    fn should_cache_result(&self) -> bool;
}

#[async_trait]
impl<T> RecipeRunner for T
where
    T: Op,
    T::Res: 'static + Send + Sync + Any,
{
    async fn run(&self, ctx: Context) -> Result<Arc<dyn Any + Send + Sync>> {
        let build_result: T::Res = Op::run(self, ctx).await?;
        let build_result: Arc<dyn Any + Send + Sync> = Arc::new(build_result);
        Ok(build_result)
    }

    fn should_cache_result(&self) -> bool {
        Op::should_cache_result(self)
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
    //dbg!(refs.len());

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
                being_evaluated: Default::default(),
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
    pub async fn get<Res: 'static + Send + Sync>(&self, asset_ref: SnoozyRef<Res>) -> Arc<Res> {
        let opaque_ref: OpaqueSnoozyRef = asset_ref.into();

        let (cycle_detector, cycle_detector_backend) = create_cycle_detector();
        let eval_context = EvalContext { cycle_detector };

        std::thread::spawn(move || {
            cycle_detector_backend.run();
        });

        ASSET_REG
            .evaluate_recipe(&opaque_ref, HashSet::new(), eval_context)
            .await;

        let recipe_info = &opaque_ref.recipe_info;
        let recipe_info = recipe_info.read().unwrap();

        match recipe_info.build_record {
            Some(RecipeBuildRecord {
                ref last_valid_build_result,
                ..
            }) => last_valid_build_result.artifact.clone().downcast().unwrap(),
            None => panic!("Requested asset {:?} failed to build", *opaque_ref),
        }
    }
}

pub fn with_snapshot<F, Ret>(callback: F) -> Ret
where
    F: FnOnce(Snapshot) -> Ret,
{
    ASSET_REG.propagate_invalidations();
    ASSET_REG.collect_garbage();
    callback(Snapshot)
}

pub fn get_snapshot() -> Snapshot {
    ASSET_REG.propagate_invalidations();
    ASSET_REG.collect_garbage();
    Snapshot
}
