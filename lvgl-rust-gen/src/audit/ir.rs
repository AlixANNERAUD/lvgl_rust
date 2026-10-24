//! Compiles LVGL's sources to textual LLVM IR with clang and reads what the
//! call graph needs.
//!
//! Callbacks are tracked by *slot*: the struct field a function pointer is
//! stored in (`_lv_anim_t.5`, `_lv_obj_class_t.3`, ...). Stores of a
//! function, or of a parameter (setters such as `lv_anim_set_deleted_cb`),
//! into a field, global initializers, and indirect calls through a pointer
//! loaded from a field are recorded per function; the graph then connects an
//! indirect call only to the functions that can be in its slot. At `-O0`
//! every value goes through an `alloca`, which the reader follows.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, atomic::AtomicUsize, atomic::Ordering},
    thread,
};

/// A call argument, as far as the audit cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// A function passed by address (`@cb`), qualified like function names.
    Function(String),
    /// The caller's own parameter.
    Param(usize),
    Int(i64),
    /// One of a few constants (`select i1 %c, i32 46, i32 45`).
    Ints(Vec<i64>),
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Call {
    /// Qualified callee name.
    pub callee: String,
    pub args: Vec<Arg>,
    /// Event codes under which the call can run (`None`: any), from guards
    /// on `lv_event_get_code(<event parameter>)`.
    pub codes: Option<Vec<i64>>,
    block: usize,
}

/// Where an indirect call gets its target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Loaded from a struct field.
    Slot(String),
    /// The function's own parameter.
    Param(usize),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indirect {
    pub target: Target,
    pub signature: String,
    pub args: Vec<Arg>,
    pub codes: Option<Vec<i64>>,
    block: usize,
}

/// A value stored into a slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stored {
    Function(String),
    Param(usize),
}

#[derive(Clone, Debug, Default)]
pub struct Function {
    /// Normalised signature: return type and parameter types.
    pub signature: String,
    pub calls: Vec<Call>,
    pub indirect: Vec<Indirect>,
    pub stores: Vec<(String, Stored)>,
    /// Functions whose address escapes elsewhere (stored to an unknown
    /// place, compared, ...).
    pub refs: Vec<String>,
    /// Parameters passed to `lv_event_get_code`.
    pub event_params: Vec<usize>,
    /// The translation unit (`widgets/dropdown/lv_dropdown`).
    pub unit: String,
}

/// A branch condition on the event code.
#[derive(Clone, Copy, Debug)]
enum Cond {
    Eq(i64),
    Ne(i64),
}

#[derive(Default)]
struct Block {
    /// Successors and what the edge implies about the code: `Some(true)`
    /// when the condition holds, `Some(false)` when it does not.
    succ: Vec<(String, Option<(Cond, bool)>)>,
    /// `switch` cases on the code: label to value; the default excludes
    /// them all.
    cases: Vec<(String, i64)>,
    default: Option<String>,
}

#[derive(Debug, Default)]
pub struct Module {
    /// Defined functions. Internal (`static`) ones are named `unit::name`.
    pub functions: BTreeMap<String, Function>,
    /// Functions in struct fields of global initializers: `(slot, function)`.
    pub global_slots: Vec<(String, String)>,
    /// Functions referenced by other global initializers.
    pub global_refs: Vec<String>,
}

pub struct CompileOptions<'a> {
    pub clang: &'a str,
    pub include_dirs: Vec<PathBuf>,
    pub defines: Vec<String>,
    pub out_dir: PathBuf,
}

/// Compiles every `.c` file of `sources` to IR in `out_dir` (in parallel);
/// the IR of `sources[i]` is returned at index `i`.
pub fn compile(sources: &[PathBuf], options: &CompileOptions) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(&options.out_dir).map_err(|e| e.to_string())?;
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    let jobs = thread::available_parallelism().map_or(4, |n| n.get());
    thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(source) = sources.get(i) else { break };
                    let result = compile_one(source, i, options);
                    results.lock().unwrap().push((i, result));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().map(|(_, r)| r).collect()
}

