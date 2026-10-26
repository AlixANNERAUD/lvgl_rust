//! Images from constant data, and image sources: data LVGL keeps pointers
//! to and reads while drawing, so always `'static` and never written.

use crate::sys;
use core::ffi::{CStr, c_void};

/// Pixel formats of a constant [`ImageDsc`]: uncompressed formats that
/// LVGL draws in place, with their palette for the indexed ones.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PixelFormat {
    /// 8-bit luminance.
    L8,
    /// 1-, 2-, 4- and 8-bit alpha (drawn in the style's color).
    A1,
    A2,
    A4,
    A8,
    /// 1-, 2-, 4- and 8-bit indexes into a palette of `2^bits` ARGB8888
    /// colors stored before the pixels.
    I1,
    I2,
    I4,
    I8,
    Rgb565,
    Argb8565,
    Al88,
    Rgb888,
    Argb8888,
    Xrgb8888,
}

impl PixelFormat {
    const fn color_format(self) -> sys::lv_color_format_t {
        use PixelFormat::*;
        match self {
            L8 => sys::lv_color_format_t_LV_COLOR_FORMAT_L8,
            A1 => sys::lv_color_format_t_LV_COLOR_FORMAT_A1,
            A2 => sys::lv_color_format_t_LV_COLOR_FORMAT_A2,
            A4 => sys::lv_color_format_t_LV_COLOR_FORMAT_A4,
            A8 => sys::lv_color_format_t_LV_COLOR_FORMAT_A8,
            I1 => sys::lv_color_format_t_LV_COLOR_FORMAT_I1,
            I2 => sys::lv_color_format_t_LV_COLOR_FORMAT_I2,
            I4 => sys::lv_color_format_t_LV_COLOR_FORMAT_I4,
            I8 => sys::lv_color_format_t_LV_COLOR_FORMAT_I8,
            Rgb565 => sys::lv_color_format_t_LV_COLOR_FORMAT_RGB565,
            Argb8565 => sys::lv_color_format_t_LV_COLOR_FORMAT_ARGB8565,
            Al88 => sys::lv_color_format_t_LV_COLOR_FORMAT_AL88,
            Rgb888 => sys::lv_color_format_t_LV_COLOR_FORMAT_RGB888,
            Argb8888 => sys::lv_color_format_t_LV_COLOR_FORMAT_ARGB8888,
            Xrgb8888 => sys::lv_color_format_t_LV_COLOR_FORMAT_XRGB8888,
        }
    }

    /// Bits per pixel (per index for the indexed formats).
    pub const fn bits_per_pixel(self) -> u32 {
        use PixelFormat::*;
        match self {
            A1 | I1 => 1,
            A2 | I2 => 2,
            A4 | I4 => 4,
            L8 | A8 | I8 => 8,
            Rgb565 | Al88 => 16,
            Argb8565 | Rgb888 => 24,
            Argb8888 | Xrgb8888 => 32,
        }
    }

    /// Bytes of palette before the pixels (`LV_COLOR_INDEXED_PALETTE_SIZE`
    /// ARGB8888 entries, `lv_bin_decoder.c:582`).
    pub const fn palette_len(self) -> usize {
        use PixelFormat::*;
        match self {
            I1 | I2 | I4 | I8 => 4 << self.bits_per_pixel(),
            _ => 0,
        }
    }
}

/// Image bytes aligned for in-place reads of 16- and 32-bit pixels
/// (`static DATA: ImageData<N> = ImageData(*include_bytes!("icon.bin"));`).
#[repr(C, align(4))]
pub struct ImageData<const N: usize>(pub [u8; N]);

/// An image drawn from constant data (`lv_image_dsc_t`), which may sit in
/// flash: made by [`ImageDsc::new`], which checks the data against the
/// format and size, or by the `unsafe` [`ImageDsc::from_raw`].
#[repr(transparent)]
pub struct ImageDsc(sys::lv_image_dsc_t);

// SAFETY: never written: no flag lets LVGL modify or free it.
unsafe impl Sync for ImageDsc {}
// SAFETY: as above; its pointers are `'static`.
unsafe impl Send for ImageDsc {}

