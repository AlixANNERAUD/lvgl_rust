//! Hand-written wrappers where LVGL needs more than a type can say: state
//! that another call must update, or values that would divide by zero.

#[cfg(lv_use_dropdown)]
mod dropdown {
    use crate::{sys, widgets::Dropdown};
    use core::ffi::CStr;

    impl Dropdown {
        /// Sets the options (`"a\nb\nc"`), which LVGL copies.
        ///
        /// `lv_dropdown_set_options`.
        pub fn set_options(&self, options: &CStr) {
            // SAFETY: shared access, live dropdown; LVGL copies the text.
            unsafe { sys::lv_dropdown_set_options(self.as_ptr(), options.as_ptr()) };
            self.repoint_list_label();
        }

        /// Sets options that LVGL keeps without copying.
        ///
        /// `lv_dropdown_set_options_static`.
        pub fn set_options_static(&self, options: &'static CStr) {
            // SAFETY: as above; the text is `'static`.
            unsafe { sys::lv_dropdown_set_options_static(self.as_ptr(), options.as_ptr()) };
            self.repoint_list_label();
        }

        /// Removes every option.
        ///
        /// `lv_dropdown_clear_options`.
        pub fn clear_options(&self) {
            // SAFETY: as above.
            unsafe { sys::lv_dropdown_clear_options(self.as_ptr()) };
            self.repoint_list_label();
        }

        /// Opening the list makes its label show `options` as static text
        /// (`lv_dropdown.c:610`), and the option setters free the old
        /// options without updating it (`:224-412`): point the label at
        /// the current options.
        fn repoint_list_label(&self) {
            // SAFETY: a live dropdown under the token; its list's first
            // child is the label (`lv_dropdown.c:818`), and the options
            // (or a literal) outlive it until the next setter.
            unsafe {
                let list = sys::lv_dropdown_get_list(self.as_ptr());
                if list.is_null() {
                    return;
                }
                let label = sys::lv_obj_get_child(list, 0);
                if !label.is_null() {
                    sys::lv_label_set_text_static(
                        label,
                        sys::lv_dropdown_get_options(self.as_ptr()),
                    );
                }
            }
        }
    }
}

#[cfg(lv_use_label)]
mod label {
    use crate::{Error, sys, widgets::Label};
    use core::ffi::CStr;

    impl Label {
        /// Shows `text` without copying it (`None`: refreshes the current
        /// text). Refused with [`Error::Internal`] on the label of a
        /// textarea, which edits its text in place (`lv_textarea.c:257`).
        ///
        /// `lv_label_set_text_static`.
        pub fn set_text_static(&self, text: Option<&'static CStr>) -> Result<(), Error> {
            #[cfg(lv_use_textarea)]
            if self.is_textarea_label() {
                return Err(Error::Internal);
            }
            // SAFETY: shared access, live label; the text is `'static`.
            unsafe {
                sys::lv_label_set_text_static(
                    self.as_ptr(),
                    text.map_or(core::ptr::null(), CStr::as_ptr),
                )
            };
            Ok(())
        }

        #[cfg(lv_use_textarea)]
        fn is_textarea_label(&self) -> bool {
            // SAFETY: a live label under the token; the parent is checked
            // to be a textarea before its label is read.
            unsafe {
                let parent = sys::lv_obj_get_parent(self.as_ptr());
                !parent.is_null()
                    && sys::lv_obj_has_class(parent, &raw const sys::lv_textarea_class)
                    && sys::lv_textarea_get_label(parent) == self.as_ptr()
            }
        }
    }
}

#[cfg(lv_use_scale)]
mod scale {
    use crate::{Error, sys, widgets::Scale};

    impl Scale {
        /// Sets the range. `min == max` is refused ([`Error::InvalidArg`]):
        /// needles divide by `max - min` (`lv_scale.c:329, 384`).
        ///
        /// `lv_scale_set_range`.
        pub fn set_range(&self, min: i32, max: i32) -> Result<(), Error> {
            if min == max {
                return Err(Error::InvalidArg);
            }
            // SAFETY: shared access, live scale.
            unsafe { sys::lv_scale_set_range(self.as_ptr(), min, max) };
            Ok(())
        }

