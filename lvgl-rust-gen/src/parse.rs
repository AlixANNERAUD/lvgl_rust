//! Reads bindgen's `bindings.rs` with `syn`: foreign functions, integer
//! constants, type aliases, structs and class statics.

use crate::Error;
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, ForeignItem, Item, Lit, Type, UnOp};

/// A C type as bindgen spells it, reduced to what the generator needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CType {
    Void,
    /// A Rust path type: primitive (`i32`, `bool`, `core::ffi::c_int`) or a
    /// bindgen name (`lv_align_t`, `lv_color_t`). Paths keep their last
    /// segment only.
    Named(String),
    Ptr {
        mutable: bool,
        pointee: Box<CType>,
    },
    /// Function pointers, arrays and anything else: never wrapped safely.
    Other(String),
}

impl CType {
    pub fn named(&self) -> Option<&str> {
        match self {
            CType::Named(name) => Some(name),
            _ => None,
        }
    }

    /// The pointee's name for `*mut T` / `*const T`.
    pub fn pointee_name(&self) -> Option<&str> {
        match self {
            CType::Ptr { pointee, .. } => pointee.named(),
            _ => None,
        }
    }

    fn from_syn(ty: &Type) -> CType {
        match ty {
            Type::Tuple(t) if t.elems.is_empty() => CType::Void,
            Type::Path(p) if p.qself.is_none() => match p.path.segments.last() {
                // `Option<unsafe extern "C" fn ..>` callbacks
                Some(seg) if seg.ident == "Option" => CType::Other(quote::quote!(#ty).to_string()),
                Some(seg) => CType::Named(seg.ident.to_string()),
                None => CType::Other(quote::quote!(#ty).to_string()),
            },
            Type::Ptr(p) => CType::Ptr {
                mutable: p.mutability.is_some(),
                pointee: Box::new(CType::from_syn(&p.elem)),
            },
            Type::Paren(p) => CType::from_syn(&p.elem),
            other => CType::Other(quote::quote!(#other).to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: CType,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: CType,
    pub variadic: bool,
}

#[derive(Default, Debug)]
pub struct Bindings {
    pub functions: BTreeMap<String, Function>,
    /// Integer constants (bool constants as 0/1).
    pub consts: BTreeMap<String, i128>,
    /// `pub type name = target;`
    pub aliases: BTreeMap<String, CType>,
    /// Struct names; `true` when bindgen saw the fields.
    pub structs: BTreeMap<String, bool>,
    /// `pub static name: T;` in extern blocks (class descriptors, ...).
    pub statics: BTreeSet<String>,
    /// The type of each static.
    pub static_types: BTreeMap<String, CType>,
}

impl Bindings {
    pub fn parse(source: &str) -> Result<Bindings, Error> {
        let file = syn::parse_file(source).map_err(|e| Error::new(format!("bindings.rs: {e}")))?;
        let mut bindings = Bindings::default();
        for item in &file.items {
            bindings.add(item);
        }
        Ok(bindings)
    }

    fn add(&mut self, item: &Item) {
        match item {
            Item::ForeignMod(block) => {
                for item in &block.items {
                    match item {
                        ForeignItem::Fn(f) => {
                            let sig = &f.sig;
                            let params = sig
                                .inputs
                                .iter()
                                .filter_map(|input| match input {
                                    syn::FnArg::Typed(t) => Some(Param {
                                        name: match &*t.pat {
                                            syn::Pat::Ident(i) => i.ident.to_string(),
                                            _ => "_".into(),
                                        },
                                        ty: CType::from_syn(&t.ty),
                                    }),
                                    syn::FnArg::Receiver(_) => None,
                                })
                                .collect();
                            let ret = match &sig.output {
                                syn::ReturnType::Default => CType::Void,
                                syn::ReturnType::Type(_, ty) => CType::from_syn(ty),
                            };
                            let name = sig.ident.to_string();
                            self.functions.insert(
                                name.clone(),
                                Function {
                                    name,
                                    params,
                                    ret,
                                    variadic: sig.variadic.is_some(),
                                },
                            );
                        }
                        ForeignItem::Static(s) => {
                            self.statics.insert(s.ident.to_string());
                            self.static_types
                                .insert(s.ident.to_string(), CType::from_syn(&s.ty));
                        }
                        _ => {}
                    }
                }
            }
            Item::Const(c) => {
                if let Some(value) = int_value(&c.expr) {
                    self.consts.insert(c.ident.to_string(), value);
                }
            }
            Item::Type(t) => {
                self.aliases
                    .insert(t.ident.to_string(), CType::from_syn(&t.ty));
            }
            Item::Struct(s) => {
                // bindgen makes opaque types `struct X { _unused: [u8; 0] }`
                // or `struct X { _address: u8 }`.
                let opaque = s.fields.iter().all(|f| {
                    f.ident
                        .as_ref()
                        .is_some_and(|i| i.to_string().starts_with('_'))
                });
                self.structs.insert(s.ident.to_string(), !opaque);
            }
            _ => {}
        }
    }

    /// The type an alias resolves to, following alias chains.
    pub fn resolve<'a>(&'a self, mut ty: &'a CType) -> &'a CType {
        for _ in 0..16 {
            match ty.named().and_then(|n| self.aliases.get(n)) {
                Some(next) => ty = next,
                None => break,
            }
        }
        ty
    }

    /// `true`, `false`, or `None` for a missing / non-boolean constant.
    pub fn flag(&self, name: &str) -> Option<bool> {
        match self.consts.get(name) {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        }
    }
}

fn int_value(expr: &Expr) -> Option<i128> {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Int(i) => i.base10_parse().ok(),
            Lit::Bool(b) => Some(b.value as i128),
            _ => None,
        },
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => int_value(&u.expr).map(|v| -v),
        Expr::Cast(c) => int_value(&c.expr),
        Expr::Paren(p) => int_value(&p.expr),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_functions_constants_aliases_and_statics() {
        let b = Bindings::parse(
            r#"
            pub const LVGL_VERSION_MAJOR: u32 = 9;
            pub const LV_USE_LABEL: u32 = 1;
            pub const NEG: i32 = -3;
            pub type lv_align_t = core::ffi::c_uint;
            pub const lv_align_t_LV_ALIGN_CENTER: lv_align_t = 9;
            #[repr(C)] pub struct _lv_obj_t { _unused: [u8; 0] }
            pub type lv_obj_t = _lv_obj_t;
            #[repr(C)] pub struct lv_color_t { pub blue: u8, pub green: u8, pub red: u8 }
            unsafe extern "C" {
                pub fn lv_obj_set_size(obj: *mut lv_obj_t, w: i32, h: i32);
                pub fn lv_obj_get_parent(obj: *const lv_obj_t) -> *mut lv_obj_t;
                pub fn lv_label_set_text_fmt(obj: *mut lv_obj_t, fmt: *const core::ffi::c_char, ...);
                pub static lv_label_class: lv_obj_class_t;
            }
            "#,
        )
        .unwrap();
        assert_eq!(b.consts["LVGL_VERSION_MAJOR"], 9);
        assert_eq!(b.consts["NEG"], -3);
        assert_eq!(b.flag("LV_USE_LABEL"), Some(true));
        let size = &b.functions["lv_obj_set_size"];
        assert_eq!(size.params[0].ty.pointee_name(), Some("lv_obj_t"));
        assert_eq!(size.params[1].ty, CType::Named("i32".into()));
        assert!(b.functions["lv_label_set_text_fmt"].variadic);
        assert!(!b.structs["_lv_obj_t"]);
        assert!(b.structs["lv_color_t"]);
        assert_eq!(
            b.resolve(&CType::Named("lv_align_t".into())),
            &CType::Named("c_uint".into())
        );
        assert!(b.statics.contains("lv_label_class"));
    }
}
