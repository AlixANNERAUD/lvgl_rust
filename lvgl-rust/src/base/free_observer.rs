//! The free observer (`free-observer`): one function the application
//! installs, called for every tracked object LVGL frees, from the OBJ_ID
//! free hook the registry already has. A host keeping data per handle (a
//! table of the handles it gave a client, events queued for an object)
//! drops it there, without an event callback per object, whose descriptor
//! LVGL would allocate.

use super::{ffi, registry::OBSERVER};
use crate::{Error, Handle, Lvgl, Obj};

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

    /// The object, at the end of its destructor (its children are gone and
    /// its memory is freed right after the observer returns): a key, never
    /// to be dereferenced or given to LVGL.
    pub fn object(&self) -> *mut ffi::lv_obj_t {
        self.object
    }
}

impl Lvgl {
    /// Installs `observer`, called for every tracked object (one with a
    /// handle) LVGL frees, whatever the path (an explicit deletion, its
    /// parent's, `clean`, a screen unloaded with auto-deletion, `deinit`):
    /// through LVGL's OBJ_ID free hook, with no per-object event callback.
    ///
    /// The registry keeps one observer. Fails with [`Error::Busy`] if one
    /// is installed already, so two components cannot replace each other's
    /// silently: remove it first with [`take_free_observer`](Lvgl::take_free_observer).
    ///
    /// The observer runs inside LVGL's destructor: it must not call LVGL
    /// or this crate (it gets no access to either, and a session cannot be
    /// entered from it), only update its own state (drop the side data of
    /// the handle, queue a notification).
    pub fn set_free_observer(&mut self, observer: fn(Freed)) -> Result<(), Error> {
        // Exclusive access: no other writer, and the free hook runs only
        // inside calls made with this access, so a test then a store is
        // the compare-and-exchange of `None` with `observer`.
        if OBSERVER.0.get().is_some() {
            return Err(Error::Busy);
        }
        OBSERVER.0.set(Some(observer));
        Ok(())
    }

    /// Removes the installed observer and returns it.
    pub fn take_free_observer(&mut self) -> Option<fn(Freed)> {
        OBSERVER.0.take()
    }
}
