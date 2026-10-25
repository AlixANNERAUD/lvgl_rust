//! Subjects: values that objects bind to and observers follow (LVGL's
//! observer module). A subject is a `static` that initialises itself on
//! first use, so LVGL never sees one of the wrong type: it checks types
//! only with `LV_CHECK_ARG`, which configurations compile out.

use crate::{
    Error, Lvgl, Obj,
    base::gen_support::getter_view,
    event::{UserData, decode, encode},
    sys,
};
use core::{cell::UnsafeCell, marker::PhantomData, mem, ptr::NonNull};

/// What observer callbacks get: shared access and the observed object.
pub struct ObserverCtx<'cb> {
    observer: NonNull<sys::lv_observer_t>,
    _p: PhantomData<(&'cb Lvgl, *mut ())>,
}

impl<'cb> ObserverCtx<'cb> {
    /// Shared access, for the call's duration.
    pub fn lv(&self) -> &'cb Lvgl {
        // SAFETY: `Lvgl` is zero-sized; observers get shared access.
        unsafe { &*NonNull::<Lvgl>::dangling().as_ptr() }
    }

    /// The object of an observer made with `observe_obj` (`None` for
    /// `observe`, or while the object is being deleted).
    pub fn target(&self) -> Option<&'cb Obj> {
        // SAFETY: a live observer during its callback.
        getter_view(unsafe { sys::lv_observer_get_target_obj(self.observer.as_ptr()) })
    }
}

