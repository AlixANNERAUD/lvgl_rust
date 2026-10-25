//! Held-region analysis: which memory a frame keeps a pointer to across an
//! event send, and which shared functions can free that memory.
//!
//! A Rust handler runs inside the C frame that sent its event, with shared
//! access. It is sound only if no frame on the stack keeps a pointer into
//! memory that a shared function can free or reallocate, and uses it after
//! the send returns. At `-O0` every C variable lives in an `alloca`, so a
//! frame *holds* region R across call site C when a local that received a
//! pointer into R (or a struct copy containing one) is read after C, and C
//! can send events.
//!
//! Regions ([`Reg`]):
//! - `Obj(T)`: an object of struct type `T`, matched by type;
//! - `Buf(path)`: the buffer whose pointer is stored at an embedded field
//!   path (`_lv_event_list_t.0/lv_array_t.0`), matched by path suffix, so
//!   one `lv_array_t` is not confused with every other;
//! - `Copy(path)`: a struct copy of the fields under `path`, which holds
//!   every buffer they point to;
//! - inside summaries, the same relative to a parameter (`P0/...`), turned
//!   into the above by the callers (with their arguments) or, at the
//!   boundary, by the parameter's struct type.
//!
//! Per function: `frees` (the regions `free_primitives` can release, through
//! calls, indirect calls and event sends, following the event code), the
//! codes each call site can `send`, and the `holds` (local and, through
//! parameters, those of callees). A hazard is a hold of R across a send of
//! a code whose handlers get shared access, and a shared function whose
//! frees contain R.
//!
//! Where precision stops (reported with `--verbose`, never counted as
//! safe): pointers with no known origin or type (call results whose
//! callee summary is not tracked, `phi`s, `inttoptr`, values that go
//! through heap memory other than fields), indirect calls with unknown
//! targets (only the escaping functions of their signature are followed),
//! loads at non-constant offsets of globals, and frees of untyped memory.
//! Index holds (a stale `row` or `cell` after a send) and memory reached
//! through two levels of pointers inside a copied struct are outside the
//! model. Holds are computed without the event code being dispatched in
//! the holding frame (more holds than real), and a local counts as holding
//! every value stored into it anywhere in the function.

use super::graph::{AuditConfig, Edge, Graph, Out};
use super::ir::{Ev, Function, Module, SiteKind, Val};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;

/// Longest field path kept; longer ones keep their suffix (a suffix
/// matches more, so this only loses precision).
const MAX_PATH: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Root {
    /// Relative to the memory a parameter points to.
    Param(usize),
    Abs,
}

/// A region of memory (see the module documentation).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reg {
    pub root: Root,
    /// Empty: the object itself; else the buffer whose pointer is stored
    /// at this embedded field path.
    pub path: Vec<String>,
    /// Every buffer whose pointer is stored under `path` (a struct copy).
    pub copy: bool,
    /// The struct type of the memory, when known.
    pub ty: Option<String>,
}

impl Reg {
    fn new(root: Root, path: Vec<String>, copy: bool, ty: Option<String>) -> Reg {
        let path = if path.len() > MAX_PATH {
            path[path.len() - MAX_PATH..].to_vec()
        } else {
            path
        };
        Reg {
            root,
            path,
            copy,
            ty,
        }
    }

    fn abs(path: Vec<String>, copy: bool, ty: Option<String>) -> Reg {
        Reg::new(Root::Abs, path, copy, ty)
    }

    /// Whether the region is named at all (else: unknown).
    pub fn known(&self) -> bool {
        !self.path.is_empty() || self.ty.is_some() || matches!(self.root, Root::Param(_))
    }
}

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = match self.root {
            Root::Param(k) => format!("P{k}/"),
            Root::Abs => String::new(),
        };
        match (self.copy, self.path.is_empty()) {
            (true, false) => write!(f, "Copy({prefix}{}/*)", self.path.join("/")),
            (true, true) => write!(f, "Copy({prefix}{}/*)", self.ty.as_deref().unwrap_or("?")),
            (false, false) => write!(f, "Buf({prefix}{})", self.path.join("/")),
            (false, true) => match (&self.root, &self.ty) {
                (Root::Param(k), None) => write!(f, "P{k}"),
                (Root::Param(k), Some(t)) => write!(f, "P{k}:Obj({t})"),
                (Root::Abs, Some(t)) => write!(f, "Obj({t})"),
                (Root::Abs, None) => write!(f, "unknown"),
            },
        }
    }
}

/// The struct type of a field step (`_lv_label_t` for `_lv_label_t.3`).
fn step_type(step: &str) -> &str {
    step.rsplit_once('.').map_or(step, |(t, _)| t)
}

/// Where a pointer points, in the frame that has it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Ptr {
    /// Into the region (its start, or an element of it).
    Obj(Reg),
    /// To an embedded field path of the region's memory.
    Field(Reg, Vec<String>),
    /// To a global's storage (never freed), or a field of it.
    Global(String, Vec<String>),
    /// To a local's storage (never freed).
    Local(String),
    /// To an embedded field path of a local struct.
    LocalField(String, Vec<String>),
    /// To memory just allocated by the frame or a callee: nobody else can
    /// hold it yet (unless it was published before the free: a limit).
    Fresh,
    Null,
    Unknown,
}

/// The buffer whose pointer is stored at field path `fp` of `r`'s memory.
fn buf(r: &Reg, fp: Vec<String>, copy: bool) -> Reg {
    match r.root {
        Root::Param(k) if r.path.is_empty() && !r.copy => Reg::new(Root::Param(k), fp, copy, None),
        _ => Reg::abs(fp, copy, None),
    }
}

/// A function's view for resolving its values.
struct Frame<'a> {
    f: &'a Function,
    /// Functions that return fresh memory.
    fresh: &'a BTreeSet<String>,
    /// Call sites of this function whose result it publishes (stores to
    /// memory, or passes to a callee that may keep it), with the first
    /// instruction that does.
    published: &'a BTreeMap<usize, usize>,
    /// The instruction being looked at (frees): a result published only
    /// after it is still fresh there (instruction order, not paths: a
    /// publication later in a loop body is a limit).
    at: Option<usize>,
}

/// What a value can be: parameters and call results (through locals and
/// field addresses), for publication and capture.
type Escapes = (BTreeSet<usize>, BTreeMap<usize, usize>);

fn origins(f: &Function, v: &Val, order: usize, seen: &mut BTreeSet<String>, out: &mut Escapes) {
    match v {
        Val::Param(k) => {
            out.0.insert(*k);
        }
        Val::Ret(n) => {
            let at = out.1.entry(*n).or_insert(order);
            *at = (*at).min(order);
        }
        Val::Field(b, _) => origins(f, b, order, seen, out),
        Val::Var(a) if seen.insert(a.clone()) => {
            for s in f.held.allocas.get(a).map_or(&[][..], |l| &l.stores) {
                origins(f, s, order, seen, out);
            }
        }
        _ => {}
    }
}

/// Module facts the frames need.
#[derive(Default)]
struct Facts {
    fresh: BTreeSet<String>,
    published: BTreeMap<String, BTreeMap<usize, usize>>,
    empty: BTreeMap<usize, usize>,
}

impl Facts {
    fn new(module: &Module, config: &AuditConfig) -> Facts {
        let leaf = |callee: &str| {
            config.free_primitives.contains_key(callee)
                || config.alloc_primitives.iter().any(|a| a == callee)
        };
        // Parameters a function may keep (store to memory, or pass to an
        // unknown or capturing callee).
        let mut capturing: BTreeSet<(String, usize)> = BTreeSet::new();
        // Where each function's values escape: (params, sites).
        let escapes = |name: &str, f: &Function, capturing: &BTreeSet<(String, usize)>| {
            let mut out: Escapes = Default::default();
            for (_, value, order) in &f.held.stores {
                origins(f, value, *order, &mut BTreeSet::new(), &mut out);
            }
            for site in &f.held.sites {
                let callee = match site.kind {
                    SiteKind::Call(n) => Some(f.calls[n].callee.as_str()),
                    SiteKind::Indirect(_) => None,
                };
                if callee.is_some_and(leaf) {
                    continue;
                }
                for (j, arg) in site.args.iter().enumerate() {
                    let keeps = match callee {
                        Some(c) => {
                            !module.functions.contains_key(c)
                                || capturing.contains(&(c.to_owned(), j))
                        }
                        None => true,
                    };
                    if keeps {
                        origins(f, arg, site.order, &mut BTreeSet::new(), &mut out);
                    }
                }
            }
            let _ = name;
            out
        };
        loop {
            let mut added = Vec::new();
            for (name, f) in &module.functions {
                for k in escapes(name, f, &capturing).0 {
                    if !capturing.contains(&(name.clone(), k)) {
                        added.push((name.clone(), k));
                    }
                }
            }
            if added.is_empty() {
                break;
            }
            capturing.extend(added);
        }
        let published: BTreeMap<String, BTreeMap<usize, usize>> = module
            .functions
            .iter()
            .map(|(name, f)| {
                let published = if config.unpublished.contains_key(name) {
                    BTreeMap::new()
                } else {
                    escapes(name, f, &capturing).1
                };
                (name.clone(), published)
            })
            .collect();
        let mut facts = Facts {
            fresh: config.alloc_primitives.iter().cloned().collect(),
            published,
            empty: BTreeMap::new(),
        };
        // Functions whose every returned pointer is fresh (or null), and
        // not published by them.
        loop {
            let mut added = Vec::new();
            for (name, f) in &module.functions {
                if facts.fresh.contains(name) || f.held.returns.is_empty() {
                    continue;
                }
                let frame = facts.frame(name, f);
                let all_fresh = f.held.returns.iter().all(|v| {
                    frame
                        .resolve_all(v)
                        .iter()
                        .all(|p| matches!(p, Ptr::Fresh | Ptr::Null))
                });
                if all_fresh {
                    added.push(name.clone());
                }
            }
            if added.is_empty() {
                break;
            }
            facts.fresh.extend(added);
        }
        facts
    }