fn compile_one(source: &Path, index: usize, options: &CompileOptions) -> Result<PathBuf, String> {
    let stem = source.file_stem().unwrap().to_string_lossy();
    let out = options.out_dir.join(format!("{index:04}-{stem}.ll"));
    let mut command = Command::new(options.clang);
    // A fixed target: the IR, and so the audit, are the same on any host.
    command
        .args([
            "-S",
            "-emit-llvm",
            "-O0",
            "-w",
            "--target=x86_64-unknown-none-elf",
            "-ffreestanding",
            "-nostdlibinc",
            "-o",
        ])
        .arg(&out);
    for dir in &options.include_dirs {
        command.arg(format!("-I{}", dir.display()));
    }
    for define in &options.defines {
        command.arg(format!("-D{define}"));
    }
    // Sources that include a C library header before their configuration
    // guard get an empty stand-in: the code using it is compiled out.
    let stubs = options.out_dir.join("stubs");
    command.arg(format!("-I{}", stubs.display()));
    command.arg(source);
    for _ in 0..16 {
        let output = command
            .output()
            .map_err(|e| format!("cannot run {}: {e}", options.clang))?;
        if output.status.success() {
            return Ok(out);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let missing = stderr.lines().find_map(|l| {
            l.split("fatal error: '")
                .nth(1)?
                .split("' file not found")
                .next()
        });
        match missing {
            Some(header) if !header.contains("..") => {
                let stub = stubs.join(header);
                fs::create_dir_all(stub.parent().unwrap()).map_err(|e| e.to_string())?;
                fs::write(&stub, "/* empty stand-in for the audit */\n")
                    .map_err(|e| e.to_string())?;
            }
            _ => return Err(format!("{} failed:\n{stderr}", source.display())),
        }
    }
    Err(format!("{}: too many missing headers", source.display()))
}

/// Every `.c` file under `dir`, sorted.
pub fn c_sources(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "c") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// What a register holds, as far as the reader can tell.
#[derive(Clone, Debug)]
enum Origin {
    Param(usize),
    Function(String),
    /// The address of a struct field.
    Field(String),
    /// A value loaded from a struct field.
    Loaded(String),
    /// The code of the event parameter (`lv_event_get_code(param)`).
    Code,
    /// A comparison of the code with a constant.
    Condition(Cond),
    /// One of a few integer constants.
    Ints(Vec<i64>),
    /// An `alloca`.
    Local(String),
    Unknown,
}

struct Reader<'a> {
    qualify: &'a dyn Fn(&str) -> String,
    types: &'a HashMap<String, Vec<String>>,
    values: HashMap<String, Origin>,
    /// Every value stored in each `alloca` (a load is known only if they
    /// all agree).
    locals: HashMap<String, Vec<Origin>>,
    blocks: Vec<(String, Block)>,
    /// An open `switch` on the code, until its closing `]`.
    switch: Option<bool>,
}

impl Module {
    /// Adds one translation unit; `unit` names its internal symbols.
    /// `literal_globals` maps global name patterns to the struct type of
    /// globals emitted with a literal type (bitfields): `lv_*_class` to
    /// `_lv_obj_class_t`.
    pub fn add_ir(&mut self, unit: &str, ir: &str, literal_globals: &[(String, String)]) {
        let mut internal = std::collections::BTreeSet::new();
        for line in ir.lines() {
            if let Some(rest) = line.strip_prefix("define ") {
                if (rest.starts_with("internal ") || rest.starts_with("private "))
                    && let Some(name) = defined_name(line)
                {
                    internal.insert(name);
                }
            } else if line.starts_with('@')
                && (line.contains(" internal ") || line.contains(" private "))
                && let Some((name, _)) = line.split_once(" = ")
            {
                internal.insert(unquote(&name[1..]).to_owned());
            }
        }
        let qualify = |name: &str| -> String {
            if internal.contains(name) {
                format!("{unit}::{name}")
            } else {
                name.to_owned()
            }
        };

        // Struct member types, to normalise nested field paths.
        let mut types: HashMap<String, Vec<String>> = HashMap::new();
        for line in ir.lines() {
            if let Some(rest) = line.strip_prefix("%struct.")
                && let Some((name, body)) = rest.split_once(" = type ")
            {
                let body = body.trim();
                if body.starts_with('{') {
                    let members = split_args(balanced(body))
                        .into_iter()
                        .map(str::to_owned)
                        .collect();
                    types.insert(struct_name(name), members);
                }
            }
        }

        let mut current: Option<(String, Function, Reader)> = None;
        for line in ir.lines() {
            if line.starts_with("define ") {
                if let Some(name) = defined_name(line) {
                    let mut reader = Reader {
                        qualify: &qualify,
                        types: &types,
                        values: HashMap::new(),
                        locals: HashMap::new(),
                        blocks: vec![("".to_owned(), Block::default())],
                        switch: None,
                    };
                    for (i, param) in param_names(line).into_iter().enumerate() {
                        reader.values.insert(param, Origin::Param(i));
                    }
                    let f = Function {
                        signature: signature_of_define(line),
                        unit: unit.to_owned(),
                        ..Function::default()
                    };
                    current = Some((qualify(&name), f, reader));
                }
                continue;
            }
            if line == "}" {
                if let Some((name, mut f, reader)) = current.take() {
                    reader.finish(&mut f);
                    self.functions.insert(name, f);
                }
                continue;
            }
            if let Some((_, f, reader)) = current.as_mut() {
                reader.instruction(line.trim(), f);
            } else if line.starts_with('@') {
                self.read_global(line, &qualify, literal_globals);
            }
        }
    }