impl ImageDsc {
    /// An image of `width` × `height` pixels in `format`, rows `stride`
    /// bytes apart (`0`: packed rows), stored in `data` after the palette
    /// of indexed formats.
    ///
    /// # Panics
    ///
    /// If `stride` is shorter than a row or `data` is shorter than the
    /// palette and `height` rows (a compile error in a `static`).
    #[cfg(target_endian = "little")]
    pub const fn new<const N: usize>(
        format: PixelFormat,
        width: u16,
        height: u16,
        stride: u16,
        data: &'static ImageData<N>,
    ) -> Self {
        let header = match header(format, width, height, stride, N) {
            Ok(header) => header,
            Err(Layout::ShortStride) => panic!("ImageDsc: stride shorter than a row"),
            Err(Layout::LongRow) => panic!("ImageDsc: row too long"),
            Err(Layout::ShortData) => panic!("ImageDsc: data shorter than the image"),
            Err(Layout::LongData) => panic!("ImageDsc: data too long"),
        };
        Self(sys::lv_image_dsc_t {
            header,
            data_size: N as u32,
            data: data.0.as_ptr(),
            reserved: core::ptr::null(),
            reserved_2: core::ptr::null(),
        })
    }

    /// An image descriptor defined elsewhere (`lv_image_conv` output, an
    /// image crate, ...).
    ///
    /// # Safety
    ///
    /// `dsc` must describe its data correctly (format, size, stride,
    /// `data_size`), suitably aligned, and neither may ever be written
    /// (no `LV_IMAGE_FLAGS_MODIFIABLE` or `ALLOCATED`).
    pub const unsafe fn from_raw(dsc: &'static sys::lv_image_dsc_t) -> &'static ImageDsc {
        // SAFETY: `ImageDsc` is a transparent wrapper.
        unsafe { &*(dsc as *const sys::lv_image_dsc_t).cast::<ImageDsc>() }
    }

    /// The descriptor.
    pub const fn as_ptr(&self) -> *const sys::lv_image_dsc_t {
        &self.0
    }
}

/// Why image data does not match its format and size.
#[derive(Clone, Copy)]
enum Layout {
    ShortStride,
    LongRow,
    ShortData,
    LongData,
}

/// The header of an image of `width` × `height` pixels in `format`, rows
/// `stride` bytes apart (`0`: packed), over `len` bytes of palette and
/// pixels.
#[cfg(target_endian = "little")]
const fn header(
    format: PixelFormat,
    width: u16,
    height: u16,
    stride: u16,
    len: usize,
) -> Result<sys::lv_image_header_t, Layout> {
    let row = (width as usize * format.bits_per_pixel() as usize).div_ceil(8);
    let stride = if stride == 0 { row } else { stride as usize };
    if stride < row {
        return Err(Layout::ShortStride);
    }
    if stride > u16::MAX as usize {
        return Err(Layout::LongRow);
    }
    if format.palette_len() + stride * height as usize > len {
        return Err(Layout::ShortData);
    }
    if len > u32::MAX as usize {
        return Err(Layout::LongData);
    }
    let cf = format.color_format() as u8;
    let [w0, w1] = width.to_le_bytes();
    let [h0, h1] = height.to_le_bytes();
    let [s0, s1] = (stride as u16).to_le_bytes();
    // `lv_image_header_t` (`lv_image_dsc.h`, little-endian layout): magic,
    // cf, flags (none: never modified, compressed or allocated), w, h,
    // stride, reserved. The transmute fails to compile if the header is not
    // 12 bytes.
    let header = [
        sys::LV_IMAGE_HEADER_MAGIC as u8,
        cf,
        0,
        0,
        w0,
        w1,
        h0,
        h1,
        s0,
        s1,
        0,
        0,
    ];
    // SAFETY: twelve bytes in the header's layout; every bit pattern of its
    // bitfields is valid.
    Ok(unsafe { core::mem::transmute::<[u8; 12], sys::lv_image_header_t>(header) })
}

/// What an image shows: a constant image, a file path, or a symbol (a
/// glyph of the style's font). LVGL tells them apart by their first byte
/// (`lv_image_src_get_type`), which the constructors check, and keeps the
/// pointer, so all are `'static`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageSrc(*const c_void);

// SAFETY: points to `'static` data that is never written.
unsafe impl Send for ImageSrc {}
// SAFETY: as above.
unsafe impl Sync for ImageSrc {}

impl ImageSrc {
    /// A constant image.
    pub const fn image(dsc: &'static ImageDsc) -> Self {
        Self(dsc.as_ptr().cast())
    }

