//! Event handlers: Rust functions LVGL calls when an object receives an
//! event, with shared access to LVGL.
//!
//! Handlers run inside whatever LVGL call sent the event, so they get
//! shared access only ([`Event::lv`]): deletions they request are deferred
//! ([`Obj::delete_later`]) and exclusive work can be posted
//! ([`Lvgl::post`]). Codes sent while LVGL holds pointers a shared call
//! could free are never delivered (see [`handler_allowed`]).

use crate::{
    Error, Lvgl, Obj, ObjMut, Widget,
    base::gen_support::getter_view,
    enums::{EventCode, HANDLER_SHARED, SafeEventCode, SendableEventCode},
    sys,
};
use core::{ffi::c_void, marker::PhantomData, mem, ptr, ptr::NonNull};

#[cfg(not(lvgl_rust_views_only))]
use crate::Handle;

/// An event being delivered to a handler. Borrows shared access to LVGL
/// for the handler's duration.
pub struct Event<'cb> {
    e: NonNull<sys::lv_event_t>,
    _p: PhantomData<(&'cb Lvgl, *mut ())>,
}

impl<'cb> Event<'cb> {
    /// Shared access, for the handler's duration.
    pub fn lv(&self) -> &'cb Lvgl {
        // SAFETY: `Lvgl` is zero-sized; handlers get shared access only.
        unsafe { &*NonNull::<Lvgl>::dangling().as_ptr() }
    }

    fn raw(&self) -> *mut sys::lv_event_t {
        self.e.as_ptr()
    }

    /// The code (`lv_event_get_code`).
    pub fn code(&self) -> EventCode {
        // SAFETY: a live event, during its dispatch.
        EventCode(unsafe { sys::lv_event_get_code(self.raw()) } as u32)
    }

    /// The object the event was sent to (before bubbling), `None` while it
    /// is being deleted.
    pub fn target(&self) -> Option<&'cb Obj> {
        // SAFETY: as above; object events target objects.
        getter_view(unsafe { sys::lv_event_get_target_obj(self.raw()) })
    }

    /// The object whose handler runs (the target, or an ancestor when the
    /// event bubbles), `None` while it is being deleted.
    pub fn current_target(&self) -> Option<&'cb Obj> {
        // SAFETY: as above.
        getter_view(unsafe { sys::lv_event_get_current_target_obj(self.raw()) })
    }

    /// Whether the target is the object of `handle`. Mints nothing.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn target_is(&self, handle: Handle<Obj>) -> bool {
        match (self.target(), handle.get(self.lv())) {
            (Some(target), Ok(object)) => target.as_ptr() == object.as_ptr(),
            _ => false,
        }
    }

    /// The object a `CHILD_CHANGED` or `CHILD_CREATED` event is about, if
    /// still alive (`None` for any other code).
    pub fn param_obj(&self) -> Option<&'cb Obj> {
        if !matches!(
            self.code(),
            EventCode::CHILD_CHANGED | EventCode::CHILD_CREATED
        ) {
            return None;
        }
        // SAFETY: for these codes LVGL passes the child or NULL
        // (`lv_obj_class.c:131-132`, `lv_obj_tree.c:77, 112, 179-185`).
        getter_view(unsafe { sys::lv_event_get_param(self.raw()) }.cast())
    }

    /// The key of a `KEY` event (`None` for any other code).
    pub fn key(&self) -> Option<u32> {
        if self.code() != EventCode::KEY {
            return None;
        }
        // SAFETY: for KEY the param is a `uint32_t *` or NULL
        // (`lv_group.c:312`, `send_key`).
        let p = unsafe { sys::lv_event_get_param(self.raw()) }.cast::<u32>();
        // SAFETY: non-null: a live `u32` for the dispatch.
        (!p.is_null()).then(|| unsafe { p.read() })
    }

    /// The input device's point for pointer events (pressed, released,
    /// clicks, long presses, hover, gesture, leave); `None` for any other
    /// code or when the event was not sent by an input device.
    pub fn point(&self) -> Option<sys::lv_point_t> {
        use EventCode as C;
        let pointer = matches!(
            self.code(),
            C::PRESSED
                | C::PRESSING
                | C::PRESS_LOST
                | C::RELEASED
                | C::SHORT_CLICKED
                | C::SINGLE_CLICKED
                | C::DOUBLE_CLICKED
                | C::TRIPLE_CLICKED
                | C::CLICKED
                | C::LONG_PRESSED
                | C::LONG_PRESSED_REPEAT
                | C::HOVER_OVER
                | C::HOVER_LEAVE
                | C::GESTURE
                | C::LEAVE
        );
        if !pointer {
            return None;
        }
        // SAFETY: input code sends these with the input device as param
        // (`lv_indev.c`); safe code can only send them with NULL.
        let indev = unsafe { sys::lv_event_get_param(self.raw()) }.cast::<sys::lv_indev_t>();
        if indev.is_null() {
            return None;
        }
        let mut point = sys::lv_point_t { x: 0, y: 0 };
        // SAFETY: a live input device, during the dispatch.
        unsafe { sys::lv_indev_get_point(indev, &mut point) };
        Some(point)
    }

    /// Stops the event from bubbling to the parents.
    pub fn stop_bubbling(&self) {
        // SAFETY: a live event.
        unsafe { sys::lv_event_stop_bubbling(self.raw()) }
    }

    /// Stops the remaining handlers of this object (not for `DELETE`).
    pub fn stop_processing(&self) {
        if self.code() != EventCode::DELETE {
            // SAFETY: a live event.
            unsafe { sys::lv_event_stop_processing(self.raw()) }
        }
    }
}

