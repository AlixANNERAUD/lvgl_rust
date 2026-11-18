//! Structural checks: operations LVGL accepts but that would leave dangling
//! pointers inside LVGL, refused before they reach it.
//!
//! - Display layers cannot be deleted, owned or reparented:
//!   `lv_display_delete` deletes them again through stored pointers.
//! - Widgets keep pointers to some of their children (a textarea's label, a
//!   menu's pages) and find others by index (a tabview's pages). The part
//!   tables of `classes.toml` (generated into `COMPOSITES`) list them; parts
//!   are found at call time, so lazily created ones count. A part, or an
//!   ancestor of one, cannot be deleted, owned or moved; a widget with parts
//!   cannot be cleaned; nothing can be moved into an indexed widget. A
//!   closed widget (a tile view, which casts every child to a tile) gets
//!   children only from its own API, and a pinned one (a calendar header,
//!   whose handlers read its parent as the calendar) never changes parent.
//!   External class descriptors are protected by default. An embedding
//!   host can allow structural edits to their children. Refusals are
//!   `Internal`.
//! - A screen being loaded, or the previous one during an animated load,
//!   cannot be deleted yet (`Busy`): LVGL keeps pointers to both. The
//!   previous screen cannot be loaded either: with `auto_delete`, LVGL
//!   deletes it first and then loads the freed object.
//! - Loading a screen can delete screens too: the previous one when a
//!   pending load finishes first (whatever the new load asks for), and
//!   the old one when `auto_delete` is set. The load is refused when one
//!   of the screens involved holds host-owned objects: `Internal` with
//!   `auto_delete`, `Busy` while another load is pending (retry when it
//!   completes). LVGL deletes after the animation, not when the load is
//!   requested: the host must not put host-owned objects in a screen an
//!   animated `auto_delete` load is about to delete (see
//!   `documentation/explanation/object-lifetime.md`).
//! - `set_parent` refuses cycles (LVGL has no guard) and screens (LVGL
//!   ignores the call).
//! - Only parentless objects can be loaded as screens.

use super::ffi::{self, lv_obj_t};
use crate::generated::MAX_PARTS;
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

/// Whether LVGL holds `o` as the previous screen of an animated load.
fn is_previous_screen(o: *mut lv_obj_t) -> bool {
    any_display(|d| unsafe { ffi::display_loading_screens(d) }[1] == o)
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

/// A composite widget class and how to find its parts (generated).
pub(crate) struct Composite {
    pub(crate) class: *const ffi::lv_obj_class_t,
    /// How deep below the widget its parts can be.
    pub(crate) max_depth: u8,
    /// LVGL finds or casts the widget's children by position: children
    /// come only from its own API, and none is moved in or reordered.
    pub(crate) closed: bool,
    /// The widget's handlers read its parent as the widget that created
    /// it: it never moves to another parent.
    pub(crate) pinned: bool,
    /// Per part: how many levels below it LVGL finds objects by index (0:
    /// none).
    pub(crate) indexed: &'static [u8],
    pub(crate) parts: fn(*mut lv_obj_t, &mut [*mut lv_obj_t; MAX_PARTS]) -> usize,
}

// SAFETY: the class pointer is the address of an immutable LVGL class
// descriptor, only passed back to LVGL.
unsafe impl Sync for Composite {}

/// A widget class without parts (generated).
pub(crate) struct ClassRef(pub(crate) *const ffi::lv_obj_class_t);

// SAFETY: as for `Composite`.
unsafe impl Sync for ClassRef {}

/// Structural permissions for an LVGL class defined outside the bindings.
///
/// When `external-class-policy` is enabled, the embedding binary must export
/// one `#[unsafe(no_mangle)]` Rust function named
/// `lvgl_rust_external_class_policy` with this signature:
///
/// ```rust,ignore
/// fn lvgl_rust_external_class_policy(
///     class: *const lvgl_rust::sys::lv_obj_class_t,
/// ) -> lvgl_rust::ExternalClassPolicy
/// ```
///
/// The hook runs under the LVGL token. Compare exact descriptor addresses,
/// return a consistent policy, and do not call into LVGL or run callbacks.
/// Return `ChildrenOnly` only when the class leaves its children under the
/// application's control. All other custom classes must be `Protected`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalClassPolicy {
    /// The object and its children are managed by the host.
    Protected,
    /// Safe code may edit the children; the host manages the object itself.
    ChildrenOnly,
}

// The host supplies a single definition when it enables the feature. The
// direct call requires no registration slot or per-object storage.
#[cfg(feature = "external-class-policy")]
unsafe extern "Rust" {
    fn lvgl_rust_external_class_policy(class: *const ffi::lv_obj_class_t) -> ExternalClassPolicy;
}

