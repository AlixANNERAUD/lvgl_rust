//! Tile views. LVGL keeps the active tile in `tile_act` and never clears it
//! when that tile is deleted (`lv_tileview.c:99, 192`), and casts any
//! object it is given, or finds at a tile's position, to a tile to read its
//! direction (97-101, 118-125, 186-193).
//!
//! The part tables declare the tile view closed (`classes.toml`): its
//! children come only from `add_tile`, so every child is a tile. Tiles can
//! be deleted; nothing in LVGL dereferences `tile_act` except
//! `lv_tileview_get_tile_active`, whose wrapper returns the pointer only
//! while it is still one of the tile view's tiles, and `set_tile` takes
//! only one of them.
//!
//! The tile view's SCROLL_END handler divides by its content width and
//! height (`lv_tileview.c:175-182`), which a size, padding or layout can
//! bring to 0 or below. The check must happen right before that handler:
//! LVGL's class handler runs between the preprocessed handlers and the
//! normal ones, and on SCROLL_END it removes the SCROLLED state
//! (`lv_obj.c:1937-1938`), which can change the padding and sends
//! STATE_CHANGED to application handlers. [`TileView::create`] therefore
//! replaces LVGL's descriptor with one that checks both sizes and then
//! calls LVGL's handler, kept as the descriptor's user data. Event removal
//! is crate-owned, so safe code cannot take the check away. The tile view
//! keeps a single descriptor, as LVGL's own.

use crate::{
    Error, Obj, ObjMut, Widget,
    base::gen_support::{check_create, created, getter_view},
    sys,
    widgets::TileView,
};

/// Whether `tile` is a child of `tileview` of the tile class. `tile` may
/// dangle: it is only compared with live children.
///
/// # Safety
///
/// `tileview` must be a live tile view, under the token.
unsafe fn is_tile_of(tileview: *mut sys::lv_obj_t, tile: *mut sys::lv_obj_t) -> bool {
    if tile.is_null() {
        return false;
    }
    // SAFETY: caller contract; children of a live object are live.
    unsafe {
        let count = sys::lv_obj_get_child_count(tileview);
        (0..count).any(|i| {
            let child = sys::lv_obj_get_child(tileview, i as i32);
            child == tile && sys::lv_obj_has_class(child, &raw const sys::lv_tileview_tile_class)
        })
    }
}

/// Calls LVGL's SCROLL_END handler, kept as the user data, only while
/// the content width and height it divides by are positive. Runs no user
/// code: nothing between this check and LVGL's division sends an event
/// (`lv_tileview.c:168-182`).
unsafe extern "C" fn checked_scroll_end(e: *mut sys::lv_event_t) {
    // SAFETY: an event of a live tile view (the object holding the
    // handler, also when the event bubbled up to it); the user data is
    // the callback `TileView::create` read from LVGL's descriptor.
    unsafe {
        let tileview = sys::lv_event_get_current_target_obj(e);
        if sys::lv_obj_get_content_width(tileview) <= 0
            || sys::lv_obj_get_content_height(tileview) <= 0
        {
            return;
        }
        let handler: sys::lv_event_cb_t = core::mem::transmute::<
            *mut core::ffi::c_void,
            sys::lv_event_cb_t,
        >(sys::lv_event_get_user_data(e));
        if let Some(handler) = handler {
            handler(e);
        }
    }
}

impl TileView {
    /// Creates a tile view in `parent`, with exclusive access (the
    /// parent's `ObjMut`), like every widget creation; the tile view is
    /// lent as a view borrowing it.
    ///
    /// [`Error::OutOfMemory`] if the tile view or its checked scroll
    /// handler (see the module) could not be allocated; in the second case
    /// the tile view stays in `parent` without a scroll handler (scrolling
    /// no longer changes the active tile).
    ///
    /// `lv_tileview_create`.
    pub fn create<'p, P: Widget>(parent: &'p mut ObjMut<'_, P>) -> Result<&'p Self, Error> {
        check_create(parent.as_raw())?;
        // SAFETY: exclusive access through the parent's `ObjMut`, as
        // `lv_tileview_create`'s access requires.
        let p = unsafe { sys::lv_tileview_create(parent.as_raw()) };
        if p.is_null() {
            return Err(Error::OutOfMemory);
        }
        // SAFETY: a tile view just created: no event is being sent to it,
        // and its only handler is its constructor's, at index 0 (themes
        // are applied after the constructor and add no handler).
        unsafe {
            let original = sys::lv_obj_get_event_dsc(p, 0);
            let handler = if original.is_null() {
                None
            } else {
                sys::lv_event_dsc_get_cb(original)
            };
            let Some(handler) = handler else {
                return Err(Error::Internal);
            };
            #[allow(clippy::unnecessary_cast)] // the bindgen alias may already be `u32`
            let checked = sys::lv_obj_add_event_cb(
                p,
                Some(checked_scroll_end),
                sys::lv_event_code_t_LV_EVENT_SCROLL_END as _,
                handler as *mut core::ffi::c_void,
            );
            // LVGL's descriptor goes in both cases: removing frees it and
            // allocates nothing. Without the checked one, scrolling no
            // longer changes the active tile.
            sys::lv_obj_remove_event(p, 0);
            if checked.is_null() {
                return Err(Error::OutOfMemory);
            }
            // SAFETY: just created, of this class; the view borrows the
            // parent's `ObjMut`.
            created::<Self>(p)
        }
    }

    /// Creates a tile view and takes ownership of it.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn create_owned<P: Widget>(
        parent: &mut ObjMut<'_, P>,
    ) -> Result<crate::Owned<Self>, Error> {
        crate::WidgetExt::own(Self::create(parent)?)
    }

    /// The active tile: the last one shown by `set_tile`,
    /// `set_tile_by_index` or scrolling, if it is still one of this tile
    /// view's tiles. The check compares addresses: after the active tile
    /// is deleted, a tile added at the same address is reported as the
    /// active one (LVGL keeps the stale pointer, `lv_tileview.c:192`).
    ///
    /// `lv_tileview_get_tile_active`.
    pub fn get_tile_active(&self) -> Option<&Obj> {
        // SAFETY: shared access, live tile view; the pointer is compared
        // with its children before any use.
        let tile = unsafe { sys::lv_tileview_get_tile_active(self.as_ptr()) };
        // SAFETY: as above.
        if unsafe { is_tile_of(self.as_ptr(), tile) } {
            getter_view(tile)
        } else {
            None
        }
    }
}

#[cfg(not(lvgl_rust_views_only))]
impl crate::ObjMut<'_, TileView> {
    /// Shows `tile`, which must be one of this tile view's tiles
    /// ([`Error::InvalidArg`](crate::Error::InvalidArg) otherwise).
    ///
    /// `lv_tileview_set_tile`.
    pub fn set_tile(&mut self, tile: crate::Handle<Obj>, anim: bool) -> Result<(), crate::Error> {
        let tile = crate::base::gen_support::resolve_obj(tile)?;
        // SAFETY: exclusive access, live tile view and object.
        if !unsafe { is_tile_of(self.as_raw(), tile) } {
            return Err(crate::Error::InvalidArg);
        }
        // SAFETY: a tile of this tile view.
        unsafe { sys::lv_tileview_set_tile(self.as_raw(), tile, anim) };
        Ok(())
    }
}
