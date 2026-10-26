//! The call graph and the classification it gives every function.
//!
//! Edges:
//! - `Call`: a direct call;
//! - `Indirect`: an indirect call reaches the functions stored in its slot
//!   (struct field) anywhere in LVGL, or passed by callers for a called
//!   parameter; when the target is not a slot or a parameter, every
//!   address-taken function of the same signature (except event handlers);
//! - `Send`: an event send with a constant code reaches the handlers
//!   registered for that code (or for every code); any other send reaches
//!   every handler;
//! - `Schedule`: passing a function as an argument (other than an event
//!   handler registration) or storing its address: it may run later, from
//!   `lv_timer_handler`.
//!
//! A function that reaches a sink through `Call`, `Indirect` and `Send`
//! edges is exclusive; one that needs a `Schedule` edge to reach a deleting
//! sink is raw (it schedules a deletion that runs without the crate's
//! checks); anything else is shared.

use super::identity::{AnimationConfig, AnimationKey, Animations};
use super::ir::{Arg, Module, Stored, Target};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// `audit.toml`: how the audit reads LVGL.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditConfig {
    /// Functions that free objects or memory objects point to, or reorder
    /// or shrink children arrays (`unit::name` for static functions).
    pub sinks: Vec<String>,
    /// The sinks that delete objects: an exclusive call that can reach one
    /// may delete its own receiver, so its `ObjMut` method consumes it.
    #[serde(default)]
    pub deleting_sinks: Vec<String>,
    /// Struct type of globals emitted with a literal type, by name pattern.
    #[serde(default)]
    pub literal_globals: BTreeMap<String, String>,
    /// Fields whose callback runs for the instance the caller just filled
    /// (`lv_anim_start` calls the animation's own `exec_cb`): a callback
    /// stored in them is attributed to the function that stores it, as a
    /// call (`"immediate"`) or as scheduled work (`"later"`), not to every
    /// indirect call through the field.
    #[serde(default)]
    pub caller_slots: BTreeMap<String, CallerSlot>,
    /// Animation functions and fields, to tell animations apart
    /// (`identity.rs`).
    #[serde(default)]
    pub animation: Option<AnimationConfig>,
    /// Calls that cannot happen in a calling context, with the reason.
    #[serde(default)]
    pub infeasible: BTreeMap<String, Infeasible>,
    /// Event senders: the index of the event code argument.
    #[serde(default)]
    pub send: BTreeMap<String, usize>,
    /// Event handler registrations: `[callback index, code index]`.
    #[serde(default)]
    pub register_event: BTreeMap<String, [usize; 2]>,
    /// Functions whose indirect calls run event handlers; the handlers are
    /// connected at the send sites instead.
    #[serde(default)]
    pub event_dispatch: Vec<String>,
    /// Widgets lvgl-rust does not wrap (deprecated): their handlers exist
    /// only on objects created through `sys`, whose code carries the
    /// obligation, and are left out of event sends.
    #[serde(default)]
    pub unwrapped_widgets: Vec<String>,
    /// Widgets that send events to objects of other widgets they neither
    /// create nor are created by (the keyboard to its textarea).
    #[serde(default)]
    pub peers: BTreeMap<String, Vec<String>>,
    /// Hand decisions that override the graph, with a reason.
    #[serde(default)]
    pub exceptions: BTreeMap<String, Exception>,
    /// Functions to omit from the table.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// The function safe code sends object events with (`lv_obj_send_event`).
    #[serde(default)]
    pub object_sender: Option<String>,
    /// Functions that return or dereference the event parameter: a handler
    /// that calls one under a code reads that code's parameter.
    #[serde(default)]
    pub param_getters: Vec<String>,
    /// Getters that check the parameter for NULL before reading it: calls
    /// to them are not reads (and are not followed).
    #[serde(default)]
    pub param_checked_getters: Vec<String>,
    /// Event codes whose handlers run where only queries make sense (draw
    /// and query codes): a hold across them restricts their handlers
    /// instead of making the freeing function exclusive.
    #[serde(default)]
    pub blockable_codes: Vec<String>,
    /// Hand decisions on event codes, with a reason.
    #[serde(default)]
    pub event_policy: BTreeMap<String, EventPolicy>,
    /// The functions that release memory, with the index of the pointer
    /// argument they free (`lv_free = 0`, `lv_realloc = 0`).
    #[serde(default)]
    pub free_primitives: BTreeMap<String, usize>,
    /// Functions that return fresh memory (`lv_malloc`): freeing what they
    /// returned, before it is published, cannot invalidate a hold.
    #[serde(default)]
    pub alloc_primitives: Vec<String>,
    /// Functions that free memory they allocated only on paths where it
    /// was not published (the analysis does not follow paths), with the
    /// reason.
    #[serde(default)]
    pub unpublished: BTreeMap<String, String>,
    /// Holds that a runtime check of the crate makes harmless.
    #[serde(default)]
    pub guarded: BTreeMap<String, Guarded>,
}

/// A hold made harmless by a runtime check (`[guarded.<name>]`): hazards
/// on these regions through these freers are not counted.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guarded {
    /// Regions as the analysis prints them (`Copy(_lv_event_list_t.0/*)`),
    /// `*` matching anything.
    pub regions: Vec<String>,
    /// The freeing functions the check covers (empty: any).
    #[serde(default)]
    pub freers: Vec<String>,
    /// The frames whose holds the check covers (empty: any).
    #[serde(default)]
    pub holders: Vec<String>,
    /// Functions through which the free must happen (its witness chain
    /// names one of them; empty: any).
    #[serde(default)]
    pub via: Vec<String>,
    /// The check, where it lives.
    pub check: String,
    pub why: String,
}

/// A hand decision on an event code (`[event_policy.LV_EVENT_X]`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventPolicy {
    /// `"shared"`, `"restricted"` or `"none"`.
    #[serde(default)]
    pub handler: Option<String>,
    /// `"no"` to forbid sending the code from safe code.
    #[serde(default)]
    pub sendable: Option<String>,
    pub why: String,
}

/// `[infeasible.<name>]`: inside calls from `context` (`"caller ->
/// callee"`), the callee never makes the listed calls (a condition on its
/// arguments that the caller always meets). The callee is specialised for
/// that caller without them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Infeasible {
    pub context: String,
    pub calls: Vec<String>,
    pub why: String,
}

