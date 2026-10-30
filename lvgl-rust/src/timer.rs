//! Timers: Rust functions LVGL calls periodically from
//! [`Lvgl::timer_handler`], with shared access.

use crate::{
    Error, Handle, Lvgl, Owned,
    base::{
        handle::private::Sealed,
        registry::{Kind, meta::*, reg},
        sync::Ordering,
    },
    event::{UserData, decode, encode},
    sys,
};
use core::{cell::UnsafeCell, marker::PhantomData, mem, ptr::NonNull};

/// An LVGL timer, as the type of [`Owned<Timer>`](Owned) and
/// [`Handle<Timer>`](Handle). Created by [`Lvgl::timer`].
pub struct Timer {
    _t: UnsafeCell<[u8; 0]>,
}

impl Sealed for Timer {}
impl crate::Tracked for Timer {}

/// What a timer function gets: shared access and its timer.
pub struct TimerCtx<'cb> {
    timer: NonNull<sys::lv_timer_t>,
    _p: PhantomData<(&'cb Lvgl, *mut ())>,
}

impl<'cb> TimerCtx<'cb> {
    /// Shared access, for the call's duration.
    pub fn lv(&self) -> &'cb Lvgl {
        // SAFETY: `Lvgl` is zero-sized; timer functions get shared access.
        unsafe { &*NonNull::<Lvgl>::dangling().as_ptr() }
    }

    /// A handle to this timer (a scan of the registry: timers have no id).
    pub fn handle(&self) -> Option<Handle<Timer>> {
        let reg = reg();
        let index = reg.find_timer(self.timer.as_ptr().cast())?;
        // SAFETY: a live slot tracking this timer.
        Some(unsafe { Owned::<Timer>::from_raw(reg.handle_raw(index)) }.into_handle())
    }

    /// Deletes this timer at the next drain. Its [`Owned`] then reports it
    /// dead.
    pub fn delete_later(&self) {
        let reg = reg();
        if let Some(index) = reg.find_timer(self.timer.as_ptr().cast()) {
            reg.slot(index).set(P_DELETE);
            reg.hdr.any_req.store(true, Ordering::Relaxed);
        }
    }
}

impl Owned<Timer> {
    /// The handle, without dropping (and so deleting) the timer.
    fn into_handle(self) -> Handle<Timer> {
        let handle = self.handle();
        mem::forget(self);
        handle
    }
}

