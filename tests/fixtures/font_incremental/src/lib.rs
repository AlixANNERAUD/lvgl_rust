use lvgl_rust_sys::lv_font_t;

unsafe extern "C" {
    static font_incremental_profile: lv_font_t;
}

pub fn font() -> *const lv_font_t {
    &raw const font_incremental_profile
}
