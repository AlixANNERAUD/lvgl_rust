//! Animation identity: which animations an animation removal can reach.
//!
//! `lv_anim_delete(var, exec_cb)` removes only the animations whose `var`
//! and `exec_cb` match (`lv_anim.c:190`), and `lv_anim_start` replaces only
//! the running animations with the same `var` and `exec_cb`
//! (`remove_concurrent_anims`, `lv_anim.c:846-850`). Without telling
//! animations apart, every removal reaches every `deleted_cb` and frees
//! every animation; this module names them.
//!
//! An animation is described by a *profile*: an `lv_anim_t` local of one
//! function, filled by setters (functions that store a parameter into a
//! field of the animation they are given) and started by `lv_anim_start`,
//! which copies it. A profile records the callbacks stored in each field,
//! the struct types of its `var`, and whether it is started with
//! `early_apply` off (`lv_anim_start` then runs no callback and replaces
//! nothing, `lv_anim.c:124, 141`). A local whose address goes anywhere
//! else (another callee, a copy, a store) is *opaque*: it matches
//! everything.
//!
//! Callbacks reach animation fields through profiles or some other way
//! (stores into fields of other memory, parameters forwarded to a helper,
//! global initializers): the latter match everything too.
//!
//! A removal is described by a *key*: the `exec_cb` it compares (a
//! constant function) and the struct type of its `var`. Two pointers of
//! struct types `A` and `B` can be equal only if one struct starts with the
//! other (C's first members, recursively); unions, byte arrays and unknown
//! types are assumed to start with anything.
//!
//! Pointer types come from the IR, which has none on pointers: a value has
//! the struct type it is indexed as (`getelementptr`), or that of the
//! callee parameter it is forwarded to (`Types`), generalised to the root
//! of its first-member chain so that a cast after a class check never
//! narrows it to one subclass.

use super::ir::{Arg, Function, Module, SiteKind, Val};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// `[animation]` in `audit.toml`: LVGL's animation functions and fields.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationConfig {
    /// The animation struct (`_lv_anim_t`).
    pub structure: String,
    /// Copies an animation into the running list (`lv_anim_start`).
    pub start: String,
    /// Fills an animation with its defaults (`lv_anim_init`): no `var`, no
    /// callbacks but the default path, `early_apply` on.
    pub init: String,
    /// Removes running animations by `var` and `exec_cb` (`lv_anim_delete`).
    pub delete: String,
    pub delete_var_arg: usize,
    pub delete_exec_arg: usize,
    /// The functions a removal goes through, in order, down to the one
    /// that runs the `deleted_cb` and frees the animation.
    pub delete_carry: Vec<String>,
    pub start_carry: Vec<String>,
    /// Removes the running animations a starting one replaces: runs the
    /// `deleted_cb` of each running animation with the same `exec_cb` and
    /// `var` as its parameter 0, which it reads again afterwards
    /// (`remove_concurrent_anims`).
    #[serde(default)]
    pub concurrent_removal: Option<String>,
    /// Unlinks a node from the running list: an animation unlinked by a
    /// frame is freed only by that frame (`held.rs`).
    #[serde(default)]
    pub unlink: Option<Unlink>,
    /// The `var` field.
    pub var_slot: String,
    /// The `exec_cb` field, which removals compare.
    pub exec_slot: String,
    /// The `deleted_cb` field, run by removals.
    pub deleted_slot: String,
    /// Sets `early_apply` (`lv_anim_set_early_apply(a, en)`).
    pub early_apply_setter: String,
    /// Event senders and the index of their parameter argument: an
    /// animation passed there is only read by LVGL's handlers.
    #[serde(default)]
    pub event_parameter: BTreeMap<String, usize>,
}

/// `unlink(list, node)` on the running list of animations.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unlink {
    pub function: String,
    pub list_arg: usize,
    pub node_arg: usize,
    /// The last step of the running list's field path (`T.k`).
    pub list_field: String,
}

/// The animations a removal can reach.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnimationKey {
    /// Those with this `exec_cb` and a `var` of this struct type (`None`:
    /// any). `"null"` is the type of a NULL `var`.
    Matching {
        exec: Option<String>,
        var: Option<String>,
    },
    /// None: the removal cannot run (`early_apply` off, or no `exec_cb`).
    Nothing,
}

impl fmt::Display for AnimationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnimationKey::Nothing => write!(f, "nothing"),
            AnimationKey::Matching { exec, var } => {
                let mut parts = Vec::new();
                if let Some(e) = exec {
                    parts.push(format!("exec={e}"));
                }
                if let Some(v) = var {
                    parts.push(format!("var={v}"));
                }
                write!(f, "{}", parts.join(","))
            }
        }
    }
}

impl AnimationKey {
    /// The suffix of the graph nodes specialised for this key.
    pub fn suffix(&self) -> String {
        format!("#animation({self})")
    }
}

