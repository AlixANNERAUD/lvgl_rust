use lvgl_rust_sys::lv_font_t;

unsafe extern "C" {
    static font_montserrat_14_latin: lv_font_t;
    static font_montserrat_14_cjk: lv_font_t;
}

pub fn latin_font() -> *const lv_font_t {
    &raw const font_montserrat_14_latin
}

pub fn cjk_font() -> *const lv_font_t {
    &raw const font_montserrat_14_cjk
}