    fn read_global(
        &mut self,
        line: &str,
        qualify: &dyn Fn(&str) -> String,
        literal: &[(String, String)],
    ) {
        let Some((name, rest)) = line.split_once(" = ") else {
            return;
        };
        let name = unquote(&name[1..]);
        let Some(at) = rest
            .find(" constant ")
            .map(|i| i + 10)
            .or_else(|| rest.find(" global ").map(|i| i + 8))
        else {
            return;
        };
        let body = rest[at..].trim_start();
        let (ty, init) = split_type(body);
        let strukt = match ty.strip_prefix("%struct.") {
            Some(s) => Some(struct_name(s)),
            None if ty.starts_with('{') => literal
                .iter()
                .find(|(pattern, _)| crate::tables::glob(pattern, name))
                .map(|(_, s)| s.clone()),
            None => None,
        };
        let init = init.trim();
        match strukt {
            Some(s) if init.starts_with('{') => {
                for (i, element) in split_args(balanced(init)).into_iter().enumerate() {
                    for symbol in symbols(element) {
                        self.global_slots
                            .push((format!("{s}.{i}"), qualify(&symbol)));
                    }
                }
            }
            _ => self.global_refs.extend(symbols(init).map(|s| qualify(&s))),
        }
    }
}

impl Reader<'_> {
    fn block(&mut self) -> &mut Block {
        &mut self.blocks.last_mut().unwrap().1
    }

    /// Computes the event codes each block can run under (forward
    /// dataflow over the CFG) and stores them on the calls.
    fn finish(self, f: &mut Function) {
        type Codes = Option<std::collections::BTreeSet<i64>>; // None: any
        let index: HashMap<&str, usize> = self
            .blocks
            .iter()
            .enumerate()
            .map(|(i, (label, _))| (label.as_str(), i))
            .collect();
        let mut state: Vec<Option<Codes>> = vec![None; self.blocks.len()];
        state[0] = Some(None);
        let narrow = |codes: &Codes, cond: Cond, holds: bool| -> Codes {
            let (value, equal) = match cond {
                Cond::Eq(v) => (v, holds),
                Cond::Ne(v) => (v, !holds),
            };
            if equal {
                let only = std::collections::BTreeSet::from([value]);
                Some(match codes {
                    None => only,
                    Some(set) => set.intersection(&only).copied().collect(),
                })
            } else {
                codes
                    .as_ref()
                    .map(|set| set.iter().copied().filter(|v| *v != value).collect())
            }
        };
        let merge = |slot: &mut Option<Codes>, codes: Codes| -> bool {
            let merged = match (slot.take(), codes) {
                (None, codes) => codes,
                (Some(None), _) | (Some(_), None) => None,
                (Some(Some(a)), Some(b)) => Some(a.union(&b).copied().collect()),
            };
            let changed = slot.as_ref() != Some(&merged);
            *slot = Some(merged);
            changed
        };
        let mut changed = true;
        let mut rounds = 0;
        while changed && rounds < 64 {
            changed = false;
            rounds += 1;
            for (i, (_, block)) in self.blocks.iter().enumerate() {
                let Some(codes) = state[i].clone() else {
                    continue;
                };
                let mut out: Vec<(usize, Codes)> = Vec::new();
                for (label, cond) in &block.succ {
                    let Some(&j) = index.get(label.as_str()) else {
                        continue;
                    };
                    let next = match cond {
                        Some((cond, holds)) => narrow(&codes, *cond, *holds),
                        None => codes.clone(),
                    };
                    out.push((j, next));
                }
                for (label, value) in &block.cases {
                    if let Some(&j) = index.get(label.as_str()) {
                        out.push((j, narrow(&codes, Cond::Eq(*value), true)));
                    }
                }
                if let Some(default) = &block.default
                    && let Some(&j) = index.get(default.as_str())
                {
                    let mut next = codes.clone();
                    for (_, value) in &block.cases {
                        next = narrow(&next, Cond::Eq(*value), false);
                    }
                    out.push((j, next));
                }
                for (j, next) in out {
                    changed |= merge(&mut state[j], next);
                }
            }
        }
        let codes_of = |block: usize| -> Option<Vec<i64>> {
            match state.get(block).cloned().flatten() {
                Some(Some(set)) => Some(set.into_iter().collect()),
                _ => None,
            }
        };
        for call in &mut f.calls {
            call.codes = codes_of(call.block);
        }
        for indirect in &mut f.indirect {
            indirect.codes = codes_of(indirect.block);
        }
    }

    fn instruction(&mut self, line: &str, f: &mut Function) {
        // Labels open blocks; the previous block falls through only via its
        // terminator, which names its successors.
        if let Some(label) = block_label(line) {
            self.blocks.push((label, Block::default()));
            return;
        }
        if let Some(on_code) = self.switch {
            // `i32 23, label %45` lines until `]`.
            if line.starts_with(']') {
                self.switch = None;
                return;
            }
            let parts = split_args(line);
            if let (Some(value), Some(label)) = (parts.first(), parts.get(1)) {
                let target = label.rsplit('%').next().unwrap_or("").to_owned();
                match (
                    on_code,
                    value.rsplit(' ').next().and_then(|v| v.parse().ok()),
                ) {
                    (true, Some(v)) => self.block().cases.push((target, v)),
                    _ => self.block().succ.push((target, None)),
                }
            }
            return;
        }
        if let Some(rest) = line.strip_prefix("br ") {
            let labels: Vec<String> = rest
                .split("label %")
                .skip(1)
                .map(|l| l.split([',', ' ']).next().unwrap_or("").to_owned())
                .collect();
            let cond = match rest.strip_prefix("i1 ") {
                Some(c) => match self.operand(c.split(',').next().unwrap_or("")) {
                    Origin::Condition(cond) => Some(cond),
                    _ => None,
                },
                None => None,
            };
            match (cond, labels.as_slice()) {
                (Some(cond), [then, otherwise]) => {
                    self.block().succ.push((then.clone(), Some((cond, true))));
                    self.block()
                        .succ
                        .push((otherwise.clone(), Some((cond, false))));
                }
                _ => {
                    for label in labels {
                        self.block().succ.push((label, None));
                    }
                }
            }
            return;
        }
        if let Some(rest) = line.strip_prefix("switch ") {
            // `switch i32 %v, label %default [`
            let mut parts = rest.splitn(2, ',');
            let on_code = matches!(self.operand(parts.next().unwrap_or("")), Origin::Code);
            let default = parts
                .next()
                .and_then(|p| p.split("label %").nth(1))
                .map(|l| l.split([' ', '[']).next().unwrap_or("").to_owned());
            if let Some(default) = default {
                if on_code {
                    self.block().default = Some(default);
                } else {
                    self.block().succ.push((default, None));
                }
            }
            self.switch = Some(on_code);
            if rest.trim_end().ends_with(']') {
                self.switch = None;
            }
            return;
        }
        let (def, inst) = match line.split_once(" = ") {
            Some((def, inst)) if def.starts_with('%') => (Some(def.to_owned()), inst.trim()),
            _ => (None, line),
        };
        if let Some(rest) = inst.strip_prefix("alloca ") {
            let _ = rest;
            if let Some(def) = def {
                self.values.insert(def.clone(), Origin::Local(def));
            }
            return;
        }
        if let Some(rest) = inst.strip_prefix("load ") {
            // `load <ty>, ptr <op>, align N`
            let operands = split_args(rest);
            let origin = match operands.get(1).map(|o| self.operand(o)) {
                Some(Origin::Local(slot)) => match self.locals.get(&slot).map(Vec::as_slice) {
                    Some([only]) => only.clone(),
                    _ => Origin::Unknown,
                },
                Some(Origin::Field(slot)) => Origin::Loaded(slot),
                _ => Origin::Unknown,
            };
            if let Some(def) = def {
                self.values.insert(def, origin);
            }
            return;
        }
        if let Some(rest) = inst.strip_prefix("select ") {
            // `select i1 %c, i32 46, i32 45`
            let operands = split_args(rest);
            let ints: Vec<i64> = operands
                .iter()
                .skip(1)
                .filter_map(|o| o.rsplit(' ').next()?.parse().ok())
                .collect();
            let origin = if operands.len() == 3 && ints.len() == 2 {
                Origin::Ints(ints)
            } else {
                Origin::Unknown
            };
            if let Some(def) = def {
                self.values.insert(def, origin);
            }
            return;
        }
        if let Some(rest) = inst.strip_prefix("icmp ") {
            // `icmp eq i32 %code, 23`
            let mut words = rest.splitn(2, ' ');
            let predicate = words.next().unwrap_or("");
            let operands = split_args(words.next().unwrap_or(""));
            let origin = match (operands.first().map(|o| self.operand(o)), operands.get(1)) {
                (Some(Origin::Code), Some(value)) => match (predicate, value.trim().parse::<i64>())
                {
                    ("eq", Ok(v)) => Origin::Condition(Cond::Eq(v)),
                    ("ne", Ok(v)) => Origin::Condition(Cond::Ne(v)),
                    _ => Origin::Unknown,
                },
                _ => Origin::Unknown,
            };
            if let Some(def) = def {
                self.values.insert(def, origin);
            }
            return;
        }
        if inst.starts_with("getelementptr ") {
            let origin = gep(inst, self.types);
            if let Some(def) = def {
                self.values.insert(def, origin);
            }
            return;
        }
        if let Some(rest) = inst.strip_prefix("store ") {
            let operands = split_args(rest);
            if operands.len() < 2 {
                return;
            }
            let value = self.operand(operands[0]);
            let address = self.operand(operands[1]);
            match (&address, &value) {
                (Origin::Local(slot), _) => {
                    let stored = self.locals.entry(slot.clone()).or_default();
                    // Distinct values make the local unknown when loaded.
                    if !stored.iter().any(|o| same(o, &value)) {
                        stored.push(value.clone());
                    }
                }
                (Origin::Field(slot), Origin::Function(g)) => {
                    f.stores.push((slot.clone(), Stored::Function(g.clone())))
                }
                (Origin::Field(slot), Origin::Param(k)) => {
                    f.stores.push((slot.clone(), Stored::Param(*k)))
                }
                (_, Origin::Function(g)) => f.refs.push(g.clone()),
                _ => {}
            }
            return;
        }
        if let Some(at) = find_call(inst) {
            self.call(&inst[at..], def, f);
            return;
        }
        // Any other use of a function symbol takes its address.
        for symbol in symbols(inst) {
            f.refs.push((self.qualify)(&symbol));
        }
        if let Some(def) = def {
            self.values.insert(def, Origin::Unknown);
        }
    }

    fn call(&mut self, after: &str, def: Option<String>, f: &mut Function) {
        let Some(open) = callee_paren(after) else {
            return;
        };
        let head = &after[..open];
        let callee = head.rsplit(' ').next().unwrap_or("");
        let args: Vec<&str> = split_args(balanced(&after[open..]));
        let block = self.blocks.len() - 1;
        if callee == "@lv_event_get_code"
            && let Some(Origin::Param(k)) = args.first().map(|a| self.operand(a))
        {
            if !f.event_params.contains(&k) {
                f.event_params.push(k);
            }
            if let Some(def) = def {
                self.values.insert(def, Origin::Code);
            }
            f.calls.push(Call {
                callee: "lv_event_get_code".into(),
                args: vec![Arg::Param(k)],
                codes: None,
                block,
            });
            return;
        }
        if let Some(def) = def.clone() {
            self.values.insert(def, Origin::Unknown);
        }
        let target = if let Some(name) = callee.strip_prefix('@') {
            let name = unquote(name);
            if name.starts_with("llvm.") {
                return;
            }
            Some((self.qualify)(name))
        } else {
            match self.operand(callee) {
                Origin::Function(g) => Some(g),
                origin => {
                    let ret = head
                        .split_whitespace()
                        .find(|t| {
                            !matches!(
                                *t,
                                "call"
                                    | "tail"
                                    | "musttail"
                                    | "notail"
                                    | "noundef"
                                    | "zeroext"
                                    | "signext"
                            ) && !t.starts_with('%')
                        })
                        .unwrap_or("void");
                    let params: Vec<&str> = args.iter().map(|a| first_type(a)).collect();
                    let target = match origin {
                        Origin::Loaded(slot) => Target::Slot(slot),
                        Origin::Param(k) => Target::Param(k),
                        _ => Target::Unknown,
                    };
                    let parsed = args.iter().map(|a| self.arg(a)).collect();
                    f.indirect.push(Indirect {
                        target,
                        signature: format!("{ret}({})", params.join(",")),
                        args: parsed,
                        codes: None,
                        block,
                    });
                    None
                }
            }
        };
        let parsed: Vec<Arg> = args.iter().map(|a| self.arg(a)).collect();
        if let Some(callee) = target {
            f.calls.push(Call {
                callee,
                args: parsed,
                codes: None,
                block,
            });
        } else {
            // Functions passed to an indirect call escape.
            for arg in parsed {
                if let Arg::Function(g) = arg {
                    f.refs.push(g);
                }
            }
        }
    }

    fn arg(&self, text: &str) -> Arg {
        match self.operand(text) {
            Origin::Function(g) => Arg::Function(g),
            Origin::Param(k) => Arg::Param(k),
            Origin::Ints(values) => Arg::Ints(values),
            _ => match text.rsplit(' ').next().and_then(|v| v.parse().ok()) {
                Some(v) => Arg::Int(v),
                None => Arg::Other,
            },
        }
    }

    /// The origin of an operand (`ptr noundef %5`, `@f`, a constant GEP).
    fn operand(&self, text: &str) -> Origin {
        let text = text.trim();
        if let Some(i) = text.find("getelementptr") {
            let expr = &text[i..];
            if let Some(open) = expr.find('(') {
                return gep_expr(balanced(&expr[open..]), self.types);
            }
        }
        let value = text.rsplit(' ').next().unwrap_or("");
        if let Some(name) = value.strip_prefix('@') {
            let name = unquote(name);
            if name.starts_with(".str") || name.starts_with("llvm.") {
                return Origin::Unknown;
            }
            return Origin::Function((self.qualify)(name));
        }
        if value.starts_with('%') {
            return self.values.get(value).cloned().unwrap_or(Origin::Unknown);
        }
        Origin::Unknown
    }
}