    /// A path for LVGL's file systems (`c"A:icons/ok.bin"`): it must start
    /// with a printable ASCII character, or LVGL would read it as another
    /// kind of source. `None` otherwise.
    pub const fn path(path: &'static CStr) -> Option<Self> {
        match first_byte(path) {
            0x20..=0x7F => Some(Self(path.as_ptr().cast())),
            _ => None,
        }
    }

    /// A symbol (`LV_SYMBOL_*`, a private-use UTF-8 character): it must
    /// start with a byte of `0x80` or more. `None` otherwise.
    pub const fn symbol(symbol: &'static CStr) -> Option<Self> {
        match first_byte(symbol) {
            0x80.. => Some(Self(symbol.as_ptr().cast())),
            _ => None,
        }
    }

    /// The source.
    pub const fn as_ptr(self) -> *const c_void {
        self.0
    }
}

impl From<&'static ImageDsc> for ImageSrc {
    fn from(dsc: &'static ImageDsc) -> Self {
        Self::image(dsc)
    }
}

const fn first_byte(s: &CStr) -> u8 {
    match s.to_bytes() {
        [first, ..] => *first,
        [] => 0,
    }
}

/// Colors between `low` and `high` drawn transparent
/// (`lv_image_colorkey_t`, the `image_colorkey` style property).
#[repr(transparent)]
pub struct ImageColorkey(sys::lv_image_colorkey_t);

impl ImageColorkey {
    /// The range from `low` to `high`, both included.
    pub const fn new(low: sys::lv_color_t, high: sys::lv_color_t) -> Self {
        Self(sys::lv_image_colorkey_t { low, high })
    }

    /// The color key.
    pub const fn as_ptr(&self) -> *const sys::lv_image_colorkey_t {
        &self.0
    }
}

#[cfg(all(feature = "heap", lv_use_image))]
pub use owned::{ImageBuffer, OwnedImageSource};

/// Image sources owned by their image (feature `heap`): the value moves
/// into a block of LVGL's heap, which is retired when the image shows
/// something else or is deleted.
#[cfg(all(feature = "heap", lv_use_image))]
mod owned {
    use super::{ImageDsc, PixelFormat, header};
    use crate::{
        Error, ObjMut,
        base::{deferred, ffi},
        enums::EventCode,
        heap::LvBox,
        sys,
        widgets::Image,
    };
    use core::{
        ffi::c_void,
        mem::{align_of, needs_drop},
        ptr,
    };

    /// The box of an owned source: the deferred-free link, then the value.
    type Link = *mut c_void;

    mod sealed {
        pub trait Sealed {}
    }

