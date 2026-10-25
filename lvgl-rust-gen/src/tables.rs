//! The reviewed tables under `tables/lvgl-<major>.<minor>/`: `access.toml`,
//! `classes.toml`, `relations.toml`, `overrides/**/*.toml` and
//! `universe.txt`. Unknown keys are errors.

use crate::{Diagnostics, Error, parse::Bindings};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// No LVGL state touched: callable without a token.
    Pure,
    /// Callable from interrupts, without a token.
    Isr,
    /// Cannot free objects: `&Lvgl` or a view.
    Shared,
    /// May free objects: `&mut Lvgl` or an `ObjMut`.
    Exclusive,
    /// Never wrapped safely; stays reachable through `sys`.
    Raw,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessEntry {
    pub access: Access,
    #[serde(default)]
    pub why: Option<String>,
    /// The call deletes its receiver: the `ObjMut` method takes `self`.
    #[serde(default)]
    pub consumes: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessTable {
    #[serde(default, rename = "fn")]
    pub functions: BTreeMap<String, AccessEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrateOwned {
    /// Defined by the crate (the OBJ_ID hooks): never wrapped, never raw.
    #[serde(default)]
    pub skip: Vec<String>,
    /// Would break the crate's invariants: raw whatever other tables say.
    #[serde(default)]
    pub raw: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composite {
    /// `lv_<widget>_get_<part>`, `child:<index>` or `shim:<widget>.<part>`.
    pub parts: Vec<String>,
    pub max_depth: u8,
    /// Parts below which LVGL finds objects by index, with how many levels
    /// it reaches (a tab view's bar: buttons and their labels, 2).
    #[serde(default)]
    pub indexed: BTreeMap<String, u8>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classes {
    /// Widget class to base class, when the base is not `obj`.
    #[serde(default)]
    pub hierarchy: BTreeMap<String, String>,
    /// Rust names for classes whose name is several words.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    /// Plain getters and setters of crate-managed resources to generate.
    #[serde(default)]
    pub generate: Vec<String>,
    /// Wrapped by hand in the core: never generated.
    #[serde(default)]
    pub manual: Vec<String>,
    #[serde(default)]
    pub crate_owned: CrateOwned,
    #[serde(default)]
    pub composite: BTreeMap<String, Composite>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    #[serde(default)]
    pub param: Option<String>,
    #[serde(default)]
    pub clear: Option<String>,
    #[serde(default)]
    pub raw: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relations {
    #[serde(default, rename = "fn")]
    pub functions: BTreeMap<String, Relation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// LVGL copies the string: `&CStr`.
    CopiedStr,
    /// LVGL keeps the pointer: `&'static CStr`.
    StaticStr,
    /// LVGL copies the pointed-to value: `&T`.
    Copied,
    /// Filled by LVGL.
    Out,
    /// Kept by LVGL (the default for pointers): raw until classified.
    Retained,
    /// Kept and written by LVGL.
    RetainedMut,
    /// A slice whose length comes from `len`.
    Slice,
    /// An index or count LVGL checks against its own bounds (reviewed):
    /// passed as is.
    Checked,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamOverride {
    pub kind: ParamKind,
    #[serde(default)]
    pub len: Option<String>,
    /// With `kind = "retained"`: the `lvgl-rust` type that guarantees what
    /// LVGL expects of the data (`ImageSrc`, `ButtonMap`, ...).
    #[serde(default)]
    pub rust: Option<String>,
}

/// Types of `lvgl-rust` for retained parameters (`rust = "..."`): whether
/// they are passed by value (`true`) or as `&'static` (`false`), and
/// whether LVGL may write to them (interior mutability: subjects). Each has
/// an `as_ptr` (`const fn` for read-only ones, used in constant styles).
pub const RETAINED_TYPES: &[(&str, bool, bool)] = &[
    ("ImageSrc", true, false),
    ("ImageColorkey", false, false),
    ("ButtonMap", false, false),
    ("GradDsc", false, false),
    ("TransitionDsc", false, false),
    ("ColorFilter", false, false),
    ("IntSubject", false, true),
    ("ColorSubject", false, true),
];

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Returns {
    /// `"borrowed_internal"`: points into object memory.
    Keyword(ReturnKeyword),
    /// `{ view = "label" }`: a part of a class the widget creates itself.
    View { view: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnKeyword {
    BorrowedInternal,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FnOverride {
    #[serde(default)]
    pub params: BTreeMap<String, ParamOverride>,
    #[serde(default)]
    pub returns: Option<Returns>,
    #[serde(default)]
    pub access: Option<Access>,
    /// `"mutable"`: only on a mutable style, never on a `ConstStyle`.
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub why: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeOverride {
    /// The `&'static` rodata wrapper of this type (only reads on it).
    #[serde(default)]
    pub rodata: Option<String>,
    /// Flags type: the newtype implements `BitOr`.
    #[serde(default)]
    pub flags: bool,
    /// Emit the newtype even if no generated function uses it (for
    /// hand-written APIs).
    #[serde(default)]
    pub emit: bool,
    #[serde(default)]
    pub why: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default, rename = "type")]
    pub types: BTreeMap<String, TypeOverride>,
    #[serde(default, rename = "fn")]
    pub functions: BTreeMap<String, FnOverride>,
}

#[derive(Debug, Default)]
pub struct Tables {
    pub access: AccessTable,
    pub classes: Classes,
    pub relations: Relations,
    pub overrides: Overrides,
    /// Every function of every supported configuration.
    pub universe: BTreeSet<String>,
    /// Files read, for `rerun-if-changed`.
    pub files: Vec<PathBuf>,
}

impl Tables {
    /// The tables for the LVGL version of `bindings`.
    pub fn directory(root: &Path, bindings: &Bindings) -> Result<PathBuf, Error> {
        let major = bindings.consts.get("LVGL_VERSION_MAJOR");
        let minor = bindings.consts.get("LVGL_VERSION_MINOR");
        let (Some(major), Some(minor)) = (major, minor) else {
            return Err(Error::new("bindings.rs has no LVGL_VERSION_MAJOR/MINOR"));
        };
        let dir = root.join(format!("lvgl-{major}.{minor}"));
        if !dir.is_dir() {
            return Err(Error::new(format!(
                "no tables for LVGL {major}.{minor} (expected {})",
                dir.display()
            )));
        }
        Ok(dir)
    }

    pub fn load(dir: &Path) -> Result<Tables, Error> {
        let mut tables = Tables::default();
        tables.access = tables.read_toml(&dir.join("access.toml"))?;
        tables.classes = tables.read_toml(&dir.join("classes.toml"))?;
        tables.relations = tables.read_toml(&dir.join("relations.toml"))?;
        let mut files = Vec::new();
        let overrides = dir.join("overrides");
        if overrides.is_dir() {
            collect_toml(&overrides, &mut files)?;
        }
        files.sort();
        for file in files {
            let part: Overrides = tables.read_toml(&file)?;
            tables.merge_overrides(part, &file)?;
        }
        let universe = dir.join("universe.txt");
        tables.files.push(universe.clone());
        tables.universe = read(&universe)?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
            .collect();
        Ok(tables)
    }

    pub fn parse(
        access: &str,
        classes: &str,
        relations: &str,
        overrides: &[&str],
        universe: &[&str],
    ) -> Result<Tables, Error> {
        let mut tables = Tables {
            access: from_toml(access, "access.toml")?,
            classes: from_toml(classes, "classes.toml")?,
            relations: from_toml(relations, "relations.toml")?,
            universe: universe.iter().map(|s| s.to_string()).collect(),
            ..Tables::default()
        };
        for (i, source) in overrides.iter().enumerate() {
            let name = format!("overrides[{i}]");
            let part = from_toml(source, &name)?;
            tables.merge_overrides(part, Path::new(&name))?;
        }
        Ok(tables)
    }

    fn read_toml<T: for<'de> Deserialize<'de>>(&mut self, path: &Path) -> Result<T, Error> {
        self.files.push(path.to_owned());
        from_toml(&read(path)?, &path.display().to_string())
    }

    fn merge_overrides(&mut self, part: Overrides, file: &Path) -> Result<(), Error> {
        for (name, entry) in part.types {
            if self.overrides.types.insert(name.clone(), entry).is_some() {
                return Err(Error::new(format!(
                    "{}: [type.{name}] is defined twice",
                    file.display()
                )));
            }
        }
        for (name, entry) in part.functions {
            if self
                .overrides
                .functions
                .insert(name.clone(), entry)
                .is_some()
            {
                return Err(Error::new(format!(
                    "{}: [fn.{name}] is defined twice",
                    file.display()
                )));
            }
        }
        Ok(())
    }

    /// Whether `name` matches a `crate_owned` pattern (`*` matches any run).
    pub fn crate_owned(&self, name: &str) -> Option<CrateOwnedKind> {
        let owned = &self.classes.crate_owned;
        if owned.skip.iter().any(|p| glob(p, name)) {
            Some(CrateOwnedKind::Skip)
        } else if owned.raw.iter().any(|p| glob(p, name)) {
            Some(CrateOwnedKind::Raw)
        } else {
            None
        }
    }

    /// Cross-checks the tables against each other and the bindings. Errors
    /// in the tables themselves always fail; entries for functions missing
    /// from these bindings but present in the universe are fine; anything
    /// else is stale (an error in strict mode, a warning otherwise).
    pub fn validate(&self, bindings: &Bindings, diags: &mut Diagnostics) {
        let known =
            |name: &str| bindings.functions.contains_key(name) || self.universe.contains(name);
        for (name, entry) in &self.access.functions {
            if !known(name) {
                diags.stale(format!("access.toml: [fn.{name}] is not an LVGL function"));
            }
            let exposed = matches!(entry.access, Access::Shared | Access::Exclusive);
            if exposed && self.crate_owned(name).is_some() {
                diags.error(format!(
                    "access.toml: [fn.{name}] is crate-owned (classes.toml) and cannot be shared or exclusive"
                ));
            }
        }
        for (name, entry) in &self.overrides.functions {
            if !known(name) {
                diags.stale(format!("overrides: [fn.{name}] is not an LVGL function"));
                continue;
            }
            if let Some(f) = bindings.functions.get(name) {
                for param in entry.params.keys() {
                    if !f.params.iter().any(|p| &p.name == param) {
                        diags.stale(format!("overrides: [fn.{name}] has no parameter `{param}`"));
                    }
                }
            }
            for (param, o) in &entry.params {
                match (&o.rust, o.kind) {
                    (Some(rust), ParamKind::Retained) => {
                        if !RETAINED_TYPES.iter().any(|(t, ..)| t == rust) {
                            diags.error(format!(
                                "overrides: [fn.{name}] `{param}`: unknown rust type `{rust}`"
                            ));
                        }
                    }
                    (Some(_), kind) => diags.error(format!(
                        "overrides: [fn.{name}] `{param}`: `rust` needs kind = \"retained\", not {kind:?}"
                    )),
                    (None, _) => {}
                }
            }
            if let (Some(access), Some(table)) = (entry.access, self.access.functions.get(name))
                && access != table.access
            {
                diags.error(format!(
                    "overrides: [fn.{name}] access {access:?} conflicts with access.toml ({:?})",
                    table.access
                ));
            }
            let exposed = matches!(entry.access, Some(Access::Shared | Access::Exclusive));
            if exposed && self.crate_owned(name).is_some() {
                diags.error(format!(
                    "overrides: [fn.{name}] is crate-owned and cannot be shared or exclusive"
                ));
            }
            if let Some(style) = &entry.style
                && style != "mutable"
            {
                diags.error(format!("overrides: [fn.{name}] style must be \"mutable\""));
            }
        }
        for name in self.overrides.types.keys() {
            if !bindings.aliases.contains_key(name) && !bindings.structs.contains_key(name) {
                diags.stale(format!(
                    "overrides: [type.{name}] is not a type of the bindings"
                ));
            }
        }
        for (name, relation) in &self.relations.functions {
            if !known(name) {
                diags.stale(format!(
                    "relations.toml: [fn.{name}] is not an LVGL function"
                ));
                continue;
            }
            if relation.raw == relation.param.is_some() {
                diags.error(format!(
                    "relations.toml: [fn.{name}] needs either `param` or `raw = true`"
                ));
            }
            if let (Some(param), Some(f)) = (&relation.param, bindings.functions.get(name))
                && !f.params.iter().any(|p| &p.name == param)
            {
                diags.stale(format!(
                    "relations.toml: [fn.{name}] has no parameter `{param}`"
                ));
            }
        }
        for name in self.classes.generate.iter().chain(&self.classes.manual) {
            if !known(name) {
                diags.stale(format!("classes.toml: unknown function {name}"));
            }
        }
        for (class, composite) in &self.classes.composite {
            for part in &composite.parts {
                let valid = if let Some(index) = part.strip_prefix("child:") {
                    index.parse::<i32>().is_ok()
                } else if let Some(shim) = part.strip_prefix("shim:") {
                    shim.split_once('.')
                        .is_some_and(|(w, p)| w == class && !p.is_empty())
                } else {
                    part.starts_with(&format!("lv_{class}_get_"))
                };
                if !valid {
                    diags.error(format!(
                        "classes.toml: [composite.{class}] part `{part}` is not a getter of {class}, child:N or shim:{class}.<part>"
                    ));
                } else if part.starts_with("lv_") && !known(part) {
                    diags.stale(format!(
                        "classes.toml: [composite.{class}] part {part} is unknown"
                    ));
                }
            }
            for part in composite.indexed.keys() {
                if !composite.parts.contains(part) {
                    diags.error(format!(
                        "classes.toml: [composite.{class}] indexed part `{part}` is not in `parts`"
                    ));
                }
            }
            if composite.max_depth == 0 {
                diags.error(format!(
                    "classes.toml: [composite.{class}] max_depth must be at least 1"
                ));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrateOwnedKind {
    Skip,
    Raw,
}

/// `*` matches any run of characters (possibly empty).
pub fn glob(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, tail)) => {
            let Some(rest) = name.strip_prefix(head) else {
                return false;
            };
            (0..=rest.len()).any(|i| rest.is_char_boundary(i) && glob(tail, &rest[i..]))
        }
    }
}

fn read(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))
}

fn from_toml<T: for<'de> Deserialize<'de>>(source: &str, name: &str) -> Result<T, Error> {
    toml::from_str(source).map_err(|e| Error::new(format!("{name}: {e}")))
}

fn collect_toml(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|e| Error::new(format!("{}: {e}", dir.display())))?;
    for entry in entries {
        let path = entry.map_err(|e| Error::new(e.to_string()))?.path();
        if path.is_dir() {
            collect_toml(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "toml") {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_runs() {
        assert!(glob("lv_obj_remove_event*", "lv_obj_remove_event_cb"));
        assert!(glob("*_set_external_data", "lv_canvas_set_external_data"));
        assert!(glob("lv_tick_inc", "lv_tick_inc"));
        assert!(!glob("lv_tick_inc", "lv_tick_inc_x"));
        assert!(!glob("*_set_cb", "lv_timer_set_cb_x"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let error = Tables::parse(
            "[fn.lv_x]\naccess = \"shared\"\nwhy_not = 1\n",
            "",
            "",
            &[],
            &[],
        )
        .unwrap_err();
        assert!(error.to_string().contains("why_not"), "{error}");
        let error = Tables::parse(
            "",
            "",
            "",
            &["[fn.lv_x]\nparams.a = { kind = \"bogus\" }\n"],
            &[],
        )
        .unwrap_err();
        assert!(error.to_string().contains("bogus"), "{error}");
    }
}