/// Whether safe handlers may run for `code`.
///
/// The codes are those `events.toml` gives shared handlers: the access
/// audit found no frame that keeps memory a shared call could free across
/// their sends (input, value, state, scroll, child, screen, size, style
/// and layout codes, `DELETE`), plus codes registered with
/// `lv_event_register_id` (only applications send them). Refused: query
/// and draw codes, sent in the middle of layout or rendering (a queued
/// label draw task keeps `label->text`, `lv_label.c:936`); `CREATE`, sent
/// before the object is complete; display and input-device codes.
pub fn handler_allowed(code: EventCode) -> bool {
    use EventCode as C;
    let bits = code.bits();
    let custom = bits > C::LAST.bits() && bits <= C::LAST_CUSTOM.bits();
    let word = bits as usize / 64;
    custom || (word < HANDLER_SHARED.len() && (HANDLER_SHARED[word] >> (bits % 64)) & 1 != 0)
}

impl Obj {
    /// Sends `code` to this object with a NULL parameter
    /// (`lv_obj_send_event`). Fails with [`Error::Dead`] if LVGL reports the
    /// object deleted meanwhile.
    ///
    /// Only the audit makes codes (`events.toml`): there is no constructor.
    ///
    /// ```compile_fail
    /// let _ = lvgl_rust::enums::SafeEventCode(10);
    /// ```
    pub fn send_event(&self, code: SafeEventCode) -> Result<(), Error> {
        send(self.as_ptr(), code.0)
    }
}

impl<'s, W: Widget> ObjMut<'s, W> {
    /// Sends `code` to this object with a NULL parameter
    /// (`lv_obj_send_event`). Its handlers may delete the object (a message
    /// box's close button on `CLICKED`), so this consumes the access and
    /// gives it back only if the object is still alive; [`Error::Dead`]
    /// otherwise.
    pub fn send_event(self, code: SendableEventCode) -> Result<Self, Error> {
        send(self.as_raw(), code.0).map(|()| self)
    }
}

fn send(obj: *mut sys::lv_obj_t, code: u32) -> Result<(), Error> {
    // SAFETY: a live object under the token; the code reads no parameter
    // (`events.toml`), and deleting codes come with exclusive access.
    let res = unsafe { sys::lv_obj_send_event(obj, code as sys::lv_event_code_t, ptr::null_mut()) };
    if res == sys::lv_result_t_LV_RESULT_OK {
        Ok(())
    } else {
        Err(Error::Dead)
    }
}

/// What a handler returns: `()` or `Result<(), Error>` (errors are
/// dropped: LVGL has no way to report them).
pub trait HandlerResult: private::Sealed {}
impl HandlerResult for () {}
impl HandlerResult for Result<(), Error> {}

mod private {
    pub trait Sealed {}
    impl Sealed for () {}
    impl Sealed for Result<(), crate::Error> {}
}

/// Values stored by value in a callback's `user_data`: padding-free and at
/// most 4 bytes on every target, so host builds accept exactly what
/// embedded ones do.
///
/// # Safety
///
/// Implementors must be `Copy`, at most 4 bytes, without padding, and valid
/// for any bit pattern produced by writing a value of the type.
pub unsafe trait UserData: Copy + Send + 'static {}

macro_rules! user_data {
    ($($t:ty),*) => { $(
        // SAFETY: a primitive of at most 4 bytes without padding.
        unsafe impl UserData for $t {}
    )* };
}
user_data!(u8, u16, u32, i8, i16, i32, f32, bool, char, ());

#[cfg(not(lvgl_rust_views_only))]
// SAFETY: a `NonZeroU32` (4 bytes).
unsafe impl<W: 'static> UserData for Handle<W> {}
#[cfg(not(lvgl_rust_views_only))]
// SAFETY: 4 bytes through the niche of `NonZeroU32`.
unsafe impl<W: 'static> UserData for Option<Handle<W>> {}

