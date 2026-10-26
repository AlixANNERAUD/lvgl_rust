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

use crate::{Obj, base::gen_support::getter_view, sys, widgets::TileView};

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

impl TileView {
    /// The active tile: the last one shown by `set_tile`,
    /// `set_tile_by_index` or scrolling, if it is still one of this tile
    /// view's tiles.
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
