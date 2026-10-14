#![no_std]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::redundant_static_lifetimes)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

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
    use super::*;

    #[test]
    fn basic_sanity_check() {
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
}
