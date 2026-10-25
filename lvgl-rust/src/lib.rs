//! Safe, idiomatic bindings to [LVGL](https://lvgl.io), built on
//! [`lvgl_rust_sys`].
//!
//! The crate is `no_std`, needs no allocator and never locks: callers prove
//! exclusive access to LVGL with a token ([`LvglAccess`]) and get a
//! [`Session`], from which every call is made.
//!
//! - Shared access (`&Lvgl`) gives zero-sized views (`&Obj`, ...) whose calls
//!   are plain FFI calls; it cannot free objects.
//! - Exclusive access (`&mut Lvgl`, [`ObjMut`]) is needed to delete, clean or
//!   reparent objects, so no view can dangle.
//! - [`Handle`]s are 4-byte references that can be stored and sent between
//!   threads; they report deleted objects instead of dangling. [`Owned`]
//!   deletes its object when dropped, from any thread.
//!
//! Build variables: `LVGL_RUST_HANDLE_SLOTS` (registry size, default 32;
//! `0` for views only, without handles) and `LVGL_RUST_MAX_CALLBACK_DEPTH`
//! (default 16) and `LVGL_RUST_POST_SLOTS` (entries of [`Lvgl::post`],
//! default 4).

#![no_std]

#[allow(dead_code)] // `HANDLE_SLOTS` is unused in views-only mode.
mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

mod anim;
mod base;
mod button_map;
#[cfg(not(lvgl_rust_views_only))]
mod component;
mod error;
mod event;
mod image;
mod manual;
mod screen;
mod style;
mod subject;
mod text;
#[cfg(not(lvgl_rust_views_only))]
mod timer;

/// Wrappers generated from this build's bindings and the reviewed tables of
/// `lvgl-rust-gen` (the list of what is wrapped is `$OUT_DIR/coverage.txt`).
/// Their docs are LVGL's header comments, written for C tools.
#[allow(
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::invalid_rust_codeblocks
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/gen.rs"));
}

pub use generated::{enums, functions, props, widgets};

#[cfg(test)]
mod tests;

pub use anim::{Anim, AnimCtx};
pub use base::*;
pub use button_map::{ButtonMap, MapEntry};
#[cfg(not(lvgl_rust_views_only))]
pub use component::Component;
pub use error::Error;
#[cfg(not(lvgl_rust_views_only))]
pub use event::EventSub;
pub use event::{Event, Handler, HandlerResult, UserData, handler_allowed};
pub use image::{ImageColorkey, ImageData, ImageDsc, ImageSrc, PixelFormat};
pub use lvgl_rust_sys as sys;
pub use screen::{ScreenAnim, ScreenLoadAnim};
pub use style::{
    AnimPath, ColorFilter, ConstProp, ConstStyle, Font, GradDir, GradDsc, Selector, Style,
    StyleMut, StyleProp, StyleRef, TransitionDsc, color,
};
pub use subject::{ColorSubject, IntSubject, ObserverCtx};
#[cfg(not(lvgl_rust_views_only))]
pub use timer::{Timer, TimerCtx};