    fn frame<'a>(&'a self, name: &str, f: &'a Function) -> Frame<'a> {
        Frame {
            f,
            fresh: &self.fresh,
            published: self.published.get(name).unwrap_or(&self.empty),
            at: None,
        }
    }
}

impl Frame<'_> {
    fn param_ty(&self, k: usize) -> Option<String> {
        // At -O0 parameters are stored to a local first: its GEP types.
        let mut types: BTreeSet<&String> = self
            .f
            .held
            .param_types
            .get(&k)
            .into_iter()
            .flatten()
            .collect();
        for alloca in self.f.held.allocas.values() {
            if alloca.stores.contains(&Val::Param(k)) {
                types.extend(alloca.types.iter());
            }
        }
        match types.len() {
            1 => types.into_iter().next().cloned(),
            _ => None,
        }
    }

    fn local_ty(&self, a: &str) -> Option<String> {
        let types = &self.f.held.allocas.get(a)?.types;
        match types.len() {
            1 => types.iter().next().cloned(),
            _ => None,
        }
    }

    /// Where `v` can point.
    fn resolve(&self, v: &Val, seen: &mut BTreeSet<String>) -> Vec<Ptr> {
        match v {
            Val::Unknown => vec![Ptr::Unknown],
            Val::Null => vec![Ptr::Null],
            Val::Ret(n) => {
                let fresh = match self.f.held.sites.get(*n).map(|s| s.kind) {
                    Some(SiteKind::Call(i)) => {
                        self.fresh.contains(&self.f.calls[i].callee)
                            && self
                                .published
                                .get(n)
                                .is_none_or(|&p| self.at.is_some_and(|at| p > at))
                    }
                    _ => false,
                };
                vec![if fresh { Ptr::Fresh } else { Ptr::Unknown }]
            }
            Val::Param(k) => vec![Ptr::Obj(Reg::new(
                Root::Param(*k),
                Vec::new(),
                false,
                self.param_ty(*k),
            ))],
            Val::Global(g) => vec![Ptr::Global(g.clone(), Vec::new())],
            Val::Local(a) => {
                // A struct copy: its fields are those of the source.
                let copies: Vec<&Val> = self.f.held.allocas.get(a).map_or(Vec::new(), |l| {
                    l.stores
                        .iter()
                        .filter_map(|s| match s {
                            Val::Copy(src) => Some(&**src),
                            _ => None,
                        })
                        .collect()
                });
                if copies.is_empty() || !seen.insert(a.clone()) {
                    return vec![Ptr::Local(a.clone())];
                }
                let out = copies.iter().flat_map(|s| self.resolve(s, seen)).collect();
                seen.remove(a);
                out
            }
            Val::Var(a) => {
                if !seen.insert(a.clone()) {
                    return vec![Ptr::Unknown];
                }
                let mut out = Vec::new();
                for s in self.f.held.allocas.get(a).map_or(&[][..], |l| &l.stores) {
                    match s {
                        // A pointer read from a struct copy: one of the
                        // buffers its source's fields point to.
                        Val::Copy(src) => {
                            for p in self.resolve(src, seen) {
                                out.push(copy_region(&p).map_or(Ptr::Unknown, Ptr::Obj));
                            }
                        }
                        s => out.extend(self.resolve(s, seen)),
                    }
                }
                seen.remove(a);
                if out.is_empty() {
                    out.push(Ptr::Unknown);
                }
                if let Some(t) = self.local_ty(a) {
                    for p in &mut out {
                        if *p == Ptr::Unknown {
                            *p = Ptr::Obj(Reg::abs(Vec::new(), false, Some(t.clone())));
                        }
                    }
                }
                out.sort();
                out.dedup();
                out
            }
            Val::Field(base, p) => self
                .resolve(base, seen)
                .into_iter()
                .map(|r| match r {
                    Ptr::Obj(reg) => Ptr::Field(reg, p.clone()),
                    Ptr::Field(reg, mut fp) => {
                        fp.extend(p.iter().cloned());
                        Ptr::Field(reg, fp)
                    }
                    Ptr::Global(g, mut fp) => {
                        fp.extend(p.iter().cloned());
                        Ptr::Global(g, fp)
                    }
                    Ptr::Local(a) => Ptr::LocalField(a, p.clone()),
                    Ptr::LocalField(a, mut fp) => {
                        fp.extend(p.iter().cloned());
                        Ptr::LocalField(a, fp)
                    }
                    Ptr::Fresh => Ptr::Fresh,
                    Ptr::Null => Ptr::Null,
                    // An object reached some unknown way: typed by the path.
                    Ptr::Unknown => Ptr::Field(
                        Reg::abs(
                            Vec::new(),
                            false,
                            p.first().map(|s| step_type(s).to_owned()),
                        ),
                        p.clone(),
                    ),
                })
                .collect(),
            Val::Load(addr) => self
                .resolve(addr, seen)
                .into_iter()
                .flat_map(|r| match r {
                    // A pointer read from a field of a local struct: what
                    // the function stored into the local, and what a callee
                    // given its address wrote there, named by the field.
                    Ptr::LocalField(a, fp) => {
                        let mut out: Vec<Ptr> = Vec::new();
                        if seen.insert(a.clone()) {
                            for s in self.f.held.allocas.get(&a).map_or(&[][..], |l| &l.stores) {
                                if !matches!(s, Val::Copy(_)) {
                                    out.extend(self.resolve(s, seen));
                                }
                            }
                            seen.remove(&a);
                        }
                        out.retain(|p| *p != Ptr::Unknown);
                        out.push(Ptr::Obj(Reg::abs(fp, false, None)));
                        out
                    }
                    r => vec![r],
                })
                .map(|r| match r {
                    Ptr::Field(reg, fp) => Ptr::Obj(buf(&reg, fp, false)),
                    // A pointer stored inside a buffer (an element).
                    Ptr::Obj(reg) if !reg.path.is_empty() => {
                        let mut path = reg.path.clone();
                        path.push("[]".into());
                        Ptr::Obj(Reg::new(reg.root, path, false, None))
                    }
                    Ptr::Global(g, fp) if fp.is_empty() => {
                        Ptr::Obj(Reg::abs(vec![format!("@{g}")], false, None))
                    }
                    Ptr::Global(_, fp) => Ptr::Obj(Reg::abs(fp, false, None)),
                    _ => Ptr::Unknown,
                })
                .collect(),
            Val::Copy(src) => self
                .resolve(src, seen)
                .iter()
                .map(|p| copy_region(p).map_or(Ptr::Unknown, Ptr::Obj))
                .collect(),
        }
    }

    fn resolve_all(&self, v: &Val) -> Vec<Ptr> {
        self.resolve(v, &mut BTreeSet::new())
    }

    /// The regions a local holds: what the pointers stored in it point
    /// into, and the buffers of struct copies. `None` entries are unknown.
    fn local_regions(&self, a: &str) -> Vec<Option<Reg>> {
        let Some(alloca) = self.f.held.allocas.get(a) else {
            return Vec::new();
        };
        let ty = self.local_ty(a);
        let mut out = Vec::new();
        for s in &alloca.stores {
            let ptrs = match s {
                Val::Copy(src) => self
                    .resolve_all(src)
                    .iter()
                    .map(copy_region)
                    .map(|r| r.map(Ptr::Obj).unwrap_or(Ptr::Unknown))
                    .collect(),
                s => self.resolve_all(s),
            };
            for p in ptrs {
                match p {
                    Ptr::Global(..)
                    | Ptr::Local(_)
                    | Ptr::LocalField(..)
                    | Ptr::Fresh
                    | Ptr::Null => {}
                    Ptr::Unknown => {
                        out.push(ty.clone().map(|t| Reg::abs(Vec::new(), false, Some(t))))
                    }
                    p => out.push(region_of(&p)),
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// The buffers a struct copy of what `p` points to keeps.
fn copy_region(p: &Ptr) -> Option<Reg> {
    match p {
        Ptr::Field(r, fp) => Some(buf(r, fp.clone(), true)),
        Ptr::Obj(r) if r.path.is_empty() => {
            Some(Reg::new(r.root.clone(), Vec::new(), true, r.ty.clone()))
        }
        Ptr::Global(_, fp) if !fp.is_empty() => Some(Reg::abs(fp.clone(), true, None)),
        _ => None,
    }
}

/// The region a pointer points into (`None`: unknown, or never freed).
fn region_of(p: &Ptr) -> Option<Reg> {
    match p {
        Ptr::Obj(r) => r.known().then(|| r.clone()),
        // An interior pointer: the object, typed by the path.
        Ptr::Field(r, fp) => {
            let mut r = r.clone();
            if r.ty.is_none() && r.path.is_empty() {
                r.ty = fp.first().map(|s| step_type(s).to_owned());
            }
            r.known().then_some(r)
        }
        _ => None,
    }
}

/// `pattern` (a callee's region) seen from the caller that passes `args`.
/// Unknown results are `None`.
fn instantiate(frame: &Frame, pattern: &Reg, args: &[Val]) -> Vec<Option<Reg>> {
    let Root::Param(j) = pattern.root else {
        return vec![Some(pattern.clone())];
    };
    let Some(arg) = args.get(j) else {
        return vec![absolute(pattern, None)];
    };
    let mut out = Vec::new();
    for p in frame.resolve_all(arg) {
        let r = match (&p, pattern.path.is_empty()) {
            (
                Ptr::Global(..) | Ptr::Local(_) | Ptr::LocalField(..) | Ptr::Fresh | Ptr::Null,
                true,
            ) => {
                continue;
            }
            (Ptr::Fresh | Ptr::Null, false) => continue,
            (Ptr::Unknown, true) => absolute(pattern, None),
            (p, true) if pattern.copy => copy_region(p),
            (p, true) => region_of(p).map(|mut r| {
                if r.ty.is_none() && r.path.is_empty() {
                    r.ty = pattern.ty.clone();
                }
                r
            }),
            (Ptr::Obj(r), false) => Some(buf(r, pattern.path.clone(), pattern.copy)),
            (Ptr::Field(r, fp), false) => {
                let mut path = fp.clone();
                path.extend(pattern.path.iter().cloned());
                Some(buf(r, path, pattern.copy))
            }
            (Ptr::Global(_, fp), false) => {
                let mut path = fp.clone();
                path.extend(pattern.path.iter().cloned());
                Some(Reg::abs(path, pattern.copy, None))
            }
            // A local struct: the values stored in its fields.
            (Ptr::Local(a) | Ptr::LocalField(a, _), false) => {
                let regions = frame.local_regions(a);
                out.extend(regions);
                continue;
            }
            (Ptr::Unknown, false) => Some(Reg::abs(pattern.path.clone(), pattern.copy, None)),
        };
        out.push(r);
    }
    out.sort();
    out.dedup();
    out
}

/// A region with its parameter root replaced by what is known of the
/// parameter (`ty`, else the pattern's own type).
fn absolute(r: &Reg, ty: Option<String>) -> Option<Reg> {
    match r.root {
        Root::Abs => Some(r.clone()),
        Root::Param(_) => {
            // The parameter's type is the type of the object, not of a
            // buffer it points to.
            let ty = if r.path.is_empty() {
                r.ty.clone().or(ty)
            } else {
                r.ty.clone()
            };
            let out = Reg::abs(r.path.clone(), r.copy, ty);
            out.known().then_some(out)
        }
    }
}

/// Whether freeing `free` can invalidate what `hold` keeps.
pub fn matches(hold: &Reg, free: &Reg) -> bool {
    if !hold.copy && hold.ty.is_some() && hold.ty == free.ty {
        return true;
    }
    if free.path.is_empty() {
        return false;
    }
    if hold.copy {
        if hold.path.is_empty() {
            let Some(t) = &hold.ty else { return false };
            return free.path.iter().any(|s| step_type(s) == t);
        }
        let n = hold.path.len();
        return free.path.len() >= n && free.path.windows(n).any(|w| w == hold.path.as_slice());
    }
    if hold.path.is_empty() {
        return false;
    }
    free.path.ends_with(&hold.path) || hold.path.ends_with(&free.path)
}

/// Event codes, as a set (`any`: every code).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Codes {
    pub any: bool,
    pub set: u128,
}

impl Codes {
    fn one(code: i64) -> Codes {
        if (0..128).contains(&code) {
            Codes {
                any: false,
                set: 1 << code,
            }
        } else {
            Codes::all()
        }
    }

    fn all() -> Codes {
        Codes { any: true, set: 0 }
    }

    fn union(&mut self, other: Codes) -> bool {
        let before = *self;
        self.any |= other.any;
        self.set |= other.set;
        *self != before
    }

    pub fn is_empty(&self) -> bool {
        !self.any && self.set == 0
    }

    pub fn contains(&self, code: i64) -> bool {
        self.any || ((0..128).contains(&code) && self.set & (1 << code) != 0)
    }

    fn list(&self, names: &BTreeMap<i64, String>) -> String {
        if self.any {
            return "any code".into();
        }
        let short: Vec<&str> = (0..128)
            .filter(|c| self.set & (1u128 << c) != 0)
            .map(|c| {
                names
                    .get(&c)
                    .map_or("?", |n| n.strip_prefix("LV_EVENT_").unwrap_or(n))
            })
            .collect();
        short.join(" ")
    }
}

type State = (usize, Option<i64>);
/// Successors of each state: `(state, transform, emitted codes)`.
type Succ = Vec<Vec<(usize, Transform, Codes)>>;

/// How a region set crosses an edge.
#[derive(Clone, Copy, Debug)]
enum Transform {
    /// Instantiate with the arguments of call site `site` of `function`.
    Site(usize, usize),
    /// Replace parameter roots of the callee by what is known of them.
    Absolute,
}

/// The analysis results.
#[derive(Debug, Default)]
pub struct Analysis {
    /// Hazards: `(code, freer, why)`.
    pub hazards: Vec<Hazard>,
    /// Review lines for `--verbose`.
    pub report: Vec<String>,
    /// Functions to make exclusive: the witness.
    pub promote: BTreeMap<String, String>,
    /// Codes whose handlers must not be shared although the code is not
    /// blockable (a freer that cannot be made exclusive): the witness.
    pub restrict: BTreeMap<i64, String>,
    /// Blockable codes with hazards: the witness.
    pub blocked: BTreeMap<i64, String>,
    /// Codes whose send frees held memory: the witness.
    pub send_hazard: BTreeMap<i64, String>,
    /// Shared, not crate-owned functions that free global memory without
    /// clearing it (fail the audit).
    pub uncleared: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Hazard {
    pub code: i64,
    pub freer: String,
    /// The frame that keeps the pointer.
    pub holder: String,
    pub hold: Reg,
    pub free: Reg,
    pub witness: String,
    pub guarded: bool,
    /// The hold is named only by a public function's parameter type.
    pub boundary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Hold {
    reg: Reg,
    /// The frame that keeps the local.
    holder: String,
    codes: (bool, u128),
    witness: String,
    /// Named only by the type of a public function's parameter (weaker
    /// evidence: preferred last as a witness).
    boundary: bool,
}

/// What the caller knows about the functions being classified.
pub struct Inputs<'a> {
    /// Public functions with their access before this analysis.
    pub access: &'a BTreeMap<String, String>,
    /// Functions whose access cannot change (hand decisions, crate-owned,
    /// or wrapped by hand): a hazard on them restricts the code instead.
    pub fixed: &'a BTreeSet<String>,
    /// Crate-owned functions (never wrapped).
    pub owned: &'a BTreeSet<String>,
    /// Handler tier of each event code before this analysis.
    pub handler: &'a BTreeMap<i64, String>,
    pub code_names: &'a BTreeMap<i64, String>,
}

struct Solver<'a> {
    module: &'a Module,
    graph: &'a Graph,
    config: &'a AuditConfig,
    names: Vec<String>,
    index: HashMap<String, usize>,
    senders: BTreeSet<String>,
    facts: Facts,
}

impl<'a> Solver<'a> {
    fn new(module: &'a Module, graph: &'a Graph, config: &'a AuditConfig) -> Self {
        let mut names: Vec<String> = graph.edges.keys().cloned().collect();
        for out in graph.edges.values().flatten() {
            names.push(out.to.clone());
        }
        names.extend(module.functions.keys().cloned());
        names.sort();
        names.dedup();
        let index = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i))
            .collect();
        let facts = Facts::new(module, config);
        Solver {
            module,
            graph,
            config,
            names,
            index,
            senders: config.send.keys().cloned().collect(),
            facts,
        }
    }

    fn frame<'f>(&'f self, name: &str, f: &'f Function) -> Frame<'f> {
        self.facts.frame(name, f)
    }

    fn function(&self, node: usize) -> Option<(&'a str, &'a Function)> {
        let name = self.names[node].as_str();
        let base = name.strip_suffix("#body").unwrap_or(name);
        self.module
            .functions
            .get_key_value(base)
            .map(|(k, f)| (k.as_str(), f))
    }

    fn admitted(code: Option<i64>, out: &Out) -> bool {
        !matches!((code, &out.guard), (Some(c), Some(g)) if !g.contains(&c))
    }

    fn next(code: Option<i64>, out: &Out) -> Option<i64> {
        match (out.sets, out.carry) {
            (Some(c), _) => Some(c),
            (None, true) => code,
            (None, false) => None,
        }
    }

    /// Codes an edge sends by itself: its constant code, or any code for a
    /// send whose code is not constant.
    fn emits(&self, out: &Out) -> Codes {
        if let Some(c) = out.sets {
            return Codes::one(c);
        }
        let computed =
            out.kind == Edge::Send || out.to.ends_with("#body") || self.senders.contains(&out.to);
        if computed {
            Codes::all()
        } else {
            Codes::default()
        }
    }

    fn live_edge(out: &Out) -> bool {
        matches!(out.kind, Edge::Call | Edge::Indirect | Edge::Send)
    }

    /// Successors of a state: `(state, transform, emitted codes)`.
    fn successors(&self, (node, code): State) -> Vec<(State, Transform, Codes)> {
        let mut out = Vec::new();
        let name = &self.names[node];
        let mut site_edges: BTreeSet<&Out> = BTreeSet::new();
        if let Some((base, _)) = self.function(node)
            && let Some((calls, indirect)) = self.graph.site_outs.get(base)
        {
            let f = self.module.functions.get(base).unwrap();
            for (i, site) in f.held.sites.iter().enumerate() {
                let outs = match site.kind {
                    SiteKind::Call(n) => calls.get(n),
                    SiteKind::Indirect(n) => indirect.get(n),
                };
                for e in outs.into_iter().flatten() {
                    site_edges.insert(e);
                    if !Self::live_edge(e) || !Self::admitted(code, e) {
                        continue;
                    }
                    let Some(&to) = self.index.get(&e.to) else {
                        continue;
                    };
                    let transform = if e.kind == Edge::Send {
                        Transform::Absolute
                    } else {
                        Transform::Site(self.index[base], i)
                    };
                    out.push(((to, Self::next(code, e)), transform, self.emits(e)));
                }
            }
            // Calls without a held site (none expected) keep their edges.
            for e in calls.iter().chain(indirect).flatten() {
                site_edges.insert(e);
            }
        }
        for e in self.graph.edges.get(name).into_iter().flatten() {
            if site_edges.contains(e) || !Self::live_edge(e) || !Self::admitted(code, e) {
                continue;
            }
            let Some(&to) = self.index.get(&e.to) else {
                continue;
            };
            out.push((
                (to, Self::next(code, e)),
                Transform::Absolute,
                self.emits(e),
            ));
        }
        if self.senders.contains(name) {
            out.push(((node, code), Transform::Absolute, Codes::all()));
        }
        out
    }

    /// Every state reachable from `starts`, with successors.
    fn explore(&self, starts: &[State]) -> (Vec<State>, Succ) {
        let mut ids: HashMap<State, usize> = HashMap::new();
        let mut states: Vec<State> = Vec::new();
        let mut succ: Succ = Vec::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut intern =
            |s: State, states: &mut Vec<State>, queue: &mut VecDeque<usize>| -> usize {
                *ids.entry(s).or_insert_with(|| {
                    states.push(s);
                    queue.push_back(states.len() - 1);
                    states.len() - 1
                })
            };
        for &s in starts {
            intern(s, &mut states, &mut queue);
        }
        while let Some(i) = queue.pop_front() {
            let next = self.successors(states[i]);
            let mapped = next
                .into_iter()
                .map(|(s, t, c)| (intern(s, &mut states, &mut queue), t, c))
                .collect();
            if succ.len() <= i {
                succ.resize(i + 1, Vec::new());
            }
            succ[i] = mapped;
        }
        succ.resize(states.len(), Vec::new());
        (states, succ)
    }

    /// The codes each state can send (least fixpoint).
    fn sends(&self, states: &[State], succ: &Succ) -> Vec<Codes> {
        let mut val = vec![Codes::default(); states.len()];
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); states.len()];
        for (i, edges) in succ.iter().enumerate() {
            for (j, _, c) in edges {
                val[i].union(*c);
                preds[*j].push(i);
            }
        }
        let mut queue: VecDeque<usize> = (0..states.len()).collect();
        let mut queued = vec![true; states.len()];
        while let Some(t) = queue.pop_front() {
            queued[t] = false;
            let v = val[t];
            for &s in &preds[t] {
                if val[s].union(v) && !queued[s] {
                    queued[s] = true;
                    queue.push_back(s);
                }
            }
        }
        val
    }

    /// Regions released by the free primitives at state `s` itself.
    fn local_frees(&self, (node, code): State) -> Vec<(Reg, String)> {
        let Some((name, f)) = self.function(node) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for site in &f.held.sites {
            let SiteKind::Call(n) = site.kind else {
                continue;
            };
            let call = &f.calls[n];
            let mut frame = self.frame(name, f);
            frame.at = Some(site.order);
            let Some(&arg) = self.config.free_primitives.get(&call.callee) else {
                continue;
            };
            if let (Some(c), Some(codes)) = (code, &call.codes)
                && !codes.contains(&c)
            {
                continue;
            }
            let Some(v) = site.args.get(arg) else {
                continue;
            };
            for p in frame.resolve_all(v) {
                if matches!(
                    p,
                    Ptr::Global(..) | Ptr::Local(_) | Ptr::LocalField(..) | Ptr::Fresh | Ptr::Null
                ) {
                    continue;
                }
                let reg = region_of(&p).unwrap_or_else(|| Reg::abs(Vec::new(), false, None));
                out.push((reg, name.to_owned()));
            }
        }
        out
    }

    fn transform(&self, t: Transform, to: State, regs: &[(Reg, String)]) -> Vec<(Reg, String)> {
        let mut out = Vec::new();
        match t {
            Transform::Site(function, site) => {
                let (caller, f) = self.function(function).unwrap();
                let mut frame = self.frame(caller, f);
                frame.at = Some(f.held.sites[site].order);
                let args = &f.held.sites[site].args;
                for (r, w) in regs {
                    if !r.known() || r.root == Root::Abs {
                        out.push((r.clone(), w.clone()));
                        continue;
                    }
                    // Where a parameter region gets its name.
                    let w = format!("{w} for {caller}");
                    for i in instantiate(&frame, r, args) {
                        out.push((
                            i.unwrap_or_else(|| Reg::abs(Vec::new(), false, None)),
                            w.clone(),
                        ));
                    }
                }
            }
            Transform::Absolute => {
                let callee = self.function(to.0);
                for (r, w) in regs {
                    let ty = match (&r.root, callee) {
                        (Root::Param(k), Some((n, f))) => self.frame(n, f).param_ty(*k),
                        _ => None,
                    };
                    out.push((
                        absolute(r, ty).unwrap_or_else(|| Reg::abs(Vec::new(), false, None)),
                        w.clone(),
                    ));
                }
            }
        }
        out
    }

    /// The regions each state can free (least fixpoint, by deltas).
    fn frees(&self, states: &[State], succ: &Succ) -> Vec<BTreeMap<Reg, String>> {
        let mut val: Vec<BTreeMap<Reg, String>> = vec![BTreeMap::new(); states.len()];
        let mut preds: Vec<Vec<(usize, Transform)>> = vec![Vec::new(); states.len()];
        for (i, edges) in succ.iter().enumerate() {
            for (j, t, _) in edges {
                preds[*j].push((i, *t));
            }
        }
        let mut queue: VecDeque<(usize, Vec<(Reg, String)>)> = VecDeque::new();
        for (i, s) in states.iter().enumerate() {
            let local = self.local_frees(*s);
            let mut delta = Vec::new();
            for (r, w) in local {
                if !val[i].contains_key(&r) {
                    val[i].insert(r.clone(), w.clone());
                    delta.push((r, w));
                }
            }
            if !delta.is_empty() {
                queue.push_back((i, delta));
            }
        }
        while let Some((t, delta)) = queue.pop_front() {
            for &(s, tr) in &preds[t] {
                let mut new = Vec::new();
                for (r, w) in self.transform(tr, states[t], &delta) {
                    if !val[s].contains_key(&r) {
                        val[s].insert(r.clone(), w.clone());
                        new.push((r, w));
                    }
                }
                if !new.is_empty() {
                    queue.push_back((s, new));
                }
            }
        }
        val
    }
}

