//! Displays: a Rust flush driver, its state and the draw buffers, in
//! application statics (or the driver on LVGL's heap, with
//! [`Display::create_boxed`]). Drivers get no token: they cannot touch
//! objects.

#[cfg(feature = "heap")]
use crate::heap::LvBox;
use crate::{DrawBuf, Error, Lvgl, layout::trailing, sys};
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

/// How the display is turned (`lv_display_rotation_t`). At 90 and 270
/// degrees, the horizontal and vertical resolutions are swapped.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Rotation {
    /// Not turned.
    Degrees0,
    /// A quarter turn.
    Degrees90,
    /// A half turn.
    Degrees180,
    /// Three quarter turns.
    Degrees270,
}

impl Rotation {
    fn raw(self) -> sys::lv_display_rotation_t {
        match self {
            Rotation::Degrees0 => sys::lv_display_rotation_t_LV_DISPLAY_ROTATION_0,
            Rotation::Degrees90 => sys::lv_display_rotation_t_LV_DISPLAY_ROTATION_90,
            Rotation::Degrees180 => sys::lv_display_rotation_t_LV_DISPLAY_ROTATION_180,
            Rotation::Degrees270 => sys::lv_display_rotation_t_LV_DISPLAY_ROTATION_270,
        }
    }
}

/// The color formats a display renders into with the software renderer
/// (each needs its `LV_DRAW_SW_SUPPORT_*` option; LVGL skips drawing in a
/// format whose support is disabled).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DisplayFormat {
    /// 16 bits: 5 red, 6 green, 5 blue.
    Rgb565,
    /// [`Rgb565`](DisplayFormat::Rgb565) with its two bytes swapped (for
    /// SPI panels).
    Rgb565Swapped,
    /// 24 bits: blue, green, red bytes.
    Rgb888,
    /// 32 bits, the fourth byte unused.
    Xrgb8888,
    /// 32 bits with alpha.
    Argb8888,
    /// 8-bit luminance.
    L8,
    /// 8-bit luminance and 8-bit alpha.
    Al88,
    /// 8-bit alpha.
    A8,
}

impl DisplayFormat {
    fn raw(self) -> sys::lv_color_format_t {
        match self {
            DisplayFormat::Rgb565 => sys::lv_color_format_t_LV_COLOR_FORMAT_RGB565,
            DisplayFormat::Rgb565Swapped => sys::lv_color_format_t_LV_COLOR_FORMAT_RGB565_SWAPPED,
            DisplayFormat::Rgb888 => sys::lv_color_format_t_LV_COLOR_FORMAT_RGB888,
            DisplayFormat::Xrgb8888 => sys::lv_color_format_t_LV_COLOR_FORMAT_XRGB8888,
            DisplayFormat::Argb8888 => sys::lv_color_format_t_LV_COLOR_FORMAT_ARGB8888,
            DisplayFormat::L8 => sys::lv_color_format_t_LV_COLOR_FORMAT_L8,
            DisplayFormat::Al88 => sys::lv_color_format_t_LV_COLOR_FORMAT_AL88,
            DisplayFormat::A8 => sys::lv_color_format_t_LV_COLOR_FORMAT_A8,
        }
    }
}

/// `LV_COORD_MAX` (`lv_area.h:216`): the largest coordinate LVGL stores.
const COORD_MAX: u32 = (1 << 29) - 1;

/// What the next refresh renders: the inputs of the buffer size checks.
#[derive(Clone, Copy)]
struct Geometry {
    /// Resolution before rotation (`hor_res`, `ver_res`).
    width: u32,
    height: u32,
    rotation: sys::lv_display_rotation_t,
    format: sys::lv_color_format_t,
    mode: RenderMode,
    /// `lv_display_set_matrix_rotation`: full and direct buffers keep the
    /// unrotated shape (`lv_refr.c:916-919`).
    matrix_rotation: bool,
}

