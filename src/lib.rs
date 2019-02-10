#![feature(specialization)]
#![cfg_attr(feature = "core_intrinsics", feature(core_intrinsics))]

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate failure;
extern crate bincode;
extern crate serde;
#[macro_use]
extern crate lazy_static;

#[macro_use]
mod macros;

use serde::ser::SerializeTuple;

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::default::Default;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::FromIterator;
use std::marker::PhantomData;
use std::mem::transmute;
use std::sync::{Arc, Mutex, RwLock};
use twox_hash::XxHash;

pub use failure::{err_msg, Error};
pub type Result<T> = std::result::Result<T, Error>;

pub trait WeakHash {
    fn weak_hash(&self) -> u64;
}

impl<T: Hash> WeakHash for T {
    fn weak_hash(&self) -> u64 {
        let mut s = XxHash::default();
        <Self as std::hash::Hash>::hash(self, &mut s);
        s.finish()
    }
}

pub trait RecipeHash {
    fn recipe_hash(&self) -> u64;
}

pub struct Context {
    opaque_ref: OpaqueSnoozyRef, // Handle for the asset that this Context was created for
    dependencies: Vec<OpaqueSnoozyRef>,
}

impl Context {
    pub fn get_invalidation_trigger(&self) -> impl Fn() {
        let queued_assets = ASSET_REG.queued_asset_invalidations.clone();
        let opaque_ref = self.opaque_ref;
        move || {
            queued_assets.lock().unwrap().push(opaque_ref);
        }
    }

    /*fn with<Res: 'static, FnRet>(&self, asset_ref: &SnoozyRef<Res>, sink: &mut FnMut(&Res) -> FnRet) -> FnRet {
        //unreachable!()	// TODO

        let opaque_ref : OpaqueSnoozyRef = (*asset_ref).into();

        let entry = ASSET_REG.lock().unwrap().recipe_info.get(&opaque_ref).unwrap().clone();
        let res = (entry.lock().unwrap().recipe_runner)();
        entry.lock().unwrap().build_result = Some(res);

        let res = match entry.lock().unwrap().build_result.as_ref().unwrap().downcast_ref::<Res>() {
            Some(as_res) => sink(as_res),
            None => unreachable!(),
        };

        res
    }*/

    pub fn get<Res: 'static + Send + Sync, SnoozyT: Into<SnoozyRef<Res>>>(
        &mut self,
        asset_ref: SnoozyT,
    ) -> Result<Arc<Res>> {
        let asset_ref = asset_ref.into();
        let opaque_ref: OpaqueSnoozyRef = asset_ref.into();

        self.dependencies.push(opaque_ref);
        let eval_success = ASSET_REG.evaluate_recipe(opaque_ref);

        if !eval_success {
            return Err(err_msg("Dependent asset build failed"));
        }

        let recipe_info_lock = ASSET_REG
            .recipe_info
            .lock()
            .unwrap()
            .get(&opaque_ref)
            .unwrap()
            .clone();
        let recipe_info = recipe_info_lock.read().unwrap();

        match recipe_info.build_result {
            Some(ref res) => match res {
                Ok(res) => Ok(res.clone().downcast::<Res>().unwrap()),
                Err(s) => Err(err_msg(s.clone())),
            },
            None => Err(format_err!(
                "Requested asset {:?} failed to build",
                asset_ref
            )),
        }

        /*let res: Arc<Res> = recipe_info
            .build_result
            .as_ref()
            .unwrap()
            .clone()
            .downcast::<Res>()
            .unwrap();
        res*/
    }
}

pub trait Op: Send + 'static + fmt::Debug {
    type Res;

    fn run(&self, ctx: &mut Context) -> Result<Self::Res>;
}

#[derive(Default, Clone, Copy, Hash, Eq, PartialEq)]
pub struct ScopeId(u64);

#[derive(Serialize, Hash)]
pub struct SnoozyRef<Res> {
    pub identity_hash: u64,
    phantom: PhantomData<Res>,
}

// So we can use Into<SnoozyRef<_>>, and accept either & or copy.
impl<Res> From<&SnoozyRef<Res>> for SnoozyRef<Res> {
    fn from(v: &SnoozyRef<Res>) -> Self {
        *v
    }
}

impl<Res> Copy for SnoozyRef<Res> {}
impl<Res> Clone for SnoozyRef<Res> {
    fn clone(&self) -> Self {
        SnoozyRef {
            identity_hash: self.identity_hash,
            phantom: PhantomData,
        }
    }
}

#[cfg(core_intrinsics)]
impl<Res> fmt::Debug for SnoozyRef<Res> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "SnoozyRef<{}> {{identity_hash: {}}}",
            unsafe { std::intrinsics::type_name::<Res>() },
            self.identity_hash
        )
    }
}

