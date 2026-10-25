//! Displays: a Rust flush driver, its state and the draw buffers, all in
//! application statics. Drivers get no token: they cannot touch objects.

use crate::{DrawBuf, Error, Lvgl, sys};
use core::{
    cell::{Cell, UnsafeCell},
    mem::MaybeUninit,
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, Ordering},
};

/// A display driver: sends rendered pixels to the screen.
pub trait Flush: Send + 'static {
    /// Sends the pixels of `req`. Returning (or dropping `req`) tells LVGL
    /// the buffer is free; [`FlushRequest::defer`] hands that over to an
    /// interrupt or another thread instead.
    fn flush(&mut self, req: FlushRequest<'_>);
}

/// How LVGL renders into the draw buffers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RenderMode {
    /// Areas rendered in turn into buffers of at least one line.
    Partial,
    /// Areas rendered in place in screen-sized buffers.
    Direct,
    /// The whole screen rendered into screen-sized buffers.
    Full,
}

impl RenderMode {
    fn raw(self) -> sys::lv_display_render_mode_t {
        match self {
            RenderMode::Partial => sys::lv_display_render_mode_t_LV_DISPLAY_RENDER_MODE_PARTIAL,
            RenderMode::Direct => sys::lv_display_render_mode_t_LV_DISPLAY_RENDER_MODE_DIRECT,
            RenderMode::Full => sys::lv_display_render_mode_t_LV_DISPLAY_RENDER_MODE_FULL,
        }
    }
}

// SAFETY: the bytes are only written by LVGL under the token, through the
// display that claimed the buffer.
unsafe impl<const BYTES: usize> Sync for DrawBuf<BYTES> {}

impl<const BYTES: usize> DrawBuf<BYTES> {
    /// A zeroed buffer.
    pub const fn new() -> Self {
        Self {
            data: UnsafeCell::new([0; BYTES]),
            claimed: Cell::new(false),
        }
    }
}

impl<const BYTES: usize> Default for DrawBuf<BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

/// One buffer of [`DrawBuffers`].
#[derive(Clone, Copy)]
struct Buffer {
    data: *mut u8,
    len: usize,
    claimed: &'static Cell<bool>,
}

/// The draw buffers of a display: one, or two to render while flushing.
pub struct DrawBuffers {
    first: Buffer,
    second: Option<Buffer>,
}

impl DrawBuffers {
    fn buffer<const N: usize>(b: &'static DrawBuf<N>) -> Buffer {
        Buffer {
            data: b.data.get().cast(),
            len: N,
            claimed: &b.claimed,
        }
    }

    /// One buffer.
    pub fn single<const N: usize>(buffer: &'static DrawBuf<N>) -> Self {
        Self {
            first: Self::buffer(buffer),
            second: None,
        }
    }

    /// Two buffers (LVGL uses the size of the smaller one).
    pub fn double<const N: usize, const M: usize>(
        first: &'static DrawBuf<N>,
        second: &'static DrawBuf<M>,
    ) -> Self {
        Self {
            first: Self::buffer(first),
            second: Some(Self::buffer(second)),
        }
    }

    fn each(&self) -> impl Iterator<Item = Buffer> + '_ {
        core::iter::once(self.first).chain(self.second)
    }
}

/// The crate's state of a display, in its [`DisplayCell`].
struct DisplayCore {
    /// Set before calling the driver, cleared by the driver (any thread or
    /// interrupt) when the buffer is free.
    flushing: AtomicBool,
    /// The rest is touched only under the token.
    claimed: Cell<bool>,
    buffers: Cell<[Option<Buffer>; 2]>,
    drop_driver: Cell<Option<unsafe fn(*const ())>>,
}

/// A display driver and the crate's state of its display: an application
/// `static`, claimed by [`Display::create`] until the display is deleted.
#[repr(C)]
pub struct DisplayCell<D> {
    core: DisplayCore,
    driver: UnsafeCell<MaybeUninit<D>>,
}

// SAFETY: the driver is used only by the flush callback (under the token)
// and by `create` / `try_delete` (exclusive access): one thread at a time,
// so `D: Send` suffices; `flushing` is atomic.
unsafe impl<D: Send> Sync for DisplayCell<D> {}

