//! Focus groups: which objects keypads and encoders move between.

use crate::{
    Error, Handle, Indev, Lvgl, Obj, ObjMut, Owned, Widget,
    base::{
        gen_support::{getter_view, resolve_obj},
        handle::private::Sealed,
        registry::{Kind, meta::*, reg},
    },
    sys,
};
use core::{cell::UnsafeCell, ptr};

/// An LVGL group, as the type of [`Owned<Group>`](Owned) and
/// [`Handle<Group>`](Handle). Created by [`Lvgl::create_group`].
pub struct Group {
    _g: UnsafeCell<[u8; 0]>,
}

impl Sealed for Group {}
impl crate::Tracked for Group {}

impl Lvgl {
    /// Creates a group. Takes a registry slot; dropping the [`Owned`]
    /// deletes the group at the next drain (its objects and input devices
    /// are detached).
    pub fn create_group(&self) -> Result<Owned<Group>, Error> {
        // SAFETY: shared access; creating a group frees nothing.
        let g = unsafe { sys::lv_group_create() };
        if g.is_null() {
            return Err(Error::OutOfMemory);
        }
        let reg = reg();
        match reg.mint(g.cast(), Kind::Group) {
            Ok(minted) => {
                reg.slot(minted.index).set(OWNED);
                // SAFETY: a fresh slot tracking this group, owned by the
                // returned value only.
                Ok(unsafe { Owned::from_raw(minted.raw) })
            }
            Err(error) => {
                // SAFETY: not handed out yet: no object or device uses it.
                unsafe { sys::lv_group_delete(g) };
                Err(error)
            }
        }
    }
}

fn group(handle: Handle<Group>) -> Result<*mut sys::lv_group_t, Error> {
    reg()
        .resolve(handle.raw(), Kind::Group, ptr::null())
        .map(|p| p.as_ptr().cast())
}

impl Handle<Group> {
    /// Adds `obj` to the group. Exclusive access: focus may move, sending
    /// FOCUSED and DEFOCUSED while LVGL walks the group.
    pub fn add_obj(self, lv: &mut Lvgl, obj: Handle<Obj>) -> Result<(), Error> {
        let _ = lv;
        let (g, o) = (group(self)?, resolve_obj(obj)?);
        // SAFETY: exclusive access; a live group and object.
        unsafe { sys::lv_group_add_obj(g, o) };
        Ok(())
    }

    /// Removes every object from the group.
    pub fn remove_all(self, lv: &mut Lvgl) -> Result<(), Error> {
        let _ = lv;
        // SAFETY: as above.
        unsafe { sys::lv_group_remove_all_objs(group(self)?) };
        Ok(())
    }

    /// Focuses the next object.
    pub fn focus_next(self, lv: &mut Lvgl) -> Result<(), Error> {
        let _ = lv;
        // SAFETY: as above.
        unsafe { sys::lv_group_focus_next(group(self)?) };
        Ok(())
    }

    /// Focuses the previous object.
    pub fn focus_prev(self, lv: &mut Lvgl) -> Result<(), Error> {
        let _ = lv;
        // SAFETY: as above.
        unsafe { sys::lv_group_focus_prev(group(self)?) };
        Ok(())
    }

    /// Switches between navigating (encoder steps move focus) and editing
    /// the focused object.
    pub fn set_editing(self, lv: &mut Lvgl, editing: bool) -> Result<(), Error> {
        let _ = lv;
        // SAFETY: as above.
        unsafe { sys::lv_group_set_editing(group(self)?, editing) };
        Ok(())
    }

    /// Whether focus wraps around at the ends.
    pub fn set_wrap(self, lv: &Lvgl, wrap: bool) -> Result<(), Error> {
        let _ = lv;
        // SAFETY: shared access; a flag.
        unsafe { sys::lv_group_set_wrap(group(self)?, wrap) };
        Ok(())
    }

    /// Makes this the default group: new focusable widgets join it.
    /// Deleting the group clears it (`lv_group.c:103`).
    pub fn set_default(self, lv: &Lvgl) -> Result<(), Error> {
        let _ = lv;
        // SAFETY: shared access; LVGL forgets the group when it is deleted.
        unsafe { sys::lv_group_set_default(group(self)?) };
        Ok(())
    }

    /// The focused object.
    pub fn focused(self, lv: &Lvgl) -> Result<Option<&Obj>, Error> {
        let _ = lv;
        // SAFETY: shared access; a live group.
        Ok(getter_view(unsafe {
            sys::lv_group_get_focused(group(self)?)
        }))
    }

    /// The number of objects in the group.
    pub fn obj_count(self, lv: &Lvgl) -> Result<u32, Error> {
        let _ = lv;
        // SAFETY: as above.
        Ok(unsafe { sys::lv_group_get_obj_count(group(self)?) })
    }
}

impl Indev {
    /// Sends this device's keys or encoder steps to `group` (`None`: no
    /// group). LVGL detaches the group when it is deleted.
    pub fn set_group(
        &mut self,
        lv: &mut Lvgl,
        group_handle: Option<Handle<Group>>,
    ) -> Result<(), Error> {
        let _ = lv;
        let g = match group_handle {
            Some(h) => group(h)?,
            None => ptr::null_mut(),
        };
        // SAFETY: exclusive access; a live device and group (or none).
        unsafe { sys::lv_indev_set_group(self.as_ptr(), g) };
        Ok(())
    }
}

impl<W: Widget> ObjMut<'_, W> {
    /// Removes the object from its group, if any. Exclusive access: the
    /// group's list node is freed while focus may move.
    pub fn remove_from_group(&mut self) {
        // SAFETY: exclusive access; a live object.
        unsafe { sys::lv_group_remove_obj(self.as_raw()) }
    }
}
