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
        let row = (width as usize * format.bits_per_pixel() as usize).div_ceil(8);
        let stride = if stride == 0 { row } else { stride as usize };
        assert!(stride >= row, "ImageDsc: stride shorter than a row");
        assert!(stride <= u16::MAX as usize, "ImageDsc: row too long");
        assert!(
            format.palette_len() + stride * height as usize <= N,
            "ImageDsc: data shorter than the image"
        );
        assert!(N <= u32::MAX as usize, "ImageDsc: data too long");
        let cf = format.color_format() as u8;
        let [w0, w1] = width.to_le_bytes();
        let [h0, h1] = height.to_le_bytes();
        let [s0, s1] = (stride as u16).to_le_bytes();
        // `lv_image_header_t` (`lv_image_dsc.h`, little-endian layout):
        // magic, cf, flags (none: never modified, compressed or allocated),
        // w, h, stride, reserved. The transmute fails to compile if the
        // header is not 12 bytes.
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
        Self(sys::lv_image_dsc_t {
            // SAFETY: twelve bytes in the header's layout; every bit
            // pattern of its bitfields is valid.
            header: unsafe { core::mem::transmute::<[u8; 12], sys::lv_image_header_t>(header) },
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
