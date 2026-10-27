//! Session lifecycle, in its own process because `deinit` is terminal.

use lvgl_rust::{Error, Lvgl, LvglAccess, Obj, init, sys, tick_inc};

struct Guard;

// SAFETY: this test is single-threaded and holds no other guard.
unsafe impl LvglAccess for Guard {}

#[test]
fn init_enter_deinit() {
    let mut guard = Guard;
    assert_eq!(Lvgl::enter(&mut guard).err(), Some(Error::NotInitialized));

    let mut lv = init(&mut guard).unwrap();
    // The crate's tick source feeds LVGL's clock.
    // SAFETY: session held.
    let start = unsafe { sys::lv_tick_get() };
    tick_inc(25);
    assert!(unsafe { sys::lv_tick_get() }.wrapping_sub(start) >= 25);

    // SAFETY: session held.
    unsafe { sys::lv_display_create(32, 32) };
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap();
    #[cfg(not(lvgl_rust_views_only))]
    let handle = {
        use lvgl_rust::WidgetExt;
        let handle = object.handle().unwrap();
        let mut buf = [0u8; 8];
        // SAFETY: live object, buffer of 8 bytes.
        let text = unsafe {
            let p = sys::lv_obj_stringify_id(object.as_ptr(), buf.as_mut_ptr().cast(), 8);
            core::ffi::CStr::from_ptr(p)
        };
        assert!(text.to_bytes().starts_with(b"#"));
        handle
    };
    let _ = object;
    #[cfg(not(lvgl_rust_views_only))]
    let owned = lv.create_screen().unwrap();
    // Owned sources retired during `deinit` (after its last drain) are
    // freed at once, while LVGL's heap still exists.
    #[cfg(feature = "heap")]
    {
        use lvgl_rust::{ImageBuffer, PixelFormat, widgets::Image};
        let raw = Image::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .as_ptr();
        // Heap blocks in use around the handler retiring the source.
        // SAFETY: session held, live object; the callbacks need no data.
        unsafe { sys::lv_obj_add_event_cb(raw, Some(count_before), DELETE, core::ptr::null_mut()) };
        let mut image = lv
            .edit(|lv| screen_of(lv).child(-1)?.downcast::<Image>())
            .unwrap();
        let dot = ImageBuffer::new(PixelFormat::Argb8888, 1, 1, 0, [0xff; 4]).unwrap();
        image.set_src_owned(dot).unwrap();
        // SAFETY: as above.
        unsafe { sys::lv_obj_add_event_cb(raw, Some(count_after), DELETE, core::ptr::null_mut()) };
    }

    Lvgl::deinit(lv).unwrap();
    // The block was freed during the deletion, not queued (the builtin
    // heap only: LVGL does not monitor the C library's `malloc`).
    #[cfg(feature = "heap")]
    {
        use std::sync::atomic::Ordering::Relaxed;
        let (before, after) = (BEFORE.load(Relaxed), AFTER.load(Relaxed));
        assert!(before != usize::MAX && after != usize::MAX);
        if before != 0 {
            assert_eq!(after, before - 1);
        }
    }
    assert_eq!(init(&mut guard).err(), Some(Error::NotInitialized));
    assert_eq!(Lvgl::enter(&mut guard).err(), Some(Error::NotInitialized));
    // Dropping an `Owned` after deinit only records a request.
    #[cfg(not(lvgl_rust_views_only))]
    {
        drop(owned);
        let _ = handle;
    }
}

#[cfg(feature = "heap")]
fn screen_of(lv: &Lvgl) -> &Obj {
    lv.screen_active().unwrap()
}

#[cfg(feature = "heap")]
const DELETE: sys::lv_event_code_t = sys::lv_event_code_t_LV_EVENT_DELETE;

#[cfg(feature = "heap")]
static BEFORE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
#[cfg(feature = "heap")]
static AFTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// Blocks in use on LVGL's builtin heap (0 with the C library's `malloc`).
#[cfg(feature = "heap")]
fn used_blocks() -> usize {
    let mut monitor = core::mem::MaybeUninit::<sys::lv_mem_monitor_t>::zeroed();
    // SAFETY: under the session; LVGL fills the whole struct.
    let monitor = unsafe {
        sys::lv_mem_monitor(monitor.as_mut_ptr());
        monitor.assume_init()
    };
    if monitor.total_size == 0 {
        0
    } else {
        monitor.used_cnt
    }
}

#[cfg(feature = "heap")]
unsafe extern "C" fn count_before(_: *mut sys::lv_event_t) {
    BEFORE.store(used_blocks(), std::sync::atomic::Ordering::Relaxed);
}

#[cfg(feature = "heap")]
unsafe extern "C" fn count_after(_: *mut sys::lv_event_t) {
    AFTER.store(used_blocks(), std::sync::atomic::Ordering::Relaxed);
}
