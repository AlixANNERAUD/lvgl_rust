//! Structural checks: operations LVGL accepts but that would leave dangling
//! pointers inside LVGL, refused before they reach it.
//!
//! - Display layers cannot be deleted, owned or reparented:
//!   `lv_display_delete` deletes them again through stored pointers.
//! - A screen being loaded, or the previous one during an animated load,
//!   cannot be deleted yet (`Busy`): LVGL keeps pointers to both.
//! - `set_parent` refuses cycles (LVGL has no guard).
//! - Only parentless objects can be loaded as screens.

use super::ffi::{self, lv_obj_t};
use crate::{Error, ScreenAnim};
use core::ptr;

/// Whether `o` is one of a display's layers (bottom, top, system).
fn is_layer(o: *mut lv_obj_t) -> bool {
    // SAFETY (whole function): token held, `o` is a live object.
    if !unsafe { ffi::obj_parent(o) }.is_null() {
        return false;
    }
    any_display(|d| unsafe { ffi::display_layers(d) }.contains(&o))
}

/// Whether LVGL holds `o` as a loading or previous screen.
fn is_loading(o: *mut lv_obj_t) -> bool {
    any_display(|d| unsafe { ffi::display_loading_screens(d) }.contains(&o))
}

fn any_display(mut f: impl FnMut(*mut ffi::lv_display_t) -> bool) -> bool {
    // SAFETY: token held; LVGL owns the display list.
    let mut display = unsafe { ffi::display_next(ptr::null_mut()) };
    while !display.is_null() {
        if f(display) {
            return true;
        }
        display = unsafe { ffi::display_next(display) };
    }
    false
}

pub(crate) fn check_delete(o: *mut lv_obj_t) -> Result<(), Error> {
    if is_layer(o) {
        return Err(Error::Internal);
    }
    if is_loading(o) {
        return Err(Error::Busy);
    }
    Ok(())
}

pub(crate) fn check_clean(_o: *mut lv_obj_t) -> Result<(), Error> {
    Ok(())
}

#[cfg(not(lvgl_rust_views_only))]
pub(crate) fn check_own(o: *mut lv_obj_t) -> Result<(), Error> {
    if is_layer(o) {
        return Err(Error::Internal);
    }
    Ok(())
}

pub(crate) fn check_set_parent(o: *mut lv_obj_t, parent: *mut lv_obj_t) -> Result<(), Error> {
    if is_layer(o) {
        return Err(Error::Internal);
    }
    // Reject `o` itself and its descendants.
    let mut ancestor = parent;
    while !ancestor.is_null() {
        if ancestor == o {
            return Err(Error::InvalidArg);
        }
        // SAFETY: token held, ancestors of a live object are live.
        ancestor = unsafe { ffi::obj_parent(ancestor) };
    }
    Ok(())
}

pub(crate) fn delete(o: *mut lv_obj_t) -> Result<(), Error> {
    check_delete(o)?;
    // SAFETY: exclusive access, checks passed.
    unsafe { ffi::obj_delete(o) };
    Ok(())
}

pub(crate) fn clean(o: *mut lv_obj_t) -> Result<(), Error> {
    check_clean(o)?;
    // SAFETY: exclusive access, checks passed.
    unsafe { ffi::obj_clean(o) };
    Ok(())
}

pub(crate) fn set_parent(o: *mut lv_obj_t, parent: *mut lv_obj_t) -> Result<(), Error> {
    check_set_parent(o, parent)?;
    // SAFETY: exclusive access, checks passed.
    unsafe { ffi::obj_set_parent(o, parent) };
    Ok(())
}

/// Loads `screen`: it must have no parent (`lv_obj_delete` forgets the
/// active screen only for parentless objects) and not be a layer.
pub(crate) fn load_screen(screen: *mut lv_obj_t, anim: ScreenAnim) -> Result<(), Error> {
    // SAFETY: token held, `screen` is a live object.
    if !unsafe { ffi::obj_parent(screen) }.is_null() {
        return Err(Error::InvalidArg);
    }
    if is_layer(screen) {
        return Err(Error::Internal);
    }
    // SAFETY: exclusive access, checks passed.
    unsafe {
        ffi::screen_load(
            screen,
            anim.kind(),
            anim.time_ms(),
            anim.delay_ms(),
            anim.auto_delete(),
        )
    };
    Ok(())
}
