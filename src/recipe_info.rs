use super::iface::Context;
use super::maybe_serialize::{AnySerialize, AnySerializeProxy, MaybeSerialize};
use super::refs::*;
use super::Result;
use async_trait::async_trait;
use std::any::Any;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

#[derive(Default, Clone)]
pub struct RecipeDebugInfo {
    pub debug_name: Option<String>,
}

#[derive(Clone)]
pub struct RecipeBuildResult {
    pub artifact: Arc<dyn Any + Send + Sync>,
    pub debug_info: RecipeDebugInfo,
}

#[derive(Clone)]
pub struct SnoozyRefDependency(pub Arc<OpaqueSnoozyRefInner>);

impl SnoozyRefDependency {
    pub fn get_transient_op_id(&self) -> usize {
        let inner: &OpaqueSnoozyRefInner = &*self.0;
        inner as *const OpaqueSnoozyRefInner as usize
    }
}

impl std::ops::Deref for SnoozyRefDependency {
    type Target = Arc<OpaqueSnoozyRefInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Eq for SnoozyRefDependency {}
impl PartialEq for SnoozyRefDependency {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(
            &*self.0 as *const OpaqueSnoozyRefInner,
            &*other.0 as *const OpaqueSnoozyRefInner,
        )
    }
}
impl Hash for SnoozyRefDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let ptr = &*self.0 as *const OpaqueSnoozyRefInner as usize;
        ptr.hash(state);
    }
}

pub struct RecipeBuildRecord {
    pub build_result: RecipeBuildResult,
    pub prev_build_result: Option<RecipeBuildResult>,
    pub build_snapshot_idx: usize,
    // Assets this one requested during the last build
    pub dependencies: HashSet<SnoozyRefDependency>,
    // Assets that requested this asset during their builds
    pub reverse_dependencies: Vec<Weak<OpaqueSnoozyRefInner>>,
}

impl RecipeBuildRecord {
    fn get_valid_reverse_dependencies(&self) -> HashSet<SnoozyRefDependency> {
        self.reverse_dependencies
            .iter()
            .filter_map(|a| Some(SnoozyRefDependency(Weak::upgrade(a)?)))
            .collect()
    }
}

pub struct RecipeMeta {
    pub op_name: &'static str,
    pub result_type_name: &'static str,
    pub(crate) serialize_proxy: Box<dyn AnySerialize>,
}

impl RecipeMeta {
    pub fn new<T>(op_name: &'static str) -> Self
    where
        T: 'static + Send + Sync + MaybeSerialize,
    {
        let serialize_proxy: Box<AnySerializeProxy<T>> = Box::new(AnySerializeProxy::new());
        let serialize_proxy = serialize_proxy as Box<dyn AnySerialize>;

        Self {
            result_type_name: std::intrinsics::type_name::<T>(),
            op_name,
            serialize_proxy,
        }
    }
}

impl Clone for RecipeMeta {
    fn clone(&self) -> Self {
        Self {
            op_name: self.op_name,
            result_type_name: self.result_type_name,
            serialize_proxy: self.serialize_proxy.clone_boxed(),
        }
    }
}

#[async_trait]
pub trait RecipeRunner: Send + Sync {
    async fn run(&self, ctx: Context) -> Result<Arc<dyn Any + Send + Sync>>;
    fn should_cache_result(&self) -> bool;
}

pub struct RecipeInfo {
    pub recipe_runner: Arc<dyn RecipeRunner>,
    pub recipe_meta: RecipeMeta,
    pub recipe_hash: u64,

    //pub rebuild_pending: bool,
    pub build_record: Option<RecipeBuildRecord>,
}

pub(crate) struct BuildRecordDiff {
    pub added_deps: Vec<SnoozyRefDependency>,
    pub removed_deps: Vec<SnoozyRefDependency>,
}

impl RecipeInfo {
    pub(crate) fn replace_desc(&mut self, other: RecipeInfo) {
        self.recipe_runner = other.recipe_runner;
        self.recipe_meta = other.recipe_meta;
        //self.rebuild_pending = other.rebuild_pending;
        self.recipe_hash = other.recipe_hash;

        // Don't replace the build_record
    }

    pub(crate) fn set_new_build_result(
        &mut self,
        build_result: RecipeBuildResult,
        dependencies: HashSet<SnoozyRefDependency>,
        build_snapshot_idx: usize,
    ) -> BuildRecordDiff {
        let (prev_deps, prev_rev_deps, prev_build_result) = self
            .build_record
            .take()
            .map(|r| {
                let rev_deps = r.get_valid_reverse_dependencies();
                (r.dependencies, rev_deps, Some(r.build_result))
            })
            .unwrap_or_default();

        let added_deps = dependencies.difference(&prev_deps).cloned().collect();
        let removed_deps = prev_deps.difference(&dependencies).cloned().collect();

        self.build_record = Some(RecipeBuildRecord {
            build_result: build_result,
            dependencies,
            // Keep the previous reverse dependencies. Until the dependent assets get rebuild,
            // we must assume they still depend on the current asset.
            reverse_dependencies: prev_rev_deps.iter().map(|a| Arc::downgrade(&a.0)).collect(),
            prev_build_result: prev_build_result,
            build_snapshot_idx,
        });

        BuildRecordDiff {
            added_deps,
            removed_deps,
        }
    }

    pub(crate) fn clone_desc(&self) -> Self {
        Self {
            recipe_runner: self.recipe_runner.clone(),
            recipe_meta: self.recipe_meta.clone(),
            //rebuild_pending: true,
            build_record: None,
            recipe_hash: self.recipe_hash,
        }
    }
}
