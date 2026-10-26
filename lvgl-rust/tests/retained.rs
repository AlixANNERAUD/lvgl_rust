//! Data LVGL keeps pointers to: images, button maps, gradients, transitions.

use lvgl_rust::{
    AnimPath, ButtonMap, ColorFilter, ConstStyle, Error, GradDir, GradDsc, ImageColorkey,
    ImageData, ImageDsc, ImageSrc, Lvgl, LvglAccess, MapEntry, Obj, PixelFormat, Selector, Session,
    Style, TransitionDsc, button_map, color, const_style,
    enums::{ButtonmatrixCtrl, Part},
    style_props, sys,
    widgets::{ButtonMatrix, Image, ImageButton},
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicU32, Ordering},
};

static LOCK: Mutex<bool> = Mutex::new(false);

struct Guard(MutexGuard<'static, bool>);

// SAFETY: a guard of the mutex serializing every test's LVGL calls.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|p| p.into_inner()))
}

fn session(guard: &mut Guard) -> Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        let lv = lvgl_rust::init(guard).unwrap();
        // SAFETY: session held; the buffer is static and only LVGL uses it.
        unsafe {
            let display = sys::lv_display_create(240, 160);
            sys::lv_display_set_flush_cb(display, Some(flush));
            sys::lv_display_set_buffers(
                display,
                core::ptr::addr_of_mut!(BUFFER).cast(),
                core::ptr::null_mut(),
                core::mem::size_of::<Buffer>() as u32,
                sys::lv_display_render_mode_t_LV_DISPLAY_RENDER_MODE_PARTIAL,
            );
        }
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

/// A partial draw buffer: 20 lines of 32-bit pixels.
#[repr(C, align(64))]
struct Buffer([u8; 240 * 20 * 4]);
static mut BUFFER: Buffer = Buffer([0; 240 * 20 * 4]);
static FLUSHES: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" fn flush(display: *mut sys::lv_display_t, _: *const sys::lv_area_t, _: *mut u8) {
    FLUSHES.fetch_add(1, Ordering::Relaxed);
    // SAFETY: called by LVGL for this display.
    unsafe { sys::lv_display_flush_ready(display) };
}

fn last(lv: &Lvgl) -> Option<&Obj> {
    lv.screen_active()?.child(-1)
}

/// 3 × 2 RGB565 pixels, rows padded to 8 bytes.
static PIXELS: ImageData<16> = ImageData([0xff; 16]);
static PICTURE: ImageDsc = ImageDsc::new(PixelFormat::Rgb565, 3, 2, 8, &PIXELS);
/// A 2-color palette (8 bytes) and 2 × 2 one-bit indexes (1 byte a row).
static INDEXED_DATA: ImageData<10> = ImageData([0; 10]);
static INDEXED: ImageDsc = ImageDsc::new(PixelFormat::I1, 2, 2, 0, &INDEXED_DATA);

#[test]
fn images_draw_constant_descriptors_and_checked_sources() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    fn last_image(lv: &Lvgl) -> Option<&Image> {
        last(lv)?.downcast()
    }
    for (dsc, size) in [(&PICTURE, (3, 2)), (&INDEXED, (2, 2))] {
        Image::create(lv.screen_active().unwrap()).unwrap();
        {
            let mut image = lv.edit(last_image).unwrap();
            image.set_src(ImageSrc::image(dsc));
            assert_eq!((image.get_src_width(), image.get_src_height()), size);
        }
        let before = FLUSHES.load(Ordering::Relaxed);
        // SAFETY: session held; renders the active screen.
        unsafe { sys::lv_refr_now(core::ptr::null_mut()) };
        assert!(FLUSHES.load(Ordering::Relaxed) > before);
        lv.edit(last).unwrap().delete().unwrap();
    }
    // LVGL reads a source starting with 0x19 (or 0x00) as a descriptor.
    assert_eq!(ImageSrc::path(c"\x19fake"), None);
    assert_eq!(ImageSrc::path(c""), None);
    assert!(ImageSrc::path(c"A:icon.bin").is_some());
    assert_eq!(ImageSrc::symbol(c"plain"), None);
    let ok = ImageSrc::symbol(c"\u{F00C}").unwrap();
    ImageButton::create(lv.screen_active().unwrap()).unwrap();
    {
        fn last_button(lv: &Lvgl) -> Option<&ImageButton> {
            last(lv)?.downcast()
        }
        let mut button = lv.edit(last_button).unwrap();
        let released = lvgl_rust::enums::ImagebuttonState::RELEASED;
        button.set_src(released, None, Some(PICTURE_SRC), None);
        button.set_src_left(released, Some(ok));
        button.delete().unwrap();
    }
}

static PICTURE_SRC: ImageSrc = ImageSrc::image(&PICTURE);

