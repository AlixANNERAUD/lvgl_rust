//! Access to LVGL: the [`LvglAccess`] token, [`Lvgl`] and [`Session`].

use super::{ffi, registry::reg, sync::Ordering};
use crate::{Error, config::MAX_CALLBACK_DEPTH};
use core::{marker::PhantomData, ops::Deref, ops::DerefMut, ptr::NonNull};

/// Proof of exclusive access to LVGL.
///
/// # Safety
///
/// Implement it only on a type whose existence proves exclusive,
/// non-recursive access to LVGL (a guard of the mutex that serializes every
/// LVGL call, a newtype dedicated to it, never a guard type shared with
/// unrelated mutexes). The end of one value's lifetime must
/// synchronize-with the start of the next one, on any thread (Release/Acquire
/// or stronger, as mutexes provide): this orders the crate's state and LVGL's
/// own C globals between successive holders.
pub unsafe trait LvglAccess {}

/// Shared (`&Lvgl`) or exclusive (`&mut Lvgl`) access to LVGL. Zero-sized,
/// `!Send` and `!Sync`; obtained from a [`Session`].
///
/// Shared access allows every call that cannot free an object: views,
/// getters, setters, creation. Calls that may delete objects need exclusive
/// access, so no view can be held across them.
pub struct Lvgl {
    _p: PhantomData<*mut ()>,
}

/// Access to LVGL for as long as the guard it was entered with is borrowed.
/// Dereferences to [`Lvgl`]. Zero-sized.
pub struct Session<'g> {
    lv: Lvgl,
    _guard: PhantomData<&'g mut ()>,
}

impl Session<'_> {
    fn new() -> Self {
        Self {
            lv: Lvgl { _p: PhantomData },
            _guard: PhantomData,
        }
    }
}

impl Deref for Session<'_> {
    type Target = Lvgl;
    fn deref(&self) -> &Lvgl {
        &self.lv
    }
}

impl DerefMut for Session<'_> {
    fn deref_mut(&mut self) -> &mut Lvgl {
        &mut self.lv
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        reg().hdr.entered.store(false, Ordering::SeqCst);
    }
}

/// Initializes LVGL (`lv_init`) and returns the first session.
///
/// Fails with [`Error::Reentrant`] if LVGL is already initialized or a
/// session is active, and with [`Error::NotInitialized`] after
/// [`Lvgl::deinit`].
#[inline]
pub fn init<'g, A: LvglAccess>(guard: &'g mut A) -> Result<Session<'g>, Error> {
    let _ = guard;
    init_raw().map(|()| Session::new())
}

fn init_raw() -> Result<(), Error> {
    let hdr = &reg().hdr;
    match hdr.init.load(Ordering::SeqCst) {
        0 => {}
        1 => return Err(Error::Reentrant),
        _ => return Err(Error::NotInitialized),
    }
    if hdr.entered.load(Ordering::SeqCst) || hdr.depth.get() != 0 {
        return Err(Error::Reentrant);
    }
    hdr.entered.store(true, Ordering::SeqCst);
    // SAFETY: first LVGL call, under the caller's exclusive access.
    unsafe {
        ffi::init();
        // After `lv_init`, which zeroes LVGL's globals.
        ffi::set_tick_cb(crate_tick_cb);
    }
    #[cfg(all(not(lvgl_rust_views_only), not(lvgl_rust_docs)))]
    super::hook::keep();
    hdr.init.store(1, Ordering::SeqCst);
    Ok(())
}

/// LVGL's tick source until [`Lvgl::set_tick_cb`] replaces it.
unsafe extern "C" fn crate_tick_cb() -> u32 {
    reg().hdr.ticks.load(Ordering::Relaxed)
}

/// Advances LVGL's clock by `ms` milliseconds. Needs no token: call it from a
/// timer interrupt or a dedicated thread. On targets without atomic
/// read-modify-write (`thumbv6m`, `riscv32imc`), concurrent callers may lose
/// ticks, so call it from one context there.
pub fn tick_inc(ms: u32) {
    let ticks = &reg().hdr.ticks;
    #[cfg(target_has_atomic = "32")]
    ticks.fetch_add(ms, Ordering::Relaxed);
    #[cfg(not(target_has_atomic = "32"))]
    ticks.store(
        ticks.load(Ordering::Relaxed).wrapping_add(ms),
        Ordering::Relaxed,
    );
}