/// Runs the analysis.
pub fn analyze(
    module: &Module,
    graph: &Graph,
    config: &AuditConfig,
    inputs: &Inputs,
) -> Result<Analysis, String> {
    for (name, g) in &config.guarded {
        if g.freers.is_empty() && g.holders.is_empty() {
            return Err(format!(
                "audit.toml: [guarded.{name}] lists neither freers nor holders"
            ));
        }
    }
    for name in config.unpublished.keys() {
        if !module.functions.contains_key(name) {
            return Err(format!("audit.toml: [unpublished] {name} is not defined"));
        }
    }
    let solver = Solver::new(module, graph, config);
    let facts = &solver.facts;
    let sender = config
        .object_sender
        .clone()
        .ok_or("audit.toml: no object_sender")?;
    let mut analysis = Analysis::default();

    // States: every function with an unknown code, and what safe code
    // sending each code runs.
    let mut starts: Vec<State> = module
        .functions
        .keys()
        .filter_map(|n| solver.index.get(n).map(|&i| (i, None)))
        .collect();
    let mut send_roots: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for &code in inputs.code_names.keys() {
        let mut roots = vec![format!("{sender}#body")];
        roots.extend(graph.handlers_of(code).map(str::to_owned));
        for r in roots {
            if let Some(&i) = solver.index.get(&r) {
                starts.push((i, Some(code)));
                send_roots.entry(code).or_default().push(i);
            }
        }
    }
    let (states, succ) = solver.explore(&starts);
    let state_id: HashMap<State, usize> = states.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let sends = solver.sends(&states, &succ);
    let frees = solver.frees(&states, &succ);

    // Holds, function by function; parameter-rooted ones climb to callers.
    let mut param_holds: BTreeMap<String, BTreeSet<Hold>> = BTreeMap::new();
    let mut holds: BTreeSet<Hold> = BTreeSet::new();
    let mut unknown_holds: BTreeMap<String, usize> = BTreeMap::new();
    // Functions each called parameter can receive (`None`: something the
    // analysis cannot name, such as a pointer loaded from memory).
    let mut flows: BTreeMap<(String, usize), Option<BTreeSet<String>>> = BTreeMap::new();
    for (name, f) in &module.functions {
        for ind in &f.indirect {
            if let super::ir::Target::Param(k) = ind.target {
                flows.insert((name.clone(), k), Some(BTreeSet::new()));
            }
        }
    }
    // The functions and parameters a value can be (`None`: something else).
    fn fn_values(
        module: &Module,
        f: &Function,
        v: &Val,
        seen: &mut BTreeSet<String>,
    ) -> Option<(BTreeSet<String>, BTreeSet<usize>)> {
        match v {
            Val::Null => Some(Default::default()),
            Val::Global(g) if module.functions.contains_key(g) => {
                Some((BTreeSet::from([g.clone()]), BTreeSet::new()))
            }
            Val::Param(k) => Some((BTreeSet::new(), BTreeSet::from([*k]))),
            Val::Var(a) => {
                if !seen.insert(a.clone()) {
                    return Some(Default::default());
                }
                let mut out: (BTreeSet<String>, BTreeSet<usize>) = Default::default();
                for s in f.held.allocas.get(a).map_or(&[][..], |l| &l.stores) {
                    let (fs, ps) = fn_values(module, f, s, seen)?;
                    out.0.extend(fs);
                    out.1.extend(ps);
                }
                Some(out)
            }
            _ => None,
        }
    }
    loop {
        let mut changed = false;
        for (name, f) in &module.functions {
            for site in &f.held.sites {
                let SiteKind::Call(n) = site.kind else {
                    continue;
                };
                let callee = &f.calls[n].callee;
                for (k, arg) in site.args.iter().enumerate() {
                    let key = (callee.clone(), k);
                    if !flows.contains_key(&key) {
                        continue;
                    }
                    let incoming: Option<BTreeSet<String>> =
                        match fn_values(module, f, arg, &mut BTreeSet::new()) {
                            None => None,
                            Some((mut fs, ps)) => {
                                let mut unknown = false;
                                for j in ps {
                                    match flows.get(&(name.clone(), j)) {
                                        Some(Some(v)) => fs.extend(v.iter().cloned()),
                                        Some(None) => unknown = true,
                                        None => {
                                            flows.insert((name.clone(), j), Some(BTreeSet::new()));
                                            changed = true;
                                        }
                                    }
                                }
                                (!unknown).then_some(fs)
                            }
                        };
                    let slot = flows.get_mut(&key).unwrap();
                    match (slot.as_mut(), incoming) {
                        (None, _) => {}
                        (Some(_), None) => {
                            *slot = None;
                            changed = true;
                        }
                        (Some(set), Some(new)) => {
                            for g in new {
                                changed |= set.insert(g);
                            }
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let sends_of = |g: &str| -> Codes {
        solver
            .index
            .get(g)
            .and_then(|&i| state_id.get(&(i, None)))
            .map_or(Codes::default(), |&s| sends[s])
    };
    let site_sends = |name: &str, f: &Function, site: usize| -> Codes {
        let mut codes = Codes::default();
        let Some((calls, indirect)) = graph.site_outs.get(name) else {
            return codes;
        };
        match f.held.sites[site].kind {
            // The handler call of an event dispatcher: handlers of any
            // code (they are connected at the send sites in the graph).
            SiteKind::Indirect(_) if config.event_dispatch.iter().any(|d| d == name) => {
                codes.union(Codes::all());
            }
            // Callbacks the installer is charged with run here too.
            SiteKind::Indirect(n) => match &f.indirect[n].target {
                super::ir::Target::Slot(slot) => {
                    for g in graph.caller_slot_funcs.get(slot).into_iter().flatten() {
                        codes.union(sends_of(g));
                    }
                }
                // A called parameter: what any caller passes (not per call
                // site); any code when a caller passes something unnamed.
                super::ir::Target::Param(k) => match flows.get(&(name.to_owned(), *k)) {
                    Some(Some(set)) => {
                        for g in set {
                            codes.union(sends_of(g));
                        }
                    }
                    _ => {
                        codes.union(Codes::all());
                    }
                },
                super::ir::Target::Unknown => {}
            },
            // Functions passed to a callee that calls them.
            SiteKind::Call(n) => {
                for arg in &f.calls[n].args {
                    if let super::ir::Arg::Function(g) = arg
                        && graph.edges.get(name).is_some_and(|outs| {
                            outs.iter().any(|e| e.to == *g && e.kind == Edge::Indirect)
                        })
                    {
                        codes.union(sends_of(g));
                    }
                }
            }
        }
        let outs = match f.held.sites[site].kind {
            SiteKind::Call(n) => calls.get(n),
            SiteKind::Indirect(n) => indirect.get(n),
        };
        for e in outs.into_iter().flatten() {
            if !Solver::live_edge(e) {
                continue;
            }
            codes.union(solver.emits(e));
            let code = match (&e.guard, e.carry) {
                (Some(g), true) if g.len() == 1 => Some(g[0]),
                _ => None,
            };
            if let Some(&to) = solver.index.get(&e.to)
                && let Some(&s) = state_id.get(&(to, Solver::next(code, e)))
            {
                codes.union(sends[s]);
            } else if let Some(&to) = solver.index.get(&e.to)
                && let Some(&s) = state_id.get(&(to, None))
            {
                codes.union(sends[s]);
            }
        }
        codes
    };
    // Local holds.
    for (name, f) in &module.functions {
        let frame = facts.frame(name, f);
        let carriers: Vec<&String> = f
            .held
            .allocas
            .iter()
            .filter(|(_, a)| a.pointer && !a.stores.is_empty())
            .map(|(n, _)| n)
            .collect();
        if carriers.is_empty() {
            continue;
        }
        let live = live_after_calls(f, &carriers);
        for (site, alive) in live {
            if alive.is_empty() {
                continue;
            }
            let codes = site_sends(name, f, site);
            if codes.is_empty() {
                continue;
            }
            let callee = site_callee(f, site);
            for a in alive {
                for reg in frame.local_regions(a) {
                    let witness = format!("{name} keeps {a} across {callee}");
                    match reg {
                        Some(reg) => {
                            let hold = Hold {
                                reg,
                                holder: name.clone(),
                                codes: (codes.any, codes.set),
                                witness,
                                boundary: false,
                            };
                            if matches!(hold.reg.root, Root::Param(_)) {
                                param_holds.entry(name.clone()).or_default().insert(hold);
                            } else {
                                holds.insert(hold);
                            }
                        }
                        None => *unknown_holds.entry(witness).or_default() += 1,
                    }
                }
            }
        }
    }
    // Parameter holds seen from callers (direct calls and indirect calls
    // with known targets), until nothing changes.
    for _ in 0..16 {
        let mut added = false;
        for (name, f) in &module.functions {
            let frame = facts.frame(name, f);
            let Some((calls, indirect)) = graph.site_outs.get(name) else {
                continue;
            };
            for (i, site) in f.held.sites.iter().enumerate() {
                let outs = match site.kind {
                    SiteKind::Call(n) => calls.get(n),
                    SiteKind::Indirect(n) => indirect.get(n),
                };
                for e in outs.into_iter().flatten() {
                    if !matches!(e.kind, Edge::Call | Edge::Indirect) || e.to.ends_with("#body") {
                        continue;
                    }
                    let Some(callee_holds) = param_holds.get(&e.to).cloned() else {
                        continue;
                    };
                    for h in callee_holds {
                        for reg in instantiate(&frame, &h.reg, &site.args) {
                            let witness = format!("{} (via {name}, site {i})", h.witness);
                            let Some(reg) = reg else {
                                *unknown_holds.entry(witness).or_default() += 1;
                                continue;
                            };
                            let hold = Hold {
                                reg,
                                holder: h.holder.clone(),
                                codes: h.codes,
                                witness,
                                boundary: false,
                            };
                            let set = if matches!(hold.reg.root, Root::Param(_)) {
                                param_holds.entry(name.clone()).or_default()
                            } else {
                                &mut holds
                            };
                            if !set
                                .iter()
                                .any(|x| x.reg == hold.reg && x.codes == hold.codes)
                            {
                                set.insert(hold);
                                added = true;
                            }
                        }
                    }
                }
            }
        }
        if !added {
            break;
        }
    }
    // Parameter holds at the boundary, by the parameter's type: public
    // functions, and functions some caller reaches without arguments the
    // analysis can map (event handlers, animation callbacks run by their
    // installer, functions passed to a callee that calls them).
    let mut site_in: BTreeSet<&str> = BTreeSet::new();
    let mut other_in: BTreeSet<&str> = BTreeSet::new();
    for (from, outs) in &graph.edges {
        let base = from.strip_suffix("#body").unwrap_or(from);
        let site_set: BTreeSet<&Out> = graph
            .site_outs
            .get(base)
            .map(|(c, i)| c.iter().chain(i).flatten().collect())
            .unwrap_or_default();
        for e in outs {
            if !Solver::live_edge(e) {
                continue;
            }
            if site_set.contains(e) && e.kind != Edge::Send {
                site_in.insert(&e.to);
            } else {
                other_in.insert(&e.to);
            }
        }
    }
    let boundary = |name: &str| {
        inputs.access.contains_key(name) || other_in.contains(name) || !site_in.contains(name)
    };
    for (name, set) in &param_holds {
        if !boundary(name) {
            continue;
        }
        let f = &module.functions[name];
        for h in set {
            let Root::Param(k) = h.reg.root else { continue };
            match absolute(&h.reg, solver.frame(name, f).param_ty(k)) {
                Some(reg) => {
                    holds.insert(Hold {
                        reg,
                        boundary: true,
                        ..h.clone()
                    });
                }
                None => {
                    if inputs.access.contains_key(name) {
                        *unknown_holds
                            .entry(format!("{} (untyped parameter of {name})", h.witness))
                            .or_default() += 1;
                    }
                }
            }
        }
    }

    // Candidate freers: shared public functions, and each code's send.
    let state_frees =
        |root: State| -> Option<&BTreeMap<Reg, String>> { state_id.get(&root).map(|&i| &frees[i]) };
    let shared_code = |code: i64| inputs.handler.get(&code).is_some_and(|h| h == "shared");
    let blockable: BTreeSet<i64> = inputs
        .code_names
        .iter()
        .filter(|(_, n)| config.blockable_codes.contains(n))
        .map(|(c, _)| *c)
        .collect();
    let mut freers: Vec<(String, BTreeMap<Reg, String>)> = Vec::new();
    for (name, access) in inputs.access {
        if access != "shared" {
            continue;
        }
        if let Some(&i) = solver.index.get(name)
            && let Some(f) = state_frees((i, None))
            && !f.is_empty()
        {
            // Relative to the freer's parameters: by their types.
            let function = module.functions.get(name);
            let regs = f
                .iter()
                .map(|(r, w)| {
                    let ty = match (&r.root, function) {
                        (Root::Param(k), Some(func)) => solver.frame(name, func).param_ty(*k),
                        _ => None,
                    };
                    let r = absolute(r, ty).unwrap_or_else(|| Reg::abs(Vec::new(), false, None));
                    (r, w.clone())
                })
                .collect();
            freers.push((name.clone(), regs));
        }
    }
    let mut unknown_frees: Vec<String> = Vec::new();
    for (name, f) in &freers {
        if f.keys().any(|r| !r.known()) {
            unknown_frees.push(name.clone());
        }
    }
    let codes_of = |h: &Hold| Codes {
        any: h.codes.0,
        set: h.codes.1,
    };
    for hold in &holds {
        let codes = codes_of(hold);
        let hazardous: Vec<i64> = inputs
            .code_names
            .keys()
            .copied()
            .filter(|&c| codes.contains(c) && (shared_code(c) || blockable.contains(&c)))
            .collect();
        if hazardous.is_empty() {
            continue;
        }
        for (freer, regs) in &freers {
            for (free, via) in regs {
                if !free.known() || !matches(&hold.reg, free) {
                    continue;
                }
                let guarded = is_guarded(config, hold, Some(freer), via);
                for &code in &hazardous {
                    analysis.hazards.push(Hazard {
                        code,
                        freer: freer.clone(),
                        holder: hold.holder.clone(),
                        hold: hold.reg.clone(),
                        free: free.clone(),
                        witness: format!("{} (frees {free} in {via})", hold.witness),
                        guarded,
                        boundary: hold.boundary,
                    });
                }
            }
        }
    }
    // Decisions, with witnesses from local holds first.
    analysis.hazards.sort_by_key(|h| h.boundary);
    let names = inputs.code_names;
    for h in &analysis.hazards {
        if h.guarded {
            continue;
        }
        let code_name = names.get(&h.code).map_or("?", String::as_str);
        let codes = code_name.to_owned();
        if blockable.contains(&h.code) {
            // Recorded below, preferring freers that stay shared.
        } else if inputs.fixed.contains(&h.freer) {
            analysis.restrict.entry(h.code).or_insert_with(|| {
                format!(
                    "{} (access fixed by hand) frees {}, held: {}",
                    h.freer, h.hold, h.witness
                )
            });
        } else {
            analysis.promote.entry(h.freer.clone()).or_insert_with(|| {
                format!("frees {}, held by {} (codes {codes})", h.hold, h.witness)
            });
        }
    }
    for h in &analysis.hazards {
        if h.guarded || !blockable.contains(&h.code) {
            continue;
        }
        let stays_shared = !analysis.promote.contains_key(&h.freer);
        let witness = format!("{} frees {}, held: {}", h.freer, h.hold, h.witness);
        match analysis.blocked.get(&h.code) {
            None => {
                analysis.blocked.insert(h.code, witness);
            }
            Some(old)
                if stays_shared
                    && analysis
                        .promote
                        .keys()
                        .any(|p| old.starts_with(&format!("{p} "))) =>
            {
                analysis.blocked.insert(h.code, witness);
            }
            _ => {}
        }
    }
    // Merge the codes of each promotion into its reason.
    let mut promoted_codes: BTreeMap<&str, Codes> = BTreeMap::new();
    for h in &analysis.hazards {
        if !h.guarded && !blockable.contains(&h.code) && !inputs.fixed.contains(&h.freer) {
            promoted_codes
                .entry(&h.freer)
                .or_default()
                .union(Codes::one(h.code));
        }
    }
    for (freer, codes) in promoted_codes {
        if let Some(why) = analysis.promote.get_mut(freer)
            && let Some(at) = why.rfind(" (codes ")
        {
            why.truncate(at);
            let list = codes.list(names);
            let words: Vec<&str> = list.split(' ').collect();
            let shown = if words.len() > 8 {
                format!("{} and {} more", words[..8].join(" "), words.len() - 8)
            } else {
                list.clone()
            };
            why.push_str(&format!(" (codes {shown})"));
        }
    }
    // Sends of each code as freers.
    for (&code, roots) in &send_roots {
        for &root in roots {
            let Some(regs) = state_frees((root, Some(code))) else {
                continue;
            };
            'found: for hold in &holds {
                let codes = codes_of(hold);
                if !names.keys().any(|&c| codes.contains(c) && shared_code(c)) {
                    continue;
                }
                for (free, via) in regs {
                    if free.known()
                        && matches(&hold.reg, free)
                        && !is_guarded(config, hold, None, via)
                    {
                        analysis.send_hazard.entry(code).or_insert_with(|| {
                            format!("frees {free} in {via}; held by {}", hold.witness)
                        });
                        break 'found;
                    }
                }
            }
        }
    }

    // Review lists.
    let mut report = Vec::new();
    let mut grouped: BTreeMap<(bool, &str, &str), Codes> = BTreeMap::new();
    for h in &analysis.hazards {
        grouped
            .entry((h.guarded, h.freer.as_str(), h.witness.as_str()))
            .or_default()
            .union(Codes::one(h.code));
    }
    for ((guarded, freer, witness), codes) in grouped {
        report.push(format!(
            "hazard{}: {freer} on {}: {witness}",
            if guarded { " (guarded)" } else { "" },
            codes.list(names)
        ));
    }
    report.sort();
    report.dedup();
    for (w, n) in &unknown_holds {
        report.push(format!("unknown hold ({n} values): {w}"));
    }
    for name in &unknown_frees {
        report.push(format!("frees memory of unknown origin: {name}"));
    }
    report.extend(aliases(module, graph, &freers, facts));
    let (lines, uncleared) = free_without_clear(module, inputs, facts);
    report.extend(lines);
    analysis.uncleared = uncleared;
    report.push(format!(
        "held-region analysis: {} states, {} holds, {} unknown holds, {} shared freers ({} with unknown frees), {} hazards",
        states.len(),
        holds.len(),
        unknown_holds.len(),
        freers.len(),
        unknown_frees.len(),
        analysis.hazards.len()
    ));
    analysis.report = report;
    Ok(analysis)
}

/// Whether a `[guarded]` entry covers a hold freed by `freer` (`None`: a
/// send) through `via` (the free's witness chain).
fn is_guarded(config: &AuditConfig, hold: &Hold, freer: Option<&str>, via: &str) -> bool {
    let shown = hold.reg.to_string();
    config.guarded.values().any(|g| {
        (g.freers.is_empty() || freer.is_some_and(|f| g.freers.iter().any(|x| x == f)))
            && (g.holders.is_empty() || g.holders.contains(&hold.holder))
            && (g.via.is_empty() || g.via.iter().any(|v| chain_has(via, v)))
            && g.regions.iter().any(|r| crate::tables::glob(r, &shown))
    })
}

/// Whether a free witness (`lv_free for f for g ...`) names `function`.
fn chain_has(witness: &str, function: &str) -> bool {
    witness
        .split(" for ")
        .any(|part| part.split(" in ").last() == Some(function) || part == function)
}

fn site_callee(f: &Function, site: usize) -> String {
    match f.held.sites[site].kind {
        SiteKind::Call(n) => f.calls[n].callee.clone(),
        SiteKind::Indirect(n) => match &f.indirect[n].target {
            super::ir::Target::Slot(s) => format!("an indirect call through {s}"),
            super::ir::Target::Param(k) => format!("an indirect call through parameter {k}"),
            super::ir::Target::Unknown => "an unresolved indirect call".into(),
        },
    }
}

/// For each call site, the carrier locals read after it returns (before
/// being overwritten), or whose address escaped.
fn live_after_calls<'a>(f: &Function, carriers: &[&'a String]) -> BTreeMap<usize, Vec<&'a String>> {
    let n = carriers.len();
    let words = n.div_ceil(64);
    let index: HashMap<&str, usize> = carriers
        .iter()
        .enumerate()
        .map(|(i, a)| (a.as_str(), i))
        .collect();
    let blocks = f.held.events.len();
    let set = |bits: &mut [u64], i: usize, on: bool| {
        if on {
            bits[i / 64] |= 1 << (i % 64);
        } else {
            bits[i / 64] &= !(1 << (i % 64));
        }
    };
    let transfer =
        |b: usize, live: &mut Vec<u64>, mut at_call: Option<&mut BTreeMap<usize, Vec<u64>>>| {
            for ev in f.held.events[b].iter().rev() {
                match ev {
                    Ev::Call(site) => {
                        if let Some(map) = at_call.as_deref_mut() {
                            map.insert(*site, live.clone());
                        }
                    }
                    Ev::Load(a) | Ev::Use(a) => {
                        if let Some(&i) = index.get(a.as_str()) {
                            set(live, i, true);
                        }
                    }
                    Ev::Store(a) => {
                        if let Some(&i) = index.get(a.as_str()) {
                            set(live, i, false);
                        }
                    }
                }
            }
        };
    let mut live_in = vec![vec![0u64; words]; blocks];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..blocks).rev() {
            let mut live = vec![0u64; words];
            for &s in f.held.succ.get(b).into_iter().flatten() {
                for (w, x) in live.iter_mut().zip(&live_in[s]) {
                    *w |= x;
                }
            }
            transfer(b, &mut live, None);
            if live != live_in[b] {
                live_in[b] = live;
                changed = true;
            }
        }
    }
    let mut at_call: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    for b in 0..blocks {
        let mut live = vec![0u64; words];
        for &s in f.held.succ.get(b).into_iter().flatten() {
            for (w, x) in live.iter_mut().zip(&live_in[s]) {
                *w |= x;
            }
        }
        transfer(b, &mut live, Some(&mut at_call));
    }
    at_call
        .into_iter()
        .map(|(site, bits)| {
            let alive = carriers
                .iter()
                .enumerate()
                .filter(|(i, a)| {
                    bits[i / 64] & (1 << (i % 64)) != 0 || f.held.allocas[a.as_str()].escaped
                })
                .map(|(_, a)| *a)
                .collect();
            (site, alive)
        })
        .collect()
}