#[test]
fn image_descriptors_check_their_data() {
    static SHORT: ImageData<11> = ImageData([0; 11]);
    let short = std::panic::catch_unwind(|| ImageDsc::new(PixelFormat::Rgb565, 3, 2, 0, &SHORT));
    assert!(short.is_err(), "12 bytes needed");
    let narrow = std::panic::catch_unwind(|| ImageDsc::new(PixelFormat::Rgb565, 3, 2, 5, &PIXELS));
    assert!(narrow.is_err(), "a row is 6 bytes");
    static NO_PALETTE: ImageData<2> = ImageData([0; 2]);
    let indexed = std::panic::catch_unwind(|| ImageDsc::new(PixelFormat::I1, 2, 2, 0, &NO_PALETTE));
    assert!(indexed.is_err(), "the palette comes first");
}

static KEYPAD: ButtonMap = button_map![c"1", c"2", c"\n", c"3"];

#[test]
fn button_maps_are_terminated_and_control_maps_cover_them() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    assert_eq!(KEYPAD.button_count(), 3);
    let matrix = ButtonMatrix::create(lv.screen_active().unwrap()).unwrap();
    matrix.set_map(&KEYPAD);
    let mut buf = [0u8; 4];
    assert_eq!(matrix.get_button_text_into(2, &mut buf), Ok(Some(c"3")));
    assert_eq!(matrix.get_button_text_into(3, &mut buf), Ok(None));
    let ctrl = [ButtonmatrixCtrl::CHECKABLE; 3];
    assert_eq!(matrix.set_ctrl_map(&ctrl[..2]), Err(Error::InvalidArg));
    matrix.set_ctrl_map(&ctrl).unwrap();
    assert!(matrix.has_button_ctrl(1, ButtonmatrixCtrl::CHECKABLE));
    // An empty text before the end would hide the buttons after it.
    let hidden = std::panic::catch_unwind(|| {
        static ENTRIES: [MapEntry; 3] = [MapEntry::new(c"a"), MapEntry::new(c""), MapEntry::END];
        ButtonMap::new(&ENTRIES)
    });
    assert!(hidden.is_err());
    lv.edit(last).unwrap().delete().unwrap();
}

static GRADIENT: GradDsc = GradDsc::new(
    GradDir::Vertical,
    &[
        sys::lv_grad_stop_t {
            color: color::hex(0xff0000),
            opa: 255,
            frac: 0,
        },
        sys::lv_grad_stop_t {
            color: color::hex(0x0000ff),
            opa: 255,
            frac: 255,
        },
    ],
);
static PRESS: TransitionDsc = TransitionDsc::new(
    style_props![BG_COLOR, BORDER_WIDTH],
    AnimPath::EaseOut,
    200,
    0,
);
static KEY: ImageColorkey = ImageColorkey::new(color::hex(0x00ff00), color::hex(0x00ff00));
static FANCY: ConstStyle = const_style![
    bg_grad(&GRADIENT),
    transition(&PRESS),
    image_colorkey(&KEY),
    color_filter_dsc(ColorFilter::shade()),
    bg_image_src(PICTURE_SRC),
];
static DYNAMIC: Style = Style::new();

#[test]
fn styles_point_to_static_data() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    {
        let mut style = lv.style(&DYNAMIC);
        style.set_bg_grad(Some(&GRADIENT));
        style.set_transition(Some(&PRESS));
        style.set_color_filter_dsc(Some(ColorFilter::shade()));
        style.set_bg_image_src(Some(PICTURE_SRC));
    }
    Obj::create(lv.screen_active().unwrap()).unwrap();
    {
        let mut object = lv.edit(last).unwrap();
        object.add_style(&FANCY, Selector::DEFAULT);
        object.add_style(&DYNAMIC, Part::MAIN);
        object.set_style_bg_grad(Some(&GRADIENT), Selector::DEFAULT);
        object.update_layout();
    }
    // Drawing reads them all.
    let before = FLUSHES.load(Ordering::Relaxed);
    // SAFETY: session held; renders the active screen.
    unsafe { sys::lv_refr_now(core::ptr::null_mut()) };
    assert!(FLUSHES.load(Ordering::Relaxed) > before);
    lv.edit(last).unwrap().delete().unwrap();
    lv.style(&DYNAMIC).reset();
}