#[cfg(not(core_intrinsics))]
impl<Res> fmt::Debug for SnoozyRef<Res> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SnoozyRef {{identity_hash: {}}}", self.identity_hash)
    }
}

#[derive(Hash, Clone, Copy, Eq, PartialEq, Debug)]
pub struct OpaqueSnoozyRef {
    identity_hash: u64,
    type_id: TypeId,
}

impl serde::Serialize for OpaqueSnoozyRef {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut tup = serializer.serialize_tuple(2)?;
        tup.serialize_element(&self.identity_hash)?;
        unsafe { tup.serialize_element(&transmute::<_, u64>(self.type_id))? };
        tup.end()
    }
}

impl<Res: 'static> Into<OpaqueSnoozyRef> for SnoozyRef<Res> {
    fn into(self) -> OpaqueSnoozyRef {
        OpaqueSnoozyRef {
            identity_hash: self.identity_hash,
            type_id: TypeId::of::<Res>(),
        }
    }
}

pub struct Scope;

impl Scope {
    /*fn def<AssetType: WeakHash, OpType: Op<Res=AssetType>>(&mut self, op: OpType) -> SnoozyRef<AssetType> {
        SnoozyRef::<AssetType> { phantom: PhantomData }
    }*/

    #[allow(dead_code)]
    pub fn def<AssetType: 'static>(
        &mut self,
        asset_ref: SnoozyRef<AssetType>,
    ) -> SnoozyRef<AssetType> {
        // TODO: redefine here
        let _opaque_ref: OpaqueSnoozyRef = asset_ref.into();
        asset_ref
    }

    /*fn new() -> Scope {
        Scope
    }*/
}

/*lazy_static! {
    static ref SCOPE_REG: HashMap<ScopeId, Box<Scope>> = {
        let mut m = HashMap::new();
        m.insert(ScopeId(0), Box::new(Scope::new()));
        m
    };
}*/

#[derive(Default)]
struct BuildRecord {
    build_succeeded: bool,

    latest_result: bool,
}

struct RecipeInfo {
    recipe_runner: Arc<(Fn(&mut Context) -> Result<Arc<Any + Send + Sync>>) + Send + Sync>,
    build_result: Option<std::result::Result<Arc<Any + Send + Sync>, String>>,
    build_record: BuildRecord,
    // Assets this one requested during the last build
    dependencies: Vec<OpaqueSnoozyRef>,
    // Assets that requested this asset during their builds
    reverse_dependencies: HashSet<OpaqueSnoozyRef>,
    recipe_hash: u64,
}

lazy_static! {
    static ref ASSET_REG: AssetReg = {
        AssetReg {
            recipe_info: Mutex::new(HashMap::new()),
            queued_asset_invalidations: Default::default(),
            being_evaluated: Default::default(),
        }
    };
}

pub struct AssetReg {
    recipe_info: Mutex<HashMap<OpaqueSnoozyRef, Arc<RwLock<RecipeInfo>>>>,
    being_evaluated: Mutex<HashMap<OpaqueSnoozyRef, bool>>,
    queued_asset_invalidations: Arc<Mutex<Vec<OpaqueSnoozyRef>>>,
}

impl AssetReg {
    fn propagate_invalidations(&self) {
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
            rec.build_record.latest_result = false;

            for dep in rec.reverse_dependencies.iter() {
                self.invalidate_asset_tree(*dep);
            }
        }
    }

    fn evaluate_recipe(&self, opaque_ref: OpaqueSnoozyRef) -> bool {
        //println!("evaluate recipe {:?}", opaque_ref);

        if self.being_evaluated.lock().unwrap()[&opaque_ref] {
            //println!("recipe already being evaluated");
            return true;
        }

        self.being_evaluated
            .lock()
            .unwrap()
            .insert(opaque_ref, true);

        let (last_build_succeeded, needs_evaluation, recipe_runner) = {
            let recipe_info_lock = self
                .recipe_info
                .lock()
                .unwrap()
                .get(&opaque_ref)
                .unwrap()
                .clone();
            let recipe_info = recipe_info_lock.write().unwrap();

            (
                recipe_info.build_record.build_succeeded,
                !recipe_info.build_record.latest_result,
                recipe_info.recipe_runner.clone(),
            )
        };

        let mut success = last_build_succeeded && !needs_evaluation;

        if needs_evaluation {
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
                    recipe_info.build_result = Some(Ok(res));

                    //println!("Published build result for asset {:?}", opaque_ref);

                    let mut build_record: BuildRecord = Default::default();

                    success = true;
                    build_record.build_succeeded = true;
                    build_record.latest_result = true;

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

                    recipe_info.build_record = build_record;
                    recipe_info.dependencies = ctx.dependencies;
                }
                Err(err) => {
                    // Build failed, but unless anything changes, this is what we're stuck with
                    recipe_info.build_record.latest_result = true;

                    //println!("Error building asset {:?}: {}", opaque_ref, err);
                    println!("Error building asset: {}", err);
                }
            }
        }

        self.being_evaluated
            .lock()
            .unwrap()
            .insert(opaque_ref, false);

        success
    }
}