/// Whether two origins name the same value.
fn same(a: &Origin, b: &Origin) -> bool {
    match (a, b) {
        (Origin::Param(x), Origin::Param(y)) => x == y,
        (Origin::Function(x), Origin::Function(y))
        | (Origin::Field(x), Origin::Field(y))
        | (Origin::Loaded(x), Origin::Loaded(y))
        | (Origin::Local(x), Origin::Local(y)) => x == y,
        (Origin::Code, Origin::Code) => true,
        _ => false,
    }
}

/// `getelementptr [inbounds] [nuw] %struct.T, ptr %p, i32 0, i32 5, ...`
fn gep(inst: &str, types: &HashMap<String, Vec<String>>) -> Origin {
    let rest = inst
        .trim_start_matches("getelementptr")
        .trim_start()
        .trim_start_matches("inbounds")
        .trim_start()
        .trim_start_matches("nuw")
        .trim_start()
        .trim_start_matches("nusw")
        .trim_start();
    gep_expr(rest, types)
}

/// The operands of a GEP: type, base, indices. The slot is the innermost
/// struct and its field (`lv_timer_state_t.11` whether reached from
/// `lv_global` or from a `lv_timer_state_t *`).
fn gep_expr(operands: &str, types: &HashMap<String, Vec<String>>) -> Origin {
    let parts = split_args(operands);
    let Some(ty) = parts.first() else {
        return Origin::Unknown;
    };
    let Some(strukt) = ty.trim().strip_prefix("%struct.") else {
        return Origin::Unknown;
    };
    // Skip the base pointer and the first (pointer arithmetic) index.
    let indices: Vec<Option<i64>> = parts
        .iter()
        .skip(3)
        .map(|i| i.rsplit(' ').next().and_then(|v| v.parse().ok()))
        .collect();
    if indices.is_empty() {
        return Origin::Unknown;
    }
    let mut current = struct_name(strukt);
    let mut i = 0;
    while i < indices.len() {
        let Some(field) = indices[i] else {
            return Origin::Field(format!("{current}.*"));
        };
        if i + 1 == indices.len() {
            return Origin::Field(format!("{current}.{field}"));
        }
        let Some(member) = types.get(&current).and_then(|m| m.get(field as usize)) else {
            // Unknown layout: keep the raw path.
            let rest: Vec<String> = indices[i..]
                .iter()
                .map(|v| v.map_or("*".into(), |v| v.to_string()))
                .collect();
            return Origin::Field(format!("{current}.{}", rest.join(".")));
        };
        let mut member = member.trim();
        // `[N x T]`: the next index selects an element.
        if member.starts_with('[') {
            member = balanced(member)
                .split_once(" x ")
                .map_or("", |(_, t)| t.trim());
            i += 1;
            if i + 1 == indices.len() {
                return Origin::Field(format!("{current}.{field}"));
            }
        }
        match member.strip_prefix("%struct.") {
            Some(inner) => current = struct_name(inner),
            None => return Origin::Field(format!("{current}.{field}")),
        }
        i += 1;
    }
    Origin::Unknown
}

