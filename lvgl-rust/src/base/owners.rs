//! Owner tags (`owners`): handles minted for one owner (a client of a host,
//! a script, a sandboxed application) that other owners cannot use, and the
//! cleanup of everything an owner had when it goes away.
//!
//! The tag lives in the top `LVGL_RUST_OWNER_BITS` bits of the slot's
//! generation, so it costs no memory: a [`Handle`] names its owner, a slot is
//! still 8 B on 32-bit targets, and fewer bits count reuses (a stale handle
//! is reported dead until its slot has been reused `2^(16 - bits)` times).

use super::{
    ffi,
    registry::{Kind, OBSERVER, OWNER_MAX, owner_of, reg},
    structural,
};
use crate::{Error, Handle, Lvgl, Obj, Widget};
use core::{fmt, num::NonZeroU8};

/// Who a handle was minted for: 1 to [`Owner::MAX`]. 1 byte.
///
/// Handles minted by [`WidgetExt::handle`](crate::WidgetExt::handle) have
/// no owner; [`WidgetExt::handle_for`](crate::WidgetExt::handle_for) tags
/// them. An object has a single slot, so a single owner (or none) at a time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Owner(NonZeroU8);

impl Owner {
    /// The largest owner: `2^LVGL_RUST_OWNER_BITS - 1`.
    pub const MAX: u8 = OWNER_MAX;

    /// The owner `value`, if it is from 1 to [`Owner::MAX`].
    pub const fn new(value: u8) -> Option<Owner> {
        if value > Self::MAX {
            return None;
        }
        match NonZeroU8::new(value) {
            Some(value) => Some(Owner(value)),
            None => None,
        }
    }

    /// Its value, from 1 to [`Owner::MAX`].
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

impl fmt::Debug for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Owner").field(&self.0.get()).finish()
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Owner {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Owner({=u8})", self.0.get())
    }
}

impl<W> Handle<W> {
    /// The owner the handle was minted for, `None` for an untagged handle.
    pub fn owner(self) -> Option<Owner> {
        Owner::new(owner_of(self.raw()))
    }
}

impl<W: Widget> Handle<W> {
    /// [`Handle::from_raw`], only if the handle was minted for `owner`: what
    /// a host calls on a raw handle a client gives it. A check of the bits,
    /// without a registry lookup.
    pub fn from_raw_for(raw: u32, owner: Owner) -> Option<Handle<W>> {
        if owner_of(raw) == owner.get() {
            Handle::from_raw(raw)
        } else {
            None
        }
    }
}

/// A tracked object being freed, given to the observer installed with
/// [`Lvgl::set_free_observer`].
#[derive(Clone, Copy, Debug)]
pub struct Freed {
    raw: u32,
    object: *mut ffi::lv_obj_t,
}

impl Freed {
    pub(crate) fn new(raw: u32, object: *mut ffi::lv_obj_t) -> Self {
        Self { raw, object }
    }

    /// The object's handle, already dead for every lookup made after the
    /// observer returns. Compare it with stored handles, or use
    /// [`Handle::into_raw`] as a key.
    pub fn handle(&self) -> Handle<Obj> {
        // SAFETY: the raw handle of the slot tracking this object.
        unsafe { Handle::from_raw_unchecked(self.raw) }
    }

    /// The owner the object's handles were minted for.
    pub fn owner(&self) -> Option<Owner> {
        Owner::new(owner_of(self.raw))
    }

    /// The object, at the end of its destructor (its children are gone and
    /// its memory is freed right after the observer returns): a key, never
    /// to be dereferenced or given to LVGL.
    pub fn object(&self) -> *mut ffi::lv_obj_t {
        self.object
    }
}

impl Lvgl {
    /// Calls `observer` for every tracked object (one with a handle, any
    /// owner or none) LVGL frees, whatever the path (an explicit deletion,
    /// its parent's, a screen unloaded with auto-deletion, `deinit`):
    /// through LVGL's OBJ_ID free hook, with no per-object event callback.
    /// `None` removes it. The registry keeps one observer.
    ///
    /// The observer runs inside LVGL's destructor: it must not call LVGL
    /// or this crate (it gets no access to either), only update its own
    /// state (release the side data of the handle, queue a notification).
    pub fn set_free_observer(&mut self, observer: Option<fn(Freed)>) {
        OBSERVER.0.set(observer);
    }

    /// Deletes every object tracked for `owner` whose parent is not, so its
    /// descendants go with it, then [releases](Lvgl::release_owner) what
    /// is left: objects whose deletion the structural checks refuse (a
    /// display layer, a part of a widget of another owner, a screen being
    /// loaded) stay alive, untracked. Returns the objects deleted.
    ///
    /// A host calls it when a client goes away. Only objects the host
    /// minted handles for are tagged: mint tagged handles only for objects
    /// the client created or that live inside them, never for shared ones
    /// (a screen, a layer), or they are deleted here.
    pub fn delete_owner(&mut self, owner: Owner) -> usize {
        let reg = reg();
        let mut deleted = 0;
        // Deletions free slots (the free hook) but never take one: indices
        // above the high-water mark stay free during the scan.
        for index in 0..reg.high_water_mark() {
            let slot = reg.slot(index);
            if !slot.is_live(Kind::Obj) || slot.owner() != owner.get() {
                continue;
            }
            let o = slot.ptr().cast::<ffi::lv_obj_t>();
            // SAFETY: exclusive access; a LIVE object slot points to a live
            // object, and its parent is live.
            let parent = unsafe { ffi::obj_parent(o) };
            if !parent.is_null() && reg.owner_of_obj(parent) == owner.get() {
                continue; // goes with its parent
            }
            if structural::delete(o).is_ok() {
                deleted += 1;
            }
        }
        reg.release_owner(owner.get());
        deleted
    }

    /// Untracks everything tracked for `owner`, deleting nothing: the
    /// owner's handles are dead afterwards and its slots free, except
    /// those an [`Owned`](crate::Owned) or a pending deferred request
    /// (`delete_later`, `clean_later`) still uses, which lose the owner
    /// tag (an `Owned` among them still deletes its object when dropped,
    /// but its [`Owned::get`](crate::Owned::get) reports
    /// [`Error::Dead`]). Returns the slots released.
    pub fn release_owner(&mut self, owner: Owner) -> usize {
        reg().release_owner(owner.get())
    }

    /// Resolves a raw handle a client gave, only if it was minted for
    /// `owner`: [`Handle::from_raw_for`] then [`Handle::get`].
    pub fn resolve_for<W: Widget>(&self, raw: u32, owner: Owner) -> Result<&W, Error> {
        Handle::<W>::from_raw_for(raw, owner)
            .ok_or(Error::Dead)?
            .get(self)
    }
}