impl Lvgl {
    /// Starts a session. Fails with [`Error::NotInitialized`] before
    /// [`init`] or after [`Lvgl::deinit`], and with [`Error::Reentrant`] if a
    /// session is active or a callback is running (a recursive lock).
    ///
    /// Requests recorded since the previous session (dropped `Owned`
    /// handles, `delete_later`, ...) are carried out first.
    #[inline]
    pub fn enter<'g, A: LvglAccess>(guard: &'g mut A) -> Result<Session<'g>, Error> {
        let _ = guard;
        enter_raw().map(|()| Session::new())
    }

    /// Runs LVGL's timers (`lv_timer_handler`), then carries out the deferred
    /// requests. Returns the time in milliseconds until the next timer.
    pub fn timer_handler(&mut self) -> u32 {
        // SAFETY: exclusive access.
        let next = unsafe { ffi::timer_handler() };
        super::drain::drain();
        next
    }

    /// Makes `f` LVGL's tick source instead of [`tick_inc`].
    pub fn set_tick_cb(&mut self, f: extern "C" fn() -> u32) {
        // SAFETY: exclusive access; a safe `extern "C"` function pointer is a
        // valid tick callback.
        unsafe { ffi::set_tick_cb(f) }
    }

    /// Deinitializes LVGL (`lv_deinit`), deleting every object. Terminal:
    /// [`init`] and [`Lvgl::enter`] fail afterwards.
    pub fn deinit(session: Session<'_>) -> Result<(), Error> {
        super::drain::drain();
        // SAFETY: exclusive access; no session can follow.
        unsafe { ffi::deinit() };
        reg().hdr.init.store(2, Ordering::SeqCst);
        drop(session);
        Ok(())
    }

    /// Exclusive access without a session, for interop with raw code.
    ///
    /// # Safety
    ///
    /// The caller must have exclusive access to LVGL for `'a` (as a session
    /// holder does) and must not create overlapping references.
    pub unsafe fn assume<'a>() -> &'a mut Lvgl {
        // SAFETY: `Lvgl` is zero-sized: any aligned non-null pointer is valid.
        unsafe { &mut *NonNull::<Lvgl>::dangling().as_ptr() }
    }

    /// Enters a callback level, for raw C callbacks (registered through
    /// `sys`) that call safe code. Returns `None` at the depth limit
    /// (`LVGL_RUST_MAX_CALLBACK_DEPTH`).
    ///
    /// # Safety
    ///
    /// Call it only from a callback that LVGL runs inside a call made by the
    /// session holder (an event, timer or animation callback), and keep the
    /// guard within that callback.
    pub unsafe fn enter_callback() -> Option<DepthGuard> {
        let depth = &reg().hdr.depth;
        let current = depth.get();
        if current >= MAX_CALLBACK_DEPTH {
            return None;
        }
        depth.set(current + 1);
        Some(DepthGuard { _p: PhantomData })
    }
}

fn enter_raw() -> Result<(), Error> {
    let hdr = &reg().hdr;
    if hdr.init.load(Ordering::SeqCst) != 1 {
        return Err(Error::NotInitialized);
    }
    if hdr.entered.load(Ordering::SeqCst) || hdr.depth.get() != 0 {
        return Err(Error::Reentrant);
    }
    hdr.entered.store(true, Ordering::SeqCst);
    super::drain::drain();
    Ok(())
}

/// One callback level (see [`Lvgl::enter_callback`]). Zero-sized, `!Send`;
/// dropping it leaves the level.
pub struct DepthGuard {
    _p: PhantomData<*mut ()>,
}

impl DepthGuard {
    /// Shared access, as an event callback gets.
    pub fn lv(&self) -> &Lvgl {
        // SAFETY: `Lvgl` is zero-sized; callbacks only get shared access.
        unsafe { &*NonNull::<Lvgl>::dangling().as_ptr() }
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        let depth = &reg().hdr.depth;
        depth.set(depth.get() - 1);
    }
}