/// An animation local of one function (see the module documentation).
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub function: String,
    pub local: String,
    /// Matches everything.
    pub opaque: bool,
    /// Functions stored in each field.
    pub slots: BTreeMap<String, BTreeSet<String>>,
    /// Fields given something other than a named function.
    pub unknown_slots: BTreeSet<String>,
    /// Struct types of `var` (`"null"` for NULL).
    pub vars: BTreeSet<String>,
    pub var_unknown: bool,
    /// `lv_anim_init` runs on it.
    pub initialised: bool,
    /// Every start of this local runs with `early_apply` off.
    pub never_early: bool,
}

impl Profile {
    /// Whether a removal with `key` can match an animation started from
    /// this local.
    pub fn admits(&self, key: &AnimationKey, config: &AnimationConfig, module: &Module) -> bool {
        let AnimationKey::Matching { exec, var } = key else {
            return false;
        };
        if self.opaque {
            return true;
        }
        let exec_ok = match exec {
            None => true,
            Some(e) => {
                !self.initialised
                    || self.unknown_slots.contains(&config.exec_slot)
                    || self
                        .slots
                        .get(&config.exec_slot)
                        .is_some_and(|s| s.contains(e))
            }
        };
        let var_ok = match var {
            None => true,
            Some(t) => self.var_unknown || self.vars.iter().any(|v| compatible(t, v, module)),
        };
        exec_ok && var_ok
    }
}

/// What a function does to the animation (or any struct) its parameter 0
/// points to.
#[derive(Clone, Debug, Default)]
struct Effects {
    /// `(parameter, field)`: stores the parameter into the field.
    setters: Vec<(usize, String)>,
    /// `(field, function)`: stores a constant (a function, or something
    /// unnamed) into the field.
    constants: Vec<(String, Option<String>)>,
    /// Fields written with non-pointer stores.
    scalars: BTreeSet<String>,
    /// Passes the pointer on or copies the struct: anything can happen.
    opaque: bool,
}

/// Where a function lands in an animation field: through a profile, or
/// some other way (`None`: matches everything).
pub type Origins = BTreeSet<Option<usize>>;

/// The animation facts of the whole module.
#[derive(Debug, Default)]
pub struct Animations {
    pub config: Option<AnimationConfig>,
    pub profiles: Vec<Profile>,
    by_local: BTreeMap<(String, String), usize>,
    /// For each animation field, the functions stored in it and their
    /// origins.
    pub landings: BTreeMap<String, BTreeMap<String, Origins>>,
    /// Pointer parameter types.
    pub types: Types,
}

