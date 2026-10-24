//! Classification: every bound function gets a [`Plan`], from ordered rules
//! (first match wins). Only `Plan::Method` is emitted; everything else stays
//! reachable through `sys` and is listed in `coverage.txt`.

use crate::{
    headers::Headers,
    parse::{Bindings, CType, Function},
    tables::{Access, CrateOwnedKind, ParamKind, ReturnKeyword, Returns, Tables},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    Skip(String),
    Raw(String),
    Manual(String),
    Method(Method),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub c_name: String,
    /// Rust name of the method or function.
    pub name: String,
    /// The widget class of the receiver (`obj`, `label`, ...), if any.
    pub class: Option<String>,
    pub shape: Shape,
    pub params: Vec<ParamPlan>,
    pub ret: RetPlan,
    pub doc: String,
}

/// Where the wrapper goes (emission rule of the design, 4.8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `lv_<w>_create(parent)`: `W::create` and `W::create_owned`.
    Create,
    /// Shared call on a view: `&self`.
    View,
    /// Exclusive call on an object: `ObjMut<'_, W>`, `self` if it consumes.
    ObjMut { consumes: bool },
    /// Shared call without an object: `&self` on `Lvgl`.
    LvglShared,
    /// Exclusive call without an object: `&mut self` on `Lvgl`.
    LvglExclusive,
    /// Pure or ISR-safe: a free function, no token.
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamPlan {
    /// Passed as is: a primitive type (`i32`, `core::ffi::c_int`, ...).
    Scalar { name: String, ty: String },
    /// An enum or flags newtype; `c_type` is the bindgen alias.
    Enum { name: String, c_type: String },
    /// A plain-data struct by value (`lv_color_t`).
    Struct { name: String, c_type: String },
    /// A plain-data struct LVGL copies, by reference.
    StructRef { name: String, c_type: String },
    /// An object pointer, as a view.
    Obj { name: String, nullable: bool },
    /// A string; `'static` when LVGL keeps the pointer.
    Str {
        name: String,
        is_static: bool,
        nullable: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetPlan {
    Unit,
    Scalar(String),
    Enum(String),
    Struct(String),
    /// An object: `Option<&Obj>`, or a typed view when an override says so.
    View(Option<String>),
}

/// Receivers whose resources the crate manages by hand.
const MANAGED: &[&str] = &[
    "lv_timer_t",
    "lv_display_t",
    "lv_indev_t",
    "lv_anim_t",
    "lv_group_t",
];

const PRIMITIVES: &[&str] = &[
    "i8",
    "i16",
    "i32",
    "i64",
    "u8",
    "u16",
    "u32",
    "u64",
    "isize",
    "usize",
    "f32",
    "f64",
    "bool",
    "c_char",
    "c_schar",
    "c_uchar",
    "c_short",
    "c_ushort",
    "c_int",
    "c_uint",
    "c_long",
    "c_ulong",
    "c_longlong",
    "c_ulonglong",
    "c_float",
    "c_double",
];

pub struct Classifier<'a> {
    pub bindings: &'a Bindings,
    pub headers: &'a Headers,
    pub tables: &'a Tables,
    /// Widget classes (`lv_<class>_class` statics), longest first.
    pub classes: Vec<String>,
}

impl<'a> Classifier<'a> {
    pub fn new(bindings: &'a Bindings, headers: &'a Headers, tables: &'a Tables) -> Self {
        let mut classes: Vec<String> = bindings
            .statics
            .iter()
            .filter_map(|s| s.strip_prefix("lv_")?.strip_suffix("_class"))
            .map(str::to_owned)
            .collect();
        classes.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        Classifier {
            bindings,
            headers,
            tables,
            classes,
        }
    }

    pub fn plans(&self) -> BTreeMap<String, Plan> {
        self.bindings
            .functions
            .values()
            .map(|f| (f.name.clone(), self.classify(f)))
            .collect()
    }

    pub fn classify(&self, f: &Function) -> Plan {
        let name = f.name.as_str();
        match self.tables.crate_owned(name) {
            Some(CrateOwnedKind::Skip) => return Plan::Skip("defined by the crate".into()),
            Some(CrateOwnedKind::Raw) => return Plan::Raw("crate-owned".into()),
            None => {}
        }
        if self.tables.classes.manual.iter().any(|m| m == name) {
            return Plan::Manual("wrapped by hand in the core".into());
        }
        let receiver = f.params.first().and_then(|p| p.ty.pointee_name());
        let managed =
            receiver.is_some_and(|r| MANAGED.contains(&r)) || name.starts_with("lv_event_dsc_");
        if managed {
            let why = if self.tables.classes.generate.iter().any(|g| g == name) {
                "generated with its owner type"
            } else {
                "crate-managed resource"
            };
            return Plan::Manual(why.into());
        }
        let header = self.headers.functions.get(name);
        if f.variadic || f.params.iter().any(|p| p.ty.named() == Some("va_list")) {
            return Plan::Skip("variadic".into());
        }
        if header.is_some_and(|h| h.deprecated) {
            return Plan::Skip("deprecated".into());
        }
        let Some(entry) = self.tables.access.functions.get(name) else {
            return Plan::Skip("no access.toml entry".into());
        };
        if entry.access == Access::Raw {
            return Plan::Raw(entry.why.clone().unwrap_or_else(|| "access.toml".into()));
        }
        let overrides = self.tables.overrides.functions.get(name);
        let nullable = |param: &str| header.is_some_and(|h| h.nullable.iter().any(|n| n == param));

        let class = self.receiver_class(f);
        let skip_receiver = class.is_some() as usize;
        let create = class
            .as_deref()
            .is_some_and(|c| name == format!("lv_{c}_create"))
            && f.params.len() == 1
            && f.ret.pointee_name() == Some("lv_obj_t");

        let mut params = Vec::new();
        if !create {
            for p in &f.params[skip_receiver..] {
                let kind = overrides
                    .and_then(|o| o.params.get(&p.name))
                    .map(|o| o.kind);
                match self.param(name, &p.name, &p.ty, kind, nullable(&p.name)) {
                    Ok(plan) => params.push(plan),
                    Err(why) => return Plan::Raw(format!("parameter `{}`: {why}", p.name)),
                }
            }
        }
        let ret = if create {
            RetPlan::Unit
        } else {
            let returns = overrides.and_then(|o| o.returns.as_ref());
            match self.ret(&f.ret, returns) {
                Ok(ret) => ret,
                Err(why) => return Plan::Raw(format!("return: {why}")),
            }
        };
        let shape = match (entry.access, &class) {
            _ if create => Shape::Create,
            (Access::Shared, Some(_)) => Shape::View,
            (Access::Exclusive, Some(_)) => Shape::ObjMut {
                consumes: entry.consumes,
            },
            (Access::Shared, None) => Shape::LvglShared,
            (Access::Exclusive, None) => Shape::LvglExclusive,
            (Access::Pure | Access::Isr, None) => Shape::Free,
            (Access::Pure | Access::Isr, Some(_)) => {
                return Plan::Raw("pure function taking an object".into());
            }
            (Access::Raw, _) => unreachable!(),
        };
        if shape == Shape::Free && matches!(ret, RetPlan::View(_)) {
            return Plan::Raw("returns an object without a token".into());
        }
        let exclusive = matches!(shape, Shape::ObjMut { .. } | Shape::LvglExclusive);
        if exclusive && params.iter().any(|p| matches!(p, ParamPlan::Obj { .. })) {
            // A view of the other object cannot coexist with `&mut Lvgl`;
            // these take a `Handle` once generated.
            return Plan::Raw("exclusive call taking another object (not generated yet)".into());
        }
        if exclusive && matches!(ret, RetPlan::View(_)) {
            return Plan::Raw("exclusive call returning an object (not generated yet)".into());
        }
        let method_name = match &class {
            Some(_) if create => "create".to_owned(),
            Some(c) => rust_ident(name.strip_prefix(&format!("lv_{c}_")).unwrap_or(name)),
            None => rust_ident(name.strip_prefix("lv_").unwrap_or(name)),
        };
        Plan::Method(Method {
            c_name: name.to_owned(),
            name: method_name,
            class,
            shape,
            params,
            ret,
            doc: header.map(|h| h.doc.clone()).unwrap_or_default(),
        })
    }

    /// The widget class of a function whose first parameter is an object
    /// and whose name starts with `lv_<class>_` (longest class first).
    fn receiver_class(&self, f: &Function) -> Option<String> {
        let first = f.params.first()?;
        let create_like = f.params.len() == 1 && f.name.ends_with("_create");
        if first.ty.pointee_name() != Some("lv_obj_t") {
            return None;
        }
        let class = self
            .classes
            .iter()
            .find(|c| f.name.starts_with(&format!("lv_{c}_")))?;
        if create_like && f.name != format!("lv_{class}_create") {
            return None;
        }
        Some(class.clone())
    }

    fn param(
        &self,
        function: &str,
        name: &str,
        ty: &CType,
        kind: Option<ParamKind>,
        nullable: bool,
    ) -> Result<ParamPlan, String> {
        let rust_name = rust_ident(name);
        match ty {
            CType::Named(_) => {
                let plan = self.value(&rust_name, ty)?;
                // An index may reach an array access that LVGL checks only
                // with `LV_USE_CHECK_ARG`: wrapped once reviewed.
                if matches!(plan, ParamPlan::Scalar { .. })
                    && is_index(name)
                    && kind != Some(ParamKind::Checked)
                {
                    return Err(
                        "index parameter, not reviewed (overrides: kind = \"checked\")".into(),
                    );
                }
                Ok(plan)
            }
            CType::Ptr { mutable, pointee } => {
                let pointee_name = pointee.named().unwrap_or("");
                if pointee_name == "lv_obj_t" {
                    return Ok(ParamPlan::Obj {
                        name: rust_name,
                        nullable,
                    });
                }
                if pointee_name == "c_char" && !mutable {
                    let is_static = match kind {
                        Some(ParamKind::CopiedStr) => false,
                        Some(ParamKind::StaticStr) => true,
                        None if function.ends_with("_static") => true,
                        _ => return Err("string kept by LVGL (retained)".into()),
                    };
                    return Ok(ParamPlan::Str {
                        name: rust_name,
                        is_static,
                        nullable,
                    });
                }
                match kind {
                    Some(ParamKind::Copied) if !mutable && self.is_plain_struct(pointee_name) => {
                        Ok(ParamPlan::StructRef {
                            name: rust_name,
                            c_type: pointee_name.to_owned(),
                        })
                    }
                    Some(kind) => Err(format!("{kind:?} parameters are not generated yet")),
                    None => Err("pointer kept by LVGL (retained)".into()),
                }
            }
            CType::Void | CType::Other(_) => Err("unsupported type".into()),
        }
    }

    fn value(&self, name: &str, ty: &CType) -> Result<ParamPlan, String> {
        let c_type = ty.named().unwrap_or_default();
        if PRIMITIVES.contains(&c_type) {
            return Ok(ParamPlan::Scalar {
                name: name.into(),
                ty: primitive_path(c_type),
            });
        }
        if self.is_enum(c_type) {
            return Ok(ParamPlan::Enum {
                name: name.into(),
                c_type: c_type.into(),
            });
        }
        if self.is_plain_struct(c_type) {
            return Ok(ParamPlan::Struct {
                name: name.into(),
                c_type: c_type.into(),
            });
        }
        match self.bindings.resolve(ty).named() {
            Some(resolved) if PRIMITIVES.contains(&resolved) => Ok(ParamPlan::Scalar {
                name: name.into(),
                ty: primitive_path(resolved),
            }),
            _ => Err(format!("unsupported type {c_type}")),
        }
    }

    fn ret(&self, ty: &CType, returns: Option<&Returns>) -> Result<RetPlan, String> {
        match ty {
            CType::Void => Ok(RetPlan::Unit),
            CType::Named(_) => Ok(match self.value("_", ty)? {
                ParamPlan::Scalar { ty, .. } => RetPlan::Scalar(ty),
                ParamPlan::Enum { c_type, .. } => RetPlan::Enum(c_type),
                ParamPlan::Struct { c_type, .. } => RetPlan::Struct(c_type),
                _ => unreachable!(),
            }),
            CType::Ptr { pointee, .. } if pointee.named() == Some("lv_obj_t") => match returns {
                Some(Returns::View { view }) => {
                    if self.classes.iter().any(|c| c == view) {
                        Ok(RetPlan::View(Some(view.clone())))
                    } else {
                        Err(format!(
                            "returns view `{view}`, which is not a widget class"
                        ))
                    }
                }
                Some(Returns::Keyword(ReturnKeyword::BorrowedInternal)) => {
                    Err("object returns cannot be borrowed_internal".into())
                }
                None => Ok(RetPlan::View(None)),
            },
            CType::Ptr { .. } => match returns {
                Some(Returns::Keyword(ReturnKeyword::BorrowedInternal)) => {
                    Err("borrowed_internal returns are not generated yet".into())
                }
                _ => Err("pointer return".into()),
            },
            CType::Other(_) => Err("unsupported return type".into()),
        }
    }

    /// An integer alias with `<alias>_<VARIANT>` constants.
    pub fn is_enum(&self, c_type: &str) -> bool {
        let integer = matches!(
            self.bindings.resolve(&CType::Named(c_type.into())).named(),
            Some("c_uint" | "c_int" | "u8" | "u16" | "u32" | "i8" | "i16" | "i32")
        );
        integer && self.bindings.aliases.contains_key(c_type) && !self.variants(c_type).is_empty()
    }

    /// `(const name, rust variant name)` for an enum type.
    pub fn variants(&self, c_type: &str) -> Vec<(String, String)> {
        let prefix = format!("{c_type}_");
        let names: Vec<&str> = self
            .bindings
            .consts
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .collect();
        let common = common_prefix(&names);
        names
            .iter()
            .map(|n| {
                let short = &n[common.len()..];
                let short = if short.is_empty() { n } else { short };
                let short = if short.starts_with(|c: char| c.is_ascii_digit()) {
                    format!("_{short}")
                } else {
                    short.to_owned()
                };
                (format!("{prefix}{n}"), short)
            })
            .collect()
    }

    fn is_plain_struct(&self, c_type: &str) -> bool {
        let target = match self.bindings.aliases.get(c_type) {
            Some(CType::Named(target)) => target.as_str(),
            _ => c_type,
        };
        self.bindings.structs.get(target).copied().unwrap_or(false)
    }
}

/// Parameter names that index an array or select an item.
fn is_index(name: &str) -> bool {
    name.contains("idx")
        || name.contains("index")
        || name == "id"
        || name.ends_with("_id")
        || name == "row"
        || name == "col"
        || name.starts_with("sel")
        || name == "i"
        || name == "n"
}

/// The longest prefix shared by every name that ends with `_`.
fn common_prefix(names: &[&str]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut len = first.len();
    for name in &names[1..] {
        len = len.min(
            first
                .bytes()
                .zip(name.bytes())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    match first[..len].rfind('_') {
        Some(end) if names.len() > 1 => first[..=end].to_owned(),
        _ => match first.rfind('_') {
            // A single variant keeps its last word.
            Some(end) if names.len() == 1 => first[..=end].to_owned(),
            _ => String::new(),
        },
    }
}

fn primitive_path(ty: &str) -> String {
    if ty.starts_with("c_") {
        format!("core::ffi::{ty}")
    } else {
        ty.to_owned()
    }
}

/// Escapes Rust keywords with a trailing underscore.
pub fn rust_ident(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do",
        "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
        "gen",
    ];
    if KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

/// `lv_label_long_mode_t` -> `LabelLongMode`.
pub fn type_name(c_type: &str) -> String {
    let base = c_type.strip_prefix("lv_").unwrap_or(c_type);
    let base = base.strip_suffix("_t").unwrap_or(base);
    camel(base)
}

pub fn camel(snake: &str) -> String {
    snake
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(type_name("lv_label_long_mode_t"), "LabelLongMode");
        assert_eq!(type_name("lv_align_t"), "Align");
        assert_eq!(rust_ident("move"), "move_");
        assert_eq!(
            common_prefix(&["LV_ALIGN_CENTER", "LV_ALIGN_TOP_LEFT"]),
            "LV_ALIGN_"
        );
        assert_eq!(common_prefix(&["LV_OPA_0", "LV_OPA_10"]), "LV_OPA_");
        assert_eq!(common_prefix(&["LV_DIR_NONE"]), "LV_DIR_");
    }
}