/// `_lv_anim_t` for `_lv_anim_t.123` (LLVM renames clashing types).
fn struct_name(name: &str) -> String {
    let name = name.trim_matches('"');
    match name.rsplit_once('.') {
        Some((base, suffix)) if suffix.chars().all(|c| c.is_ascii_digit()) => base.to_owned(),
        _ => name.to_owned(),
    }
}

/// Splits `<type> <initializer>` at the end of the type.
fn split_type(body: &str) -> (&str, &str) {
    if body.starts_with('{') || body.starts_with('<') || body.starts_with('[') {
        let mut depth = 0;
        for (i, c) in body.char_indices() {
            match c {
                '{' | '<' | '[' | '(' => depth += 1,
                '}' | '>' | ']' | ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return (&body[..=i], &body[i + 1..]);
                    }
                }
                _ => {}
            }
        }
        (body, "")
    } else {
        body.split_once(' ').unwrap_or((body, ""))
    }
}

/// `45:` or `if.then:` (a block label, possibly followed by a comment).
fn block_label(line: &str) -> Option<String> {
    let (label, rest) = line.split_once(':')?;
    let valid = !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$' | '-'));
    let rest = rest.trim_start();
    (valid && (rest.is_empty() || rest.starts_with(';'))).then(|| label.to_owned())
}