#[test]
fn an_open_dropdown_list_follows_its_options() {
    use lvgl_rust::widgets::{Dropdown, Label};
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let index = Dropdown::create(lv.screen_active().unwrap())
        .unwrap()
        .get_index();
    fn dropdown(lv: &Lvgl, index: i32) -> Option<&Dropdown> {
        lv.screen_active()?.child(index)?.downcast()
    }
    dropdown(&lv, index).unwrap().set_options(c"a\nb");
    lv.edit(|lv| dropdown(lv, index)).unwrap().open();
    let check = |lv: &Lvgl, expected: &core::ffi::CStr| {
        let dd = dropdown(lv, index).unwrap();
        let label = dd
            .get_list()
            .unwrap()
            .child(0)
            .unwrap()
            .downcast::<Label>()
            .unwrap();
        let mut buf = [0u8; 64];
        assert_eq!(label.get_text_into(&mut buf), Ok(Some(expected)));
    };
    check(&lv, c"a\nb");
    // The old options are freed: the list label must not keep them.
    dropdown(&lv, index)
        .unwrap()
        .set_options(c"a much longer list\nthat reallocates\nc");
    check(&lv, c"a much longer list\nthat reallocates\nc");
    // SAFETY: session held; lays out and draws the open list.
    unsafe { sys::lv_refr_now(core::ptr::null_mut()) };
    dropdown(&lv, index).unwrap().clear_options();
    check(&lv, c"");
    dropdown(&lv, index).unwrap().set_options_static(c"x\ny");
    check(&lv, c"x\ny");
    // SAFETY: as above.
    unsafe { sys::lv_refr_now(core::ptr::null_mut()) };
    lv.edit(|lv| dropdown(lv, index)).unwrap().delete().unwrap();
}

#[test]
fn textarea_labels_and_degenerate_scales_are_refused() {
    use lvgl_rust::widgets::{Label, Scale, TextArea};
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let textarea = TextArea::create(lv.screen_active().unwrap()).unwrap();
    let label = textarea.get_label().unwrap().downcast::<Label>().unwrap();
    // The textarea edits this text in place: static text would be written.
    assert_eq!(label.set_text_static(Some(c"x")), Err(Error::Internal));
    let other = Label::create(lv.screen_active().unwrap()).unwrap();
    other.set_text_static(Some(c"fine")).unwrap();
    let scale = Scale::create(lv.screen_active().unwrap()).unwrap();
    assert_eq!(scale.set_range(5, 5), Err(Error::InvalidArg));
    scale.set_range(0, 10).unwrap();
    assert_eq!(scale.set_min_value(10), Err(Error::InvalidArg));
    assert_eq!(scale.set_max_value(0), Err(Error::InvalidArg));
    scale.set_max_value(20).unwrap();
    for _ in 0..3 {
        lv.edit(last).unwrap().delete().unwrap();
    }
}

/// Owned sources (`--features heap`): moved to LVGL's heap and retired when
/// replaced or deleted (AddressSanitizer runs this with C `malloc`, so a
/// block freed too early would show as a use after free).
#[cfg(feature = "heap")]
#[test]
fn owned_image_sources_live_until_replaced_or_deleted() {
    use lvgl_rust::ImageBuffer;
    let mut guard = guard();
    let mut lv = session(&mut guard);
    fn last_image(lv: &Lvgl) -> Option<&Image> {
        last(lv)?.downcast()
    }
    fn render() {
        let before = FLUSHES.load(Ordering::Relaxed);
        // SAFETY: session held; redraws the whole active screen.
        unsafe {
            sys::lv_obj_invalidate(sys::lv_screen_active());
            sys::lv_refr_now(core::ptr::null_mut());
        }
        assert!(FLUSHES.load(Ordering::Relaxed) > before);
    }
    fn events(lv: &Lvgl) -> u32 {
        // SAFETY: session held, live object.
        unsafe { sys::lv_obj_get_event_count(last(lv).unwrap().as_ptr()) }
    }
    assert_eq!(
        ImageBuffer::new(PixelFormat::Rgb565, 4, 4, 0, [0; 8]).err(),
        Some(Error::InvalidArg)
    );
    Image::create(lv.screen_active().unwrap()).unwrap();
    let base = events(&lv);
    {
        let mut image = lv.edit(last_image).unwrap();
        let pixels = ImageBuffer::new(PixelFormat::Rgb565, 4, 3, 0, [0x55; 24]).unwrap();
        image.set_src_owned(pixels).unwrap();
        assert_eq!((image.get_src_width(), image.get_src_height()), (4, 3));
    }
    assert_eq!(events(&lv), base + 1);
    render();
    {
        let mut image = lv.edit(last_image).unwrap();
        image
            .set_src_owned(ImageDsc::new(PixelFormat::Rgb565, 3, 2, 8, &PIXELS))
            .unwrap();
        assert_eq!((image.get_src_width(), image.get_src_height()), (3, 2));
    }
    // The replaced block was retired with its handler.
    assert_eq!(events(&lv), base + 1);
    lv.timer_handler();
    render();
    {
        let mut image = lv.edit(last_image).unwrap();
        // A plain source keeps the block until the next owned one.
        image.set_src(ImageSrc::image(&INDEXED));
    }
    assert_eq!(events(&lv), base + 1);
    render();
    {
        let mut image = lv.edit(last_image).unwrap();
        let dot = ImageBuffer::new(PixelFormat::Argb8888, 1, 1, 0, [0xff; 4]).unwrap();
        image.set_src_owned(dot).unwrap();
        assert_eq!((image.get_src_width(), image.get_src_height()), (1, 1));
    }
    assert_eq!(events(&lv), base + 1);
    render();
    lv.edit(last).unwrap().delete().unwrap();
    lv.timer_handler();
    render();
}