impl<D> DisplayCell<D> {
    /// An unclaimed cell.
    pub const fn new() -> Self {
        Self {
            core: DisplayCore {
                flushing: AtomicBool::new(false),
                claimed: Cell::new(false),
                buffers: Cell::new([None, None]),
                drop_driver: Cell::new(None),
            },
            driver: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

impl<D> Default for DisplayCell<D> {
    fn default() -> Self {
        Self::new()
    }
}

/// Pixels to send, handed to [`Flush::flush`].
pub struct FlushRequest<'a> {
    area: sys::lv_area_t,
    pixels: &'a mut [u8],
    stride: usize,
    flushing: &'a AtomicBool,
    deferred: bool,
}

impl<'a> FlushRequest<'a> {
    /// The area of the screen to update (offset applied).
    pub fn area(&self) -> sys::lv_area_t {
        self.area
    }

    /// Bytes between the starts of two rows.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The rendered pixels: `stride` × area height bytes in partial mode,
    /// the whole buffer (rows of the screen) in direct and full modes.
    pub fn pixels(&mut self) -> &mut [u8] {
        self.pixels
    }

    /// The buffer stays busy until the returned [`FlushDone`] is ready
    /// (or dropped), from any thread or interrupt.
    ///
    /// LVGL may need the buffer again before `timer_handler` returns: to
    /// flush the next area of the same refresh, or to render into it with
    /// a single buffer. It then waits (spinning) on its own thread for the
    /// completion, so complete from an interrupt or another thread (DMA
    /// done), never later on the thread that runs `timer_handler`.
    pub fn defer(mut self) -> FlushDone {
        self.deferred = true;
        FlushDone(NonNull::from(self.flushing))
    }
}

impl Drop for FlushRequest<'_> {
    fn drop(&mut self) {
        if !self.deferred {
            self.flushing.store(false, Ordering::Release);
        }
    }
}

/// A deferred flush completion: `Send` and interrupt-safe.
pub struct FlushDone(NonNull<AtomicBool>);

// SAFETY: points to an atomic in a `'static` cell.
unsafe impl Send for FlushDone {}

impl FlushDone {
    /// The buffer is free: one Release store, the last access.
    pub fn ready(self) {}
}

impl Drop for FlushDone {
    fn drop(&mut self) {
        // SAFETY: an atomic of a `'static` `DisplayCell`.
        unsafe { self.0.as_ref() }.store(false, Ordering::Release);
    }
}

unsafe extern "C" fn flush_trampoline<D: Flush>(
    disp: *mut sys::lv_display_t,
    area: *const sys::lv_area_t,
    px_map: *mut u8,
) {
    // SAFETY: LVGL calls this for a display made by `Display::create`, whose
    // driver data is this cell, under the token.
    unsafe {
        let cell = &*sys::lv_display_get_driver_data(disp).cast::<DisplayCell<D>>();
        let core = &cell.core;
        core.flushing.store(true, Ordering::Relaxed);
        let area = *area;
        let buf = sys::lv_display_get_buf_active(disp);
        let stride = if buf.is_null() {
            0
        } else {
            (*buf).header.stride() as usize
        };
        // The pixels lie in one of the display's buffers: never past its
        // end, and in partial mode at most `stride` × height bytes.
        let mut len = 0;
        for b in core.buffers.get().into_iter().flatten() {
            if px_map >= b.data && px_map < b.data.add(b.len) {
                len = b.len - px_map.offset_from(b.data) as usize;
            }
        }
        if sys::lv_display_get_render_mode(disp) == RenderMode::Partial.raw() {
            let rows = (area.y2 - area.y1 + 1).max(0) as usize;
            len = len.min(stride.saturating_mul(rows));
        }
        let pixels = if len == 0 {
            &mut []
        } else {
            core::slice::from_raw_parts_mut(px_map, len)
        };
        let driver = (*cell.driver.get()).assume_init_mut();
        driver.flush(FlushRequest {
            area,
            pixels,
            stride,
            flushing: &core.flushing,
            deferred: false,
        });
    }
}

/// LVGL waits here for the driver; it clears its own flag afterwards
/// (`lv_refr.c:1493-1497`), on its own thread.
unsafe extern "C" fn wait_trampoline(disp: *mut sys::lv_display_t) {
    // SAFETY: the driver data of a crate display starts with its core: a
    // `DisplayCell` is `DisplayCore` followed by the driver, and only the
    // core is read.
    let core = unsafe { &*sys::lv_display_get_driver_data(disp).cast::<DisplayCore>() };
    while core.flushing.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
}

unsafe fn drop_driver<D>(cell: *const ()) {
    // SAFETY: called once by `try_delete`, for an initialised driver.
    unsafe { (*(*cell.cast::<DisplayCell<D>>()).driver.get()).assume_init_drop() }
}

/// A display made by [`Display::create`]: 8 bytes. Dropping it keeps the
/// display; deletion is explicit ([`Display::try_delete`]).
pub struct Display {
    disp: NonNull<sys::lv_display_t>,
    core: NonNull<DisplayCore>,
}

// SAFETY: a pointer to LVGL's display and to a `'static` cell, used only
// with the token.
unsafe impl Send for Display {}

impl Display {
    /// Creates a `width` × `height` display drawing through `driver`, whose
    /// state lives in `cell`, into `buffers`. The first display becomes the
    /// default one. Fails with [`Error::AlreadyOwned`] if the cell or a
    /// buffer is in use, and [`Error::InvalidArg`] if a buffer is too small
    /// for `mode` (one line in partial mode, the screen otherwise).
    pub fn create<D: Flush>(
        lv: &mut Lvgl,
        cell: &'static DisplayCell<D>,
        driver: D,
        width: u32,
        height: u32,
        buffers: DrawBuffers,
        mode: RenderMode,
    ) -> Result<Display, Error> {
        let _ = lv;
        let core = &cell.core;
        if core.claimed.get() || buffers.each().any(|b| b.claimed.get()) {
            return Err(Error::AlreadyOwned);
        }
        let (Ok(w), Ok(h)) = (i32::try_from(width), i32::try_from(height)) else {
            return Err(Error::InvalidArg);
        };
        if w == 0 || h == 0 {
            return Err(Error::InvalidArg);
        }
        // SAFETY: exclusive access.
        let disp = unsafe { sys::lv_display_create(w, h) };
        let Some(disp) = NonNull::new(disp) else {
            return Err(Error::OutOfMemory);
        };
        let d = disp.as_ptr();
        // SAFETY: a display just created.
        let stride =
            unsafe { sys::lv_draw_buf_width_to_stride(width, sys::lv_display_get_color_format(d)) }
                as usize;
        let need = match mode {
            RenderMode::Partial => stride,
            RenderMode::Direct | RenderMode::Full => stride.saturating_mul(height as usize),
        };
        let size = buffers.each().map(|b| b.len).min().unwrap_or(0);
        if size < need.max(1) || u32::try_from(size).is_err() {
            // SAFETY: not handed out; nothing uses it yet.
            unsafe { sys::lv_display_delete(d) };
            return Err(Error::InvalidArg);
        }
        for b in buffers.each() {
            b.claimed.set(true);
        }
        core.claimed.set(true);
        core.buffers.set([Some(buffers.first), buffers.second]);
        core.drop_driver.set(Some(drop_driver::<D>));
        core.flushing.store(false, Ordering::Relaxed);
        // SAFETY: the cell is claimed: nothing else touches the driver.
        unsafe {
            (*cell.driver.get()).write(driver);
            sys::lv_display_set_driver_data(d, ptr::from_ref(cell).cast_mut().cast());
            sys::lv_display_set_flush_cb(d, Some(flush_trampoline::<D>));
            sys::lv_display_set_flush_wait_cb(d, Some(wait_trampoline));
            sys::lv_display_set_buffers(
                d,
                buffers.first.data.cast(),
                buffers.second.map_or(ptr::null_mut(), |b| b.data.cast()),
                size as u32,
                mode.raw(),
            );
        }
        Ok(Display {
            disp,
            core: NonNull::from(core),
        })
    }