fn is_builtin(class: *const ffi::lv_obj_class_t) -> bool {
    class == ffi::obj_class()
        || ffi::non_composites().iter().any(|c| c.0 == class)
        || ffi::composites().iter().any(|c| c.class == class)
}

fn external_policy(class: *const ffi::lv_obj_class_t) -> ExternalClassPolicy {
    #[cfg(feature = "external-class-policy")]
    // SAFETY: the embedding host provides the documented hook contract.
    let policy = unsafe { lvgl_rust_external_class_policy(class) };
    #[cfg(not(feature = "external-class-policy"))]
    let policy = {
        let _ = class;
        ExternalClassPolicy::Protected
    };
    policy
}

fn is_external(o: *mut lv_obj_t) -> bool {
    // SAFETY: token held, `o` is a live object.
    !is_builtin(unsafe { ffi::obj_class_of(o) })
}

fn contains_external(o: *mut lv_obj_t) -> bool {
    is_external(o) || children_contain_external(o)
}

fn children_contain_external(o: *mut lv_obj_t) -> bool {
    for index in 0.. {
        // SAFETY: token held, `o` is live. LVGL returns null after the last child.
        let child = unsafe { ffi::obj_child(o, index) };
        if child.is_null() {
            break;
        }
        if contains_external(child) {
            return true;
        }
    }
    false
}

fn parent(o: *mut lv_obj_t) -> *mut lv_obj_t {
    // SAFETY: token held, `o` is a live object; its ancestors are live.
    unsafe { ffi::obj_parent(o) }
}

/// The composite entries matching `o`'s class (base classes included).
fn composites(o: *mut lv_obj_t) -> impl Iterator<Item = &'static Composite> {
    // SAFETY: token held, `o` is a live object.
    ffi::composites()
        .iter()
        .filter(move |c| unsafe { ffi::obj_has_class(o, c.class) })
}

/// The parts of `widget` for composite `c`.
fn parts_of(widget: *mut lv_obj_t, c: &Composite) -> ([*mut lv_obj_t; MAX_PARTS], usize) {
    let mut parts = [ptr::null_mut(); MAX_PARTS];
    let count = (c.parts)(widget, &mut parts).min(MAX_PARTS);
    (parts, count)
}

/// Levels from `o` up to `ancestor` (0 when equal), if it is one.
fn distance(mut o: *mut lv_obj_t, ancestor: *mut lv_obj_t) -> Option<u32> {
    let mut levels = 0;
    while !o.is_null() {
        if o == ancestor {
            return Some(levels);
        }
        o = parent(o);
        levels += 1;
    }
    None
}

/// Calls `f` with every part of every composite widget above `o` (and its
/// index depth), stopping when `f` returns true.
/// Not generic, so the walk is compiled once.
#[inline(never)]
fn any_part_above(o: *mut lv_obj_t, f: &mut dyn FnMut(*mut lv_obj_t, u8) -> bool) -> bool {
    let mut widget = parent(o);
    let mut levels: u32 = 0;
    while !widget.is_null() {
        levels += 1;
        for c in composites(widget) {
            // Parts sit at most `max_depth` below the widget, and LVGL
            // indexes at most `indexed` levels below a part.
            let reach = c.max_depth as u32 + c.indexed.iter().copied().max().unwrap_or(0) as u32;
            if levels > reach {
                continue;
            }
            let (parts, count) = parts_of(widget, c);
            for (i, &part) in parts[..count].iter().enumerate() {
                let depth = c.indexed.get(i).copied().unwrap_or(0);
                if !part.is_null() && f(part, depth) {
                    return true;
                }
            }
        }
        widget = parent(widget);
    }
    false
}

/// A plain object, a widget the part tables describe, or one of their
/// parts (a message box's content has its own class).
fn is_container(o: *mut lv_obj_t) -> bool {
    // SAFETY: token held, `o` is a live object.
    let class = unsafe { ffi::obj_class_of(o) };
    if class == ffi::obj_class() {
        return true;
    }
    if !is_builtin(class) {
        return external_policy(class) == ExternalClassPolicy::ChildrenOnly;
    }
    let leaf = || {
        ffi::leaves()
            .iter()
            .any(|c| unsafe { ffi::obj_has_class(o, c.0) })
    };
    leaf() || composites(o).next().is_some() || any_part_above(o, &mut |part, _| part == o)
}

/// Whether every object from `o` up is a container.
fn known_from(mut o: *mut lv_obj_t) -> bool {
    while !o.is_null() {
        if !is_container(o) {
            return false;
        }
        o = parent(o);
    }
    true
}

