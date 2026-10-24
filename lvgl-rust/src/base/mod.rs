//! The hand-written core: access, registry, views, ownership, deletion.

#[cfg(not(any(miri, loom)))]
mod ffi;
#[cfg(any(miri, loom))]
#[path = "ffi_stub.rs"]
pub(crate) mod ffi;

pub(crate) mod drain;
#[cfg(not(lvgl_rust_views_only))]
mod handle;
#[cfg(all(not(lvgl_rust_views_only), not(lvgl_rust_docs)))]
mod hook;
mod objmut;
pub(crate) mod registry;
mod session;
mod structural;
mod sync;
mod view;

#[cfg(not(lvgl_rust_views_only))]
pub use handle::{Handle, HandleCell, Owned};
pub use objmut::ObjMut;
pub use session::{DepthGuard, Lvgl, LvglAccess, Session, init, tick_inc};
#[cfg(not(lvgl_rust_views_only))]
pub use view::WidgetExt;
pub use view::{IsA, Obj, Widget};

/// What the generated wrappers (`gen.rs`) use from the core.
pub(crate) mod gen_support {
    pub(crate) use super::view::{getter_view, ptr_of, view};
}