    /// Makes this display the default one (new screens and input devices
    /// go there).
    pub fn set_default(&self, lv: &mut Lvgl) {
        let _ = lv;
        // SAFETY: exclusive access, a live display.
        unsafe { sys::lv_display_set_default(self.disp.as_ptr()) }
    }

    /// Deletes the display, its screens and their objects, then drops the
    /// driver and releases the cell and buffers. Fails with [`Error::Busy`]
    /// (giving the display back) while a flush is pending.
    pub fn try_delete(self, lv: &mut Lvgl) -> Result<(), (Display, Error)> {
        let _ = lv;
        // SAFETY: the core of a `'static` cell.
        let core = unsafe { self.core.as_ref() };
        if core.flushing.load(Ordering::Acquire) {
            return Err((self, Error::Busy));
        }
        // SAFETY: exclusive access at depth 0 (`&mut Lvgl`); the driver is
        // dropped once, after LVGL stopped using it.
        unsafe {
            sys::lv_display_delete(self.disp.as_ptr());
            if let Some(drop) = core.drop_driver.take() {
                drop(self.core.as_ptr().cast());
            }
        }
        for b in core.buffers.take().into_iter().flatten() {
            b.claimed.set(false);
        }
        core.claimed.set(false);
        Ok(())
    }

    /// The LVGL display, for raw calls.
    pub fn as_ptr(&self) -> *mut sys::lv_display_t {
        self.disp.as_ptr()
    }
}
