//! `set_text_static` keeps the pointer it is given: the text must be
//! `'static`.

use core::ffi::CStr;
use lvgl_rust::{Lvgl, LvglAccess, widgets::Label};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let mut screen = lv.screen_active_mut().unwrap();
    let label = Label::create(&mut screen).unwrap();
    let text = *b"local\0";
    let _ = label.set_text_static(Some(CStr::from_bytes_with_nul(&text).unwrap()));
}