/// The parameter a value is, at `-O0` (a parameter or the local it was
/// spilled to).
fn param_of(f: &Function, v: &Val) -> Option<usize> {
    match v {
        Val::Param(k) => Some(*k),
        Val::Var(a) => match f.held.allocas.get(a).map(|l| l.stores.as_slice()) {
            Some([Val::Param(k)]) => Some(*k),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `v` is the pointer parameter 0 or a pointer into its memory.
fn derived_from_param0(f: &Function, v: &Val) -> bool {
    match v {
        Val::Field(base, _) => derived_from_param0(f, base),
        Val::Copy(src) => derived_from_param0(f, src),
        v => param_of(f, v) == Some(0),
    }
}

/// Whether `v` is the address of local `a`, a pointer into it, or a copy
/// of its contents.
fn derived_from_local(v: &Val, a: &str) -> bool {
    match v {
        Val::Local(x) => x == a,
        Val::Field(base, _) | Val::Copy(base) => derived_from_local(base, a),
        _ => false,
    }
}

fn effects(f: &Function) -> Effects {
    let mut out = Effects::default();
    for (address, value, _) in &f.held.stores {
        if derived_from_param0(f, value) {
            out.opaque = true; // the pointer (or a copy) escapes
        }
        match address {
            Val::Field(base, path) if param_of(f, base) == Some(0) => {
                let field = path.join("/");
                match value {
                    Val::Copy(_) => out.opaque = true,
                    Val::Global(g) => out.constants.push((field, Some(g.clone()))),
                    v => match param_of(f, v) {
                        Some(k) => out.setters.push((k, field)),
                        None => out.constants.push((field, None)),
                    },
                }
            }
            a if derived_from_param0(f, a) => out.opaque = true,
            _ => {}
        }
    }
    for address in &f.held.scalar_stores {
        if let Val::Field(base, path) = address
            && param_of(f, base) == Some(0)
        {
            out.scalars.insert(path.join("/"));
        }
    }
    for alloca in f.held.allocas.values() {
        if alloca
            .stores
            .iter()
            .any(|s| matches!(s, Val::Copy(src) if derived_from_param0(f, src)))
        {
            out.opaque = true;
        }
    }
    for site in &f.held.sites {
        if site.args.iter().any(|a| derived_from_param0(f, a)) {
            out.opaque = true;
        }
    }
    out
}

/// The struct types pointer parameters point to, inferred from their uses:
/// a `getelementptr` of struct `T` on the parameter (or the local it was
/// spilled to), or the parameter forwarded to a callee parameter of known
/// type. Every type is generalised to the root of its first-member chain
/// (`_lv_label_t` starts with `_lv_obj_t`: both are `_lv_obj_t`), so a
/// cast under a class or kind check (`(lv_label_t *)obj` after
/// `lv_obj_check_type`) never narrows a pointer to one subclass. A
/// parameter used as two unrelated roots, or as a struct that can start
/// with anything, has no type.
#[derive(Debug, Default)]
pub struct Types {
    /// Roots per `(function, parameter)`; `"*"`: anything.
    params: BTreeMap<(String, usize), BTreeSet<String>>,
}

/// The root of a struct's first-member chain (`"*"`: it can start with
/// anything).
fn root(module: &Module, strukt: &str) -> String {
    match prefix_chain(module, strukt) {
        Some(chain) => chain.last().cloned().unwrap_or_else(|| strukt.to_owned()),
        None => "*".to_owned(),
    }
}

fn single(roots: BTreeSet<String>) -> Option<String> {
    match roots.len() {
        1 => roots.into_iter().next().filter(|r| r != "*"),
        _ => None,
    }
}

impl Types {
    pub fn new(module: &Module) -> Types {
        let mut params: BTreeMap<(String, usize), BTreeSet<String>> = BTreeMap::new();
        let mut forwards: Vec<((String, usize), (String, usize))> = Vec::new();
        for (name, f) in &module.functions {
            for (k, types) in &f.held.param_types {
                let set = params.entry((name.clone(), *k)).or_default();
                set.extend(types.iter().map(|t| root(module, t)));
            }
            for alloca in f.held.allocas.values() {
                for v in &alloca.stores {
                    if let Val::Param(k) = v {
                        let set = params.entry((name.clone(), *k)).or_default();
                        set.extend(alloca.types.iter().map(|t| root(module, t)));
                    }
                }
            }
            for site in &f.held.sites {
                let SiteKind::Call(n) = site.kind else {
                    continue;
                };
                let callee = &f.calls[n].callee;
                if !module.functions.contains_key(callee) {
                    continue;
                }
                for (j, arg) in site.args.iter().enumerate() {
                    if let Some(k) = param_of(f, arg) {
                        forwards.push(((name.clone(), k), (callee.clone(), j)));
                    }
                }
            }
        }
        loop {
            let mut changed = false;
            for (to, from) in &forwards {
                let Some(add) = params.get(from).cloned() else {
                    continue;
                };
                let set = params.entry(to.clone()).or_default();
                let before = set.len();
                set.extend(add);
                changed |= set.len() != before;
            }
            if !changed {
                break;
            }
        }
        Types { params }
    }

    /// The root type of parameter `k` of `function`, when known.
    pub fn param(&self, function: &str, k: usize) -> Option<String> {
        single(self.params.get(&(function.to_owned(), k))?.clone())
    }

    /// The root type a pointer value of `function` points to, when known
    /// (`"null"` for NULL).
    pub fn value(&self, module: &Module, function: &str, f: &Function, v: &Val) -> Option<String> {
        match v {
            Val::Null => Some("null".into()),
            Val::Param(k) => self.param(function, *k),
            Val::Var(a) => {
                let alloca = f.held.allocas.get(a)?;
                if let [Val::Param(k)] = alloca.stores.as_slice() {
                    return self.param(function, *k);
                }
                single(alloca.types.iter().map(|t| root(module, t)).collect())
            }
            Val::Local(a) => f
                .held
                .allocas
                .get(a)?
                .ty
                .strip_prefix("%struct.")
                .map(|t| root(module, &super::ir::struct_name(t)))
                .filter(|r| r != "*"),
            Val::Field(_, path) => {
                let (strukt, index) = path.last()?.rsplit_once('.')?;
                let member = module
                    .types
                    .get(strukt)?
                    .get(index.parse::<usize>().ok()?)?;
                member
                    .trim()
                    .strip_prefix("%struct.")
                    .map(|t| root(module, &super::ir::struct_name(t)))
                    .filter(|r| r != "*")
            }
            _ => None,
        }
    }
}

/// What a struct starts with.
enum Start {
    Struct(String),
    /// Anything (a union, bytes, an unknown type).
    Opaque,
    /// A scalar or a pointer: nothing else starts here.
    End,
}

fn first_member(module: &Module, strukt: &str) -> Start {
    let Some(members) = module.types.get(strukt) else {
        return Start::Opaque;
    };
    let Some(first) = members.first().map(|m| m.trim()) else {
        return Start::End; // an empty struct
    };
    let element = match first.strip_prefix('[') {
        // An array starts with its first element.
        Some(rest) => rest
            .split_once(" x ")
            .map_or("", |(_, t)| t.trim_end_matches(']').trim()),
        None => first,
    };
    if let Some(name) = element.strip_prefix("%struct.") {
        Start::Struct(super::ir::struct_name(name))
    } else if element.starts_with('%')
        || element.starts_with('{')
        || element.starts_with('<')
        // A byte buffer may hold anything.
        || (first.starts_with('[') && element == "i8")
    {
        Start::Opaque
    } else {
        Start::End
    }
}

/// The structs a struct starts with (itself first); `None` if one of them
/// starts with anything.
fn prefix_chain(module: &Module, strukt: &str) -> Option<Vec<String>> {
    let mut chain = vec![strukt.to_owned()];
    loop {
        match first_member(module, chain.last().unwrap()) {
            Start::Struct(next) if !chain.contains(&next) => chain.push(next),
            Start::Struct(_) | Start::End => return Some(chain),
            Start::Opaque => return None,
        }
    }
}

/// Whether pointers to struct types `a` and `b` can be equal.
pub fn compatible(a: &str, b: &str, module: &Module) -> bool {
    if a == b {
        return true;
    }
    if a == "null" || b == "null" {
        return false;
    }
    match (prefix_chain(module, a), prefix_chain(module, b)) {
        (Some(ca), Some(cb)) => ca.iter().any(|t| t == b) || cb.iter().any(|t| t == a),
        _ => true,
    }
}

impl Animations {
    /// `param_slots`: the fields each function parameter is stored into;
    /// `passthrough`: the callee parameters each parameter is passed to.
    pub fn new(
        module: &Module,
        config: Option<&AnimationConfig>,
        param_slots: &BTreeMap<(String, usize), Vec<String>>,
        passthrough: &BTreeMap<(String, usize), Vec<(String, usize)>>,
    ) -> Result<Animations, String> {
        let Some(config) = config else {
            return Ok(Animations::default());
        };
        for name in [
            &config.start,
            &config.init,
            &config.delete,
            &config.early_apply_setter,
        ]
        .into_iter()
        .chain(&config.delete_carry)
        .chain(&config.start_carry)
        .chain(&config.concurrent_removal)
        .chain(config.unlink.as_ref().map(|u| &u.function))
        {
            if !module.functions.contains_key(name) {
                return Err(format!(
                    "audit.toml: [animation] names {name}, which is not defined"
                ));
            }
        }
        let effects: BTreeMap<&str, Effects> = module
            .functions
            .iter()
            .map(|(n, f)| (n.as_str(), effects(f)))
            .collect();
        // The field `early_apply` lives in (a bitfield).
        let early_fields = &effects[config.early_apply_setter.as_str()].scalars;
        let animation_type = format!("%struct.{}", config.structure);
        let is_animation_field =
            |slot: &str| slot.split('.').next() == Some(config.structure.as_str());

        let mut out = Animations {
            config: Some(config.clone()),
            types: Types::new(module),
            ..Animations::default()
        };
        for (name, f) in &module.functions {
            for (local, alloca) in &f.held.allocas {
                if alloca.ty != animation_type {
                    continue;
                }
                let profile = Self::profile(
                    module,
                    &out.types,
                    config,
                    &effects,
                    early_fields,
                    name,
                    f,
                    local,
                    &is_animation_field,
                );
                out.by_local
                    .insert((name.clone(), local.clone()), out.profiles.len());
                out.profiles.push(profile);
            }
        }

        // Where every function lands in animation fields.
        let mut land = |slot: &str, g: &str, origin: Option<usize>| {
            out.landings
                .entry(slot.to_owned())
                .or_default()
                .entry(g.to_owned())
                .or_default()
                .insert(origin);
        };
        for (slot, g) in &module.global_slots {
            if is_animation_field(slot) {
                land(slot, g, None);
            }
        }
        for (name, f) in &module.functions {
            for (slot, stored) in &f.stores {
                if let super::ir::Stored::Function(g) = stored
                    && is_animation_field(slot)
                {
                    land(slot, g, None);
                }
            }
            let sites: BTreeMap<usize, &super::ir::Site> = f
                .held
                .sites
                .iter()
                .filter_map(|s| match s.kind {
                    SiteKind::Call(n) => Some((n, s)),
                    SiteKind::Indirect(_) => None,
                })
                .collect();
            for (n, call) in f.calls.iter().enumerate() {
                for (i, arg) in call.args.iter().enumerate() {
                    let Arg::Function(g) = arg else { continue };
                    // A direct setter of the local given as argument 0.
                    let direct: BTreeSet<&String> = effects
                        .get(call.callee.as_str())
                        .map(|e| {
                            e.setters
                                .iter()
                                .filter(|(k, _)| *k == i)
                                .map(|(_, s)| s)
                                .collect()
                        })
                        .unwrap_or_default();
                    let profile = sites
                        .get(&n)
                        .and_then(|s| match s.args.first() {
                            Some(Val::Local(a)) => out.by_local.get(&(name.clone(), a.clone())),
                            _ => None,
                        })
                        .copied()
                        .filter(|&p| !out.profiles[p].opaque);
                    // Every field the argument can reach through setters
                    // and helpers.
                    let mut seen = BTreeSet::new();
                    let mut stack = vec![(call.callee.clone(), i, true)];
                    while let Some((callee, k, first)) = stack.pop() {
                        if !seen.insert((callee.clone(), k)) {
                            continue;
                        }
                        for slot in param_slots.get(&(callee.clone(), k)).into_iter().flatten() {
                            if !is_animation_field(slot) {
                                continue;
                            }
                            let origin = if first && direct.contains(slot) {
                                profile
                            } else {
                                None
                            };
                            land(slot, g, origin);
                        }
                        for next in passthrough.get(&(callee, k)).into_iter().flatten() {
                            stack.push((next.0.clone(), next.1, false));
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn profile(
        module: &Module,
        types: &Types,
        config: &AnimationConfig,
        effects: &BTreeMap<&str, Effects>,
        early_fields: &BTreeSet<String>,
        name: &str,
        f: &Function,
        local: &str,
        is_animation_field: &dyn Fn(&str) -> bool,
    ) -> Profile {
        let alloca = &f.held.allocas[local];
        let mut p = Profile {
            function: name.to_owned(),
            local: local.to_owned(),
            // Direct stores: only scalars into fields are understood
            // (`early_apply` among them, below).
            opaque: alloca.escaped
                || alloca.stores.len() > alloca.scalar_fields.len()
                || alloca.scalar_fields.iter().any(String::is_empty),
            ..Profile::default()
        };
        // Copies of it, and its address stored anywhere.
        let copied = f.held.allocas.values().any(|l| {
            l.stores
                .iter()
                .any(|s| matches!(s, Val::Copy(src) if derived_from_local(src, local)))
        }) || f
            .held
            .stores
            .iter()
            .any(|(a, v, _)| derived_from_local(a, local) || derived_from_local(v, local));
        p.opaque |= copied;

        let mut inits = Vec::new();
        let mut starts = Vec::new();
        let mut early_off = Vec::new();
        let mut early_other = alloca
            .scalar_fields
            .iter()
            .any(|field| early_fields.contains(field));
        let mut var_set = false;
        for site in &f.held.sites {
            let uses: Vec<usize> = site
                .args
                .iter()
                .enumerate()
                .filter(|(_, a)| derived_from_local(a, local))
                .map(|(j, _)| j)
                .collect();
            if uses.is_empty() {
                continue;
            }
            let SiteKind::Call(n) = site.kind else {
                p.opaque = true; // given to an unknown function
                continue;
            };
            let call = &f.calls[n];
            let callee = call.callee.as_str();
            if let Some(&k) = config.event_parameter.get(callee)
                && uses == [k]
                && site.args[k] == Val::Local(local.to_owned())
            {
                continue; // an event parameter, only read
            }
            if uses != [0] || site.args[0] != Val::Local(local.to_owned()) {
                p.opaque = true; // a field, or another argument
                continue;
            }
            if callee == config.start {
                starts.push(site.order);
            } else if callee == config.init {
                inits.push(site.order);
            } else if callee == config.early_apply_setter {
                match call.args.get(1) {
                    Some(Arg::Int(0)) => early_off.push(site.order),
                    _ => early_other = true,
                }
            } else if callee == config.delete {
                // A removal by this local's address: no animation has it.
            } else {
                let Some(e) = effects.get(callee) else {
                    p.opaque = true; // not defined in LVGL
                    continue;
                };
                if e.opaque {
                    p.opaque = true;
                    continue;
                }
                if e.scalars.iter().any(|s| early_fields.contains(s)) {
                    early_other = true;
                }
                for (k, slot) in &e.setters {
                    if *slot == config.var_slot {
                        var_set = true;
                        match site
                            .args
                            .get(*k)
                            .and_then(|v| types.value(module, name, f, v))
                        {
                            Some(t) => {
                                p.vars.insert(t);
                            }
                            None => p.var_unknown = true,
                        }
                    } else if is_animation_field(slot) {
                        match call.args.get(*k) {
                            Some(Arg::Function(g)) => {
                                p.slots.entry(slot.clone()).or_default().insert(g.clone());
                            }
                            _ => {
                                p.unknown_slots.insert(slot.clone());
                            }
                        }
                    }
                }
                for (slot, g) in &e.constants {
                    if *slot == config.var_slot {
                        var_set = true;
                        p.var_unknown = true;
                    } else if is_animation_field(slot) {
                        match g {
                            Some(g) => {
                                p.slots.entry(slot.clone()).or_default().insert(g.clone());
                            }
                            None => {
                                p.unknown_slots.insert(slot.clone());
                            }
                        }
                    }
                }
            }
        }
        p.initialised = !inits.is_empty();
        if !var_set {
            if p.initialised {
                p.vars.insert("null".into()); // zeroed by init
            } else {
                p.var_unknown = true;
            }
        }
        // `early_apply` is off at every start: set off after every init
        // that precedes the start (instruction order; any other write
        // makes it unknown).
        p.never_early = !early_other
            && !starts.is_empty()
            && starts.iter().all(|&s| {
                early_off
                    .iter()
                    .any(|&e| e < s && !inits.iter().any(|&i| e < i && i < s))
            });
        p
    }

    /// Whether `deleted` (a function in the `deleted_cb` field) can run for
    /// a removal with `key`.
    pub fn admits_deleted(&self, key: &AnimationKey, deleted: &str, module: &Module) -> bool {
        let Some(config) = &self.config else {
            return true;
        };
        self.admits_origins(key, self.origins(&config.deleted_slot, deleted), module)
    }

    /// The origins of `g` in field `slot` (none: never stored there).
    pub fn origins(&self, slot: &str, g: &str) -> Option<&Origins> {
        self.landings.get(slot)?.get(g)
    }

    /// Whether an animation of one of `origins` can match `key`.
    pub fn admits_origins(
        &self,
        key: &AnimationKey,
        origins: Option<&Origins>,
        module: &Module,
    ) -> bool {
        let Some(config) = &self.config else {
            return true;
        };
        if *key == AnimationKey::Nothing {
            return false;
        }
        // A function no LVGL code stores here holds a callback installed
        // through `sys` (that code's obligation): no edge, as for slots.
        origins.into_iter().flatten().any(|o| match o {
            None => true,
            Some(p) => self.profiles[*p].admits(key, config, module),
        })
    }

    /// The key of the removal at call `n` of `f` (`None`: any animation).
    pub fn delete_key(
        &self,
        module: &Module,
        f_name: &str,
        f: &Function,
        n: usize,
    ) -> Option<AnimationKey> {
        let config = self.config.as_ref()?;
        let call = &f.calls[n];
        if call.callee != config.delete {
            return None;
        }
        let exec = match call.args.get(config.delete_exec_arg) {
            Some(Arg::Function(e)) => Some(e.clone()),
            _ => None,
        };
        let site = f
            .held
            .sites
            .iter()
            .find(|s| matches!(s.kind, SiteKind::Call(m) if m == n))?;
        // `var == NULL` matches every animation (lv_anim.c:190).
        let var = site
            .args
            .get(config.delete_var_arg)
            .and_then(|v| self.types.value(module, f_name, f, v))
            .filter(|t| t != "null");
        if exec.is_none() && var.is_none() {
            return None;
        }
        Some(AnimationKey::Matching { exec, var })
    }

    /// The keys of the replacements `lv_anim_start` makes at call `n` of
    /// `f`, one per `exec_cb` the started local can have (`None`: any).
    pub fn start_keys(&self, f_name: &str, f: &Function, n: usize) -> Option<Vec<AnimationKey>> {
        let config = self.config.as_ref()?;
        if f.calls[n].callee != config.start {
            return None;
        }
        let site = f
            .held
            .sites
            .iter()
            .find(|s| matches!(s.kind, SiteKind::Call(m) if m == n))?;
        let Some(Val::Local(local)) = site.args.first() else {
            return None;
        };
        let p = &self.profiles[*self.by_local.get(&(f_name.to_owned(), local.clone()))?];
        if p.opaque {
            return None;
        }
        if p.never_early {
            return Some(vec![AnimationKey::Nothing]);
        }
        if !p.initialised || p.unknown_slots.contains(&config.exec_slot) {
            return None;
        }
        let execs = p.slots.get(&config.exec_slot).cloned().unwrap_or_default();
        if execs.is_empty() {
            // No `exec_cb`: nothing is replaced (lv_anim.c:836, 849).
            return Some(vec![AnimationKey::Nothing]);
        }
        let var = match (p.var_unknown, p.vars.len()) {
            (false, 1) => p.vars.iter().next().cloned(),
            _ => None,
        };
        Some(
            execs
                .into_iter()
                .map(|e| AnimationKey::Matching {
                    exec: Some(e),
                    var: var.clone(),
                })
                .collect(),
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const TYPES: &str = r#"
%struct.A = type { ptr, i32 }
%struct.B = type <{ %struct.A, i32 }>
%struct.D = type { i32 }
%struct.U = type { %union.anon }
%struct.C = type { [4 x i8], ptr }
define dso_local void @uses_b(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.B, ptr %3, i32 0, i32 1
  ret void
}
define dso_local void @forwards(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  call void @uses_b(ptr noundef %3)
  ret void
}
define dso_local void @conflict(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.D, ptr %3, i32 0, i32 0
  %5 = load ptr, ptr %2, align 8
  call void @uses_b(ptr noundef %5)
  ret void
}
define dso_local void @opaque_use(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.U, ptr %3, i32 0, i32 0
  ret void
}
"#;

    /// A miniature `lv_anim.c` and its users, compiled as at `-O0`.
    pub(crate) const ANIMATIONS: &str = r#"
%struct._lv_anim_t = type { ptr, ptr, ptr, ptr, ptr, ptr, i8 }
%struct.A = type { ptr, i32 }
%struct.D = type { i32 }
define dso_local void @lv_anim_init(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %3, i32 0, i32 6
  store i8 1, ptr %4, align 8
  ret void
}
define dso_local void @lv_anim_set_var(ptr noundef %0, ptr noundef %1) {
  %3 = alloca ptr, align 8
  %4 = alloca ptr, align 8
  store ptr %0, ptr %3, align 8
  store ptr %1, ptr %4, align 8
  %5 = load ptr, ptr %4, align 8
  %6 = load ptr, ptr %3, align 8
  %7 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %6, i32 0, i32 0
  store ptr %5, ptr %7, align 8
  ret void
}
define dso_local void @lv_anim_set_exec_cb(ptr noundef %0, ptr noundef %1) {
  %3 = alloca ptr, align 8
  %4 = alloca ptr, align 8
  store ptr %0, ptr %3, align 8
  store ptr %1, ptr %4, align 8
  %5 = load ptr, ptr %4, align 8
  %6 = load ptr, ptr %3, align 8
  %7 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %6, i32 0, i32 1
  store ptr %5, ptr %7, align 8
  ret void
}
define dso_local void @lv_anim_set_deleted_cb(ptr noundef %0, ptr noundef %1) {
  %3 = alloca ptr, align 8
  %4 = alloca ptr, align 8
  store ptr %0, ptr %3, align 8
  store ptr %1, ptr %4, align 8
  %5 = load ptr, ptr %4, align 8
  %6 = load ptr, ptr %3, align 8
  %7 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %6, i32 0, i32 5
  store ptr %5, ptr %7, align 8
  ret void
}
define dso_local void @lv_anim_set_early_apply(ptr noundef %0, i1 noundef zeroext %1) {
  %3 = alloca ptr, align 8
  %4 = alloca i8, align 1
  store ptr %0, ptr %3, align 8
  %5 = zext i1 %1 to i8
  store i8 %5, ptr %4, align 1
  %6 = load i8, ptr %4, align 1
  %7 = load ptr, ptr %3, align 8
  %8 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %7, i32 0, i32 6
  store i8 %6, ptr %8, align 8
  ret void
}
define dso_local ptr @lv_anim_start(ptr noundef %0) {
  %2 = alloca ptr, align 8
  %3 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %4 = load ptr, ptr %2, align 8
  call void @remove_concurrent_anims(ptr noundef %4)
  %5 = call ptr @lv_malloc(i64 noundef 56)
  store ptr %5, ptr %3, align 8
  %6 = load ptr, ptr %3, align 8
  %7 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %6, i32 0, i32 1
  %8 = load ptr, ptr %7, align 8
  %9 = load ptr, ptr %3, align 8
  %10 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %9, i32 0, i32 0
  %11 = load ptr, ptr %10, align 8
  call void %8(ptr noundef %11, i32 noundef 0)
  %12 = load ptr, ptr %3, align 8
  %13 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %12, i32 0, i32 2
  %14 = load ptr, ptr %13, align 8
  ret ptr %12
}
define internal void @remove_concurrent_anims(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %3, i32 0, i32 5
  %5 = load ptr, ptr %4, align 8
  call void %5(ptr noundef %3)
  call void @lv_free(ptr noundef %3)
  ret void
}
@anim_head = internal global ptr null, align 8
define dso_local zeroext i1 @lv_anim_delete(ptr noundef %0, ptr noundef %1) {
  %3 = alloca ptr, align 8
  store ptr %0, ptr %3, align 8
  %4 = load ptr, ptr @anim_head, align 8
  call void @remove_anim(ptr noundef %4)
  ret i1 true
}
define internal void @remove_anim(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %3, i32 0, i32 5
  %5 = load ptr, ptr %4, align 8
  call void %5(ptr noundef %3)
  call void @lv_free(ptr noundef %3)
  ret void
}
define dso_local ptr @lv_malloc(i64 noundef %0) {
  ret ptr null
}
define dso_local void @lv_free(ptr noundef %0) {
  ret void
}
define dso_local void @sink() {
  ret void
}
define internal void @exec_a(ptr noundef %0, i32 noundef %1) {
  %3 = alloca ptr, align 8
  store ptr %0, ptr %3, align 8
  %4 = load ptr, ptr %3, align 8
  %5 = getelementptr inbounds nuw %struct.A, ptr %4, i32 0, i32 1
  %6 = call i32 @lv_obj_send_event(ptr noundef null, i32 noundef 15, ptr noundef null)
  ret void
}
define internal void @exec_b(ptr noundef %0, i32 noundef %1) {
  ret void
}
define internal void @deleted_bad(ptr noundef %0) {
  call void @sink()
  ret void
}
define dso_local i32 @lv_obj_send_event(ptr noundef %0, i32 noundef %1, ptr noundef %2) {
  ret i32 0
}
define dso_local void @lv_setup_a(ptr noundef %0) {
  %2 = alloca ptr, align 8
  %3 = alloca %struct._lv_anim_t, align 8
  store ptr %0, ptr %2, align 8
  call void @lv_anim_init(ptr noundef %3)
  %4 = load ptr, ptr %2, align 8
  %6 = getelementptr inbounds nuw %struct.A, ptr %4, i32 0, i32 1
  call void @lv_anim_set_var(ptr noundef %3, ptr noundef %4)
  call void @lv_anim_set_exec_cb(ptr noundef %3, ptr noundef @exec_a)
  %5 = call ptr @lv_anim_start(ptr noundef %3)
  ret void
}
define dso_local void @lv_setup_bad(ptr noundef %0) {
  %2 = alloca %struct._lv_anim_t, align 8
  call void @lv_anim_init(ptr noundef %2)
  call void @lv_anim_set_exec_cb(ptr noundef %2, ptr noundef @exec_b)
  call void @lv_anim_set_deleted_cb(ptr noundef %2, ptr noundef @deleted_bad)
  %3 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %2, i32 0, i32 3
  store i32 7, ptr %3, align 8
  %4 = call ptr @lv_anim_start(ptr noundef %2)
  ret void
}
define dso_local void @lv_setup_quiet(ptr noundef %0) {
  %2 = alloca %struct._lv_anim_t, align 8
  call void @lv_anim_init(ptr noundef %2)
  call void @lv_anim_set_early_apply(ptr noundef %2, i1 noundef zeroext false)
  call void @lv_anim_set_exec_cb(ptr noundef %2, ptr noundef @exec_b)
  call void @lv_anim_set_deleted_cb(ptr noundef %2, ptr noundef @deleted_bad)
  %3 = call ptr @lv_anim_start(ptr noundef %2)
  ret void
}
define dso_local void @lv_setup_escaping(ptr noundef %0) {
  %2 = alloca ptr, align 8
  %3 = alloca %struct._lv_anim_t, align 8
  store ptr %0, ptr %2, align 8
  call void @lv_anim_init(ptr noundef %3)
  %4 = load ptr, ptr %2, align 8
  store ptr %3, ptr %4, align 8
  call void @lv_anim_set_exec_cb(ptr noundef %3, ptr noundef @exec_b)
  %5 = call ptr @lv_anim_start(ptr noundef %3)
  ret void
}
define dso_local void @lv_stop_a(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = call zeroext i1 @lv_anim_delete(ptr noundef %3, ptr noundef @exec_a)
  ret void
}
define dso_local void @lv_stop_b(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = call zeroext i1 @lv_anim_delete(ptr noundef %3, ptr noundef @exec_b)
  ret void
}
define dso_local void @lv_stop_all(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = call zeroext i1 @lv_anim_delete(ptr noundef %3, ptr noundef null)
  ret void
}
define dso_local void @lv_stop_d(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.D, ptr %3, i32 0, i32 0
  %5 = load ptr, ptr %2, align 8
  %6 = call zeroext i1 @lv_anim_delete(ptr noundef %5, ptr noundef null)
  ret void
}
"#;

    pub(crate) const ANIMATION_CONFIG: &str = r#"
sinks = ["sink"]
object_sender = "lv_obj_send_event"
free_primitives = { lv_free = 0 }
alloc_primitives = ["lv_malloc"]
[send]
lv_obj_send_event = 1
[caller_slots]
"_lv_anim_t.1" = "immediate"
"_lv_anim_t.2" = "immediate"
[animation]
structure = "_lv_anim_t"
start = "lv_anim_start"
init = "lv_anim_init"
delete = "lv_anim_delete"
delete_var_arg = 0
delete_exec_arg = 1
delete_carry = ["lv_anim_delete", "t::remove_anim"]
start_carry = ["lv_anim_start", "t::remove_concurrent_anims"]
var_slot = "_lv_anim_t.0"
exec_slot = "_lv_anim_t.1"
deleted_slot = "_lv_anim_t.5"
early_apply_setter = "lv_anim_set_early_apply"
"#;

    pub(crate) fn animation_module() -> Module {
        let mut m = Module::default();
        m.add_ir("t", ANIMATIONS, &[]);
        m
    }

    #[test]
    fn profiles_record_what_each_local_animation_is() {
        use super::super::graph::{AuditConfig, Graph};
        let m = animation_module();
        let config: AuditConfig = toml::from_str(ANIMATION_CONFIG).unwrap();
        let graph = Graph::build(&m, &config).unwrap();
        let profile = |f: &str| {
            graph
                .animations
                .profiles
                .iter()
                .find(|p| p.function == f)
                .unwrap()
        };
        let a = profile("lv_setup_a");
        assert!(!a.opaque && a.initialised && !a.never_early);
        assert_eq!(a.vars, BTreeSet::from(["A".to_owned()]));
        // A scalar written into a field leaves the profile readable.
        let bad = profile("lv_setup_bad");
        assert!(!bad.opaque);
        assert_eq!(bad.vars, BTreeSet::from(["null".to_owned()]));
        assert!(profile("lv_setup_quiet").never_early);
        // Its address stored elsewhere: anything can happen to it.
        assert!(profile("lv_setup_escaping").opaque);
    }

    #[test]
    fn struct_prefixes_decide_pointer_equality() {
        let mut m = Module::default();
        m.add_ir("t", TYPES, &[]);
        // A packed struct starting with A can share A's address.
        assert!(compatible("A", "B", &m));
        assert!(compatible("B", "A", &m));
        // Different roots cannot.
        assert!(!compatible("A", "D", &m));
        assert!(!compatible("D", "null", &m));
        // A union or a byte buffer at the start can hold anything.
        assert!(compatible("U", "D", &m));
        assert!(compatible("C", "A", &m));
        // An unknown type too.
        assert!(compatible("Missing", "D", &m));
        assert_eq!(root(&m, "B"), "A");
        assert_eq!(root(&m, "U"), "*");
    }

    #[test]
    fn parameter_types_follow_uses_and_forwarding() {
        let mut m = Module::default();
        m.add_ir("t", TYPES, &[]);
        let types = Types::new(&m);
        // Generalised to the root of the first-member chain.
        assert_eq!(types.param("uses_b", 0).as_deref(), Some("A"));
        assert_eq!(types.param("forwards", 0).as_deref(), Some("A"));
        // Used as two unrelated roots, or as a union: no type.
        assert_eq!(types.param("conflict", 0), None);
        assert_eq!(types.param("opaque_use", 0), None);
        assert_eq!(types.param("uses_b", 1), None);
    }
}
