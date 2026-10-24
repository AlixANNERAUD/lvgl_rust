#![no_std]

/// Raw bindgen output; lints are relaxed here only.
#[allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    unknown_lints,
    unnecessary_transmutes,
    clippy::all,
    clippy::missing_safety_doc
)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

pub use bindings::*;

pub fn _bindgen_raw_src() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/bindings.rs"))
}

pub fn _bindgen_raw_c_header() -> &'static str {
    include_str!("../lvgl/lvgl.h")
}

#[cfg(feature = "use-string-functions")]
mod string_impl;

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// LVGL keeps global state: tests that initialize it must not overlap.
    fn lvgl_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn basic_sanity_check() {
        let _lock = lvgl_lock();
        unsafe {
            lv_init();

            let horizontal_resolution = lv_display_get_horizontal_resolution(core::ptr::null_mut());
            assert_eq!(horizontal_resolution, 0);

            let vertical_resolution = lv_display_get_vertical_resolution(core::ptr::null_mut());
            assert_eq!(vertical_resolution, 0);

            #[cfg(feature = "shadcn-theme")]
            {
                let display = lv_display_create(320, 240);
                let dark = lv_theme_shadcn_palette_zinc_dark();
                let fonts = lv_theme_shadcn_fonts_t::default();
                let theme = lv_theme_shadcn_init(
                    display,
                    dark,
                    &fonts,
                    lv_theme_shadcn_flags_t_LV_THEME_SHADCN_FLAG_NONE,
                );
                assert!(!theme.is_null());
                assert!(lv_theme_shadcn_is_active(display));
                let screen = lv_display_get_screen_active(display);
                let button = lv_button_create(screen);
                assert!(lv_color_eq(
                    lv_obj_get_style_bg_color(button, 0),
                    (*dark).primary
                ));
                let light = lv_theme_shadcn_palette_zinc_light();
                lv_theme_shadcn_set_palette(display, light);
                assert!(lv_color_eq(
                    lv_obj_get_style_bg_color(button, 0),
                    (*light).primary
                ));
                lv_theme_shadcn_deinit(display);
                assert!(!lv_theme_shadcn_is_active(display));
                lv_display_delete(display);
            }
            lv_deinit();
        }
    }

    #[test]
    fn color_shims_round_trip_channels() {
        unsafe {
            let color = _LV_COLOR_MAKE(0x12, 0x34, 0x56);
            assert_eq!(_LV_COLOR_GET_R(color), 0x12);
            assert_eq!(_LV_COLOR_GET_G(color), 0x34);
            assert_eq!(_LV_COLOR_GET_B(color), 0x56);
            assert_eq!(_LV_COLOR_GET_A(color), 0);
        }
    }

    /// The `color_depth_*` features must reach the compiled C library.
    #[test]
    fn display_uses_configured_color_depth() {
        // Mirrors the build script: the smallest enabled depth wins.
        let expected = if cfg!(feature = "color_depth_1") {
            1
        } else if cfg!(feature = "color_depth_8") {
            8
        } else if cfg!(feature = "color_depth_16") {
            16
        } else if cfg!(feature = "color_depth_24") {
            24
        } else if cfg!(feature = "color_depth_32") {
            32
        } else {
            16 // Vendored `lv_conf.h` default.
        };
        let _lock = lvgl_lock();
        unsafe {
            lv_init();
            let display = lv_display_create(8, 8);
            let format = lv_display_get_color_format(display);
            assert_eq!(lv_color_format_get_bpp(format), expected);
            lv_display_delete(display);
            lv_deinit();
        }
    }

    #[test]
    fn obj_is_deleting_is_set_during_delete_event() {
        use core::sync::atomic::{AtomicBool, Ordering};

        static SEEN_DELETING: AtomicBool = AtomicBool::new(false);
        unsafe extern "C" fn on_delete(event: *mut lv_event_t) {
            let target = unsafe { lv_event_get_target_obj(event) };
            SEEN_DELETING.store(
                unsafe { lvgl_sys_obj_is_deleting(target) },
                Ordering::Relaxed,
            );
        }

        let _lock = lvgl_lock();
        unsafe {
            lv_init();
            let display = lv_display_create(8, 8);
            let object = lv_obj_create(lv_display_get_screen_active(display));
            assert!(!lvgl_sys_obj_is_deleting(object));
            lv_obj_add_event_cb(
                object,
                Some(on_delete),
                lv_event_code_t_LV_EVENT_DELETE,
                core::ptr::null_mut(),
            );
            lv_obj_delete(object);
            assert!(SEEN_DELETING.load(Ordering::Relaxed));
            lv_display_delete(display);
        }
    }

    #[test]
    fn menu_parts_are_exposed_and_sidebar_parts_follow_the_sidebar() {
        let _lock = lvgl_lock();
        unsafe {
            lv_init();
            let display = lv_display_create(64, 64);
            let menu = lv_menu_create(lv_display_get_screen_active(display));
            let part = |part| lvgl_sys_menu_part(menu, part);

            let main = part(lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_MAIN);
            let header = part(lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_MAIN_HEADER);
            assert!(!part(lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_STORAGE).is_null());
            assert!(!main.is_null());
            assert_eq!(lv_obj_get_parent(main), menu);
            assert_eq!(lv_obj_get_parent(header), main);
            assert!(!part(lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_MAIN_HEADER_TITLE).is_null());
            assert!(
                !part(lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_MAIN_HEADER_BACK_BUTTON).is_null()
            );

            let sidebar = lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_SIDEBAR;
            assert!(part(sidebar).is_null());
            let page = lv_menu_page_create(menu, core::ptr::null());
            lv_menu_set_sidebar_page(menu, page);
            assert!(!part(sidebar).is_null());
            assert!(!part(lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_SIDEBAR_HEADER).is_null());
            // The sidebar is deleted, but LVGL keeps the stale pointers.
            lv_menu_set_sidebar_page(menu, core::ptr::null_mut());
            assert!(part(sidebar).is_null());
            assert!(part(lvgl_sys_menu_part_t_LVGL_SYS_MENU_PART_SIDEBAR_HEADER_TITLE).is_null());
            assert!(part(99).is_null());

            lv_display_delete(display);
        }
    }

    #[test]
    fn image_cache_drop_is_callable() {
        let _lock = lvgl_lock();
        unsafe {
            lv_init();
            lvgl_sys_image_cache_drop(c"A:missing.png".as_ptr().cast());
            lvgl_sys_image_cache_drop(core::ptr::null());
        }
    }

    #[test]
    fn raw_sources_are_exposed() {
        assert!(_bindgen_raw_src().contains("pub fn lv_init()"));
        assert!(_bindgen_raw_c_header().contains("lvgl"));
    }

    /// Pins the vendored LVGL release. `git submodule status` cannot be trusted
    /// for this: upstream tags such as `v9.6.0` are lightweight, so it reports
    /// the last annotated tag (`v9.3.0-<n>-g<sha>`) instead.
    #[test]
    fn vendored_lvgl_version_matches_readme() {
        let version =
            std::format!("{LVGL_VERSION_MAJOR}.{LVGL_VERSION_MINOR}.{LVGL_VERSION_PATCH}");
        assert_eq!(version, "9.6.0", "update the README and AGENTS.md too");
        let readme = include_str!("../README.md");
        assert!(
            readme.contains(&std::format!("LVGL-{version}-")),
            "README badge does not mention LVGL {version}"
        );
    }
}
