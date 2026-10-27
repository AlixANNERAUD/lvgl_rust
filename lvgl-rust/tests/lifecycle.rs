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

    Lvgl::deinit(lv).unwrap();
    assert_eq!(init(&mut guard).err(), Some(Error::NotInitialized));
    assert_eq!(Lvgl::enter(&mut guard).err(), Some(Error::NotInitialized));
    // Dropping an `Owned` after deinit only records a request.
    #[cfg(not(lvgl_rust_views_only))]
    {
        drop(owned);
        let _ = handle;
    }
}