unsafe extern "C" fn trampoline<D, F>(t: *mut sys::lv_timer_t)
where
    D: UserData,
    F: Fn(&TimerCtx<'_>, D) + Send + 'static,
{
    let Some(timer) = NonNull::new(t) else { return };
    // SAFETY: LVGL runs timers only from `lv_timer_handler`, called by the
    // session holder; the guard stays within this callback.
    let Some(_depth) = (unsafe { Lvgl::enter_callback() }) else {
        return;
    };
    // SAFETY: `F` is zero-sized (checked by `timer`).
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    // SAFETY: stored by `timer` with `encode::<D>`.
    let data = unsafe { decode::<D>(sys::lv_timer_get_user_data(t)) };
    f(
        &TimerCtx {
            timer,
            _p: PhantomData,
        },
        data,
    );
}

impl Lvgl {
    /// Calls `f(ctx, data)` every `period_ms` from
    /// [`Lvgl::timer_handler`]. `f` is a function item or a closure without
    /// captures, and `data` a [`UserData`] value stored in the timer: no
    /// allocation besides LVGL's timer. Takes a registry slot
    /// ([`Error::RegistryFull`], or [`Error::OutOfMemory`] when a
    /// `growable-registry` cannot grow); dropping the [`Owned`] deletes the
    /// timer at the next drain.
    pub fn timer<D, F>(&self, period_ms: u32, data: D, f: F) -> Result<Owned<Timer>, Error>
    where
        D: UserData,
        F: Fn(&TimerCtx<'_>, D) + Send + 'static,
    {
        const {
            assert!(
                mem::size_of::<F>() == 0,
                "timer takes a function item or a closure without captures"
            )
        };
        mem::forget(f);
        // SAFETY: shared access; the trampoline and its data stay valid for
        // the timer's lifetime.
        let t = unsafe { sys::lv_timer_create(Some(trampoline::<D, F>), period_ms, encode(data)) };
        if t.is_null() {
            return Err(Error::OutOfMemory);
        }
        // SAFETY: a timer just created. A repeat count that runs out pauses
        // it instead of freeing it behind the registry (`lv_timer.c`).
        unsafe { sys::lv_timer_set_auto_delete(t, false) };
        let reg = reg();
        match reg.mint(t.cast(), Kind::Timer) {
            Ok(minted) => {
                reg.slot(minted.index).set(OWNED);
                // SAFETY: a fresh slot tracking this timer, owned by the
                // returned value only.
                Ok(unsafe { Owned::from_raw(minted.raw) })
            }
            Err(error) => {
                // SAFETY: not handed out yet.
                unsafe { sys::lv_timer_delete(t) };
                Err(error)
            }
        }
    }
}

impl Handle<Timer> {
    fn with(self, lv: &Lvgl, f: impl FnOnce(*mut sys::lv_timer_t)) -> Result<(), Error> {
        let _ = lv;
        let t = reg().resolve(self.raw(), Kind::Timer, core::ptr::null())?;
        f(t.as_ptr().cast());
        Ok(())
    }

    /// Stops calling the timer until [`resume`](Handle::resume).
    pub fn pause(self, lv: &Lvgl) -> Result<(), Error> {
        // SAFETY: a live timer of the crate, under the token.
        self.with(lv, |t| unsafe { sys::lv_timer_pause(t) })
    }

    /// Calls the timer again.
    pub fn resume(self, lv: &Lvgl) -> Result<(), Error> {
        // SAFETY: as above.
        self.with(lv, |t| unsafe { sys::lv_timer_resume(t) })
    }

    /// Changes the period.
    pub fn set_period(self, lv: &Lvgl, period_ms: u32) -> Result<(), Error> {
        // SAFETY: as above.
        self.with(lv, |t| unsafe { sys::lv_timer_set_period(t, period_ms) })
    }

    /// Calls the timer `count` more times, then pauses it (`-1`: forever).
    pub fn set_repeat_count(self, lv: &Lvgl, count: i32) -> Result<(), Error> {
        // SAFETY: as above; auto-delete is off, so running out pauses it.
        self.with(lv, |t| unsafe { sys::lv_timer_set_repeat_count(t, count) })
    }

    /// Calls the timer at the next `timer_handler`.
    pub fn ready(self, lv: &Lvgl) -> Result<(), Error> {
        // SAFETY: as above.
        self.with(lv, |t| unsafe { sys::lv_timer_ready(t) })
    }

    /// Restarts the period from now.
    pub fn reset(self, lv: &Lvgl) -> Result<(), Error> {
        // SAFETY: as above.
        self.with(lv, |t| unsafe { sys::lv_timer_reset(t) })
    }
}

/// Header of a [`Lvgl::timer_boxed`] closure: how to drop it (the drain
/// frees boxes of any closure type).
#[cfg(feature = "heap")]
pub(crate) struct BoxedTimer {
    pub(crate) free: unsafe fn(*mut core::ffi::c_void),
}

#[cfg(feature = "heap")]
unsafe extern "C" fn boxed_trampoline<F>(t: *mut sys::lv_timer_t)
where
    F: Fn(&TimerCtx<'_>) + Send + 'static,
{
    let Some(timer) = NonNull::new(t) else { return };
    // SAFETY: as in `trampoline`.
    let Some(_depth) = (unsafe { Lvgl::enter_callback() }) else {
        return;
    };
    // SAFETY: the user data is the timer's box (`timer_boxed`), freed only
    // by the drain, at depth 0, after deleting the timer.
    let f = crate::heap::LvBox::<BoxedTimer, F>::value(unsafe { sys::lv_timer_get_user_data(t) });
    unsafe {
        (*f)(&TimerCtx {
            timer,
            _p: PhantomData,
        })
    };
}

#[cfg(feature = "heap")]
impl Lvgl {
    /// Like [`timer`](Lvgl::timer) with a closure: `f` and what it captures
    /// live on LVGL's heap until the timer is deleted.
    pub fn timer_boxed<F>(&self, period_ms: u32, f: F) -> Result<Owned<Timer>, Error>
    where
        F: Fn(&TimerCtx<'_>) + Send + 'static,
    {
        use crate::heap::LvBox;
        let header = BoxedTimer {
            free: LvBox::<BoxedTimer, F>::free,
        };
        let block = LvBox::new(header, f)
            .map_err(|_| Error::OutOfMemory)?
            .into_raw();
        // SAFETY: shared access; the box outlives the timer (freed by the
        // drain after deleting it).
        let t = unsafe { sys::lv_timer_create(Some(boxed_trampoline::<F>), period_ms, block) };
        if t.is_null() {
            // SAFETY: not handed to LVGL.
            unsafe { LvBox::<BoxedTimer, F>::free(block) };
            return Err(Error::OutOfMemory);
        }
        // SAFETY: as in `timer`.
        unsafe { sys::lv_timer_set_auto_delete(t, false) };
        let reg = reg();
        match reg.mint(t.cast(), Kind::Timer) {
            Ok(minted) => {
                reg.slot(minted.index).set(OWNED | TIMER_BOXED);
                // SAFETY: as in `timer`.
                Ok(unsafe { Owned::from_raw(minted.raw) })
            }
            Err(error) => {
                // SAFETY: not handed out yet.
                unsafe {
                    sys::lv_timer_delete(t);
                    LvBox::<BoxedTimer, F>::free(block);
                }
                Err(error)
            }
        }
    }
}