/// Splits `"caller -> callee"`.
fn call_pair(text: &str) -> Result<(&str, &str), String> {
    text.split_once(" -> ")
        .map(|(a, b)| (a.trim(), b.trim()))
        .ok_or_else(|| format!("audit.toml: expected \"caller -> callee\", got {text:?}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallerSlot {
    Immediate,
    Later,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exception {
    pub access: String,
    pub why: String,
    #[serde(default)]
    pub consumes: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Edge {
    Call,
    Indirect,
    Send,
    Schedule,
}

/// An edge, with what it does to the event code being dispatched.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Out {
    pub(crate) to: String,
    pub(crate) kind: Edge,
    /// Taken only for these codes (`None`: any).
    pub(crate) guard: Option<Vec<i64>>,
    /// Starts dispatching this code.
    pub(crate) sets: Option<i64>,
    /// Forwards the event being dispatched (its code stays known).
    pub(crate) carry: bool,
    /// An indirect call through the animation `deleted_cb` field.
    pub(crate) deleted_slot: bool,
}

impl Out {
    fn new(to: impl Into<String>, kind: Edge) -> Self {
        Out {
            to: to.into(),
            kind,
            guard: None,
            sets: None,
            carry: false,
            deleted_slot: false,
        }
    }
}

/// `dropdown` for `widgets/dropdown/lv_dropdown`; `None` for core code.
fn widget_of(unit: &str) -> Option<&str> {
    unit.strip_prefix("widgets/")?.split('/').next()
}

/// The edges of each call site of a function: `(calls, indirect)`.
pub(crate) type SiteOuts = (Vec<Vec<Out>>, Vec<Vec<Out>>);

pub struct Graph {
    pub(crate) edges: BTreeMap<String, Vec<Out>>,
    /// The edges of each call site: `(calls, indirect)` of every function,
    /// indexed like `Function::calls` and `Function::indirect`.
    pub(crate) site_outs: BTreeMap<String, SiteOuts>,
    /// Functions stored in each of `caller_slots` (their indirect calls
    /// have no edges: the installers call them).
    pub(crate) caller_slot_funcs: BTreeMap<String, BTreeSet<String>>,
    slots: BTreeMap<String, BTreeSet<String>>,
    sinks: BTreeSet<String>,
    deleting: BTreeSet<String>,
    /// Event handlers and the codes they are registered for (`None`: any).
    pub handlers: BTreeMap<String, BTreeSet<Option<i64>>>,
    /// Animation profiles and where callbacks land (`identity.rs`).
    pub animations: Animations,
    /// Nodes specialised for an animation key.
    pub animation_nodes: BTreeMap<String, AnimationKey>,
}

/// Every edge of `from`, in `edges` and in its per-site lists.
fn outs_mut<'a>(
    edges: &'a mut BTreeMap<String, Vec<Out>>,
    site_outs: &'a mut BTreeMap<String, SiteOuts>,
    from: &str,
) -> impl Iterator<Item = &'a mut Out> {
    let plain = edges.get_mut(from).into_iter().flatten();
    let sites = site_outs
        .get_mut(from)
        .into_iter()
        .flat_map(|(c, i)| c.iter_mut().chain(i.iter_mut()))
        .flatten();
    plain.chain(sites)
}

/// Adds node `special`: a copy of `base` whose edges `map` rewrites or
/// drops.
fn specialise(
    edges: &mut BTreeMap<String, Vec<Out>>,
    site_outs: &mut BTreeMap<String, SiteOuts>,
    base: &str,
    special: &str,
    map: impl Fn(&Out) -> Option<Out>,
) {
    let plain: Vec<Out> = edges
        .get(base)
        .into_iter()
        .flatten()
        .filter_map(&map)
        .collect();
    edges.insert(special.to_owned(), plain);
    if let Some((c, i)) = site_outs.get(base) {
        let rewrite = |sites: &Vec<Vec<Out>>| -> Vec<Vec<Out>> {
            sites
                .iter()
                .map(|s| s.iter().filter_map(&map).collect())
                .collect()
        };
        let special_sites = (rewrite(c), rewrite(i));
        site_outs.insert(special.to_owned(), special_sites);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Shared,
    /// Reaches a sink: the witness path.
    Exclusive(Vec<String>),
    /// Schedules a deletion: the witness path.
    Raw(Vec<String>),
}

type ParamKey = (String, usize);

impl Graph {
    pub fn build(module: &Module, config: &AuditConfig) -> Result<Graph, String> {
        // Event handlers, by registered code, and the widgets that register
        // them (`None`: core code, on arbitrary objects).
        let mut handlers: BTreeMap<String, BTreeSet<Option<i64>>> = BTreeMap::new();
        let mut registered_by: BTreeMap<String, BTreeSet<Option<String>>> = BTreeMap::new();
        for f in module.functions.values() {
            for call in &f.calls {
                if let Some(&[cb, code]) = config.register_event.get(&call.callee)
                    && let Some(Arg::Function(handler)) = call.args.get(cb)
                {
                    let code = match call.args.get(code) {
                        Some(Arg::Int(c)) if *c != 0 => Some(*c),
                        Some(Arg::Ints(_)) => None,
                        // A trigger chosen by the registrar's caller:
                        // the handler runs only if that registrar is
                        // used, which then schedules it (below).
                        Some(Arg::Param(_)) => continue,
                        _ => None, // LV_EVENT_ALL or computed
                    };
                    if widget_of(&f.unit)
                        .is_some_and(|w| config.unwrapped_widgets.iter().any(|u| u == w))
                    {
                        continue;
                    }
                    handlers.entry(handler.clone()).or_default().insert(code);
                    registered_by
                        .entry(handler.clone())
                        .or_default()
                        .insert(widget_of(&f.unit).map(str::to_owned));
                }
            }
        }

        // Slots and parameter summaries.
        let mut slot_funcs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (slot, g) in &module.global_slots {
            slot_funcs
                .entry(slot.clone())
                .or_default()
                .insert(g.clone());
        }
        let mut param_slots: BTreeMap<ParamKey, Vec<String>> = BTreeMap::new();
        let mut param_called: BTreeSet<ParamKey> = BTreeSet::new();
        let mut passthrough: BTreeMap<ParamKey, Vec<ParamKey>> = BTreeMap::new();
        for (name, f) in &module.functions {
            for (slot, stored) in &f.stores {
                match stored {
                    Stored::Function(g) => {
                        slot_funcs
                            .entry(slot.clone())
                            .or_default()
                            .insert(g.clone());
                    }
                    Stored::Param(k) => param_slots
                        .entry((name.clone(), *k))
                        .or_default()
                        .push(slot.clone()),
                }
            }
            for indirect in &f.indirect {
                if let Target::Param(k) = indirect.target {
                    param_called.insert((name.clone(), k));
                }
            }
            for call in &f.calls {
                for (i, arg) in call.args.iter().enumerate() {
                    if let Arg::Param(k) = arg {
                        passthrough
                            .entry((name.clone(), *k))
                            .or_default()
                            .push((call.callee.clone(), i));
                    }
                }
            }
        }
        // Functions passed as arguments flow into the callee's slots, or are
        // called by it (an indirect edge from the caller).
        // A function whose address only ever lands in known slots can only
        // be called through those slots; the others escape, and unresolved
        // indirect calls may reach them.
        let mut escaped: BTreeSet<String> = BTreeSet::new();
        let mut caller_slot_funcs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for f in module.functions.values() {
            for (slot, stored) in &f.stores {
                if let Stored::Function(g) = stored
                    && config.caller_slots.contains_key(slot)
                {
                    caller_slot_funcs
                        .entry(slot.clone())
                        .or_default()
                        .insert(g.clone());
                }
            }
        }
        let mut flow_edges: Vec<(String, String)> = Vec::new();
        let caller_slot = |slot: &str| config.caller_slots.get(slot).copied();
        let mut caller_edges: Vec<(String, String, Edge)> = Vec::new();
        for (name, f) in &module.functions {
            escaped.extend(f.refs.iter().cloned());
            for call in &f.calls {
                for (i, arg) in call.args.iter().enumerate() {
                    if let Arg::Function(g) = arg {
                        let mut seen = BTreeSet::new();
                        let mut stack = vec![(call.callee.clone(), i)];
                        while let Some(key) = stack.pop() {
                            if !seen.insert(key.clone()) {
                                continue;
                            }
                            if !module.functions.contains_key(&key.0) {
                                escaped.insert(g.clone()); // an unknown callee keeps it
                            }
                            for slot in param_slots.get(&key).into_iter().flatten() {
                                if caller_slot(slot).is_some() {
                                    caller_slot_funcs
                                        .entry(slot.clone())
                                        .or_default()
                                        .insert(g.clone());
                                }
                                match caller_slot(slot) {
                                    Some(CallerSlot::Immediate) => {
                                        caller_edges.push((name.clone(), g.clone(), Edge::Indirect))
                                    }
                                    Some(CallerSlot::Later) => {
                                        caller_edges.push((name.clone(), g.clone(), Edge::Schedule))
                                    }
                                    None => {
                                        slot_funcs
                                            .entry(slot.clone())
                                            .or_default()
                                            .insert(g.clone());
                                    }
                                }
                            }
                            if param_called.contains(&key) {
                                flow_edges.push((name.clone(), g.clone()));
                            }
                            stack.extend(passthrough.get(&key).into_iter().flatten().cloned());
                        }
                    }
                }
            }
        }
        escaped.extend(module.global_refs.iter().cloned());
        let taken = escaped;
        let animations = Animations::new(
            module,
            config.animation.as_ref(),
            &param_slots,
            &passthrough,
        )?;
        // Removals specialised by the animations they can reach.
        let mut animation_requests: BTreeSet<(String, AnimationKey)> = BTreeSet::new();
        let mut by_signature: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for name in &taken {
            if handlers.contains_key(name) {
                continue;
            }
            if let Some(f) = module.functions.get(name) {
                by_signature.entry(&f.signature).or_default().push(name);
            }
        }

        // Event parameters: passed to `lv_event_get_code`, forwarded to
        // another function's event parameter, or the event of a handler
        // (parameter 0) or class callback (parameter 1).
        let mut event_params: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for (name, f) in &module.functions {
            event_params
                .entry(name.clone())
                .or_default()
                .extend(f.event_params.iter().copied());
        }
        for h in handlers.keys() {
            event_params.entry(h.clone()).or_default().insert(0);
        }
        for (slot, funcs) in &slot_funcs {
            if slot == "_lv_obj_class_t.3" {
                for f in funcs {
                    event_params.entry(f.clone()).or_default().insert(1);
                }
            }
        }
        let forwards = |caller: &str,
                        args: &[Arg],
                        callee: &str,
                        params: &BTreeMap<String, BTreeSet<usize>>|
         -> bool {
            let (Some(mine), Some(theirs)) = (params.get(caller), params.get(callee)) else {
                return false;
            };
            args.iter()
                .enumerate()
                .any(|(j, a)| matches!(a, Arg::Param(k) if mine.contains(k)) && theirs.contains(&j))
        };
        // Propagate backwards: a parameter forwarded to an event parameter
        // is one too.
        for _ in 0..8 {
            let mut added = Vec::new();
            for (name, f) in &module.functions {
                let direct = f.calls.iter().map(|c| (vec![c.callee.clone()], &c.args));
                let indirect = f.indirect.iter().filter_map(|i| match &i.target {
                    Target::Slot(slot) => Some((
                        slot_funcs
                            .get(slot)
                            .into_iter()
                            .flatten()
                            .cloned()
                            .collect(),
                        &i.args,
                    )),
                    _ => None,
                });
                for (callees, args) in direct.chain(indirect) {
                    for callee in callees {
                        let Some(theirs) = event_params.get(&callee) else {
                            continue;
                        };
                        for (j, a) in args.iter().enumerate() {
                            if let Arg::Param(k) = a
                                && theirs.contains(&j)
                                && !event_params[name].contains(k)
                            {
                                added.push((name.clone(), *k));
                            }
                        }
                    }
                }
            }
            if added.is_empty() {
                break;
            }
            for (name, k) in added {
                event_params.entry(name).or_default().insert(k);
            }
        }
        // The event machinery: senders build the event from their code, and
        // dispatchers forward it.
        let machinery = |name: &str| {
            config.send.contains_key(name) || config.event_dispatch.iter().any(|d| d == name)
        };

        // Which widgets' objects a widget's code can send to: its own, those
        // of widgets it creates, those of widgets that create it (it may be
        // their part), and its listed peers.
        let widgets: BTreeSet<&str> = module
            .functions
            .values()
            .filter_map(|f| widget_of(&f.unit))
            .collect();
        let mut creates: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for f in module.functions.values() {
            if let Some(w) = widget_of(&f.unit) {
                for call in &f.calls {
                    let created = call
                        .callee
                        .strip_prefix("lv_")
                        .and_then(|c| c.strip_suffix("_create"));
                    if let Some(created) = created.filter(|c| widgets.contains(c)) {
                        creates.entry(w).or_default().insert(created);
                    }
                }
            }
        }
        let reaches = |sender: Option<&str>, registrar: Option<&str>| -> bool {
            match (sender, registrar) {
                (None, _) | (_, None) => true,
                (Some(s), Some(r)) => {
                    s == r
                        || creates.get(r).is_some_and(|c| c.contains(s))
                        || creates.get(s).is_some_and(|c| c.contains(r))
                        || config
                            .peers
                            .get(s)
                            .is_some_and(|p| p.iter().any(|x| x == r))
                }
            }
        };
        let scoped = |handler: &str, sender: Option<&str>| -> bool {
            registered_by
                .get(handler)
                .into_iter()
                .flatten()
                .any(|r| reaches(sender, r.as_deref()))
        };
        let all_handlers: Vec<String> = handlers.keys().cloned().collect();
        let handlers_for = |code: Option<i64>, sender: Option<&str>| -> Vec<String> {
            handlers
                .iter()
                .filter(|(_, codes)| match code {
                    Some(code) => codes.contains(&None) || codes.contains(&Some(code)),
                    None => true,
                })
                .filter(|(h, _)| scoped(h, sender))
                .map(|(h, _)| h.clone())
                .collect()
        };

        let mut edges: BTreeMap<String, Vec<Out>> = BTreeMap::new();
        let mut site_outs: BTreeMap<String, SiteOuts> = BTreeMap::new();
        for (name, f) in &module.functions {
            let out = edges.entry(name.clone()).or_default();
            let sites = site_outs.entry(name.clone()).or_default();
            let dispatch = config.event_dispatch.contains(name);
            let sender = config.send.contains_key(name);
            let in_machinery = machinery(name);
            let sender_widget = widget_of(&f.unit);
            for (n, call) in f.calls.iter().enumerate() {
                let start = out.len();
                if call.callee == "lv_event_get_code" {
                    sites.0.push(Vec::new());
                    continue;
                }
                let carry = in_machinery || forwards(name, &call.args, &call.callee, &event_params);
                let constant_codes = match config.send.get(&call.callee).map(|&i| call.args.get(i))
                {
                    Some(Some(Arg::Int(code))) => Some(vec![*code]),
                    Some(Some(Arg::Ints(codes))) => Some(codes.clone()),
                    _ => None,
                };
                match config.send.get(&call.callee).map(|&i| call.args.get(i)) {
                    // Constant codes: the sender's body, and the handlers
                    // of each code, dispatching it.
                    Some(Some(Arg::Int(_) | Arg::Ints(_))) => {
                        for code in constant_codes.unwrap_or_default() {
                            let mut body = Out::new(format!("{}#body", call.callee), Edge::Call);
                            body.sets = Some(code);
                            body.guard = call.codes.clone();
                            out.push(body);
                            for h in handlers_for(Some(code), sender_widget) {
                                let mut e = Out::new(h, Edge::Send);
                                e.sets = Some(code);
                                e.guard = call.codes.clone();
                                out.push(e);
                            }
                        }
                    }
                    // A computed code from widget code: the sender's body
                    // and that widget's reachable handlers.
                    Some(_) if sender_widget.is_some() => {
                        let mut body = Out::new(format!("{}#body", call.callee), Edge::Call);
                        body.guard = call.codes.clone();
                        out.push(body);
                        for h in handlers_for(None, sender_widget) {
                            let mut e = Out::new(h, Edge::Send);
                            e.guard = call.codes.clone();
                            out.push(e);
                        }
                    }
                    _ => {
                        // An animation removal goes to the removal
                        // specialised for the animations it can match.
                        let keys = match animations.delete_key(module, name, f, n) {
                            Some(key) => Some(vec![key]),
                            None => animations.start_keys(name, f, n),
                        };
                        let targets: Vec<String> = match keys {
                            Some(keys) => keys
                                .into_iter()
                                .map(|key| {
                                    let to = format!("{}{}", call.callee, key.suffix());
                                    animation_requests.insert((call.callee.clone(), key));
                                    to
                                })
                                .collect(),
                            None => vec![call.callee.clone()],
                        };
                        for to in targets {
                            let mut e = Out::new(to, Edge::Call);
                            e.guard = call.codes.clone();
                            e.carry = carry;
                            out.push(e);
                        }
                    }
                }
                if config.event_dispatch.contains(&call.callee) && !sender && !dispatch {
                    for h in &handlers_for(None, sender_widget) {
                        let mut e = Out::new(h.clone(), Edge::Send);
                        e.guard = call.codes.clone();
                        e.carry = carry;
                        out.push(e);
                    }
                }
                let registration = config.register_event.contains_key(&call.callee);
                for arg in &call.args {
                    if let Arg::Function(g) = arg {
                        let immediate = caller_edges.iter().any(|(from, to, kind)| {
                            from == name && to == g && *kind == Edge::Indirect
                        });
                        if !(registration && handlers.contains_key(g)) && !immediate {
                            out.push(Out::new(g.clone(), Edge::Schedule));
                        }
                    }
                }
                sites.0.push(out[start..].to_vec());
            }
            for r in &f.refs {
                out.push(Out::new(r.clone(), Edge::Schedule));
            }
            for (slot, stored) in &f.stores {
                if let Stored::Function(g) = stored {
                    let kind = match caller_slot(slot) {
                        Some(CallerSlot::Immediate) => Edge::Indirect,
                        _ => Edge::Schedule,
                    };
                    out.push(Out::new(g.clone(), kind));
                }
            }
            if dispatch {
                sites.1 = vec![Vec::new(); f.indirect.len()];
            } else {
                for indirect in &f.indirect {
                    let start = out.len();
                    // A slot no LVGL code writes holds NULL or a callback
                    // installed through `sys`, which is that code's
                    // obligation: no edge.
                    let targets: Vec<String> = match &indirect.target {
                        Target::Slot(slot) if caller_slot(slot).is_some() => Vec::new(),
                        Target::Slot(slot) => slot_funcs
                            .get(slot)
                            .into_iter()
                            .flatten()
                            .cloned()
                            .collect(),
                        Target::Param(_) => Vec::new(), // via the callers' flows
                        Target::Unknown => by_signature
                            .get(indirect.signature.as_str())
                            .into_iter()
                            .flatten()
                            .map(|g| (*g).to_owned())
                            .collect(),
                    };
                    let deleted_slot = match (&indirect.target, &config.animation) {
                        (Target::Slot(slot), Some(animation)) => *slot == animation.deleted_slot,
                        _ => false,
                    };
                    for g in targets {
                        let carry =
                            in_machinery || forwards(name, &indirect.args, &g, &event_params);
                        let mut e = Out::new(g.clone(), Edge::Indirect);
                        e.guard = indirect.codes.clone();
                        e.carry = carry;
                        e.deleted_slot = deleted_slot;
                        out.push(e);
                    }
                    sites.1.push(out[start..].to_vec());
                }
            }
        }
        for (from, to) in flow_edges {
            edges
                .entry(from)
                .or_default()
                .push(Out::new(to, Edge::Indirect));
        }
        for (from, to, kind) in &caller_edges {
            edges
                .entry(from.clone())
                .or_default()
                .push(Out::new(to.clone(), *kind));
        }

        // Calls that cannot happen in a context: the callee specialised
        // for that caller.
        for (name, inf) in &config.infeasible {
            let (from, to) = call_pair(&inf.context)?;
            if !edges
                .get(from)
                .is_some_and(|o| o.iter().any(|o| o.to == to))
            {
                return Err(format!(
                    "audit.toml: [infeasible.{name}] {from} does not call {to}"
                ));
            }
            for c in &inf.calls {
                if !edges.get(to).is_some_and(|o| o.iter().any(|o| o.to == *c)) {
                    return Err(format!(
                        "audit.toml: [infeasible.{name}] {to} does not call {c}"
                    ));
                }
            }
            let special = format!("{to}#infeasible({name})");
            specialise(&mut edges, &mut site_outs, to, &special, |o| {
                (!(inf.calls.contains(&o.to) && o.kind == Edge::Call)).then(|| o.clone())
            });
            for o in outs_mut(&mut edges, &mut site_outs, from) {
                if o.to == to {
                    o.to = special.clone();
                }
            }
        }
        // Animation removals: the chain of functions down to the one that
        // runs `deleted_cb`, specialised for each key, with only the deleted
        // callbacks of animations the key can match.
        let mut animation_nodes: BTreeMap<String, AnimationKey> = BTreeMap::new();
        if let Some(animation) = &config.animation {
            for (callee, key) in &animation_requests {
                let chain = if *callee == animation.delete {
                    &animation.delete_carry
                } else {
                    &animation.start_carry
                };
                if chain.first() != Some(callee) {
                    return Err(format!(
                        "audit.toml: [animation] the carry list of {callee} must start with it"
                    ));
                }
                let suffix = key.suffix();
                for x in chain {
                    let special = format!("{x}{suffix}");
                    specialise(&mut edges, &mut site_outs, x, &special, |o| {
                        if o.deleted_slot && !animations.admits_deleted(key, &o.to, module) {
                            return None;
                        }
                        let mut o = o.clone();
                        if chain.contains(&o.to) && matches!(o.kind, Edge::Call | Edge::Indirect) {
                            o.to = format!("{}{suffix}", o.to);
                        }
                        Some(o)
                    });
                    animation_nodes.insert(special, key.clone());
                }
            }
        }

        // A sender's code is its parameter: called on its own (or with a
        // computed code), it reaches every handler.
        for sender in config.send.keys() {
            if let Some(body) = edges.get(sender).cloned() {
                edges.insert(format!("{sender}#body"), body);
                let out = edges.get_mut(sender).unwrap();
                out.extend(all_handlers.iter().map(|h| Out::new(h.clone(), Edge::Send)));
            }
        }
        for out in edges.values_mut() {
            out.sort();
            out.dedup();
        }

        let sinks: BTreeSet<String> = config.sinks.iter().cloned().collect();
        for sink in &sinks {
            if !module.functions.contains_key(sink) {
                return Err(format!(
                    "audit.toml: sink {sink} is not defined in the LVGL sources"
                ));
            }
        }
        let deleting: BTreeSet<String> = config.deleting_sinks.iter().cloned().collect();
        if let Some(extra) = deleting.difference(&sinks).next() {
            return Err(format!("audit.toml: deleting sink {extra} is not a sink"));
        }
        Ok(Graph {
            edges,
            site_outs,
            caller_slot_funcs,
            slots: slot_funcs,
            sinks,
            deleting,
            handlers,
            animations,
            animation_nodes,
        })
    }

    /// The per-site edges of a graph node (specialised nodes have their
    /// own; `#body` nodes use their function's).
    pub(crate) fn site_outs_of(&self, node: &str) -> Option<&SiteOuts> {
        self.site_outs
            .get(node)
            .or_else(|| self.site_outs.get(node.split('#').next().unwrap_or(node)))
    }

    /// Exclusive if a sink is reachable now; raw if the call schedules a
    /// deletion (it runs later from `lv_timer_handler`, without the crate's
    /// structural checks); shared otherwise. A scheduled callback that only
    /// shrinks or reorders runs in that exclusive context and is harmless.
    pub fn verdict(&self, start: &str) -> Verdict {
        // A scheduled deletion makes the call raw even if it is also
        // exclusive: the deletion runs later without the structural checks.
        let all = [Edge::Call, Edge::Indirect, Edge::Send, Edge::Schedule];
        if let Some(path) = self.search_scheduled(start, &all, &self.deleting) {
            return Verdict::Raw(path);
        }
        if let Some(path) = self.path(start, &[Edge::Call, Edge::Indirect, Edge::Send]) {
            return Verdict::Exclusive(path);
        }
        Verdict::Shared
    }

    /// A path to a sink that takes at least one `Schedule` edge.
    fn search_scheduled(
        &self,
        start: &str,
        kinds: &[Edge],
        sinks: &BTreeSet<String>,
    ) -> Option<Vec<String>> {
        type State<'a> = (&'a str, bool);
        let mut previous: BTreeMap<State, State> = BTreeMap::new();
        let first: State = (start, false);
        let mut queue = VecDeque::from([first]);
        let mut seen = BTreeSet::from([first]);
        while let Some(state) = queue.pop_front() {
            let (node, scheduled) = state;
            if sinks.contains(node) && !scheduled {
                continue; // reached now: exclusive, not scheduled
            }
            if scheduled && sinks.contains(node) {
                let mut path = vec![node.to_owned()];
                let mut at = state;
                while let Some(p) = previous.get(&at) {
                    path.push(p.0.to_owned());
                    at = *p;
                }
                path.reverse();
                return Some(path);
            }
            for out in self.edges.get(node).into_iter().flatten() {
                if !kinds.contains(&out.kind) {
                    continue;
                }
                let next: State = (out.to.as_str(), scheduled || out.kind == Edge::Schedule);
                if seen.insert(next) {
                    previous.insert(next, state);
                    queue.push_back(next);
                }
            }
        }
        None
    }

    /// Whether an exclusive call can delete objects (its receiver included).
    pub fn deletes(&self, start: &str) -> bool {
        self.search(
            start,
            None,
            &[Edge::Call, Edge::Indirect, Edge::Send],
            &self.deleting,
        )
        .is_some()
    }

    /// Shortest path from `start` (dispatching `code`, if known) to a sink
    /// through `kinds` edges.
    fn path_from(&self, start: &str, code: Option<i64>, kinds: &[Edge]) -> Option<Vec<String>> {
        self.search(start, code, kinds, &self.sinks)
    }

    fn search(
        &self,
        start: &str,
        code: Option<i64>,
        kinds: &[Edge],
        sinks: &BTreeSet<String>,
    ) -> Option<Vec<String>> {
        type State<'a> = (&'a str, Option<i64>);
        let mut previous: BTreeMap<State, State> = BTreeMap::new();
        let first: State = (start, code);
        let mut queue = VecDeque::from([first]);
        let mut seen = BTreeSet::from([first]);
        while let Some(state) = queue.pop_front() {
            let (node, code) = state;
            if sinks.contains(node) {
                let mut path = vec![node.to_owned()];
                let mut at = state;
                while let Some(p) = previous.get(&at) {
                    path.push(p.0.to_owned());
                    at = *p;
                }
                path.reverse();
                return Some(path);
            }
            for out in self.edges.get(node).into_iter().flatten() {
                if !kinds.contains(&out.kind) {
                    continue;
                }
                if let (Some(code), Some(guard)) = (code, &out.guard)
                    && !guard.contains(&code)
                {
                    continue;
                }
                let next_code = match (out.sets, out.carry) {
                    (Some(c), _) => Some(c),
                    (None, true) => code,
                    (None, false) => None,
                };
                let next: State = (out.to.as_str(), next_code);
                if seen.insert(next) {
                    previous.insert(next, state);
                    queue.push_back(next);
                }
            }
        }
        None
    }

    fn path(&self, start: &str, kinds: &[Edge]) -> Option<Vec<String>> {
        self.path_from(start, None, kinds)
    }

    /// Event codes whose handlers reach a sink (`None`: handlers registered
    /// for every code).
    pub fn deleting_codes(&self) -> BTreeSet<Option<i64>> {
        let mut codes = BTreeSet::new();
        for (handler, registered) in &self.handlers {
            for code in registered {
                if self
                    .path_from(handler, *code, &[Edge::Call, Edge::Indirect, Edge::Send])
                    .is_some()
                {
                    codes.insert(*code);
                }
            }
        }
        codes
    }

    /// Handlers registered for `code` or for every code.
    pub fn handlers_of(&self, code: i64) -> impl Iterator<Item = &str> {
        self.handlers
            .iter()
            .filter(move |(_, codes)| codes.contains(&None) || codes.contains(&Some(code)))
            .map(|(h, _)| h.as_str())
    }

    /// The functions that run when safe code sends `code` to an object
    /// through `sender`: the sender's own body (class event callbacks,
    /// bubbling) and every handler registered for the code.
    fn send_roots(&self, sender: &str, code: i64) -> Vec<String> {
        let mut roots = vec![format!("{sender}#body")];
        roots.extend(self.handlers_of(code).map(str::to_owned));
        roots
    }

    /// A path to a sink when `code` is sent through `sender`, if any.
    pub fn send_deletes(&self, sender: &str, code: i64) -> Option<Vec<String>> {
        let kinds = [Edge::Call, Edge::Indirect, Edge::Send];
        self.send_roots(sender, code)
            .iter()
            .find_map(|root| self.search(root, Some(code), &kinds, &self.sinks))
    }

    /// Whether code that runs for `code` sent through `sender` reads the
    /// event parameter: a call to one of `getters` from a function that has
    /// the event (reached through edges that forward it), in a block that
    /// can run for `code`. Returns the getters reached and a witness.
    pub fn param_reads(
        &self,
        module: &Module,
        sender: &str,
        code: i64,
        getters: &[String],
        checked: &[String],
    ) -> Option<(BTreeSet<String>, Vec<String>)> {
        let mut found: BTreeSet<String> = BTreeSet::new();
        let mut witness: Option<Vec<String>> = None;
        let mut previous: BTreeMap<&str, &str> = BTreeMap::new();
        let roots = self.send_roots(sender, code);
        let mut seen: BTreeSet<&str> = roots.iter().map(String::as_str).collect();
        let mut queue: VecDeque<&str> = roots.iter().map(String::as_str).collect();
        while let Some(node) = queue.pop_front() {
            if getters.iter().chain(checked).any(|g| g == node) {
                continue; // the call itself was counted (or is checked)
            }
            let function = module
                .functions
                .get(node.strip_suffix("#body").unwrap_or(node));
            for call in function.into_iter().flat_map(|f| &f.calls) {
                if getters.contains(&call.callee)
                    && call.codes.as_ref().is_none_or(|c| c.contains(&code))
                {
                    found.insert(call.callee.clone());
                    if witness.is_none() {
                        let mut path = vec![call.callee.clone(), node.to_owned()];
                        let mut at = node;
                        while let Some(p) = previous.get(at) {
                            path.push((*p).to_owned());
                            at = p;
                        }
                        path.reverse();
                        witness = Some(path);
                    }
                }
            }
            // Only edges that forward this event: nested sends make new
            // events, whose parameters their senders provide.
            for out in self.edges.get(node).into_iter().flatten() {
                if !matches!(out.kind, Edge::Call | Edge::Indirect)
                    || !out.carry
                    || out.sets.is_some()
                    || out.guard.as_ref().is_some_and(|g| !g.contains(&code))
                {
                    continue;
                }
                if seen.insert(&out.to) {
                    previous.insert(&out.to, node);
                    queue.push_back(&out.to);
                }
            }
        }
        witness.map(|w| (found, w))
    }

    /// Functions of a slot that reach a sink, with the witness, for review.
    pub fn deleting_slot_functions(&self, slot: &str) -> Vec<(String, Vec<String>)> {
        self.slots
            .get(slot)
            .into_iter()
            .flatten()
            .filter_map(|f| {
                Some((
                    f.clone(),
                    self.path(f, &[Edge::Call, Edge::Indirect, Edge::Send])?,
                ))
            })
            .collect()
    }

    /// The witness from a handler to a sink, for review.
    pub fn handler_path(&self, handler: &str) -> Option<Vec<String>> {
        self.path(handler, &[Edge::Call, Edge::Indirect, Edge::Send])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AuditConfig {
        toml::from_str(
            r#"
sinks = ["tree::obj_delete_core"]
deleting_sinks = ["tree::obj_delete_core"]
event_dispatch = ["lv_event_send"]
[send]
lv_obj_send_event = 1
[register_event]
lv_obj_add_event_cb = [1, 2]
"#,
        )
        .unwrap()
    }

    #[test]
    fn classifies_by_reachable_sinks() {
        let mut m = Module::default();
        m.add_ir(
            "tree",
            r#"
define dso_local void @lv_obj_delete(ptr noundef %0) {
  call void @obj_delete_core(ptr %0)
  ret void
}
define internal void @obj_delete_core(ptr noundef %0) {
  ret void
}
define dso_local void @lv_obj_delete_async(ptr noundef %0) {
  call void @lv_async_call(ptr noundef @async_cb, ptr %0)
  ret void
}
define internal void @async_cb(ptr noundef %0) {
  call void @lv_obj_delete(ptr %0)
  ret void
}
define dso_local void @lv_obj_send_event(ptr noundef %0, i32 noundef %1, ptr noundef %2) {
  call void @lv_event_send(ptr %0)
  ret void
}
define dso_local void @lv_event_send(ptr noundef %0) {
  %3 = getelementptr inbounds nuw %struct._lv_event_dsc_t, ptr %0, i32 0, i32 0
  %4 = load ptr, ptr %3, align 8
  call void %4(ptr noundef %0)
  ret void
}
define dso_local void @lv_msgbox_add_close_button(ptr noundef %0) {
  %2 = call ptr @lv_obj_add_event_cb(ptr %0, ptr noundef @close_cb, i32 noundef 10, ptr null)
  ret void
}
define internal void @close_cb(ptr noundef %0) {
  call void @lv_obj_delete(ptr %0)
  ret void
}
define dso_local void @lv_obj_set_size(ptr noundef %0) {
  call void @lv_obj_send_event(ptr %0, i32 noundef 41, ptr null)
  ret void
}
define dso_local void @lv_obj_click(ptr noundef %0) {
  call void @lv_obj_send_event(ptr %0, i32 noundef 10, ptr null)
  ret void
}
define dso_local void @lv_anim_set_deleted_cb(ptr noundef %0, ptr noundef %1) {
  %3 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %0, i32 0, i32 5
  store ptr %1, ptr %3, align 8
  ret void
}
define dso_local void @remove_anim(ptr noundef %0) {
  %3 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %0, i32 0, i32 5
  %4 = load ptr, ptr %3, align 8
  call void %4(ptr noundef %0)
  ret void
}
define dso_local void @harmless(ptr noundef %0) {
  ret void
}
define dso_local void @lv_anim_setup(ptr noundef %0) {
  call void @lv_anim_set_deleted_cb(ptr noundef %0, ptr noundef @harmless)
  ret void
}
define dso_local void @lv_display_delete(ptr noundef %0) {
  call void @obj_delete_core(ptr %0)
  ret void
}
define dso_local void @use_display_delete(ptr noundef %0) {
  store ptr @lv_display_delete, ptr %0, align 8
  ret void
}
"#,
            &[],
        );
        let graph = Graph::build(&m, &config()).unwrap();
        assert_eq!(
            graph.verdict("lv_obj_delete"),
            Verdict::Exclusive(vec!["lv_obj_delete".into(), "tree::obj_delete_core".into()])
        );
        assert!(matches!(
            graph.verdict("lv_obj_delete_async"),
            Verdict::Raw(_)
        ));
        // Installing a deleting handler is not itself a deletion.
        assert_eq!(graph.verdict("lv_msgbox_add_close_button"), Verdict::Shared);
        // A send reaches only handlers of its code.
        assert_eq!(graph.verdict("lv_obj_set_size"), Verdict::Shared);
        assert!(matches!(
            graph.verdict("lv_obj_click"),
            Verdict::Exclusive(_)
        ));
        // The sender itself, with any code, can reach every handler.
        assert!(matches!(
            graph.verdict("lv_obj_send_event"),
            Verdict::Exclusive(_)
        ));
        assert_eq!(graph.deleting_codes(), BTreeSet::from([Some(10)]));
        // An indirect call reaches only what its slot can hold, although a
        // deleting function of the same signature has its address taken.
        assert_eq!(graph.verdict("remove_anim"), Verdict::Shared);
    }

    #[test]
    fn animation_removals_reach_only_the_animations_they_match() {
        use super::super::identity::tests::{ANIMATION_CONFIG, animation_module};
        let m = animation_module();
        let config: AuditConfig = toml::from_str(ANIMATION_CONFIG).unwrap();
        let graph = Graph::build(&m, &config).unwrap();
        let exclusive = |f: &str| matches!(graph.verdict(f), Verdict::Exclusive(_));
        // `deleted_bad` belongs to animations whose `exec_cb` is `exec_b`
        // and whose `var` is NULL.
        assert!(!exclusive("lv_stop_a"));
        assert!(exclusive("lv_stop_b"));
        // No `exec_cb` and a `var` of unknown type: every animation.
        assert!(exclusive("lv_stop_all"));
        // A `var` of another struct type.
        assert!(!exclusive("lv_stop_d"));
        // Starting replaces the animations with the same `exec_cb` and
        // `var`, unless `early_apply` is off.
        assert!(!exclusive("lv_setup_a"));
        assert!(exclusive("lv_setup_bad"));
        assert!(!exclusive("lv_setup_quiet"));
        // A local whose address escapes can be anything.
        assert!(exclusive("lv_setup_escaping"));
        // The removal itself, called with unknown arguments.
        assert!(exclusive("lv_anim_delete"));
    }

    #[test]
    fn infeasible_calls_are_removed_only_in_their_context() {
        let mut m = Module::default();
        m.add_ir(
            "t",
            r#"
define internal void @obj_delete_core(ptr noundef %0) {
  ret void
}
define dso_local void @lv_group_remove_obj(ptr noundef %0) {
  call void @obj_delete_core(ptr %0)
  ret void
}
define dso_local void @lv_group_add_obj(ptr noundef %0) {
  call void @lv_group_remove_obj(ptr %0)
  ret void
}
define dso_local void @lv_obj_class_init_obj(ptr noundef %0) {
  call void @lv_group_add_obj(ptr %0)
  ret void
}
define dso_local void @lv_tabview_add_tab(ptr noundef %0) {
  call void @lv_group_add_obj(ptr %0)
  ret void
}
"#,
            &[],
        );
        let entry = r#"
sinks = ["t::obj_delete_core"]
[infeasible.fresh]
context = "lv_obj_class_init_obj -> lv_group_add_obj"
calls = ["lv_group_remove_obj"]
why = "test"
"#;
        let config: AuditConfig = toml::from_str(entry).unwrap();
        let graph = Graph::build(&m, &config).unwrap();
        assert_eq!(graph.verdict("lv_obj_class_init_obj"), Verdict::Shared);
        assert!(matches!(
            graph.verdict("lv_tabview_add_tab"),
            Verdict::Exclusive(_)
        ));
        assert!(matches!(
            graph.verdict("lv_group_add_obj"),
            Verdict::Exclusive(_)
        ));
        // Entries that do not match the call graph are errors.
        for wrong in [
            entry.replace("lv_obj_class_init_obj ->", "lv_group_remove_obj ->"),
            entry.replace("calls = [\"lv_group_remove_obj\"]", "calls = [\"lv_free\"]"),
            entry.replace(" -> lv_group_add_obj", ""),
        ] {
            let config: AuditConfig = toml::from_str(&wrong).unwrap();
            assert!(Graph::build(&m, &config).is_err(), "{wrong}");
        }
    }

    #[test]
    fn event_code_guards_prune_paths_of_other_codes() {
        let mut m = Module::default();
        m.add_ir(
            "w",
            r#"
@lv_dropdown_class = dso_local constant { ptr, ptr, ptr, ptr } { ptr null, ptr null, ptr null, ptr @lv_dropdown_event }, align 8
define internal void @lv_dropdown_event(ptr noundef %0, ptr noundef %1) {
  %3 = alloca ptr, align 8
  %4 = alloca i32, align 4
  store ptr %1, ptr %3, align 8
  %5 = load ptr, ptr %3, align 8
  %6 = call i32 @lv_event_get_code(ptr noundef %1)
  store i32 %6, ptr %4, align 4
  %7 = load i32, ptr %4, align 4
  %8 = icmp eq i32 %7, 14
  br i1 %8, label %9, label %10

9:                                                ; preds = %2
  call void @lv_dropdown_open(ptr noundef %0)
  br label %13

10:                                               ; preds = %2
  %11 = load i32, ptr %4, align 4
  switch i32 %11, label %13 [
    i32 15, label %12
  ]

12:
  call void @lv_obj_invalidate(ptr noundef %0)
  br label %13

13:
  ret void
}
define dso_local void @lv_dropdown_open(ptr noundef %0) {
  call void @lv_obj_move_to_index(ptr %0)
  ret void
}
define dso_local void @lv_obj_move_to_index(ptr noundef %0) {
  ret void
}
define dso_local void @lv_obj_invalidate(ptr noundef %0) {
  ret void
}
define dso_local void @lv_obj_event_base(ptr noundef %0, ptr noundef %1) {
  %3 = getelementptr inbounds nuw %struct._lv_obj_class_t, ptr %0, i32 0, i32 3
  %4 = load ptr, ptr %3, align 8
  call void %4(ptr noundef %0, ptr noundef %1)
  ret void
}
define dso_local void @lv_obj_send_event(ptr noundef %0, i32 noundef %1, ptr noundef %2) {
  call void @lv_obj_event_base(ptr noundef %0, ptr noundef %0)
  ret void
}
define dso_local void @lv_obj_focus(ptr noundef %0) {
  call void @lv_obj_send_event(ptr %0, i32 noundef 14, ptr null)
  ret void
}
define dso_local void @lv_obj_set_size(ptr noundef %0) {
  call void @lv_obj_send_event(ptr %0, i32 noundef 15, ptr null)
  ret void
}
define dso_local void @lv_anim_set_exec_cb(ptr noundef %0, ptr noundef %1) {
  %3 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %0, i32 0, i32 1
  store ptr %1, ptr %3, align 8
  ret void
}
define dso_local void @lv_anim_start(ptr noundef %0) {
  %3 = getelementptr inbounds nuw %struct._lv_anim_t, ptr %0, i32 0, i32 1
  %4 = load ptr, ptr %3, align 8
  call void %4(ptr noundef %0)
  ret void
}
define internal void @deleting_exec(ptr noundef %0) {
  call void @lv_obj_move_to_index(ptr %0)
  ret void
}
define internal void @plain_exec(ptr noundef %0) {
  ret void
}
define dso_local void @lv_screen_slide(ptr noundef %0) {
  call void @lv_anim_set_exec_cb(ptr noundef %0, ptr noundef @deleting_exec)
  call void @lv_anim_start(ptr noundef %0)
  ret void
}
define dso_local void @lv_obj_fade(ptr noundef %0) {
  call void @lv_anim_set_exec_cb(ptr noundef %0, ptr noundef @plain_exec)
  call void @lv_anim_start(ptr noundef %0)
  ret void
}
"#,
            &[("lv_*_class".into(), "_lv_obj_class_t".into())],
        );
        let mut config = config();
        config.sinks = vec!["lv_obj_move_to_index".into()];
        config.deleting_sinks = vec!["lv_obj_move_to_index".into()];
        config.caller_slots = BTreeMap::from([("_lv_anim_t.1".into(), CallerSlot::Immediate)]);
        let graph = Graph::build(&m, &config).unwrap();
        // FOCUSED (14) opens the list; SIZE_CHANGED (15) does not.
        assert!(matches!(
            graph.verdict("lv_obj_focus"),
            Verdict::Exclusive(_)
        ));
        assert_eq!(graph.verdict("lv_obj_set_size"), Verdict::Shared);
        // An animation runs its own callbacks, not every animation's.
        assert!(matches!(
            graph.verdict("lv_screen_slide"),
            Verdict::Exclusive(_)
        ));
        assert_eq!(graph.verdict("lv_obj_fade"), Verdict::Shared);
    }
}