/// Blocks in use on LVGL's builtin heap; `None` with the C library's
/// `malloc` (AddressSanitizer), which LVGL does not monitor.
#[cfg(feature = "heap")]
fn used_blocks() -> Option<usize> {
    let mut monitor = core::mem::MaybeUninit::<sys::lv_mem_monitor_t>::zeroed();
    // SAFETY: session held by the caller; LVGL fills the whole struct.
    let monitor = unsafe {
        sys::lv_mem_monitor(monitor.as_mut_ptr());
        monitor.assume_init()
    };
    (monitor.total_size != 0).then_some(monitor.used_cnt)
}

/// A retired owned source is freed by the next drain (entering a session
/// drains without running LVGL's timers), whether it was replaced or its
/// image deleted, and the list is empty afterwards.
#[cfg(feature = "heap")]
#[test]
fn retired_image_sources_are_freed_at_the_next_drain() {
    use lvgl_rust::ImageBuffer;
    let mut guard = guard();
    let mut lv = session(&mut guard);
    fn last_image(lv: &Lvgl) -> Option<&Image> {
        last(lv)?.downcast()
    }
    Image::create(lv.screen_active().unwrap()).unwrap();
    for _ in 0..2 {
        let pixels = ImageBuffer::new(PixelFormat::Rgb565, 4, 3, 0, [0x55; 24]).unwrap();
        lv.edit(last_image).unwrap().set_src_owned(pixels).unwrap();
    }
    // The first block is queued; re-entering frees exactly it.
    let queued = used_blocks();
    drop(lv);
    let lv = session(&mut guard);
    let drained = used_blocks();
    assert_eq!(drained, queued.map(|count| count - 1));
    drop(lv);
    let mut lv = session(&mut guard);
    assert_eq!(used_blocks(), drained);
    // Deleting the image retires the second block.
    lv.edit(last).unwrap().delete().unwrap();
    let queued = used_blocks();
    drop(lv);
    let lv = session(&mut guard);
    assert_eq!(used_blocks(), queued.map(|count| count - 1));
    drop(lv);
}

/// `set_src_buffer` fills a large image in its block: checked before any
/// allocation, drawn, retired like other owned sources.
#[cfg(feature = "heap")]
#[test]
fn image_buffers_are_filled_in_place() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    fn last_image(lv: &Lvgl) -> Option<&Image> {
        last(lv)?.downcast()
    }
    Image::create(lv.screen_active().unwrap()).unwrap();
    let used = used_blocks();
    {
        let mut image = lv.edit(last_image).unwrap();
        // Too few bytes for 64 rows: refused before anything is allocated
        // or `fill` runs.
        assert_eq!(
            image.set_src_buffer::<64>(PixelFormat::Rgb565, 64, 64, 0, |_| unreachable!()),
            Err(Error::InvalidArg)
        );
    }
    assert_eq!(used_blocks(), used);
    {
        let mut image = lv.edit(last_image).unwrap();
        image
            .set_src_buffer::<{ 64 * 64 * 2 }>(PixelFormat::Rgb565, 64, 64, 0, |pixels| {
                assert!(pixels.iter().all(|&byte| byte == 0));
                for (i, byte) in pixels.iter_mut().enumerate() {
                    *byte = i as u8;
                }
            })
            .unwrap();
        assert_eq!((image.get_src_width(), image.get_src_height()), (64, 64));
    }
    // SAFETY: session held; the source is the block's descriptor, whose
    // data points into the same block.
    unsafe {
        let dsc = sys::lv_image_get_src(last(&lv).unwrap().as_ptr()).cast::<sys::lv_image_dsc_t>();
        assert_eq!((*dsc).data_size, 64 * 64 * 2);
        let data = core::slice::from_raw_parts((*dsc).data, 64 * 64 * 2);
        assert!(data.iter().enumerate().all(|(i, &byte)| byte == i as u8));
        sys::lv_obj_invalidate(sys::lv_screen_active());
        sys::lv_refr_now(core::ptr::null_mut());
    }
    lv.edit(last).unwrap().delete().unwrap();
    let queued = used_blocks();
    drop(lv);
    let _lv = session(&mut guard);
    // The next drain freed the block.
    assert_eq!(used_blocks(), queued.map(|count| count - 1));
}