/// `d` in a pointer-sized slot, starting from null (no uninitialized bytes).
pub(crate) fn encode<D: UserData>(d: D) -> *mut c_void {
    const { assert!(mem::size_of::<D>() <= 4) };
    let mut p = ptr::null_mut::<c_void>();
    // SAFETY: `D` fits in a pointer (at most 4 bytes).
    unsafe { ptr::write_unaligned((&raw mut p).cast::<D>(), d) };
    p
}

/// The `D` that [`encode`] stored in `p`.
///
/// # Safety
///
/// `p` must come from `encode::<D>`.
pub(crate) unsafe fn decode<D: UserData>(p: *mut c_void) -> D {
    // SAFETY: caller contract; read from a local copy.
    unsafe { ptr::read_unaligned((&raw const p).cast::<D>()) }
}

/// A context and its function in one `static`, for [`Obj::on_static`].
pub struct Handler<C: Sync + 'static> {
    pub ctx: C,
    pub f: fn(&Event<'_>, &'static C),
}

/// Runs `body` for the event `e` if its code is allowed and the callback
/// depth limit is not reached. Non-generic: the trampolines only add the
/// user call.
fn dispatch(e: *mut sys::lv_event_t, body: &mut dyn FnMut(&Event<'_>)) {
    let Some(e) = NonNull::new(e) else { return };
    let event = Event { e, _p: PhantomData };
    if !handler_allowed(event.code()) {
        return;
    }
    // SAFETY: LVGL runs handlers only inside calls made by the session
    // holder; the guard stays within this callback.
    let Some(_depth) = (unsafe { Lvgl::enter_callback() }) else {
        return;
    };
    body(&event);
}

unsafe extern "C" fn fn_trampoline(e: *mut sys::lv_event_t) {
    // SAFETY: a live event during its dispatch.
    let p = unsafe { sys::lv_event_get_user_data(e) };
    // SAFETY: stored by `on_fn` from a `fn(&Event)`.
    let f = unsafe { mem::transmute::<*mut c_void, fn(&Event<'_>)>(p) };
    dispatch(e, &mut |event| f(event));
}

unsafe extern "C" fn zst_trampoline<F, R>(e: *mut sys::lv_event_t)
where
    F: Fn(&Event<'_>) -> R + Send + 'static,
    R: HandlerResult,
{
    // SAFETY: `F` is zero-sized (checked by `on`): a dangling pointer is a
    // valid reference to it, and no `F` value is ever created or dropped.
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    dispatch(e, &mut |event| {
        let _ = f(event);
    });
}

unsafe extern "C" fn data_trampoline<D, F, R>(e: *mut sys::lv_event_t)
where
    D: UserData,
    F: Fn(&Event<'_>, D) -> R + Send + 'static,
    R: HandlerResult,
{
    // SAFETY: as above.
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    // SAFETY: stored by `on_with` with `encode::<D>`.
    let d = unsafe { decode::<D>(sys::lv_event_get_user_data(e)) };
    dispatch(e, &mut |event| {
        let _ = f(event, d);
    });
}

unsafe extern "C" fn static_trampoline<C: Sync + 'static>(e: *mut sys::lv_event_t) {
    // SAFETY: stored by `on_static` from a `&'static Handler<C>`.
    let h = unsafe { &*sys::lv_event_get_user_data(e).cast::<Handler<C>>() };
    dispatch(e, &mut |event| (h.f)(event, &h.ctx));
}

/// A removable handler, from [`Obj::on_removable`].
#[cfg(not(lvgl_rust_views_only))]
#[must_use = "dropping it keeps the handler; call `remove` to remove it"]
pub struct EventSub {
    object: Handle<Obj>,
    dsc: *const sys::lv_event_dsc_t,
    cb: unsafe extern "C" fn(*mut sys::lv_event_t),
}

// SAFETY: the descriptor pointer is only compared, never dereferenced.
#[cfg(not(lvgl_rust_views_only))]
unsafe impl Send for EventSub {}

#[cfg(not(lvgl_rust_views_only))]
impl EventSub {
    /// Removes the handler, unless its object was deleted (then it is gone
    /// already). Exclusive access: the object's handler list shrinks.
    pub fn remove(self, lv: &mut Lvgl) {
        let Ok(object) = self.object.get(lv) else {
            return;
        };
        let o = object.as_ptr();
        // SAFETY: a live object under exclusive access; the stored
        // descriptor is compared with the object's own, never dereferenced.
        unsafe {
            let count = sys::lv_obj_get_event_count(o);
            for i in 0..count {
                let dsc = sys::lv_obj_get_event_dsc(o, i);
                // The callback LVGL returns is the pointer value `self.cb`
                // was registered with: an exact round trip.
                let same_cb =
                    sys::lv_event_dsc_get_cb(dsc).is_some_and(|cb| ptr::fn_addr_eq(cb, self.cb));
                if ptr::eq(dsc, self.dsc) && same_cb {
                    sys::lv_obj_remove_event(o, i);
                    return;
                }
            }
        }
    }
}

#[cfg(not(lvgl_rust_views_only))]
unsafe extern "C" fn removable_trampoline<D, F, R>(e: *mut sys::lv_event_t)
where
    D: UserData,
    F: Fn(&Event<'_>, D) -> R + Send + 'static,
    R: HandlerResult,
{
    // A family of its own (never folded with `data_trampoline`): `remove`
    // matches on the address.
    core::hint::black_box(0x52u8);
    // SAFETY: as in `data_trampoline`.
    unsafe { data_trampoline::<D, F, R>(e) }
}

impl Obj {
    fn add_handler(
        &self,
        code: EventCode,
        cb: unsafe extern "C" fn(*mut sys::lv_event_t),
        user_data: *mut c_void,
    ) -> Result<*mut sys::lv_event_dsc_t, Error> {
        if code != EventCode::ALL && !handler_allowed(code) {
            return Err(Error::InvalidArg);
        }
        // Inside a handler, LVGL may be sending to this object: growing its
        // list then reallocates it under a copy of the header that LVGL
        // frees if the object is deleted during that send
        // (`lv_event.c:111, 145`). Outside handlers nothing sends.
        // SAFETY: a live object, under the token.
        if crate::base::callback_depth() != 0
            && unsafe { sys::lvgl_sys_obj_events_traversing(self.as_ptr()) }
        {
            return Err(Error::Busy);
        }
        // SAFETY: shared access; the trampoline and its data stay valid for
        // the object's lifetime (functions, `'static` data, values).
        let dsc = unsafe {
            sys::lv_obj_add_event_cb(
                self.as_ptr(),
                Some(cb),
                code.bits() as sys::lv_event_code_t,
                user_data,
            )
        };
        if dsc.is_null() {
            Err(Error::OutOfMemory)
        } else {
            Ok(dsc)
        }
    }

    /// Calls `f` for every `code` event of this object (`EventCode::ALL`:
    /// every allowed code). Fails with [`Error::InvalidArg`] for codes
    /// handlers never receive ([`handler_allowed`]).
    pub fn on_fn(&self, code: EventCode, f: fn(&Event<'_>)) -> Result<(), Error> {
        self.add_handler(code, fn_trampoline, f as *mut c_void)
            .map(drop)
    }

    /// Calls the zero-sized closure `f` (no captures, or zero-sized ones)
    /// for every `code` event. Captured values are never dropped.
    pub fn on<F, R>(&self, code: EventCode, f: F) -> Result<(), Error>
    where
        F: Fn(&Event<'_>) -> R + Send + 'static,
        R: HandlerResult,
    {
        const {
            assert!(
                mem::size_of::<F>() == 0,
                "use on_with or on_static for state"
            )
        };
        mem::forget(f);
        self.add_handler(code, zst_trampoline::<F, R>, ptr::null_mut())
            .map(drop)
    }

    /// Calls the zero-sized closure `f` with `data` (a [`UserData`] value,
    /// stored in the handler: no allocation) for every `code` event.
    pub fn on_with<D, F, R>(&self, code: EventCode, data: D, f: F) -> Result<(), Error>
    where
        D: UserData,
        F: Fn(&Event<'_>, D) -> R + Send + 'static,
        R: HandlerResult,
    {
        const { assert!(mem::size_of::<F>() == 0, "use on_static for larger state") };
        mem::forget(f);
        self.add_handler(code, data_trampoline::<D, F, R>, encode(data))
            .map(drop)
    }

    /// Calls `handler.f` with `handler.ctx` for every `code` event.
    pub fn on_static<C: Sync + 'static>(
        &self,
        code: EventCode,
        handler: &'static Handler<C>,
    ) -> Result<(), Error> {
        let data = (handler as *const Handler<C>).cast_mut().cast();
        self.add_handler(code, static_trampoline::<C>, data)
            .map(drop)
    }

    /// Like [`on_with`](Obj::on_with), and returns an [`EventSub`] that
    /// removes the handler. Takes a registry slot for the object.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn on_removable<D, F, R>(&self, code: EventCode, data: D, f: F) -> Result<EventSub, Error>
    where
        D: UserData,
        F: Fn(&Event<'_>, D) -> R + Send + 'static,
        R: HandlerResult,
    {
        use crate::WidgetExt;
        const { assert!(mem::size_of::<F>() == 0, "use on_static for larger state") };
        let object = self.handle()?;
        mem::forget(f);
        let cb = removable_trampoline::<D, F, R>;
        let dsc = self.add_handler(code, cb, encode(data))?;
        Ok(EventSub { object, dsc, cb })
    }
}
