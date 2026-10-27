//! `ObjMut::into_base` only goes to a base class, as `Handle::upcast` does.

use lvgl_rust::{Lvgl, LvglAccess, widgets::Label};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.edit(|lv| lv.screen_active()).unwrap();
    let _label = screen.into_base::<Label>();
}
