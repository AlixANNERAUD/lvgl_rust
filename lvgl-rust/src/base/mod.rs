//! The hand-written core: access, registry, views, ownership, deletion.

#[cfg(not(any(miri, loom)))]
mod ffi;
#[cfg(any(miri, loom))]
#[path = "ffi_stub.rs"]
pub(crate) mod ffi;

pub(crate) mod drain;
#[cfg(not(lvgl_rust_views_only))]
pub(crate) mod handle;
#[cfg(all(not(lvgl_rust_views_only), not(lvgl_rust_docs)))]
mod hook;
mod objmut;
pub(crate) mod post;
pub(crate) mod registry;
mod session;
pub(crate) mod structural;
pub(crate) mod sync;
mod view;

#[cfg(not(lvgl_rust_views_only))]
pub use handle::{Handle, HandleCell, Owned, Tracked};
pub use objmut::ObjMut;
pub use session::{DepthGuard, Lvgl, LvglAccess, Session, init, tick_inc};
#[cfg(not(lvgl_rust_views_only))]
pub use view::WidgetExt;
pub use view::{IsA, Obj, Widget};

/// The current callback depth (0 outside callbacks).
pub(crate) fn callback_depth() -> u8 {
    registry::reg().hdr.depth.get()
}

/// What the generated wrappers (`gen.rs`) use from the core.
pub(crate) mod gen_support {
    pub(crate) use super::structural::check_create;

    /// The object of a handle, under exclusive access.
    #[cfg(not(lvgl_rust_views_only))]
    pub(crate) fn resolve_obj(
        handle: crate::Handle<crate::Obj>,
    ) -> Result<*mut super::ffi::lv_obj_t, crate::Error> {
        super::registry::reg()
            .resolve(handle.raw(), super::registry::Kind::Obj, core::ptr::null())
            .map(|p| p.as_ptr().cast())
    }
    pub(crate) use super::view::{getter_view, ptr_of, view};
    pub(crate) use crate::style::font;
    pub(crate) use crate::text::{borrow_str, copy_str};
}
