//! Emission of `gen.rs` from the plans: widget view types, enum newtypes and
//! one wrapper per `Plan::Method`.

use crate::classify::{
    Classifier, Method, ParamPlan, Plan, RetPlan, Shape, camel, rust_ident, type_name,
};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

pub fn emit(classifier: &Classifier, plans: &BTreeMap<String, Plan>) -> String {
    let methods: Vec<&Method> = plans
        .values()
        .filter_map(|p| match p {
            Plan::Method(m) => Some(m),
            _ => None,
        })
        .collect();

    // Widget types: classes with at least one wrapper, and their bases.
    let mut classes: BTreeSet<String> = methods.iter().filter_map(|m| m.class.clone()).collect();
    for class in classes.clone() {
        let mut base = base_of(classifier, &class);
        while let Some(b) = base {
            classes.insert(b.clone());
            base = base_of(classifier, &b);
        }
    }
    classes.remove("obj");

    let mut enums = BTreeSet::new();
    for m in &methods {
        for p in &m.params {
            if let ParamPlan::Enum { c_type, .. } = p {
                enums.insert(c_type.clone());
            }
        }
        if let RetPlan::Enum(c_type) = &m.ret {
            enums.insert(c_type.clone());
        }
    }

    for (c_type, t) in &classifier.tables.overrides.types {
        if t.emit && classifier.is_enum(c_type) {
            enums.insert(c_type.clone());
        }
    }
    let enum_items = enums.iter().map(|e| emit_enum(classifier, e));
    let widget_items = classes.iter().map(|c| emit_widget(classifier, c));

    let mut by_class: BTreeMap<String, Vec<TokenStream>> = BTreeMap::new();
    let mut by_objmut: BTreeMap<String, Vec<TokenStream>> = BTreeMap::new();
    let mut on_lvgl = Vec::new();
    let mut on_style = Vec::new();
    let mut free = Vec::new();
    // `ObjMut` methods of a class are emitted for that class only (generic
    // impls of different classes would overlap); base object methods are
    // generic over every widget. A class method named like a base object
    // method gets the class as a prefix.
    // The core's own `ObjMut` methods count as base object methods.
    const CORE_OBJMUT: &[&str] = &[
        "view",
        "as_base",
        "as_raw",
        "delete",
        "clean",
        "move_to_index",
        "remove_from_group",
        "set_parent",
        "set_parent_to",
        "load_screen",
    ];
    let obj_exclusive: BTreeSet<String> = methods
        .iter()
        .filter(|m| matches!(m.shape, Shape::ObjMut { .. }) && m.class.as_deref() == Some("obj"))
        .map(|m| m.name.clone())
        .chain(CORE_OBJMUT.iter().map(|n| (*n).to_owned()))
        .collect();
    // A string inside the object: a copy-out `x_into` on the view and a
    // borrow on `ObjMut`.
    let methods: Vec<Method> = methods
        .iter()
        .flat_map(|m| {
            if m.ret == RetPlan::BorrowedStr && m.shape == Shape::View {
                let mut into = (*m).clone();
                into.name = format!("{}_into", m.name);
                let mut borrow = (*m).clone();
                borrow.shape = Shape::ObjMut { consumes: false };
                vec![into, borrow]
            } else {
                vec![(*m).clone()]
            }
        })
        .collect();
    for m in &methods {
        let renamed;
        let m = if matches!(m.shape, Shape::ObjMut { .. })
            && m.class.as_deref() != Some("obj")
            && obj_exclusive.contains(&m.name)
        {
            let mut copy = (*m).clone();
            copy.name = format!("{}_{}", m.class.as_deref().unwrap_or(""), m.name);
            renamed = copy;
            &renamed
        } else {
            m
        };
        let tokens = emit_method(classifier, m);
        match (&m.shape, &m.class) {
            (Shape::Create | Shape::View, Some(class)) => {
                by_class.entry(class.clone()).or_default().push(tokens)
            }
            (Shape::ObjMut { .. }, Some(class)) => {
                by_objmut.entry(class.clone()).or_default().push(tokens)
            }
            (Shape::LvglShared | Shape::LvglExclusive, _) => on_lvgl.push(tokens),
            (Shape::Style, _) => on_style.push(tokens),
            (Shape::Free, _) => free.push(tokens),
            _ => unreachable!("class-less view method {}", m.c_name),
        }
    }
    let class_impls = by_class.iter().map(|(class, items)| {
        let ty = class_type(classifier, class);
        quote! { impl #ty { #(#items)* } }
    });
    let objmut_impls = by_objmut.iter().map(|(class, items)| {
        if class == "obj" {
            quote! { impl<W: crate::Widget> crate::ObjMut<'_, W> { #(#items)* } }
        } else {
            let ty = class_type(classifier, class);
            quote! { impl crate::ObjMut<'_, #ty> { #(#items)* } }
        }
    });
    let lvgl_impl = (!on_lvgl.is_empty()).then(|| quote! { impl crate::Lvgl { #(#on_lvgl)* } });
    let style_impl =
        (!on_style.is_empty()).then(|| quote! { impl crate::StyleMut<'_> { #(#on_style)* } });
    let props = methods.iter().filter_map(|m| emit_prop(classifier, m));
    let prop_ids: Vec<TokenStream> = methods
        .iter()
        .filter(|m| m.shape == Shape::Style)
        .filter_map(|m| {
            let property = m.name.strip_prefix("set_")?;
            let id = Ident::new(&style_prop_id(classifier, property)?, Span::call_site());
            let name = Ident::new(&property.to_ascii_uppercase(), Span::call_site());
            Some(quote! {
                pub const #name: Self = Self(crate::sys::#id as crate::sys::lv_style_prop_t);
            })
        })
        .collect();
    let prop_ids = (!prop_ids.is_empty()).then(|| {
        quote! {
            #[allow(clippy::unnecessary_cast)]
            impl crate::StyleProp { #(#prop_ids)* }
        }
    });
    let events = emit_events(classifier);
    let fonts = emit_fonts(classifier);
    let draw_buf = emit_draw_buf(classifier);
    let composites = emit_composites(classifier);

    let file = quote! {
        /// Enum and flags types, with the values of these bindings.
        pub mod enums {
            #(#enum_items)*
            #events
        }

        /// Widget view types.
        pub mod widgets {
            #(#widget_items)*
        }

        /// Functions that need no token.
        pub mod functions {
            #(#free)*
        }

        /// Properties of a [`ConstStyle`](crate::ConstStyle), for
        /// [`const_style!`](crate::const_style): one constructor per style
        /// property of these bindings.
        pub mod props {
            #(#props)*
        }

        #(#class_impls)*
        #(#objmut_impls)*
        #lvgl_impl
        #style_impl
        #prop_ids
        #fonts
        #draw_buf
        #composites
    };
    let file: syn::File = syn::parse2(file).expect("generated code parses");
    format!(
        "// Generated by lvgl-rust-gen from the bindings of this build. Do not edit.\n\n{}",
        prettyplease::unparse(&file)
    )
}

fn base_of(classifier: &Classifier, class: &str) -> Option<String> {
    if class == "obj" {
        return None;
    }
    Some(
        classifier
            .tables
            .classes
            .hierarchy
            .get(class)
            .cloned()
            .unwrap_or_else(|| "obj".into()),
    )
}

fn class_name(classifier: &Classifier, class: &str) -> String {
    classifier
        .tables
        .classes
        .names
        .get(class)
        .cloned()
        .unwrap_or_else(|| camel(class))
}

fn class_type(classifier: &Classifier, class: &str) -> TokenStream {
    if class == "obj" {
        quote!(crate::Obj)
    } else {
        let ident = Ident::new(&class_name(classifier, class), Span::call_site());
        quote!(crate::widgets::#ident)
    }
}

fn emit_widget(classifier: &Classifier, class: &str) -> TokenStream {
    let name = Ident::new(&class_name(classifier, class), Span::call_site());
    let base = base_of(classifier, class).unwrap_or_else(|| "obj".into());
    let base_ty = class_type(classifier, &base);
    let class_static = format_ident!("lv_{}_class", class);
    let mut ancestors = Vec::new();
    let mut next = Some(base.clone());
    while let Some(b) = next {
        ancestors.push(class_type(classifier, &b));
        next = base_of(classifier, &b);
    }
    let doc = format!(" A `{class}` widget (`lv_{class}_t`).");
    quote! {
        #[doc = #doc]
        #[repr(transparent)]
        pub struct #name(#base_ty);

        impl core::ops::Deref for #name {
            type Target = #base_ty;
            #[inline]
            fn deref(&self) -> &#base_ty {
                &self.0
            }
        }

        // SAFETY: zero-sized wrapper of its base, at the object's address.
        unsafe impl crate::Widget for #name {
            type Base = #base_ty;
            #[inline]
            fn class() -> *const crate::sys::lv_obj_class_t {
                core::ptr::addr_of!(crate::sys::#class_static)
            }
        }

        // SAFETY: every object of this class is an instance of itself.
        unsafe impl crate::IsA<#name> for #name {}
        #(
            // SAFETY: LVGL classes derive from their base (`base_class`).
            unsafe impl crate::IsA<#ancestors> for #name {}
        )*
    }
}

fn emit_enum(classifier: &Classifier, c_type: &str) -> TokenStream {
    let name = Ident::new(&type_name(c_type), Span::call_site());
    let flags = classifier
        .tables
        .overrides
        .types
        .get(c_type)
        .is_some_and(|t| t.flags);
    let mut seen = BTreeSet::new();
    let consts = classifier
        .variants(c_type)
        .into_iter()
        .filter(|(_, short)| seen.insert(short.clone()))
        .map(|(c_name, short)| {
            let c_ident = Ident::new(&c_name, Span::call_site());
            let ident = Ident::new(&rust_ident(&short), Span::call_site());
            quote! {
                pub const #ident: Self = Self(crate::sys::#c_ident as u32);
            }
        });
    let doc = format!(" `{c_type}`.");
    let bit_ops = flags.then(|| {
        quote! {
            impl core::ops::BitOr for #name {
                type Output = Self;
                #[inline]
                fn bitor(self, rhs: Self) -> Self {
                    Self(self.0 | rhs.0)
                }
            }
            impl core::ops::BitOrAssign for #name {
                #[inline]
                fn bitor_assign(&mut self, rhs: Self) {
                    self.0 |= rhs.0;
                }
            }
        }
    });
    quote! {
        #[doc = #doc]
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        pub struct #name(pub(crate) u32);

        #[allow(clippy::unnecessary_cast)] // the bindgen alias may already be `u32`
        impl #name {
            #(#consts)*

            /// The raw value.
            #[inline]
            pub const fn bits(self) -> u32 {
                self.0
            }
        }
        #bit_ops
    }
}

/// From `events.toml`: the handler mask and the event codes safe code may
/// send (codes missing from these bindings are left out).
fn emit_events(classifier: &Classifier) -> TokenStream {
    let present: Vec<(&String, &crate::tables::EventEntry)> = classifier
        .tables
        .events
        .code
        .iter()
        .filter(|(name, _)| {
            classifier
                .bindings
                .consts
                .contains_key(&format!("lv_event_code_t_{name}"))
        })
        .collect();
    let words = present
        .iter()
        .map(|(_, e)| e.value as usize / 64 + 1)
        .max()
        .unwrap_or(1);
    let mut mask = vec![0u64; words];
    for (_, e) in &present {
        if e.handler == "shared" {
            mask[e.value as usize / 64] |= 1 << (e.value % 64);
        }
    }
    let mask = mask.iter().map(|w| {
        let lit = proc_macro2::Literal::u64_unsuffixed(*w);
        quote!(#lit)
    });
    let consts = |sendable: &[&str]| -> Vec<TokenStream> {
        present
            .iter()
            .filter(|(_, e)| sendable.contains(&e.sendable.as_str()))
            .map(|(name, _)| {
                let c_ident = format_ident!("lv_event_code_t_{}", name);
                let short = name.strip_prefix("LV_EVENT_").unwrap_or(name);
                let ident = Ident::new(&rust_ident(short), Span::call_site());
                quote! { pub const #ident: Self = Self(crate::sys::#c_ident as u32); }
            })
            .collect()
    };
    let words = proc_macro2::Literal::usize_unsuffixed(words);
    let safe = consts(&["shared"]);
    let sendable = consts(&["shared", "exclusive"]);
    quote! {
        /// Event codes whose handlers get shared access, one bit per code
        /// (`events.toml`, generated by the access audit).
        pub(crate) const HANDLER_SHARED: [u64; #words] = [#(#mask),*];

        /// An event code safe code may send with shared access
        /// ([`Obj::send_event`](crate::Obj::send_event)): no handler LVGL
        /// installs deletes or frees what other frames hold for it, and
        /// none reads its parameter (sent as NULL).
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        pub struct SafeEventCode(pub(crate) u32);

        #[allow(clippy::unnecessary_cast)] // the bindgen alias may already be `u32`
        impl SafeEventCode {
            #(#safe)*
        }

        /// An event code safe code may send with exclusive access
        /// ([`ObjMut::send_event`](crate::ObjMut::send_event)): its
        /// handlers may delete objects, the receiver included, but read no
        /// parameter (sent as NULL).
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        pub struct SendableEventCode(pub(crate) u32);

        #[allow(clippy::unnecessary_cast)] // the bindgen alias may already be `u32`
        impl SendableEventCode {
            #(#sendable)*
        }

        impl From<SafeEventCode> for SendableEventCode {
            #[inline]
            fn from(code: SafeEventCode) -> Self {
                Self(code.0)
            }
        }
    }
}

fn emit_method(classifier: &Classifier, m: &Method) -> TokenStream {
    let c_fn = Ident::new(&m.c_name, Span::call_site());
    let name = Ident::new(&m.name, Span::call_site());
    let doc = doc_attrs(&doc_text(&m.doc, &m.c_name));

    if m.shape == Shape::Create {
        let class = m.class.as_deref().unwrap_or("obj");
        let ty = class_type(classifier, class);
        return quote! {
            #doc
            pub fn create(parent: &crate::Obj) -> Result<&Self, crate::Error> {
                crate::base::gen_support::check_create(parent.as_ptr())?;
                // SAFETY: shared access through the parent view; creation
                // frees nothing.
                let p = unsafe { crate::sys::#c_fn(parent.as_ptr()) };
                if p.is_null() {
                    return Err(crate::Error::OutOfMemory);
                }
                // SAFETY: just created, of this class.
                Ok(unsafe { crate::base::gen_support::view::<#ty>(p) })
            }

            /// Creates the widget and takes ownership of it.
            #[cfg(not(lvgl_rust_views_only))]
            pub fn create_owned(parent: &crate::Obj) -> Result<crate::Owned<#ty>, crate::Error> {
                crate::WidgetExt::own(Self::create(parent)?)
            }
        };
    }

    let mut args = Vec::new();
    let mut call_args = Vec::new();
    match m.shape {
        Shape::View => call_args.push(quote!(crate::base::gen_support::ptr_of(self))),
        Shape::ObjMut { .. } | Shape::Style => call_args.push(quote!(self.as_raw())),
        _ => {}
    }
    let mut out = None;
    let mut generics = quote!();
    for p in &m.params {
        if let ParamPlan::Out { name, ty } = p {
            let n = Ident::new(name, Span::call_site());
            let ty: syn::Type = syn::parse_str(ty).unwrap();
            call_args.push(quote!(&mut #n));
            out = Some((n, ty));
            continue;
        }
        let (arg, call) = emit_param(p);
        args.push(arg);
        call_args.push(call);
    }
    let receiver = match m.shape {
        Shape::View | Shape::LvglShared => quote!(&self,),
        Shape::ObjMut { consumes: true } => quote!(self,),
        Shape::ObjMut { consumes: false } | Shape::LvglExclusive | Shape::Style => {
            quote!(&mut self,)
        }
        Shape::Free | Shape::Create => quote!(),
    };
    if m.ret == RetPlan::BorrowedStr && m.shape == Shape::View {
        args.push(quote!(buf: &'b mut [u8]));
        generics = quote!(<'b>);
    }
    // Other objects of exclusive calls are handles, resolved first.
    let handles: Vec<TokenStream> = m
        .params
        .iter()
        .filter_map(|p| match p {
            ParamPlan::ObjHandle { name, nullable } => {
                let n = Ident::new(name, Span::call_site());
                Some(if *nullable {
                    quote! {
                        let #n = match #n {
                            Some(h) => crate::base::gen_support::resolve_obj(h)?,
                            None => core::ptr::null_mut(),
                        };
                    }
                } else {
                    quote!(let #n = crate::base::gen_support::resolve_obj(#n)?;)
                })
            }
            _ => None,
        })
        .collect();
    let call = quote!(crate::sys::#c_fn(#(#call_args),*));
    let (ret_ty, body) = match &m.ret {
        RetPlan::BorrowedStr if m.shape == Shape::View => (
            quote!(-> Result<Option<&'b core::ffi::CStr>, crate::Error>),
            quote!(crate::base::gen_support::copy_str(unsafe { #call }, buf)),
        ),
        // Exclusive access, borrowed for as long as the string: nothing
        // can change or free it meanwhile.
        RetPlan::BorrowedStr => (
            quote!(-> Option<&core::ffi::CStr>),
            quote!(crate::base::gen_support::borrow_str(unsafe { #call })),
        ),
        RetPlan::Unit => (quote!(), quote!(unsafe { #call })),
        RetPlan::Scalar(ty) => {
            let ty: syn::Type = syn::parse_str(ty).unwrap();
            (quote!(-> #ty), quote!(unsafe { #call }))
        }
        RetPlan::Struct(c_type) => {
            let ty = Ident::new(c_type, Span::call_site());
            (quote!(-> crate::sys::#ty), quote!(unsafe { #call }))
        }
        RetPlan::Enum(c_type) => {
            let ty = Ident::new(&type_name(c_type), Span::call_site());
            (
                quote!(-> crate::enums::#ty),
                quote!(crate::enums::#ty(unsafe { #call } as u32)),
            )
        }
        RetPlan::Observer => (
            quote!(-> Result<(), crate::Error>),
            quote! {
                if unsafe { #call }.is_null() {
                    Err(crate::Error::OutOfMemory)
                } else {
                    Ok(())
                }
            },
        ),
        RetPlan::Font => (
            quote!(-> Option<&crate::Font>),
            quote!(crate::base::gen_support::font(unsafe { #call })),
        ),
        RetPlan::View(None) => (
            quote!(-> Option<&crate::Obj>),
            quote!(crate::base::gen_support::getter_view(unsafe { #call })),
        ),
        RetPlan::View(Some(class)) => {
            let ty = class_type(classifier, class);
            (
                quote!(-> Option<&#ty>),
                quote! {
                    crate::base::gen_support::getter_view(unsafe { #call })
                        // SAFETY: the override certifies the class of this part.
                        .map(|o| unsafe { crate::base::gen_support::view::<#ty>(o.as_ptr()) })
                },
            )
        }
    };
    // An out parameter is returned, after the C return value if any.
    let (ret_ty, body) = match out {
        None => (ret_ty, body),
        Some((n, ty)) => {
            let ret_ty = if ret_ty.is_empty() {
                quote!(-> #ty)
            } else {
                let inner = strip_arrow(ret_ty);
                quote!(-> (#inner, #ty))
            };
            let value = if matches!(m.ret, RetPlan::Unit) {
                quote! { #body; #n }
            } else {
                quote! { let value = #body; (value, #n) }
            };
            (
                ret_ty,
                quote! { let mut #n: #ty = Default::default(); #value },
            )
        }
    };
    // Resolving handles can fail: the result becomes a `Result`.
    let returns_result = matches!(m.ret, RetPlan::Observer);
    let (ret_ty, body, cfg) = if handles.is_empty() {
        (ret_ty, body, quote!())
    } else {
        let cfg = quote!(#[cfg(not(lvgl_rust_views_only))]);
        if returns_result {
            (ret_ty, quote! { #(#handles)* #body }, cfg)
        } else {
            if ret_ty.is_empty() {
                (
                    quote!(-> Result<(), crate::Error>),
                    quote! { #(#handles)* #body; Ok(()) },
                    cfg,
                )
            } else {
                let inner = strip_arrow(ret_ty);
                (
                    quote!(-> Result<#inner, crate::Error>),
                    quote! { #(#handles)* let value = { #body }; Ok(value) },
                    cfg,
                )
            }
        }
    };
    // Safety of every call: the receiver (view, `ObjMut`, `Lvgl`) proves
    // the access the table assigns to the function, and each parameter is
    // mapped to a type that upholds what LVGL expects of it.
    quote! {
        #doc
        #cfg
        #[inline]
        #[allow(clippy::unnecessary_cast, clippy::too_many_arguments)]
        pub fn #name #generics(#receiver #(#args),*) #ret_ty {
            #body
        }
    }
}

/// `T` from the tokens of `-> T`.
fn strip_arrow(ret: TokenStream) -> TokenStream {
    ret.into_iter().skip(2).collect()
}

fn emit_param(p: &ParamPlan) -> (TokenStream, TokenStream) {
    let ident = |n: &str| Ident::new(n, Span::call_site());
    match p {
        ParamPlan::Scalar { name, ty } => {
            let n = ident(name);
            let ty: syn::Type = syn::parse_str(ty).unwrap();
            (quote!(#n: #ty), quote!(#n))
        }
        ParamPlan::Enum { name, c_type } => {
            let n = ident(name);
            let ty = ident(&type_name(c_type));
            let c = ident(c_type);
            (
                quote!(#n: crate::enums::#ty),
                quote!(#n.0 as crate::sys::#c),
            )
        }
        ParamPlan::Struct { name, c_type } => {
            let n = ident(name);
            let c = ident(c_type);
            (quote!(#n: crate::sys::#c), quote!(#n))
        }
        ParamPlan::StructRef { name, c_type } => {
            let n = ident(name);
            let c = ident(c_type);
            (quote!(#n: &crate::sys::#c), quote!(#n))
        }
        ParamPlan::Obj {
            name,
            nullable: false,
        } => {
            let n = ident(name);
            (quote!(#n: &crate::Obj), quote!(#n.as_ptr()))
        }
        ParamPlan::Obj {
            name,
            nullable: true,
        } => {
            let n = ident(name);
            (
                quote!(#n: Option<&crate::Obj>),
                quote!(#n.map(crate::Obj::as_ptr).unwrap_or(core::ptr::null_mut())),
            )
        }
        ParamPlan::Str {
            name,
            is_static,
            nullable,
        } => {
            let n = ident(name);
            let ty = if *is_static {
                quote!(&'static core::ffi::CStr)
            } else {
                quote!(&core::ffi::CStr)
            };
            if *nullable {
                (
                    quote!(#n: Option<#ty>),
                    quote!(#n.map_or(core::ptr::null(), core::ffi::CStr::as_ptr)),
                )
            } else {
                (quote!(#n: #ty), quote!(#n.as_ptr()))
            }
        }
        ParamPlan::Style {
            name,
            nullable: false,
        } => {
            let n = ident(name);
            (
                quote!(#n: impl Into<crate::StyleRef>),
                quote!(#n.into().as_ptr()),
            )
        }
        ParamPlan::Style {
            name,
            nullable: true,
        } => {
            let n = ident(name);
            (
                quote!(#n: Option<crate::StyleRef>),
                quote!(#n.map_or(core::ptr::null(), crate::StyleRef::as_ptr)),
            )
        }
        ParamPlan::Font {
            name,
            nullable: false,
        } => {
            let n = ident(name);
            (quote!(#n: &'static crate::Font), quote!(#n.as_ptr()))
        }
        ParamPlan::Font {
            name,
            nullable: true,
        } => {
            let n = ident(name);
            (
                quote!(#n: Option<&'static crate::Font>),
                quote!(#n.map_or(core::ptr::null(), crate::Font::as_ptr)),
            )
        }
        ParamPlan::Retained {
            name,
            ty,
            by_value,
            nullable,
        } => {
            let n = ident(name);
            let t = ident(ty);
            let ty = if *by_value {
                quote!(crate::#t)
            } else {
                quote!(&'static crate::#t)
            };
            if *nullable {
                (
                    quote!(#n: Option<#ty>),
                    quote!(#n.map_or(core::ptr::null(), |v| v.as_ptr().cast())),
                )
            } else {
                (quote!(#n: #ty), quote!(#n.as_ptr().cast()))
            }
        }
        ParamPlan::ObjHandle { name, nullable } => {
            let n = ident(name);
            if *nullable {
                (quote!(#n: Option<crate::Handle<crate::Obj>>), quote!(#n))
            } else {
                (quote!(#n: crate::Handle<crate::Obj>), quote!(#n))
            }
        }
        ParamPlan::Selector { name } => {
            let n = ident(name);
            (
                quote!(#n: impl Into<crate::Selector>),
                quote!(#n.into().bits()),
            )
        }
        ParamPlan::Out { .. } => unreachable!("handled by emit_method"),
    }
}

/// A `const` property constructor for a style setter
/// (`lv_style_set_<p>(style, value)`), if its value fits a constant.
fn emit_prop(classifier: &Classifier, m: &Method) -> Option<TokenStream> {
    if m.shape != Shape::Style || m.ret != RetPlan::Unit {
        return None;
    }
    let property = m.name.strip_prefix("set_")?;
    let [param] = m.params.as_slice() else {
        return None;
    };
    let id = Ident::new(&style_prop_id(classifier, property)?, Span::call_site());
    // `lv_style_value_t` field, as the setters of `lv_style_gen.c` fill it.
    let (arg, value) = match param {
        ParamPlan::Scalar { ty, .. }
            if !matches!(
                ty.as_str(),
                "f32" | "f64" | "core::ffi::c_float" | "core::ffi::c_double"
            ) =>
        {
            let ty: syn::Type = syn::parse_str(ty).unwrap();
            (quote!(value: #ty), quote!(num: value as i32))
        }
        ParamPlan::Enum { c_type, .. } => {
            let ty = Ident::new(&type_name(c_type), Span::call_site());
            (
                quote!(value: crate::enums::#ty),
                quote!(num: value.0 as i32),
            )
        }
        ParamPlan::Struct { c_type, .. } if c_type == "lv_color_t" => {
            (quote!(value: crate::sys::lv_color_t), quote!(color: value))
        }
        ParamPlan::Font {
            nullable: false, ..
        } => (
            quote!(value: &'static crate::Font),
            quote!(ptr: (value as *const crate::Font).cast()),
        ),
        // A property set to NULL is unset: constants always point to data.
        ParamPlan::Retained { ty, by_value, .. } => {
            let t = Ident::new(ty, Span::call_site());
            let ty = if *by_value {
                quote!(crate::#t)
            } else {
                quote!(&'static crate::#t)
            };
            (quote!(value: #ty), quote!(ptr: value.as_ptr().cast()))
        }
        _ => return None,
    };
    let name = Ident::new(&m.name["set_".len()..], Span::call_site());
    let doc = format!(" `{}` (`{}`).", property, m.c_name);
    Some(quote! {
        #[doc = #doc]
        #[inline]
        #[allow(clippy::unnecessary_cast)]
        pub const fn #name(#arg) -> crate::ConstProp {
            crate::ConstProp::new(
                crate::sys::#id as crate::sys::lv_style_prop_t,
                crate::sys::lv_style_value_t { #value },
            )
        }
    })
}

/// `DrawBuf`, aligned to `LV_DRAW_BUF_ALIGN` (a literal: `repr(align)`
/// takes no constant expression).
fn emit_draw_buf(classifier: &Classifier) -> TokenStream {
    let Some(&align) = classifier.bindings.consts.get("LV_DRAW_BUF_ALIGN") else {
        return quote!();
    };
    let align = proc_macro2::Literal::usize_unsuffixed(align.max(1) as usize);
    quote! {
        /// A draw buffer for a [`Display`](crate::Display): a `static`
        /// aligned to `LV_DRAW_BUF_ALIGN`, claimed by the display that uses
        /// it.
        #[repr(C, align(#align))]
        pub struct DrawBuf<const BYTES: usize> {
            pub(crate) data: core::cell::UnsafeCell<[u8; BYTES]>,
            pub(crate) claimed: core::cell::Cell<bool>,
        }
    }
}

/// The constant of a style property (`LV_STYLE_BG_COLOR`, ...).
fn style_prop_id(classifier: &Classifier, property: &str) -> Option<String> {
    let id = format!("LV_STYLE_{}", property.to_ascii_uppercase());
    ["_lv_style_id_t_", ""]
        .iter()
        .map(|prefix| format!("{prefix}{id}"))
        .find(|c| classifier.bindings.consts.contains_key(c))
}

/// `Font` constructors for the fonts these bindings declare
/// (`lv_font_montserrat_14`, ...).
fn emit_fonts(classifier: &Classifier) -> TokenStream {
    let fonts = classifier
        .bindings
        .static_types
        .iter()
        .filter(|(_, ty)| ty.named() == Some("lv_font_t"))
        .map(|(c_name, _)| {
            let c_ident = Ident::new(c_name, Span::call_site());
            let name = Ident::new(
                &rust_ident(c_name.strip_prefix("lv_font_").unwrap_or(c_name)),
                Span::call_site(),
            );
            let doc = format!(" `{c_name}`.");
            quote! {
                #[doc = #doc]
                #[inline]
                pub const fn #name() -> &'static Self {
                    // SAFETY: a constant font of the C library.
                    unsafe { Self::from_raw(&*core::ptr::addr_of!(crate::sys::#c_ident)) }
                }
            }
        })
        .collect::<Vec<_>>();
    if fonts.is_empty() {
        return quote!();
    }
    quote! {
        impl crate::Font {
            #(#fonts)*
        }
    }
}

/// The header comment up to its first `@` tag, plus the C name.
fn doc_text(doc: &str, c_name: &str) -> String {
    let summary: Vec<&str> = doc
        .lines()
        .take_while(|l| !l.trim_start().starts_with('@'))
        .collect();
    let summary = summary.join("\n");
    let summary = summary.trim();
    if summary.is_empty() {
        format!(" `{c_name}`.")
    } else {
        format!(" {summary}\n\n `{c_name}`.")
    }
}

/// One `#[doc]` attribute per line, so the output uses `///` comments.
fn doc_attrs(text: &str) -> TokenStream {
    let lines = text.lines().map(|line| {
        let line = if line.is_empty() || line.starts_with(' ') {
            line.to_owned()
        } else {
            format!(" {line}")
        };
        quote!(#[doc = #line])
    });
    quote!(#(#lines)*)
}

/// The composite widgets of `classes.toml` present in these bindings: the
/// structural checks find their parts at call time.
fn emit_composites(classifier: &Classifier) -> TokenStream {
    let bindings = classifier.bindings;
    let mut entries = Vec::new();
    let mut leaves = Vec::new();
    let mut functions = Vec::new();
    let mut max_parts = 1usize;
    for (class, composite) in &classifier.tables.classes.composite {
        if !bindings.statics.contains(&format!("lv_{class}_class")) {
            continue;
        }
        let mut parts = Vec::new();
        let mut complete = true;
        for part in &composite.parts {
            let expr = if let Some(index) = part.strip_prefix("child:") {
                let index: i32 = index.parse().unwrap();
                quote!(crate::sys::lv_obj_get_child(o, #index))
            } else if let Some(shim) = part.strip_prefix("shim:") {
                let field = shim.split_once('.').map_or(shim, |(_, f)| f);
                let constant = format!(
                    "lvgl_sys_{class}_part_t_LVGL_SYS_{}_PART_{}",
                    class.to_ascii_uppercase(),
                    field.to_ascii_uppercase().replace("BTN", "BUTTON")
                );
                if !bindings.consts.contains_key(&constant) {
                    complete = false;
                    break;
                }
                let shim_fn = format_ident!("lvgl_sys_{}_part", class);
                let constant = Ident::new(&constant, Span::call_site());
                quote!(crate::sys::#shim_fn(o, crate::sys::#constant))
            } else {
                if !bindings.functions.contains_key(part) {
                    complete = false;
                    break;
                }
                let getter = Ident::new(part, Span::call_site());
                quote!(crate::sys::#getter(o))
            };
            parts.push(expr);
        }
        if !complete {
            continue; // a part getter is compiled out: treat the class as unknown
        }
        max_parts = max_parts.max(parts.len());
        let class_static = format_ident!("lv_{}_class", class);
        let count = parts.len();
        // Widgets without parts only need their class.
        if count == 0 {
            leaves.push(quote!(crate::base::structural::ClassRef(
                core::ptr::addr_of!(crate::sys::#class_static)
            )));
            continue;
        }
        let parts_fn = format_ident!("{}_parts", class);
        {
            let indices = 0..count;
            functions.push(quote! {
                #[cfg_attr(miri, allow(dead_code))]
                fn #parts_fn(o: *mut crate::sys::lv_obj_t, out: &mut [*mut crate::sys::lv_obj_t; MAX_PARTS]) -> usize {
                    // SAFETY: `o` is a live object of this class (checked by
                    // the caller with `lv_obj_has_class`), under the token.
                    unsafe { #( out[#indices] = #parts; )* }
                    #count
                }
            });
        }
        let max_depth = composite.max_depth;
        let indexed: Vec<u8> = composite
            .parts
            .iter()
            .map(|p| composite.indexed.get(p).copied().unwrap_or(0))
            .collect();
        entries.push(quote! {
            crate::base::structural::Composite {
                class: core::ptr::addr_of!(crate::sys::#class_static),
                max_depth: #max_depth,
                indexed: &[#(#indexed),*],
                parts: #parts_fn,
            }
        });
    }
    quote! {
        /// Most parts of a composite widget.
        pub(crate) const MAX_PARTS: usize = #max_parts;
        /// Composite widgets: their internal parts (`classes.toml`).
        // Under Miri, `ffi_stub.rs` models the class checks without these.
        #[cfg_attr(miri, allow(dead_code))]
        pub(crate) static COMPOSITES: &[crate::base::structural::Composite] = &[#(#entries),*];
        /// Widgets that create no child and keep no pointer to their
        /// children.
        #[cfg_attr(miri, allow(dead_code))]
        pub(crate) static LEAVES: &[crate::base::structural::ClassRef] = &[#(#leaves),*];
        #(#functions)*
    }
}
