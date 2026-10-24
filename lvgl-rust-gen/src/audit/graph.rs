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
    #[serde(default)]
    pub anim_delete: Option<AnimDelete>,
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
}

/// `lv_anim_delete(var, exec_cb)` runs the `deleted_cb` of the animations
/// whose `exec_cb` matches: with a constant `exec_cb`, only the deleted
/// callbacks installed together with it are reached.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimDelete {
    pub function: String,
    pub exec_arg: usize,
    /// Functions through which the `exec_cb` being deleted stays known.
    pub carry: Vec<String>,
    pub exec_slots: Vec<String>,
    pub deleted_slot: String,
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
struct Out {
    to: String,
    kind: Edge,
    /// Taken only for these codes (`None`: any).
    guard: Option<Vec<i64>>,
    /// Starts dispatching this code.
    sets: Option<i64>,
    /// Forwards the event being dispatched (its code stays known).
    carry: bool,
    /// A deleted callback, run only when the `exec_cb` being deleted is one
    /// of these (`*`: any).
    execs: Option<Vec<String>>,
    /// Starts deleting the animations of this `exec_cb`.
    sets_exec: Option<String>,
    /// Keeps the `exec_cb` being deleted known.
    carry_exec: bool,
}

impl Out {
    fn new(to: impl Into<String>, kind: Edge) -> Self {
        Out {
            to: to.into(),
            kind,
            guard: None,
            sets: None,
            carry: false,
            execs: None,
            sets_exec: None,
            carry_exec: false,
        }
    }
}

/// `dropdown` for `widgets/dropdown/lv_dropdown`; `None` for core code.
fn widget_of(unit: &str) -> Option<&str> {
    unit.strip_prefix("widgets/")?.split('/').next()
}

pub struct Graph {
    edges: BTreeMap<String, Vec<Out>>,
    slots: BTreeMap<String, BTreeSet<String>>,
    sinks: BTreeSet<String>,
    deleting: BTreeSet<String>,
    /// Event handlers and the codes they are registered for (`None`: any).
    pub handlers: BTreeMap<String, BTreeSet<Option<i64>>>,
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
        let mut flow_edges: Vec<(String, String)> = Vec::new();
        let caller_slot = |slot: &str| config.caller_slots.get(slot).copied();
        let mut caller_edges: Vec<(String, String, Edge)> = Vec::new();
        // Animation callbacks installed by each function, to pair a
        // `deleted_cb` with the `exec_cb` installed with it.
        let mut anim_execs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut anim_deleted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut note_anim = |installer: &str, slot: &str, g: &str| {
            if let Some(anim) = &config.anim_delete {
                if anim.exec_slots.iter().any(|s| s == slot) {
                    anim_execs
                        .entry(installer.to_owned())
                        .or_default()
                        .insert(g.to_owned());
                } else if anim.deleted_slot == slot {
                    anim_deleted
                        .entry(installer.to_owned())
                        .or_default()
                        .insert(g.to_owned());
                }
            }
        };
        for (name, f) in &module.functions {
            for (slot, stored) in &f.stores {
                if let Stored::Function(g) = stored {
                    note_anim(name, slot, g);
                }
            }
        }
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
                                note_anim(name, slot, g);
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
        let mut deleted_by: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (installer, deleted) in &anim_deleted {
            let execs = anim_execs.get(installer);
            for d in deleted {
                let by = deleted_by.entry(d.clone()).or_default();
                match execs {
                    Some(execs) if !execs.is_empty() => by.extend(execs.iter().cloned()),
                    _ => {
                        by.insert("*".into());
                    }
                }
            }
        }
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
        for (name, f) in &module.functions {
            let out = edges.entry(name.clone()).or_default();
            let dispatch = config.event_dispatch.contains(name);
            let sender = config.send.contains_key(name);
            let in_machinery = machinery(name);
            let sender_widget = widget_of(&f.unit);
            for call in &f.calls {
                if call.callee == "lv_event_get_code" {
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
                        let mut e = Out::new(call.callee.clone(), Edge::Call);
                        e.guard = call.codes.clone();
                        e.carry = carry;
                        if let Some(anim) = &config.anim_delete {
                            if anim.function == call.callee
                                && let Some(Arg::Function(exec)) = call.args.get(anim.exec_arg)
                            {
                                e.sets_exec = Some(exec.clone());
                            }
                            e.carry_exec = anim.carry.iter().any(|c| c == name);
                        }
                        out.push(e);
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
            if !dispatch {
                for indirect in &f.indirect {
                    // A slot no LVGL code writes holds NULL or a callback
                    // installed through `sys`, which is that code's
                    // obligation: no edge.
                    let targets: Vec<String> = match &indirect.target {
                        Target::Slot(slot) if caller_slot(slot).is_some() => continue,
                        Target::Slot(slot) => slot_funcs
                            .get(slot)
                            .into_iter()
                            .flatten()
                            .cloned()
                            .collect(),
                        Target::Param(_) => continue, // via the callers' flows
                        Target::Unknown => by_signature
                            .get(indirect.signature.as_str())
                            .into_iter()
                            .flatten()
                            .map(|g| (*g).to_owned())
                            .collect(),
                    };
                    let deleted_slot = match (&indirect.target, &config.anim_delete) {
                        (Target::Slot(slot), Some(anim)) => *slot == anim.deleted_slot,
                        _ => false,
                    };
                    for g in targets {
                        let carry =
                            in_machinery || forwards(name, &indirect.args, &g, &event_params);
                        let mut e = Out::new(g.clone(), Edge::Indirect);
                        e.guard = indirect.codes.clone();
                        e.carry = carry;
                        if deleted_slot {
                            e.execs = Some(
                                deleted_by
                                    .get(&g)
                                    .map(|b| b.iter().cloned().collect())
                                    .unwrap_or_else(|| vec!["*".to_owned()]),
                            );
                        }
                        out.push(e);
                    }
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
            slots: slot_funcs,
            sinks,
            deleting,
            handlers,
        })
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
        type State<'a> = (&'a str, Option<i64>, Option<&'a str>);
        let mut previous: BTreeMap<State, State> = BTreeMap::new();
        let first: State = (start, code, None);
        let mut queue = VecDeque::from([first]);
        let mut seen = BTreeSet::from([first]);
        while let Some(state) = queue.pop_front() {
            let (node, code, exec) = state;
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
                if let (Some(exec), Some(execs)) = (exec, &out.execs)
                    && !execs.iter().any(|e| e == exec || e == "*")
                {
                    continue;
                }
                let next_code = match (out.sets, out.carry) {
                    (Some(c), _) => Some(c),
                    (None, true) => code,
                    (None, false) => None,
                };
                let next_exec = match (&out.sets_exec, out.carry_exec) {
                    (Some(e), _) => Some(e.as_str()),
                    (None, true) => exec,
                    (None, false) => None,
                };
                let next: State = (out.to.as_str(), next_code, next_exec);
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