pub fn calculate_hash<T: Hash>(t: &T) -> u64 {
    let mut s = XxHash::default();
    t.hash(&mut s);
    s.finish()
}

pub trait PodVecHash {
    fn pod_vec_hash(&self) -> Option<u64>;
}

impl<T> PodVecHash for T {
    default fn pod_vec_hash(&self) -> Option<u64> {
        None
    }
}

impl<T> PodVecHash for Vec<T>
where
    T: Copy,
{
    fn pod_vec_hash(&self) -> Option<u64> {
        //let time0 = std::time::Instant::now();

        let mut s = XxHash::default();
        unsafe {
            let p = self.as_ptr();
            let item_sizeof = std::mem::size_of::<T>();
            let len = self.len() * item_sizeof;
            std::slice::from_raw_parts(p as *const u8, len).hash(&mut s);
        }
        //println!("[fast path] Hash calculated in {:?}", time0.elapsed());

        Some(s.finish())
    }
}

pub fn calculate_serialized_hash<T: serde::Serialize>(t: &T) -> u64 {
    if let Some(h) = PodVecHash::pod_vec_hash(t) {
        h
    } else {
        //let time0 = std::time::Instant::now();
        let mut s = XxHash::default();
        let encoded: Vec<u8> = bincode::serialize(&t).unwrap();
        //println!("Hashable data serialized in {:?}", time0.elapsed());
        //let time0 = std::time::Instant::now();
        encoded.hash(&mut s);
        //println!("Hash calculated in {:?}", time0.elapsed());
        s.finish()
    }
}

pub fn def<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + RecipeHash>(
    op: OpType,
    op_source_hash: u64,
) -> SnoozyRef<AssetType> {
    def_named(op.recipe_hash() ^ op_source_hash, op)
}

pub fn def_named<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + RecipeHash>(
    identity_hash: u64,
    op: OpType,
) -> SnoozyRef<AssetType> {
    let recipe_hash = op.recipe_hash();

    let res = SnoozyRef::<AssetType> {
        identity_hash,
        phantom: PhantomData,
    };

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
                        build_result: None,
                        build_record: Default::default(),
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
                    entry.build_record = Default::default();
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

pub fn def_initial<AssetType: 'static + Send + Sync, OpType: Op<Res = AssetType> + RecipeHash>(
    identity_hash: u64,
    op: OpType,
) -> SnoozyRef<AssetType> {
    let res = def_named(identity_hash, op);
    ASSET_REG.evaluate_recipe(res.into());
    res
}

pub struct Snapshot;
impl Snapshot {
    /*fn with<Res: 'static, F: FnOnce(&Res)>(&self, asset_ref: &SnoozyRef<Res>, sink: F) {
        let opaque_ref : OpaqueSnoozyRef = (*asset_ref).into();

        let entry = ASSET_REG.lock().unwrap().recipe_info.get(&opaque_ref).unwrap().clone();
        let res = (entry.lock().unwrap().recipe_runner)();
        entry.lock().unwrap().build_result = Some(res);

        match entry.lock().unwrap().build_result.as_ref().unwrap().downcast_ref::<Res>() {
            Some(as_res) => sink(as_res),
            None => unreachable!(),
        };
    }*/

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

        match recipe_info.build_result {
            Some(ref res) => match res {
                Ok(res) => res.clone().downcast::<Res>().unwrap(),
                Err(s) => panic!(err_msg(s.clone())),
            },
            None => panic!("Requested asset {:?} failed to build", asset_ref),
        }
    }
}

/*impl<Res: 'static + Send + Sync> std::ops::Div<SnoozyRef<Res>> for &mut Snapshot {
    type Output = Arc<Res>;

    fn div(self, rhs: SnoozyRef<Res>) -> Arc<Res> {
        self.get(rhs)
    }
}*/

pub fn with_snapshot<F, Ret>(callback: F) -> Ret
where
    F: FnOnce(&mut Snapshot) -> Ret,
{
    ASSET_REG.propagate_invalidations();
    let mut snap = Snapshot;
    callback(&mut snap)
}