impl Geometry {
    /// The display's current geometry.
    ///
    /// # Safety
    ///
    /// A live display, under the token.
    unsafe fn of(disp: *mut sys::lv_display_t) -> Self {
        // SAFETY: caller contract.
        unsafe {
            let mode = match sys::lv_display_get_render_mode(disp) {
                m if m == RenderMode::Direct.raw() => RenderMode::Direct,
                m if m == RenderMode::Full.raw() => RenderMode::Full,
                _ => RenderMode::Partial,
            };
            Geometry {
                width: sys::lv_display_get_original_horizontal_resolution(disp).max(0) as u32,
                height: sys::lv_display_get_original_vertical_resolution(disp).max(0) as u32,
                rotation: sys::lv_display_get_rotation(disp),
                format: sys::lv_display_get_color_format(disp),
                mode,
                matrix_rotation: sys::lv_display_get_matrix_rotation(disp),
            }
        }
    }

    /// The resolution after rotation.
    fn rotated(&self) -> (u32, u32) {
        if self.rotation == Rotation::Degrees90.raw() || self.rotation == Rotation::Degrees270.raw()
        {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    /// Whether LVGL's `uint32_t` arithmetic on this geometry cannot wrap:
    /// both sizes at most `LV_COORD_MAX` (`lv_area.h:216`), the stride of
    /// either side (`width * bpp`, rounded, `lv_draw_buf.c:702-709`) and
    /// the screen's buffer size in either orientation
    /// (`_calculate_draw_buf_size`, `lv_draw_buf.c:730-746`, used by the
    /// reshape) and pixel count times pixel size (the tile maths of
    /// `refr_area`, `lv_refr.c:933-937`) all within `u32`. A wrapped stride
    /// would pass the buffer checks with a tiny value while LVGL renders
    /// rows as wide as the screen.
    ///
    /// # Safety
    ///
    /// Under the token.
    unsafe fn addressable(&self) -> bool {
        use sys::*;
        let coord_max = u64::from(COORD_MAX);
        let (width, height) = (u64::from(self.width), u64::from(self.height));
        if !(1..=coord_max).contains(&width) || !(1..=coord_max).contains(&height) {
            return false;
        }
        // SAFETY: under the token; a pure lookup.
        let bpp = u64::from(unsafe { lv_color_format_get_bpp(self.format) });
        let stride_align = u64::from(LV_DRAW_BUF_STRIDE_ALIGN).max(1);
        let align = u64::from(LV_DRAW_BUF_ALIGN).max(1);
        let limit = u64::from(u32::MAX);
        // Every value is below 2^29 * 2^5, so nothing here wraps in u64.
        let stride = |side: u64| (side * bpp).div_ceil(8).next_multiple_of(stride_align);
        let fits_u32 = |side: u64, rows: u64| {
            side * bpp + 7 <= limit
                && stride(side) <= limit
                && (stride(side) as u128 * rows as u128 * 3 / 2 + 1024)
                    .next_multiple_of(align as u128)
                    <= limit as u128
        };
        fits_u32(width, height)
            && fits_u32(height, width)
            && width as u128 * height as u128 * bpp.div_ceil(8).max(1) as u128 <= limit as u128
    }

    /// Whether buffers of `size` bytes (a multiple of `LV_DRAW_BUF_ALIGN`)
    /// hold what LVGL renders: in partial mode one row of the rotated width
    /// (`get_max_row`, `lv_refr.c:1353-1364`, then the reshape of each area,
    /// `lv_refr.c:905-907`), in full and direct modes the whole screen
    /// (`lv_refr.c:908-925`), with the size formula of
    /// `_calculate_draw_buf_size` (`lv_draw_buf.c:730-746`).
    ///
    /// # Safety
    ///
    /// Under the token (the stride comes from LVGL's draw buffer handlers).
    unsafe fn fits(&self, size: usize) -> bool {
        use sys::*;
        // SAFETY: under the token.
        if !unsafe { self.addressable() } {
            return false;
        }
        let (width, rows) = match self.mode {
            RenderMode::Partial => (self.rotated().0, 1),
            RenderMode::Direct | RenderMode::Full if self.matrix_rotation => {
                (self.width, self.height)
            }
            RenderMode::Direct | RenderMode::Full => self.rotated(),
        };
        if width == 0 || rows == 0 {
            return false;
        }
        let format = self.format;
        let palette = if format == lv_color_format_t_LV_COLOR_FORMAT_I1 {
            2 * 4
        } else if format == lv_color_format_t_LV_COLOR_FORMAT_I2 {
            4 * 4
        } else if format == lv_color_format_t_LV_COLOR_FORMAT_I4 {
            16 * 4
        } else if format == lv_color_format_t_LV_COLOR_FORMAT_I8 {
            256 * 4
        } else {
            0
        };
        // Partial mode picks its rows without the alpha plane of RGB565A8
        // (`get_max_row`), then cannot reshape the buffer to them.
        let alpha_plane = format == lv_color_format_t_LV_COLOR_FORMAT_RGB565A8;
        if alpha_plane && self.mode == RenderMode::Partial {
            return false;
        }
        // SAFETY: under the token.
        let stride = unsafe { lv_draw_buf_width_to_stride(width, self.format) } as usize;
        if stride == 0 {
            return false;
        }
        let mut bytes = stride.checked_mul(rows as usize);
        if alpha_plane {
            bytes = bytes.and_then(|b| b.checked_add((stride / 2).checked_mul(rows as usize)?));
        }
        let align = (LV_DRAW_BUF_ALIGN as usize).max(1);
        match bytes.and_then(|b| b.checked_add(palette)) {
            Some(b) => b.checked_next_multiple_of(align).is_some_and(|b| b <= size),
            None => false,
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

/// The crate's state of a display: the start of its [`DisplayCell`], or of
/// its block on LVGL's heap ([`Display::create_boxed`]), the driver right
/// after it ([`trailing`]). The display's driver data points to it.
struct DisplayCore {
    /// Set before calling the driver, cleared by the driver (any thread or
    /// interrupt) when the buffer is free.
    flushing: AtomicBool,
    /// The rest is touched only under the token.
    claimed: Cell<bool>,
    buffers: Cell<[Option<Buffer>; 2]>,
    /// Drops the driver, then releases the cell or frees the block.
    release: Cell<Option<unsafe fn(*const DisplayCore)>>,
}

impl DisplayCore {
    const fn new() -> Self {
        Self {
            flushing: AtomicBool::new(false),
            claimed: Cell::new(false),
            buffers: Cell::new([None, None]),
            release: Cell::new(None),
        }
    }
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
            core: DisplayCore::new(),
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

// SAFETY: points to the atomic of a display's core, which stays allocated
// while the flag is set ([`Display::try_delete`] refuses until then).
unsafe impl Send for FlushDone {}

impl FlushDone {
    /// The buffer is free: one Release store, the last access.
    pub fn ready(self) {}
}

impl Drop for FlushDone {
    fn drop(&mut self) {
        // SAFETY: the core outlives the set flag; this store, which clears
        // it, is the last access (a boxed core may be freed right after).
        unsafe { self.0.as_ref() }.store(false, Ordering::Release);
    }
}

unsafe extern "C" fn flush_trampoline<D: Flush>(
    disp: *mut sys::lv_display_t,
    area: *const sys::lv_area_t,
    px_map: *mut u8,
) {
    // SAFETY: LVGL calls this for a display made by `Display::create` or
    // `create_boxed`, whose driver data is a core followed by a `D`, under
    // the token.
    unsafe {
        let base = sys::lv_display_get_driver_data(disp)
            .cast::<DisplayCore>()
            .cast_const();
        let core = &*base;
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
        let driver = &mut *trailing::<DisplayCore, D>(base);
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
    // SAFETY: the driver data of a crate display points to its core; only
    // the core is read.
    let core = unsafe { &*sys::lv_display_get_driver_data(disp).cast::<DisplayCore>() };
    while core.flushing.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
}

/// Drops the driver of a [`DisplayCell`] and releases the cell.
unsafe fn release_cell<D>(core: *const DisplayCore) {
    // SAFETY: called once by `try_delete`, for an initialised driver after
    // LVGL stopped using it; `core` points to the whole cell.
    unsafe {
        trailing::<DisplayCore, D>(core).drop_in_place();
        (*core).claimed.set(false);
    }
}

/// Drops the driver of a boxed display and frees its block.
#[cfg(feature = "heap")]
unsafe fn release_boxed<D>(core: *const DisplayCore) {
    // SAFETY: called once by `try_delete`, after LVGL stopped using the
    // block, and while no `FlushDone` points into it (the flag is clear).
    unsafe { LvBox::<DisplayCore, D>::free(core.cast_mut().cast()) }
}

/// A display made by [`Display::create`] or [`Display::create_boxed`]: 8
/// bytes. Dropping it keeps the display; deletion is explicit
/// ([`Display::try_delete`]).
pub struct Display {
    disp: NonNull<sys::lv_display_t>,
    core: NonNull<DisplayCore>,
}

// SAFETY: a pointer to LVGL's display and to the crate's core of it, used
// only with the token.
unsafe impl Send for Display {}

impl Display {
    /// Creates a `width` × `height` display drawing through `driver`, whose
    /// state lives in `cell`, into `buffers`. The first display becomes the
    /// default one. Fails with [`Error::AlreadyOwned`] if the cell or a
    /// buffer is in use (or both buffers are the same), and
    /// [`Error::InvalidArg`] if a buffer is too small for `mode` (one line
    /// in partial mode, the screen otherwise).
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
        if cell.core.claimed.get() {
            return Err(Error::AlreadyOwned);
        }
        let (disp, size) = Self::prepare(width, height, &buffers, mode)?;
        // The cell's pointer, so that the driver after the core is in reach.
        let core = NonNull::from(cell).cast::<DisplayCore>();
        // SAFETY: the cell is unclaimed: nothing else touches the driver,
        // which `trailing` finds at the cell's field (`repr(C)`).
        unsafe {
            debug_assert_eq!(
                trailing::<DisplayCore, D>(core.as_ptr()),
                cell.driver.get().cast::<D>()
            );
            (*cell.driver.get()).write(driver);
            Ok(Self::install::<D>(
                disp,
                core,
                buffers,
                size,
                mode,
                release_cell::<D>,
            ))
        }
    }

    /// Like [`Display::create`], with the driver and the crate's state in
    /// one block of LVGL's heap instead of an application `static`, freed
    /// when the display is deleted ([`Display::try_delete`], which refuses
    /// while a flush is pending). Fails with [`Error::OutOfMemory`] if the
    /// block cannot be allocated.
    #[cfg(feature = "heap")]
    pub fn create_boxed<D: Flush>(
        lv: &mut Lvgl,
        driver: D,
        width: u32,
        height: u32,
        buffers: DrawBuffers,
        mode: RenderMode,
    ) -> Result<Display, Error> {
        let _ = lv;
        let (disp, size) = Self::prepare(width, height, &buffers, mode)?;
        let Ok(block) = LvBox::new(DisplayCore::new(), driver) else {
            // SAFETY: not handed out; nothing uses it yet.
            unsafe { sys::lv_display_delete(disp.as_ptr()) };
            return Err(Error::OutOfMemory);
        };
        // The block starts with the core; the driver follows it.
        let Some(core) = NonNull::new(block.into_raw().cast::<DisplayCore>()) else {
            unreachable!("an allocated block");
        };
        // SAFETY: a fresh block holding a core and an initialised driver.
        Ok(unsafe { Self::install::<D>(disp, core, buffers, size, mode, release_boxed::<D>) })
    }

    /// Checks the buffers and creates the display: gives it back with the
    /// buffer size LVGL gets (the smaller buffer, rounded down to
    /// `LV_DRAW_BUF_ALIGN` so that LVGL's rounded sizes fit it).
    fn prepare(
        width: u32,
        height: u32,
        buffers: &DrawBuffers,
        mode: RenderMode,
    ) -> Result<(NonNull<sys::lv_display_t>, usize), Error> {
        if buffers.each().any(|b| b.claimed.get())
            || buffers.second.is_some_and(|b| b.data == buffers.first.data)
        {
            return Err(Error::AlreadyOwned);
        }
        let (Ok(w), Ok(h)) = (i32::try_from(width), i32::try_from(height)) else {
            return Err(Error::InvalidArg);
        };
        if w == 0 || h == 0 || width > COORD_MAX || height > COORD_MAX {
            return Err(Error::InvalidArg);
        }
        let align = (sys::LV_DRAW_BUF_ALIGN as usize).max(1);
        let size = buffers.each().map(|b| b.len).min().unwrap_or(0);
        let size = size.min(u32::MAX as usize) / align * align;
        // SAFETY: exclusive access.
        let disp = unsafe { sys::lv_display_create(w, h) };
        let Some(disp) = NonNull::new(disp) else {
            return Err(Error::OutOfMemory);
        };
        // SAFETY: a display just created, under the token.
        let fits = unsafe {
            let geometry = Geometry {
                mode,
                ..Geometry::of(disp.as_ptr())
            };
            geometry.fits(size)
        };
        if !fits {
            // SAFETY: not handed out; nothing uses it yet.
            unsafe { sys::lv_display_delete(disp.as_ptr()) };
            return Err(Error::InvalidArg);
        }
        Ok((disp, size))
    }

    /// Claims the buffers and hands the display to the driver at `core`.
    ///
    /// # Safety
    ///
    /// `core` points to an unclaimed core followed by an initialised `D`
    /// (with provenance over both), which `release` drops and releases;
    /// `disp` and `size` come from `prepare` with these buffers.
    unsafe fn install<D: Flush>(
        disp: NonNull<sys::lv_display_t>,
        core: NonNull<DisplayCore>,
        buffers: DrawBuffers,
        size: usize,
        mode: RenderMode,
        release: unsafe fn(*const DisplayCore),
    ) -> Display {
        // SAFETY: caller contract.
        unsafe {
            let c = core.as_ref();
            for b in buffers.each() {
                b.claimed.set(true);
            }
            c.claimed.set(true);
            c.buffers.set([Some(buffers.first), buffers.second]);
            c.release.set(Some(release));
            c.flushing.store(false, Ordering::Relaxed);
            let d = disp.as_ptr();
            sys::lv_display_set_driver_data(d, core.as_ptr().cast());
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
        Display { disp, core }
    }

    /// Makes this display the default one (new screens and input devices
    /// go there).
    pub fn set_default(&self, lv: &mut Lvgl) {
        let _ = lv;
        // SAFETY: exclusive access, a live display.
        unsafe { sys::lv_display_set_default(self.disp.as_ptr()) }
    }

    /// The resolution after rotation: the size of the screens.
    pub fn resolution(&self, lv: &Lvgl) -> (u32, u32) {
        let _ = lv;
        // SAFETY: under the token, a live display.
        unsafe { Geometry::of(self.disp.as_ptr()) }.rotated()
    }

    /// Applies a change of the geometry if the buffers hold what LVGL then
    /// renders, and reshapes them; [`Error::InvalidArg`] if LVGL's sizes
    /// would wrap, [`Error::TooSmall`] if the buffers are too small.
    fn change_geometry(
        &self,
        change: impl FnOnce(&mut Geometry),
        apply: impl FnOnce(*mut sys::lv_display_t),
    ) -> Result<(), Error> {
        let d = self.disp.as_ptr();
        // SAFETY: under the token (every caller has `&mut Lvgl`), a live
        // display with its buffers.
        unsafe {
            let mut geometry = Geometry::of(d);
            change(&mut geometry);
            if !geometry.addressable() {
                return Err(Error::InvalidArg);
            }
            if !geometry.fits(sys::lv_display_get_draw_buf_size(d) as usize) {
                return Err(Error::TooSmall);
            }
            apply(d);
            // Checked above with the same rules: every buffer takes the
            // new shape.
            let reshaped = sys::lvgl_sys_display_reshape_buffers(d);
            debug_assert!(reshaped);
        }
        Ok(())
    }

    /// Changes the resolution (before rotation). Fails with
    /// [`Error::InvalidArg`] for a zero size, a size above `LV_COORD_MAX`
    /// or one whose buffer sizes would wrap LVGL's 32-bit arithmetic, and
    /// with
    /// [`Error::TooSmall`] if the draw buffers cannot hold a row of the
    /// new width (partial mode) or the new screen (full and direct modes).
    ///
    /// LVGL checks buffer sizes only when they are set, and only with
    /// `LV_USE_CHECK_ARG` (`lv_display.c:550-571`); the resolution setter
    /// never does (`lv_display.c:283-299`).
    pub fn set_resolution(&self, lv: &mut Lvgl, width: u32, height: u32) -> Result<(), Error> {
        let _ = lv;
        let (Ok(w), Ok(h)) = (i32::try_from(width), i32::try_from(height)) else {
            return Err(Error::InvalidArg);
        };
        if w == 0 || h == 0 {
            return Err(Error::InvalidArg);
        }
        self.change_geometry(
            |g| {
                g.width = width;
                g.height = height;
            },
            // SAFETY: exclusive access, a live display.
            |d| unsafe { sys::lv_display_set_resolution(d, w, h) },
        )
    }

    /// Turns the display (the driver rotates the pixels it sends). Fails
    /// with [`Error::TooSmall`] if the draw buffers cannot hold what LVGL
    /// renders once the resolutions are swapped.
    pub fn set_rotation(&self, lv: &mut Lvgl, rotation: Rotation) -> Result<(), Error> {
        let _ = lv;
        self.change_geometry(
            |g| g.rotation = rotation.raw(),
            // SAFETY: exclusive access, a live display.
            |d| unsafe { sys::lv_display_set_rotation(d, rotation.raw()) },
        )
    }

    /// Changes the color format LVGL renders in. Fails with
    /// [`Error::TooSmall`] if the draw buffers cannot hold what LVGL
    /// renders in the new format (LVGL rewrites the format of the existing
    /// buffers without checking them, `lv_display.c:658-660`).
    pub fn set_color_format(&self, lv: &mut Lvgl, format: DisplayFormat) -> Result<(), Error> {
        let _ = lv;
        self.change_geometry(
            |g| g.format = format.raw(),
            // SAFETY: exclusive access, a live display; the whole screen is
            // drawn again in the new format.
            |d| unsafe {
                sys::lv_display_set_color_format(d, format.raw());
                sys::lv_obj_invalidate(sys::lv_display_get_layer_sys(d));
            },
        )
    }

    /// Changes how LVGL renders into the draw buffers. Partial mode is
    /// always possible; full and direct modes fail with
    /// [`Error::TooSmall`] unless the buffers hold the whole screen.
    pub fn set_render_mode(&self, lv: &mut Lvgl, mode: RenderMode) -> Result<(), Error> {
        let _ = lv;
        self.change_geometry(
            |g| g.mode = mode,
            // SAFETY: exclusive access, a live display; the whole screen is
            // drawn again in the new mode.
            |d| unsafe {
                sys::lv_display_set_render_mode(d, mode.raw());
                sys::lv_obj_invalidate(sys::lv_display_get_layer_sys(d));
            },
        )
    }

    /// Splits each rendered area into at most `count` tiles, for parallel
    /// draw units. Fails with [`Error::InvalidArg`] outside `1..=255` or
    /// above the draw buffer size in bytes: LVGL divides by
    /// `buffer size / count` (`lv_refr.c:933-937`), which is zero then.
    /// The geometry setters never change the buffer size.
    pub fn set_tile_count(&self, lv: &mut Lvgl, count: u32) -> Result<(), Error> {
        let _ = lv;
        // SAFETY: under the token, a live display with its buffers.
        let size = unsafe { sys::lv_display_get_draw_buf_size(self.disp.as_ptr()) };
        if !(1..256).contains(&count) || count > size {
            return Err(Error::InvalidArg);
        }
        // SAFETY: exclusive access, a live display.
        unsafe { sys::lv_display_set_tile_cnt(self.disp.as_ptr(), count) };
        Ok(())
    }

    /// Deletes the display, its screens and their objects, then drops the
    /// driver and releases the cell (or frees the block of a boxed
    /// display) and the buffers. Fails with [`Error::Busy`] (giving the
    /// display back) while a flush is pending.
    pub fn try_delete(self, lv: &mut Lvgl) -> Result<(), (Display, Error)> {
        let _ = lv;
        let release = {
            // SAFETY: the core stays allocated until `release` runs.
            let core = unsafe { self.core.as_ref() };
            if core.flushing.load(Ordering::Acquire) {
                return Err((self, Error::Busy));
            }
            // SAFETY: exclusive access at depth 0 (`&mut Lvgl`).
            unsafe { sys::lv_display_delete(self.disp.as_ptr()) };
            for b in core.buffers.take().into_iter().flatten() {
                b.claimed.set(false);
            }
            core.release.take()
        };
        if let Some(release) = release {
            // SAFETY: once, after LVGL stopped using the driver, with no
            // flush pending; the core is not used afterwards.
            unsafe { release(self.core.as_ptr()) };
        }
        Ok(())
    }

    /// The LVGL display, for raw calls.
    pub fn as_ptr(&self) -> *mut sys::lv_display_t {
        self.disp.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indev::IndevCell;

    #[repr(align(16))]
    struct Wide(#[allow(dead_code)] u8);

    /// The flush and read callbacks find the driver with `trailing`: it
    /// must be the cell's field, whatever the driver's alignment.
    #[test]
    fn drivers_follow_their_headers() {
        fn display<D>(cell: &DisplayCell<D>) {
            let base = ptr::from_ref(cell).cast::<DisplayCore>();
            assert_eq!(
                trailing::<DisplayCore, D>(base),
                cell.driver.get().cast::<D>()
            );
        }
        fn indev<R>(cell: &IndevCell<R>) {
            let base = ptr::from_ref(cell).cast::<Cell<bool>>();
            assert_eq!(trailing::<Cell<bool>, R>(base), cell.reader_ptr());
        }
        display(&DisplayCell::<()>::new());
        display(&DisplayCell::<u8>::new());
        display(&DisplayCell::<u64>::new());
        display(&DisplayCell::<f64>::new());
        display(&DisplayCell::<Wide>::new());
        indev(&IndevCell::<()>::new());
        indev(&IndevCell::<u16>::new());
        indev(&IndevCell::<u64>::new());
        indev(&IndevCell::<Wide>::new());
    }

    /// Boxed drivers (`LvBox`, feature `heap`) put the driver at
    /// `trailing` in a block of `block_size` bytes. Under Miri, writing
    /// the header and the driver through a pointer whose provenance covers
    /// only those bytes, at every block address the heap can return,
    /// checks that both fit and are aligned (the miri task builds without
    /// `heap`, so it never runs `LvBox` itself).
    #[test]
    fn boxed_drivers_fit_their_blocks() {
        use crate::layout::block_size;
        use core::mem::MaybeUninit;

        #[repr(C, align(32))]
        struct Arena([MaybeUninit<u8>; 256]);

        fn check<H, T>(header: impl Fn() -> H, value: impl Fn() -> T) {
            // What every allocator LVGL uses guarantees (`HEAP_ALIGN`).
            let block_align = size_of::<usize>();
            let size = block_size::<H, T>(block_align);
            let mut arena = Arena([MaybeUninit::uninit(); 256]);
            for offset in (0..32).step_by(block_align) {
                let block = arena.0[offset..offset + size].as_mut_ptr().cast::<H>();
                let driver = trailing::<H, T>(block);
                // SAFETY: what `LvBox::new` and `free` do; Miri reports
                // any access outside the block or misaligned.
                unsafe {
                    block.write(header());
                    driver.write(value());
                    ptr::drop_in_place(driver);
                    ptr::drop_in_place(block);
                }
            }
        }
        check(DisplayCore::new, || ());
        check(DisplayCore::new, || 7u8);
        check(DisplayCore::new, || 7u64);
        check(DisplayCore::new, || 7.0f64);
        check(DisplayCore::new, || Wide(7));
        check(DisplayCore::new, || [7u8; 13]);
        check(|| Cell::new(false), || ());
        check(|| Cell::new(false), || 7u16);
        check(|| Cell::new(false), || 7u64);
        check(|| Cell::new(false), || Wide(7));
        check(|| Cell::new(false), || [7u8; 13]);
    }
}