        /// Sets the minimum, refused if it equals the maximum.
        ///
        /// `lv_scale_set_min_value`.
        pub fn set_min_value(&self, min: i32) -> Result<(), Error> {
            // SAFETY: as above.
            if min == unsafe { sys::lv_scale_get_range_max_value(self.as_ptr()) } {
                return Err(Error::InvalidArg);
            }
            // SAFETY: as above.
            unsafe { sys::lv_scale_set_min_value(self.as_ptr(), min) };
            Ok(())
        }

        /// Sets the maximum, refused if it equals the minimum.
        ///
        /// `lv_scale_set_max_value`.
        pub fn set_max_value(&self, max: i32) -> Result<(), Error> {
            // SAFETY: as above.
            if max == unsafe { sys::lv_scale_get_range_min_value(self.as_ptr()) } {
                return Err(Error::InvalidArg);
            }
            // SAFETY: as above.
            unsafe { sys::lv_scale_set_max_value(self.as_ptr(), max) };
            Ok(())
        }
    }
}

/// Relations (`relations.toml`): an object that keeps another one gets it
/// cleared when that one is deleted.
#[cfg(all(lv_use_keyboard, lv_use_textarea, not(lvgl_rust_views_only)))]
mod keyboard {
    use crate::{
        Error, Handle, ObjMut, WidgetExt,
        base::gen_support::resolve_obj,
        event::{decode, encode},
        sys,
        widgets::{Keyboard, TextArea},
    };
    use core::ptr;

    impl ObjMut<'_, Keyboard> {
        /// Types into `textarea` (`None`: no textarea). LVGL keeps it; the
        /// keyboard forgets it when the textarea is deleted. Takes a
        /// registry slot for the keyboard.
        ///
        /// `lv_keyboard_set_textarea`.
        pub fn set_textarea(&mut self, textarea: Option<Handle<TextArea>>) -> Result<(), Error> {
            let keyboard = self.view().handle()?;
            let ta = match textarea {
                Some(h) => resolve_obj(h.upcast())?,
                None => ptr::null_mut(),
            };
            if !ta.is_null() {
                watch(ta, keyboard)?;
            }
            // SAFETY: exclusive access; a live textarea or NULL.
            unsafe { sys::lv_keyboard_set_textarea(self.as_raw(), ta) };
            Ok(())
        }
    }

    /// Installs, once per keyboard, a DELETE handler on `ta` that clears the
    /// keyboard's pointer.
    fn watch(ta: *mut sys::lv_obj_t, keyboard: Handle<Keyboard>) -> Result<(), Error> {
        let data = encode(keyboard);
        // SAFETY: a live object under exclusive access; descriptors are only
        // compared.
        unsafe {
            for i in 0..sys::lv_obj_get_event_count(ta) {
                let dsc = sys::lv_obj_get_event_dsc(ta, i);
                let same_cb = sys::lv_event_dsc_get_cb(dsc)
                    .is_some_and(|cb| ptr::fn_addr_eq(cb, forget as unsafe extern "C" fn(_)));
                if same_cb && sys::lv_event_dsc_get_user_data(dsc) == data {
                    return Ok(());
                }
            }
            let dsc = sys::lv_obj_add_event_cb(
                ta,
                Some(forget),
                sys::lv_event_code_t_LV_EVENT_DELETE,
                data,
            );
            if dsc.is_null() {
                Err(Error::OutOfMemory)
            } else {
                Ok(())
            }
        }
    }

    /// The textarea is being deleted: the keyboard (if alive) forgets it if
    /// it still types into it. Runs no user code.
    unsafe extern "C" fn forget(e: *mut sys::lv_event_t) {
        // SAFETY: a DELETE event of the textarea, with a keyboard handle
        // stored by `watch`.
        unsafe {
            let keyboard = decode::<Handle<Keyboard>>(sys::lv_event_get_user_data(e));
            let Ok(kb) = resolve_obj(keyboard.upcast()) else {
                return;
            };
            let ta = sys::lv_event_get_target_obj(e);
            if sys::lv_keyboard_get_textarea(kb) == ta {
                sys::lvgl_sys_keyboard_forget_textarea(kb);
            }
        }
    }
}