    /// What [`ObjMut::set_src_owned`] takes: an [`ImageDsc`] (the
    /// descriptor is owned, its data stays `'static`) or an
    /// [`ImageBuffer`] (descriptor and pixels owned). Sealed.
    pub trait OwnedImageSource: sealed::Sealed + Send + 'static {
        /// Finishes the value in its block and returns the source LVGL
        /// keeps: the descriptor, at the start of the value.
        ///
        /// # Safety
        ///
        /// `this` points to the value in its final place.
        #[doc(hidden)]
        unsafe fn source(this: *mut Self) -> *const c_void;
    }

    impl sealed::Sealed for ImageDsc {}
    impl OwnedImageSource for ImageDsc {
        unsafe fn source(this: *mut Self) -> *const c_void {
            this.cast_const().cast()
        }
    }

    /// An image whose descriptor and `N` bytes of palette and pixels live
    /// in its block of LVGL's heap, for small images made at run time.
    /// Checked like [`ImageDsc::new`].
    ///
    /// The value goes through the stack (`N` bytes, possibly more than once
    /// without optimizations) on its way to the heap: for larger images,
    /// [`ObjMut::set_src_buffer`] fills the pixels in their block.
    #[repr(C)]
    pub struct ImageBuffer<const N: usize> {
        /// First: the source LVGL keeps is the block's value.
        dsc: sys::lv_image_dsc_t,
        data: super::ImageData<N>,
    }

    // SAFETY: the descriptor's data pointer is null until the value sits in
    // its block, then points into the same value; nothing else is shared.
    unsafe impl<const N: usize> Send for ImageBuffer<N> {}

    impl<const N: usize> ImageBuffer<N> {
        /// An image of `width` × `height` pixels in `format`, rows `stride`
        /// bytes apart (`0`: packed rows), stored in `data` after the
        /// palette of indexed formats. [`Error::InvalidArg`] if `data` is
        /// shorter than the palette and `height` rows, or `stride` shorter
        /// than a row.
        #[cfg(target_endian = "little")]
        pub fn new(
            format: PixelFormat,
            width: u16,
            height: u16,
            stride: u16,
            data: [u8; N],
        ) -> Result<Self, Error> {
            let header = header(format, width, height, stride, N).map_err(|_| Error::InvalidArg)?;
            Ok(Self {
                dsc: sys::lv_image_dsc_t {
                    header,
                    data_size: N as u32,
                    // Set by `source`, once the value has its address.
                    data: ptr::null(),
                    reserved: ptr::null(),
                    reserved_2: ptr::null(),
                },
                data: super::ImageData(data),
            })
        }
    }

    impl<const N: usize> sealed::Sealed for ImageBuffer<N> {}
    impl<const N: usize> OwnedImageSource for ImageBuffer<N> {
        unsafe fn source(this: *mut Self) -> *const c_void {
            // SAFETY: caller contract: the value no longer moves.
            unsafe { (*this).dsc.data = (&raw const (*this).data).cast() };
            this.cast_const().cast()
        }
    }

    /// The source in a block made by `set_src_owned`: every owned source
    /// aligns like a descriptor, so its value sits at the same offset.
    fn source_of(block: *mut c_void) -> *const c_void {
        LvBox::<Link, sys::lv_image_dsc_t>::value(block)
            .cast_const()
            .cast()
    }

    /// Makes LVGL forget `block`'s source and queues the block for the
    /// next drain.
    ///
    /// # Safety
    ///
    /// Token held; `block` is an owned source no image shows any more (or
    /// whose image is being deleted) and that nothing else retires.
    unsafe fn retire(block: *mut c_void) {
        // SAFETY: the source is still valid: `lv_image_cache_drop` reads it
        // (`lv_image_src_get_type`) and drops the header cache, the image
        // cache entries keyed by its address and the draw units' resources
        // (`lv_image_cache.c:81-96`), so a later block at the same address
        // cannot hit a stale entry.
        unsafe {
            sys::lvgl_sys_image_cache_drop(source_of(block));
            // An owned source needs no drop (checked by `set_src_owned`);
            // the link is the box's header.
            deferred::defer_free(block);
        }
    }

    /// DELETE of an image with an owned source. Runs before the image's
    /// destructor, which never reads a source it did not copy
    /// (`lv_image.c:700-710`); the block is freed at the next drain.
    unsafe extern "C" fn retire_trampoline(e: *mut sys::lv_event_t) {
        // SAFETY: the user data of this descriptor is its block, retired
        // only here or after the descriptor was removed.
        unsafe { retire(sys::lv_event_get_user_data(e)) }
    }

    impl ObjMut<'_, Image> {
        /// Shows `image`, which moves into a block of LVGL's heap (a
        /// pointer-sized header) owned by this image. The block is retired
        /// when a later `set_src_owned` replaces it or the image is
        /// deleted, and freed at the next drain. [`Error::InvalidArg`] if
        /// LVGL refuses the image (it then shows nothing).
        ///
        /// A plain `set_src` keeps the previous block until the next
        /// `set_src_owned` or the deletion.
        pub fn set_src_owned<T: OwnedImageSource>(&mut self, image: T) -> Result<(), Error> {
            let block = LvBox::<Link, T>::new(ptr::null_mut(), image)
                .map_err(|_| Error::OutOfMemory)?
                .into_raw();
            // SAFETY: a block of an owned source, the value in place.
            unsafe { self.install::<T>(block) }
        }

        /// Shows an image of `width` × `height` pixels in `format`, rows
        /// `stride` bytes apart (`0`: packed rows), made in place: the
        /// block (an [`ImageBuffer`] after a pointer-sized header) is
        /// allocated first, zeroed, then `fill` writes the `N` bytes of
        /// palette and pixels into it, so they never go through the stack.
        /// Owned and retired like [`ObjMut::set_src_owned`].
        /// [`Error::InvalidArg`] as for [`ImageBuffer::new`] (before
        /// anything is allocated), or if LVGL refuses the image.
        #[cfg(target_endian = "little")]
        pub fn set_src_buffer<const N: usize>(
            &mut self,
            format: PixelFormat,
            width: u16,
            height: u16,
            stride: u16,
            fill: impl FnOnce(&mut [u8; N]),
        ) -> Result<(), Error> {
            let header = header(format, width, height, stride, N).map_err(|_| Error::InvalidArg)?;
            // SAFETY: zero bytes are a valid `ImageBuffer` (null pointers,
            // zero numbers).
            let block = unsafe { LvBox::<Link, ImageBuffer<N>>::new_zeroed(ptr::null_mut()) }
                .ok_or(Error::OutOfMemory)?
                .into_raw();
            /// Frees the block if `fill` panics.
            struct Unwind(*mut c_void);
            impl Drop for Unwind {
                fn drop(&mut self) {
                    // SAFETY: never given to LVGL; the value needs no drop.
                    unsafe { ffi::free(self.0) };
                }
            }
            let unwind = Unwind(block);
            let buffer = LvBox::<Link, ImageBuffer<N>>::value(block);
            // SAFETY: the zeroed value in its fresh block, borrowed by
            // `fill` alone; then the descriptor is written in place.
            unsafe {
                fill(&mut (*buffer).data.0);
                (*buffer).dsc.header = header;
                (*buffer).dsc.data_size = N as u32;
            }
            core::mem::forget(unwind);
            // SAFETY: a block of an owned source, the value in place.
            unsafe { self.install::<ImageBuffer<N>>(block) }
        }

        /// Shows the owned source in `block`, retiring the ones it
        /// replaces (and `block` itself if LVGL refuses it).
        ///
        /// # Safety
        ///
        /// `block` comes from an `LvBox<Link, T>` with `T` in place and
        /// not yet given to LVGL; it is consumed.
        unsafe fn install<T: OwnedImageSource>(&mut self, block: *mut c_void) -> Result<(), Error> {
            const {
                assert!(!needs_drop::<T>(), "owned sources need no drop");
                assert!(
                    align_of::<T>() == align_of::<sys::lv_image_dsc_t>(),
                    "owned sources align like a descriptor"
                );
            }
            // SAFETY: the value in its block, which never moves.
            let src = unsafe { T::source(LvBox::<Link, T>::value(block)) };
            let o = self.as_raw();
            if let Err(error) = self
                .view()
                .add_handler(EventCode::DELETE, retire_trampoline, block)
            {
                // SAFETY: never given to LVGL.
                unsafe { LvBox::<Link, T>::free(block) };
                return Err(error);
            }
            // SAFETY: exclusive access (`lv_image_set_src` is exclusive);
            // the source lives until the block is retired.
            let current = unsafe {
                sys::lv_image_set_src(o, src);
                sys::lv_image_get_src(o)
            };
            // SAFETY: exclusive access, depth 0: LVGL is not walking the
            // object's handlers, so removing one is allowed.
            unsafe { retire_stale(o, current) };
            if current == src {
                Ok(())
            } else {
                Err(Error::InvalidArg)
            }
        }
    }

    /// Retires every owned source of `o` but `current`: the one replaced,
    /// those a plain `set_src` replaced before, and the new one if LVGL
    /// refused it (`lv_image_set_src` keeps no source it cannot decode,
    /// `lv_image.c:172-190`).
    ///
    /// # Safety
    ///
    /// Exclusive access to the live image `o`.
    unsafe fn retire_stale(o: *mut sys::lv_obj_t, current: *const c_void) {
        // SAFETY: caller contract; indices are walked downwards, so a
        // removal never shifts one still to visit.
        unsafe {
            let mut i = sys::lv_obj_get_event_count(o);
            while i > 0 {
                i -= 1;
                let dsc = sys::lv_obj_get_event_dsc(o, i);
                let ours = sys::lv_event_dsc_get_cb(dsc).is_some_and(|cb| {
                    ptr::fn_addr_eq(cb, retire_trampoline as unsafe extern "C" fn(_))
                });
                if !ours {
                    continue;
                }
                let block = sys::lv_event_dsc_get_user_data(dsc);
                if source_of(block) != current && sys::lv_obj_remove_event(o, i) {
                    retire(block);
                }
            }
        }
    }
}