fn find_call(line: &str) -> Option<usize> {
    for marker in ["call ", "invoke "] {
        if let Some(i) = line.find(marker) {
            let before = &line[..i];
            if before.is_empty()
                || before.ends_with("tail ")
                || before.ends_with("musttail ")
                || before.ends_with("notail ")
            {
                return Some(i);
            }
        }
    }
    None
}

/// Index of the `(` opening the argument list of the callee.
fn callee_paren(after: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in after.char_indices() {
        match c {
            '(' if depth == 0 => {
                let last = after[..i].rsplit(' ').next().unwrap_or("");
                if last.starts_with('@') || last.starts_with('%') {
                    return Some(i);
                }
                depth += 1;
            }
            '(' | '{' | '[' | '<' => depth += 1,
            ')' | '}' | ']' | '>' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The text between the bracket at the start of `s` and its match.
fn balanced(s: &str) -> &str {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '{' | '[' | '<' => depth += 1,
            ')' | '}' | ']' | '>' => {
                depth -= 1;
                if depth == 0 {
                    return &s[1..i];
                }
            }
            _ => {}
        }
    }
    s.get(1..).unwrap_or("")
}

fn split_args(s: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '{' | '[' | '<' => depth += 1,
            ')' | '}' | ']' | '>' => depth -= 1,
            ',' if depth == 0 => {
                args.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        args.push(last);
    }
    args
}

fn first_type(arg: &str) -> &str {
    arg.split_whitespace().next().unwrap_or("")
}

/// `@name` references in a line (excluding intrinsics and strings).
fn symbols(line: &str) -> impl Iterator<Item = String> + '_ {
    line.match_indices('@').filter_map(move |(i, _)| {
        let rest = &line[i + 1..];
        let name: String = if let Some(quoted) = rest.strip_prefix('"') {
            quoted.split('"').next()?.to_owned()
        } else {
            rest.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$'))
                .collect()
        };
        (!name.is_empty() && !name.starts_with('.') && !name.starts_with("llvm.")).then_some(name)
    })
}

