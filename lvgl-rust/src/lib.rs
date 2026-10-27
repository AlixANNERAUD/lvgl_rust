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
//!
//! Diagnostic features, free when off: `defmt` (`defmt::Format` for
//! [`Error`], handles and the public enums and plain types) and
//! `debug-counters` (`DebugCounters`: what the crate did or refused
//! without returning an error, in 8 bytes of `.bss`).

#![no_std]

#[allow(dead_code)] // `HANDLE_SLOTS` is unused in views-only mode.
mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

#[cfg(feature = "allocator")]
mod allocator;
mod anim;
mod base;
mod button_map;
#[cfg(not(lvgl_rust_views_only))]
mod component;
mod counters;
mod display;
mod error;
mod event;
#[cfg(lv_use_grid)]
mod grid;
#[cfg(not(lvgl_rust_views_only))]
mod group;
#[cfg(feature = "heap")]
mod heap;
mod image;
#[cfg(any(feature = "heap", feature = "allocator"))]
mod heap_align {
    /// What every allocator LVGL uses guarantees: the builtin TLSF aligns
    /// to a pointer (`lv_tlsf.c`), C `malloc` to at least that, and an
    /// `LvglAllocator` must.
    pub(crate) const HEAP_ALIGN: usize = core::mem::size_of::<usize>();
}
mod indev;
mod manual;
mod query;
#[cfg(feature = "async")]
mod queue;
mod scale_labels;
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

pub use generated::{DrawBuf, enums, functions, widgets};

/// Property constructors for [`const_style!`]: one per style setter whose
/// value fits a constant.
pub mod props {
    pub use crate::generated::props::*;

    /// `layout` (`lv_style_set_layout`).
    ///
    /// # Panics
    ///
    /// On `LV_LAYOUT_GRID` (a compile error in a `static`): the grid layout
    /// is set only with its templates, by `ObjMut::set_grid_dsc_array`.
    pub const fn layout(value: u16) -> crate::ConstProp {
        assert!(
            !crate::manual::is_grid(value),
            "the grid layout is set by ObjMut::set_grid_dsc_array"
        );
        crate::ConstProp::new(
            crate::sys::_lv_style_id_t_LV_STYLE_LAYOUT as crate::sys::lv_style_prop_t,
            crate::sys::lv_style_value_t { num: value as i32 },
        )
    }

    /// `grid_cell_column_span` (`lv_style_set_grid_cell_column_span`).
    ///
    /// # Panics
    ///
    /// Above [`GridTemplate::MAX_SPAN`](crate::GridTemplate::MAX_SPAN) (a
    /// compile error in a `static`).
    #[cfg(lv_use_grid)]
    pub const fn grid_cell_column_span(value: i32) -> crate::ConstProp {
        assert!(crate::grid::span_ok(value), "a grid span is at most 65535");
        crate::ConstProp::new(
            crate::sys::_lv_style_id_t_LV_STYLE_GRID_CELL_COLUMN_SPAN
                as crate::sys::lv_style_prop_t,
            crate::sys::lv_style_value_t { num: value },
        )
    }

    /// `grid_cell_row_span` (`lv_style_set_grid_cell_row_span`).
    ///
    /// # Panics
    ///
    /// Above [`GridTemplate::MAX_SPAN`](crate::GridTemplate::MAX_SPAN) (a
    /// compile error in a `static`).
    #[cfg(lv_use_grid)]
    pub const fn grid_cell_row_span(value: i32) -> crate::ConstProp {
        assert!(crate::grid::span_ok(value), "a grid span is at most 65535");
        crate::ConstProp::new(
            crate::sys::_lv_style_id_t_LV_STYLE_GRID_CELL_ROW_SPAN as crate::sys::lv_style_prop_t,
            crate::sys::lv_style_value_t { num: value },
        )
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "allocator")]
pub use allocator::{LvglAllocator, SizedHeader};
pub use anim::{Anim, AnimCtx};
pub use base::*;
#[cfg(lv_use_keyboard)]
pub use button_map::KeyboardMap;
pub use button_map::{ButtonMap, MapEntry};
#[cfg(not(lvgl_rust_views_only))]
pub use component::Component;
#[cfg(feature = "debug-counters")]
pub use counters::DebugCounters;
pub use display::{Display, DisplayCell, DrawBuffers, Flush, FlushDone, FlushRequest, RenderMode};
pub use error::Error;
#[cfg(not(lvgl_rust_views_only))]
pub use event::EventSub;
pub use event::{Event, Handler, HandlerResult, UserData, handler_allowed};
#[cfg(lv_use_grid)]
pub use grid::{GridTemplate, Track};
#[cfg(not(lvgl_rust_views_only))]
pub use group::Group;
pub use image::{ImageColorkey, ImageData, ImageDsc, ImageSrc, PixelFormat};
pub use indev::{Indev, IndevCell, IndevData, IndevKind, Read};
pub use lvgl_rust_sys as sys;
pub use query::{
    CoverCheck, CoverResult, DrawTask, ExtDrawSize, HitTest, Layer, QueryEvent, SelfSize,
    query_handler_allowed,
};
#[cfg(feature = "async")]
pub use queue::{Receiver, Recv, StaticQueue, StaticRx, next_event};
pub use scale_labels::{ScaleLabel, ScaleLabels};
pub use screen::{ScreenAnim, ScreenLoadAnim};
pub use style::{
    AnimPath, ColorFilter, ConstProp, ConstStyle, Font, GradDir, GradDsc, Selector, Style,
    StyleMut, StyleProp, StyleRef, TransitionDsc, color,
};
pub use subject::{ColorSubject, IntSubject, ObserverCtx};
#[cfg(not(lvgl_rust_views_only))]
pub use timer::{Timer, TimerCtx};
