//! Generates the safe LVGL wrappers of `lvgl-rust` from the bindings and
//! headers of `lvgl_rust_sys`. Runs on the build host only.
//!
//! 1. Parse `bindings.rs` ([`parse`]) and scan the headers ([`headers`]).
//! 2. Load the reviewed tables for the bindings' LVGL version ([`tables`]).
//! 3. Classify every function into a plan ([`classify`]).
//! 4. Emit `gen.rs` ([`emit`]) and `coverage.txt`.
//!
//! A function without an `access.toml` entry is never emitted: it stays
//! reachable through `lvgl_rust_sys`.

pub mod classify;
pub mod emit;
pub mod headers;
pub mod parse;
pub mod tables;

use std::{
    fmt,
    path::{Path, PathBuf},
};

/// The tables shipped with this crate (`tables/lvgl-<major>.<minor>/`).
pub fn tables_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tables")
}

#[derive(Debug)]
pub struct Error(String);

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Error(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Problems found in the tables. Errors always fail; stale entries (for
/// functions or parameters missing from these bindings) fail only in strict
/// mode, so a user's `lv_conf.h` never breaks the build.
#[derive(Debug, Default)]
pub struct Diagnostics {
    pub errors: Vec<String>,
    pub stale: Vec<String>,
}

impl Diagnostics {
    pub fn error(&mut self, message: String) {
        self.errors.push(message);
    }

    pub fn stale(&mut self, message: String) {
        self.stale.push(message);
    }
}

pub struct Config {
    pub bindings: PathBuf,
    /// Header directories to scan (LVGL's `include/`, the shims).
    pub header_dirs: Vec<PathBuf>,
    pub tables_root: PathBuf,
    /// Stale table entries are errors (maintainer CI).
    pub strict: bool,
}

#[derive(Debug)]
pub struct Output {
    /// The contents of `gen.rs`.
    pub code: String,
    /// One line per bound function: its plan.
    pub coverage: String,
    /// `lv_use_<x>` cfg names, and whether each is enabled.
    pub cfgs: Vec<(String, bool)>,
    pub warnings: Vec<String>,
    /// Files whose changes must rerun the generator.
    pub inputs: Vec<PathBuf>,
}

pub fn generate(config: &Config) -> Result<Output, Error> {
    let source = std::fs::read_to_string(&config.bindings)
        .map_err(|e| Error::new(format!("cannot read {}: {e}", config.bindings.display())))?;
    let bindings = parse::Bindings::parse(&source)?;
    let dirs: Vec<&Path> = config.header_dirs.iter().map(PathBuf::as_path).collect();
    let headers = headers::Headers::scan(&dirs).map_err(|e| Error::new(format!("headers: {e}")))?;
    let dir = tables::Tables::directory(&config.tables_root, &bindings)?;
    let tables = tables::Tables::load(&dir)?;
    let mut output = generate_from(&bindings, &headers, &tables, config.strict)?;
    output.inputs.extend(tables.files.iter().cloned());
    output.inputs.push(config.bindings.clone());
    Ok(output)
}

/// [`generate`] on already loaded inputs.
pub fn generate_from(
    bindings: &parse::Bindings,
    headers: &headers::Headers,
    tables: &tables::Tables,
    strict: bool,
) -> Result<Output, Error> {
    let mut diags = Diagnostics::default();
    tables.validate(bindings, &mut diags);
    let mut errors = diags.errors;
    if strict {
        errors.append(&mut diags.stale);
    }
    if !errors.is_empty() {
        return Err(Error::new(format!(
            "invalid tables:\n  {}",
            errors.join("\n  ")
        )));
    }
    let classifier = classify::Classifier::new(bindings, headers, tables);
    let plans = classifier.plans();
    let code = emit::emit(&classifier, &plans);
    let coverage = plans
        .iter()
        .map(|(name, plan)| match plan {
            classify::Plan::Method(m) => format!("{name}: {:?} {}\n", m.shape, m.name),
            classify::Plan::Skip(why) => format!("{name}: skip ({why})\n"),
            classify::Plan::Raw(why) => format!("{name}: raw ({why})\n"),
            classify::Plan::Manual(why) => format!("{name}: manual ({why})\n"),
        })
        .collect();
    let cfgs = bindings
        .consts
        .iter()
        .filter(|(name, _)| name.starts_with("LV_USE_"))
        .map(|(name, value)| (name.to_ascii_lowercase(), *value == 1))
        .collect();
    Ok(Output {
        code,
        coverage,
        cfgs,
        warnings: diags.stale,
        inputs: Vec::new(),
    })
}