/// The field path a pointer store writes, absolute (`None`: a local or an
/// unknown place).
fn store_path(frame: &Frame, address: &Val) -> Vec<Vec<String>> {
    frame
        .resolve_all(address)
        .into_iter()
        .filter_map(|p| match p {
            Ptr::Field(_, fp) | Ptr::Global(_, fp) if !fp.is_empty() => Some(fp),
            Ptr::Global(g, _) => Some(vec![format!("@{g}")]),
            _ => None,
        })
        .collect()
}

/// Review list: fields that alias a buffer owned elsewhere (a pointer
/// loaded from field R stored into field Q), and the freers of R that do
/// not update Q.
fn aliases(
    module: &Module,
    graph: &Graph,
    freers: &[(String, BTreeMap<Reg, String>)],
    facts: &Facts,
) -> Vec<String> {
    // (Q, R) -> where.
    let mut found: BTreeMap<(Vec<String>, Reg), String> = BTreeMap::new();
    // Functions that store a parameter into a field: (k, Q).
    let mut stores_param: BTreeMap<&str, Vec<(usize, Vec<String>)>> = BTreeMap::new();
    let mut writes: BTreeMap<&str, BTreeSet<Vec<String>>> = BTreeMap::new();
    for (name, f) in &module.functions {
        let frame = facts.frame(name, f);
        for (address, value, _) in &f.held.stores {
            let qs = store_path(&frame, address);
            for q in &qs {
                writes.entry(name).or_default().insert(q.clone());
            }
            for p in frame.resolve_all(value) {
                let Some(r) = region_of(&p) else { continue };
                for q in &qs {
                    match r.root {
                        Root::Param(k) if r.path.is_empty() => {
                            stores_param.entry(name).or_default().push((k, q.clone()))
                        }
                        _ if !r.path.is_empty() && r.path != *q && !r.copy => {
                            found
                                .entry((q.clone(), absolute(&r, None).unwrap_or(r.clone())))
                                .or_insert_with(|| name.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    // One level of callers for parameter stores.
    for (name, f) in &module.functions {
        let frame = facts.frame(name, f);
        let Some((calls, _)) = graph.site_outs.get(name) else {
            continue;
        };
        for site in &f.held.sites {
            let SiteKind::Call(n) = site.kind else {
                continue;
            };
            let Some(outs) = calls.get(n) else { continue };
            for e in outs {
                let Some(stored) = stores_param.get(e.to.as_str()) else {
                    continue;
                };
                for (k, q) in stored {
                    let pattern = Reg::new(Root::Param(*k), Vec::new(), false, None);
                    for r in instantiate(&frame, &pattern, &site.args)
                        .into_iter()
                        .flatten()
                    {
                        if !r.path.is_empty() && r.path != *q && !r.copy {
                            found
                                .entry((q.clone(), absolute(&r, None).unwrap_or(r.clone())))
                                .or_insert_with(|| format!("{name} via {}", e.to));
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for ((q, r), by) in &found {
        let stale: Vec<&str> = freers
            .iter()
            .filter(|(g, regs)| {
                regs.keys().any(|free| free.known() && matches(r, free))
                    && !writes
                        .get(g.as_str())
                        .is_some_and(|w| w.iter().any(|p| p.ends_with(q) || q.ends_with(p)))
            })
            .map(|(g, _)| g.as_str())
            .collect();
        if !stale.is_empty() {
            out.push(format!(
                "alias: {} keeps {r} (stored by {by}); shared freers that do not update it: {}",
                q.join("/"),
                stale.join(", ")
            ));
        }
    }
    out
}

/// Review list: frees of memory a global names (rooted at `lv_global_t`
/// or a global pointer) with no later store to that place in the same
/// function. Returns the lines and the shared, not crate-owned, public
/// functions among them.
fn free_without_clear(
    module: &Module,
    inputs: &Inputs,
    facts: &Facts,
) -> (Vec<String>, Vec<String>) {
    let mut lines = Vec::new();
    let mut fail = Vec::new();
    for (name, f) in &module.functions {
        let frame = facts.frame(name, f);
        for site in &f.held.sites {
            let SiteKind::Call(n) = site.kind else {
                continue;
            };
            if !matches!(f.calls[n].callee.as_str(), "lv_free" | "lv_realloc") {
                continue;
            }
            let Some(v) = site.args.first() else { continue };
            for p in frame.resolve_all(v) {
                let Ptr::Obj(r) = p else { continue };
                let global = r.path.first().is_some_and(|s| {
                    s.starts_with('@') || matches!(step_type(s), "lv_global_t" | "_lv_global_t")
                });
                if !global || r.root != Root::Abs {
                    continue;
                }
                let cleared = f.held.stores.iter().any(|(address, _, order)| {
                    *order > site.order
                        && store_path(&frame, address)
                            .iter()
                            .any(|q| q.ends_with(&r.path) || r.path.ends_with(q))
                });
                if !cleared {
                    lines.push(format!(
                        "free without clear: {name} frees {r} and never clears it"
                    ));
                    if inputs.access.get(name).is_some_and(|a| a == "shared")
                        && !inputs.owned.contains(name)
                    {
                        fail.push(name.clone());
                    }
                }
            }
        }
    }
    lines.sort();
    lines.dedup();
    fail.sort();
    fail.dedup();
    (lines, fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IR: &str = r#"
%struct._lv_event_list_t = type { %struct.lv_array_t, i8 }
%struct.lv_array_t = type { ptr, i32, i32 }
%struct.T = type { ptr }
%struct.D = type { ptr, ptr }
%struct.U = type { i32 }
define internal void @nothing() {
  ret void
}
define dso_local void @lv_free(ptr noundef %0) {
  ret void
}
define dso_local ptr @lv_realloc(ptr noundef %0, i64 noundef %1) {
  ret ptr null
}
define dso_local i32 @lv_obj_send_event(ptr noundef %0, i32 noundef %1, ptr noundef %2) {
  ret i32 0
}
define internal void @cleanup(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.lv_array_t, ptr %3, i32 0, i32 0
  %5 = load ptr, ptr %4, align 8
  %6 = call ptr @lv_realloc(ptr noundef %5, i64 noundef 8)
  ret void
}
define dso_local void @lv_list_add(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct._lv_event_list_t, ptr %3, i32 0, i32 0
  call void @cleanup(ptr noundef %4)
  ret void
}
define dso_local void @lv_list_send(ptr noundef %0) {
  %2 = alloca ptr, align 8
  %3 = alloca %struct.lv_array_t, align 8
  store ptr %0, ptr %2, align 8
  %4 = load ptr, ptr %2, align 8
  %5 = getelementptr inbounds nuw %struct._lv_event_list_t, ptr %4, i32 0, i32 0
  call void @llvm.memcpy.p0.p0.i64(ptr align 8 %3, ptr align 8 %5, i64 16, i1 false)
  %6 = call i32 @lv_obj_send_event(ptr noundef null, i32 noundef 10, ptr noundef null)
  call void @cleanup(ptr noundef %3)
  ret void
}
define dso_local void @lv_list_send_recopy(ptr noundef %0) {
  %2 = alloca ptr, align 8
  %3 = alloca %struct.lv_array_t, align 8
  store ptr %0, ptr %2, align 8
  %4 = load ptr, ptr %2, align 8
  %5 = getelementptr inbounds nuw %struct._lv_event_list_t, ptr %4, i32 0, i32 0
  call void @llvm.memcpy.p0.p0.i64(ptr align 8 %3, ptr align 8 %5, i64 16, i1 false)
  %6 = call i32 @lv_obj_send_event(ptr noundef null, i32 noundef 10, ptr noundef null)
  call void @llvm.memcpy.p0.p0.i64(ptr align 8 %3, ptr align 8 %5, i64 16, i1 false)
  call void @cleanup(ptr noundef %3)
  ret void
}
define dso_local void @lv_list_send_fresh(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = call i32 @lv_obj_send_event(ptr noundef null, i32 noundef 10, ptr noundef null)
  %4 = load ptr, ptr %2, align 8
  %5 = getelementptr inbounds nuw %struct._lv_event_list_t, ptr %4, i32 0, i32 0
  call void @cleanup(ptr noundef %5)
  ret void
}
define internal void @use_text(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = call i32 @lv_obj_send_event(ptr noundef null, i32 noundef 10, ptr noundef null)
  %4 = load ptr, ptr %2, align 8
  ret void
}
define dso_local void @lv_t_refresh(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.T, ptr %3, i32 0, i32 0
  %5 = load ptr, ptr %4, align 8
  call void @use_text(ptr noundef %5)
  ret void
}
define dso_local void @lv_t_set_text(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.T, ptr %3, i32 0, i32 0
  %5 = load ptr, ptr %4, align 8
  call void @lv_free(ptr noundef %5)
  ret void
}
define dso_local void @lv_t_set_text_static(ptr noundef %0, ptr noundef %1) {
  %3 = alloca ptr, align 8
  %4 = alloca ptr, align 8
  store ptr %0, ptr %3, align 8
  store ptr %1, ptr %4, align 8
  %5 = load ptr, ptr %4, align 8
  %6 = load ptr, ptr %3, align 8
  %7 = getelementptr inbounds nuw %struct.T, ptr %6, i32 0, i32 0
  store ptr %5, ptr %7, align 8
  ret void
}
define dso_local void @lv_d_open(ptr noundef %0, ptr noundef %1) {
  %3 = alloca ptr, align 8
  %4 = alloca ptr, align 8
  store ptr %0, ptr %3, align 8
  store ptr %1, ptr %4, align 8
  %5 = load ptr, ptr %3, align 8
  %6 = getelementptr inbounds nuw %struct.D, ptr %5, i32 0, i32 0
  %7 = load ptr, ptr %6, align 8
  %8 = load ptr, ptr %4, align 8
  call void @lv_t_set_text_static(ptr noundef %8, ptr noundef %7)
  ret void
}
define dso_local void @lv_d_set_options(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.D, ptr %3, i32 0, i32 0
  %5 = load ptr, ptr %4, align 8
  call void @lv_free(ptr noundef %5)
  ret void
}
define dso_local void @lv_u_hold(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = call i32 @lv_obj_send_event(ptr noundef null, i32 noundef 30, ptr noundef null)
  %4 = load ptr, ptr %2, align 8
  %5 = getelementptr inbounds nuw %struct.U, ptr %4, i32 0, i32 0
  ret void
}
define dso_local void @lv_u_delete(ptr noundef %0) {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %3 = load ptr, ptr %2, align 8
  %4 = getelementptr inbounds nuw %struct.U, ptr %3, i32 0, i32 0
  call void @lv_free(ptr noundef %3)
  ret void
}
"#;

    fn run(guarded: &str) -> Analysis {
        let mut m = Module::default();
        m.add_ir("t", IR, &[]);
        let config: AuditConfig = toml::from_str(&format!(
            r#"
sinks = ["t::nothing"]
object_sender = "lv_obj_send_event"
free_primitives = {{ lv_free = 0, lv_realloc = 0 }}
blockable_codes = ["LV_EVENT_COVER_CHECK"]
[send]
lv_obj_send_event = 1
{guarded}
"#
        ))
        .unwrap();
        let graph = Graph::build(&m, &config).unwrap();
        let access: BTreeMap<String, String> = m
            .functions
            .keys()
            .filter(|n| {
                n.starts_with("lv_")
                    && !matches!(n.as_str(), "lv_free" | "lv_realloc" | "lv_obj_send_event")
            })
            .map(|n| (n.clone(), "shared".to_owned()))
            .collect();
        let names = BTreeMap::from([
            (10, "LV_EVENT_CLICKED".to_owned()),
            (30, "LV_EVENT_COVER_CHECK".to_owned()),
        ]);
        let handler = BTreeMap::from([(10, "shared".to_owned()), (30, "restricted".to_owned())]);
        let none = BTreeSet::new();
        analyze(
            &m,
            &graph,
            &config,
            &Inputs {
                access: &access,
                fixed: &none,
                owned: &none,
                handler: &handler,
                code_names: &names,
            },
        )
        .unwrap()
    }

    fn held_by<'a>(a: &'a Analysis, holder: &str) -> Vec<&'a Hazard> {
        a.hazards.iter().filter(|h| h.holder == holder).collect()
    }

    #[test]
    fn struct_copy_held_across_a_send_makes_its_freer_exclusive() {
        // T9 (a): `lv_array_t copy = list->array; send; cleanup(&copy)`.
        let a = run("");
        let hazards = held_by(&a, "lv_list_send");
        assert!(!hazards.is_empty(), "{:#?}", a.hazards);
        assert!(hazards.iter().all(|h| h.code == 10));
        assert!(hazards.iter().any(|h| h.freer == "lv_list_add"
            && h.hold.to_string() == "Copy(_lv_event_list_t.0/*)"
            && h.free.to_string() == "Buf(_lv_event_list_t.0/lv_array_t.0)"));
        let why = &a.promote["lv_list_add"];
        assert!(why.starts_with("frees Copy(_lv_event_list_t.0/*), held by lv_list_send keeps %3 across lv_obj_send_event"), "{why}");
        assert!(why.ends_with("(codes CLICKED)"), "{why}");
        // Copied again after the send: the stale copy is dead.
        assert!(held_by(&a, "lv_list_send_recopy").is_empty());
    }

    #[test]
    fn a_field_read_again_after_the_send_is_not_a_hold() {
        // T9 (b).
        let a = run("");
        assert!(held_by(&a, "lv_list_send_fresh").is_empty());
    }

    #[test]
    fn a_parameter_hold_is_charged_to_the_callee_with_the_callers_region() {
        // T9 (c): `use_text(t->text)` keeps its parameter across a send.
        let a = run("");
        let hazards = held_by(&a, "t::use_text");
        assert!(
            hazards
                .iter()
                .any(|h| h.freer == "lv_t_set_text" && h.hold.to_string() == "Buf(T.0)"),
            "{:#?}",
            a.hazards
        );
        assert!(a.promote.contains_key("lv_t_set_text"));
    }

    #[test]
    fn blockable_codes_restrict_handlers_instead() {
        // T9 (d): held across COVER_CHECK only.
        let a = run("");
        assert!(
            held_by(&a, "lv_u_hold")
                .iter()
                .any(|h| h.freer == "lv_u_delete" && h.code == 30)
        );
        assert!(a.blocked.contains_key(&30));
        assert!(!a.promote.contains_key("lv_u_delete"));
        assert!(a.restrict.is_empty());
    }

    #[test]
    fn stored_aliases_are_listed_for_review() {
        // T9 (e): `lv_t_set_text_static(label, d->options)`.
        let a = run("");
        assert!(
            a.report
                .iter()
                .any(|l| l.starts_with("alias: T.0 keeps Buf(D.0)")
                    && l.contains("lv_d_set_options")),
            "{:#?}",
            a.report
        );
    }

    #[test]
    fn guarded_holds_are_not_counted() {
        let a = run(r#"
[guarded.list]
regions = ["Copy(*_lv_event_list_t.0/*)"]
freers = ["lv_list_add"]
check = "test"
why = "test"
"#);
        let hazards = held_by(&a, "lv_list_send");
        assert!(hazards.iter().any(|h| h.freer == "lv_list_add"));
        assert!(
            hazards
                .iter()
                .filter(|h| h.freer == "lv_list_add")
                .all(|h| h.guarded)
        );
        assert!(!a.promote.contains_key("lv_list_add"));
    }

    #[test]
    fn regions_match_by_type_path_suffix_or_copy() {
        let obj = |t: &str| Reg::abs(Vec::new(), false, Some(t.into()));
        let buf = |p: &[&str]| Reg::abs(p.iter().map(|s| s.to_string()).collect(), false, None);
        let copy = |p: &[&str]| Reg::abs(p.iter().map(|s| s.to_string()).collect(), true, None);
        assert!(matches(&obj("_lv_indev_t"), &obj("_lv_indev_t")));
        assert!(!matches(&obj("_lv_indev_t"), &obj("_lv_timer_t")));
        assert!(matches(&buf(&["_lv_label_t.1"]), &buf(&["_lv_label_t.1"])));
        assert!(matches(
            &buf(&["a.0", "_lv_label_t.1"]),
            &buf(&["_lv_label_t.1"])
        ));
        // One lv_array_t is not every lv_array_t.
        assert!(!matches(
            &buf(&["x.0", "lv_array_t.0"]),
            &buf(&["y.0", "lv_array_t.0"])
        ));
        assert!(matches(
            &copy(&["e.0"]),
            &buf(&["o.2", "e.0", "lv_array_t.0"])
        ));
        assert!(!matches(
            &copy(&["e.0"]),
            &buf(&["o.2", "f.0", "lv_array_t.0"])
        ));
    }
}