fn defined_name(line: &str) -> Option<String> {
    let at = line.find('@')?;
    let rest = &line[at + 1..];
    let name = if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next()?
    } else {
        rest.split('(').next()?
    };
    Some(name.to_owned())
}

fn param_names(line: &str) -> Vec<String> {
    let at = line.find('@').unwrap_or(0);
    let Some(open) = line[at..].find('(').map(|i| i + at) else {
        return Vec::new();
    };
    split_args(balanced(&line[open..]))
        .iter()
        .filter_map(|p| {
            p.rsplit(' ')
                .next()
                .filter(|n| n.starts_with('%'))
                .map(str::to_owned)
        })
        .collect()
}

fn signature_of_define(line: &str) -> String {
    let at = line.find('@').unwrap_or(0);
    let head: Vec<&str> = line[..at].split_whitespace().collect();
    let ret = head.last().copied().unwrap_or("void");
    let params = match line[at..].find('(') {
        Some(open) => balanced(&line[at + open..]),
        None => "",
    };
    let params: Vec<&str> = split_args(params).iter().map(|a| first_type(a)).collect();
    format!("{ret}({})", params.join(","))
}

fn unquote(name: &str) -> &str {
    name.trim_matches('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_callbacks_through_fields_locals_and_parameters() {
        let mut m = Module::default();
        let literal = [("lv_*_class".to_owned(), "_lv_obj_class_t".to_owned())];
        m.add_ir(
            "anim",
            r#"
@lv_msgbox_class = dso_local constant { ptr, ptr, ptr } { ptr @lv_obj_class, ptr @lv_msgbox_constructor, ptr null }, align 8
define dso_local void @lv_anim_set_deleted_cb(ptr noundef %0, ptr noundef %1) #0 {
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
define internal void @remove_anim(ptr noundef %0) #0 {
  %2 = alloca ptr, align 8
  store ptr %0, ptr %2, align 8
  %11 = load ptr, ptr %2, align 8
  %12 = getelementptr inbounds nuw %struct._lv_anim_t.3, ptr %11, i32 0, i32 5
  %13 = load ptr, ptr %12, align 8
  call void %13(ptr noundef %11)
  call void @lv_ll_remove(ptr noundef getelementptr inbounds nuw (%struct.lv_anim_state_t, ptr @lv_global, i32 0, i32 4), ptr noundef %11)
  ret void
}
define dso_local ptr @lv_msgbox_add_close_button(ptr noundef %0) #0 {
  %23 = call ptr @lv_obj_add_event_cb(ptr noundef %22, ptr noundef @close_cb, i32 noundef 10, ptr noundef null)
  call void @lv_anim_set_deleted_cb(ptr noundef %a, ptr noundef @anim_deleted)
  call void %0(ptr noundef %x)
  ret ptr %23
}
define internal void @close_cb(ptr noundef %0) #0 {
  ret void
}
"#,
            &literal,
        );
        let setter = &m.functions["lv_anim_set_deleted_cb"];
        assert_eq!(
            setter.stores,
            [("_lv_anim_t.5".to_owned(), Stored::Param(1))]
        );
        let remove = &m.functions["anim::remove_anim"];
        assert_eq!(
            remove.indirect[0].target,
            Target::Slot("_lv_anim_t.5".into())
        );
        assert_eq!(remove.indirect[0].signature, "void(ptr)");
        let add = &m.functions["lv_msgbox_add_close_button"];
        assert_eq!(add.calls[0].args[1], Arg::Function("anim::close_cb".into()));
        assert_eq!(add.calls[0].args[2], Arg::Int(10));
        assert_eq!(add.calls[1].args[1], Arg::Function("anim_deleted".into()));
        assert_eq!(add.indirect[0].target, Target::Param(0));
        assert!(
            m.global_slots
                .contains(&("_lv_obj_class_t.1".into(), "lv_msgbox_constructor".into()))
        );
    }
}