/// Calls `body` for an observer callback, under the callback depth limit.
fn observed(
    observer: *mut sys::lv_observer_t,
    body: &mut dyn FnMut(&ObserverCtx<'_>, *mut core::ffi::c_void),
) {
    let Some(observer) = NonNull::new(observer) else {
        return;
    };
    // SAFETY: LVGL notifies observers only inside calls made by the session
    // holder.
    let Some(_depth) = (unsafe { Lvgl::enter_callback() }) else {
        return;
    };
    // SAFETY: a live observer during its callback.
    let data = unsafe { sys::lv_observer_get_user_data(observer.as_ptr()) };
    body(
        &ObserverCtx {
            observer,
            _p: PhantomData,
        },
        data,
    );
}

/// Adds an observer calling `cb` with `data`, attached to `obj` (removed
/// with it) or permanent (`obj` null: the subject is `'static`).
///
/// LVGL calls `cb` once right away, and then on every change; it is a crate
/// trampoline running user code with shared access, so adding it needs no
/// more (the audit marks these functions exclusive for LVGL's own observer
/// callbacks, which this never passes).
fn add_observer(
    subject: *mut sys::lv_subject_t,
    obj: Option<&Obj>,
    cb: unsafe extern "C" fn(*mut sys::lv_observer_t, *mut sys::lv_subject_t),
    data: *mut core::ffi::c_void,
) -> Result<(), Error> {
    let observer = match obj {
        Some(obj) => {
            // Attaching registers a DELETE handler on the object: refused
            // while LVGL sends to it (see `Obj::on`).
            // SAFETY: a live object, under the token.
            if crate::base::callback_depth() != 0
                && unsafe { sys::lvgl_sys_obj_events_traversing(obj.as_ptr()) }
            {
                return Err(Error::Busy);
            }
            // SAFETY: a live object and an initialised subject, under the
            // token.
            unsafe { sys::lv_subject_add_observer_obj(subject, Some(cb), obj.as_ptr(), data) }
        }
        // SAFETY: an initialised `'static` subject, under the token.
        None => unsafe { sys::lv_subject_add_observer(subject, Some(cb), data) },
    };
    if observer.is_null() {
        Err(Error::OutOfMemory)
    } else {
        Ok(())
    }
}

macro_rules! subject {
    (
        $(#[$doc:meta])*
        $name:ident, $value:ty, $type_const:ident, $init:ident, $get:ident, $previous:ident, $set:ident
    ) => {
        $(#[$doc])*
        pub struct $name {
            s: UnsafeCell<sys::lv_subject_t>,
            initial: $value,
        }

        // SAFETY: only LVGL calls read or write the subject, under the
        // token; setting it needs exclusive access.
        unsafe impl Sync for $name {}

        impl $name {
            /// A subject with `initial` as its value (set on first use).
            pub const fn new(initial: $value) -> Self {
                // SAFETY: all zeroes: an uninitialised subject (type 0,
                // which no LVGL type uses), initialised before any use.
                Self { s: UnsafeCell::new(unsafe { mem::zeroed() }), initial }
            }

            /// The subject, initialised. Token held (every caller has
            /// shared access at least).
            pub(crate) fn as_ptr(&'static self) -> *mut sys::lv_subject_t {
                let s = self.s.get();
                // SAFETY: under the token; zero means not initialised yet.
                unsafe {
                    if (*s).type_() == 0 {
                        sys::$init(s, self.initial);
                    }
                    debug_assert_eq!((*s).type_(), sys::$type_const as u32);
                }
                s
            }

            /// The current value.
            pub fn get(&'static self, lv: &Lvgl) -> $value {
                let _ = lv;
                // SAFETY: an initialised subject of this type.
                unsafe { sys::$get(self.as_ptr()) }
            }

            /// The value before the last change.
            pub fn previous(&'static self, lv: &Lvgl) -> $value {
                let _ = lv;
                // SAFETY: as above.
                unsafe { sys::$previous(self.as_ptr()) }
            }

            /// Sets the value and notifies the observers and bound objects.
            /// Exclusive access: bound objects react (hiding an object can
            /// move focus, which can reparent a dropdown list).
            pub fn set(&'static self, lv: &mut Lvgl, value: $value) {
                let _ = lv;
                // SAFETY: as above, under exclusive access.
                unsafe { sys::$set(self.as_ptr(), value) }
            }

            /// Calls `f(ctx, data, value)` now and on every change, forever.
            /// `f` is a function item or a closure without captures.
            pub fn observe<D, F>(&'static self, lv: &Lvgl, data: D, f: F) -> Result<(), Error>
            where
                D: UserData,
                F: Fn(&ObserverCtx<'_>, D, $value) + Send + 'static,
            {
                let _ = lv;
                const { assert!(mem::size_of::<F>() == 0, "observe takes a function item or a closure without captures") };
                mem::forget(f);
                add_observer(self.as_ptr(), None, Self::trampoline::<D, F>, encode(data))
            }

            /// Like [`observe`](Self::observe), until `obj` is deleted.
            pub fn observe_obj<D, F>(&'static self, obj: &Obj, data: D, f: F) -> Result<(), Error>
            where
                D: UserData,
                F: Fn(&ObserverCtx<'_>, D, $value) + Send + 'static,
            {
                const { assert!(mem::size_of::<F>() == 0, "observe_obj takes a function item or a closure without captures") };
                mem::forget(f);
                add_observer(self.as_ptr(), Some(obj), Self::trampoline::<D, F>, encode(data))
            }

            unsafe extern "C" fn trampoline<D, F>(observer: *mut sys::lv_observer_t, subject: *mut sys::lv_subject_t)
            where
                D: UserData,
                F: Fn(&ObserverCtx<'_>, D, $value) + Send + 'static,
            {
                // SAFETY: `F` is zero-sized (checked by `observe*`).
                let f = unsafe { NonNull::<F>::dangling().as_ref() };
                // SAFETY: a live subject of this type, notifying.
                let value = unsafe { sys::$get(subject) };
                // SAFETY: stored by `observe*` with `encode::<D>`.
                observed(observer, &mut |ctx, data| f(ctx, unsafe { decode::<D>(data) }, value));
            }
        }
    };
}

subject! {
    /// An integer subject (`LV_SUBJECT_TYPE_INT`), for flag, state and
    /// value bindings:
    ///
    /// ```no_run
    /// use lvgl_rust::IntSubject;
    ///
    /// static VOLUME: IntSubject = IntSubject::new(50);
    /// ```
    IntSubject, i32, lv_subject_type_t_LV_SUBJECT_TYPE_INT,
    lv_subject_init_int, lv_subject_get_int, lv_subject_get_previous_int, lv_subject_set_int
}

subject! {
    /// A color subject (`LV_SUBJECT_TYPE_COLOR`).
    ColorSubject, sys::lv_color_t, lv_subject_type_t_LV_SUBJECT_TYPE_COLOR,
    lv_subject_init_color, lv_subject_get_color, lv_subject_get_previous_color, lv_subject_set_color
}

#[cfg(lv_use_label)]
impl crate::ObjMut<'_, crate::widgets::Label> {
    /// Shows `subject` as a decimal number. (LVGL's other formats are raw:
    /// nothing ties a format string to the subject's type.)
    ///
    /// `lv_label_bind_text` with a NULL format.
    pub fn bind_text(&mut self, subject: &'static IntSubject) -> Result<(), Error> {
        // SAFETY: exclusive access; an initialised int subject, for which
        // LVGL uses "%d" (`lv_label.c:692-695`).
        let observer =
            unsafe { sys::lv_label_bind_text(self.as_raw(), subject.as_ptr(), core::ptr::null()) };
        if observer.is_null() {
            Err(Error::OutOfMemory)
        } else {
            Ok(())
        }
    }
}