/// How far `o` is below a part whose descendants LVGL indexes, and how many
/// levels it indexes: `(distance, depth)`.
fn indexed_reach(o: *mut lv_obj_t) -> Option<(u32, u8)> {
    let mut found = None;
    any_part_above(o, &mut |part, depth| {
        if depth == 0 {
            return false;
        }
        found = distance(o, part).map(|d| (d, depth));
        found.is_some()
    });
    found
}

/// Whether deleting, owning or moving `x` would break a widget: `x` is a
/// part or contains one, LVGL finds `x` by index, or a widget above `x` is
/// not in the tables, or the subtree contains a host-owned object.
fn is_protected(x: *mut lv_obj_t) -> bool {
    contains_external(x)
        || !known_from(parent(x))
        || any_part_above(x, &mut |part, _| distance(part, x).is_some())
        || indexed_reach(x).is_some_and(|(d, depth)| (1..=depth as u32).contains(&d))
}

/// Whether `p` can receive a new child without shifting an index LVGL uses:
/// not a closed or indexed widget, and not a part (or within the indexed
/// levels of a part) whose children LVGL counts.
fn accepts_children(p: *mut lv_obj_t) -> bool {
    known_from(p)
        && !composites(p).any(|c| c.closed || c.indexed.iter().any(|&d| d > 0))
        && !indexed_reach(p).is_some_and(|(d, depth)| d < depth as u32)
}

pub(crate) fn check_delete(o: *mut lv_obj_t) -> Result<(), Error> {
    if is_layer(o) || is_protected(o) {
        return Err(Error::Internal);
    }
    if is_loading(o) {
        return Err(Error::Busy);
    }
    Ok(())
}

pub(crate) fn check_clean(o: *mut lv_obj_t) -> Result<(), Error> {
    // Its own parts would go; so would a part of a widget above it, the
    // children of an indexed part, or an unknown widget's internals. A
    // plain part's children (a message box's content) are the application's.
    let has_parts = composites(o).any(|c| parts_of(o, c).1 > 0);
    let contains_part = any_part_above(o, &mut |part, _| part != o && distance(part, o).is_some());
    let counted = indexed_reach(o).is_some_and(|(d, depth)| d < depth as u32);
    if !known_from(o) || has_parts || contains_part || counted || children_contain_external(o) {
        return Err(Error::Internal);
    }
    Ok(())
}

#[cfg(not(lvgl_rust_views_only))]
pub(crate) fn check_own(o: *mut lv_obj_t) -> Result<(), Error> {
    if is_layer(o) || is_protected(o) {
        return Err(Error::Internal);
    }
    Ok(())
}

pub(crate) fn check_set_parent(o: *mut lv_obj_t, parent: *mut lv_obj_t) -> Result<(), Error> {
    if is_layer(o) || is_protected(o) {
        return Err(Error::Internal);
    }
    if self::parent(o).is_null() {
        return Err(Error::InvalidArg); // a screen: LVGL would ignore the call
    }
    let pinned = composites(o).any(|c| c.pinned);
    if pinned || !accepts_children(parent) {
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

/// Creating a child of `parent` appends it to `parent`'s children, which a
/// widget can mistake for one of its own (a tileview casts every child to
/// a tile) or count as one of its indexed children.
pub(crate) fn check_create(parent: *mut lv_obj_t) -> Result<(), Error> {
    if accepts_children(parent) {
        Ok(())
    } else {
        Err(Error::Internal)
    }
}

/// Moving `o` among its siblings shifts their indexes: the same conditions
/// as moving it out of its parent and back in.
pub(crate) fn move_to_index(o: *mut lv_obj_t, index: i32) -> Result<(), Error> {
    if is_layer(o) || is_protected(o) {
        return Err(Error::Internal);
    }
    let p = parent(o);
    if p.is_null() {
        return Err(Error::InvalidArg); // a screen: LVGL would ignore the call
    }
    if !accepts_children(p) {
        return Err(Error::Internal);
    }
    // SAFETY: exclusive access, checks passed.
    unsafe { ffi::obj_move_to_index(o, index) };
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
    if is_previous_screen(screen) {
        return Err(Error::Busy);
    }
    // SAFETY: the token is held and the screen is live.
    let candidates = unsafe { ffi::screen_deletion_candidates(screen) };
    // A new load deletes the old screen when it asks for it. While another
    // load is pending it also finishes that one first (the new flag is not
    // consulted), which deletes the previous screen if the pending load
    // asked for it, and the pending load's own flag is not public: any
    // screen involved may go. A screen with host-owned objects must not.
    let pending = !candidates[1].is_null();
    if (anim.auto_delete() || pending)
        && candidates
            .into_iter()
            .any(|o| !o.is_null() && o != screen && contains_external(o))
    {
        // Permanent with `auto_delete`; otherwise only until the pending
        // load has completed.
        return Err(if anim.auto_delete() {
            Error::Internal
        } else {
            Error::Busy
        });
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
