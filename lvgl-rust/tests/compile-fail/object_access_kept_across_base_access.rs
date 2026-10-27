//! `ObjMut::into_base` moves the access: a consuming call on the base
//! (`delete`) cannot leave the original `ObjMut` usable.

use lvgl_rust::{Lvgl, LvglAccess, Obj, widgets::Button};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let button = lv
        .edit(|lv| lv.screen_active()?.child(0)?.downcast::<Button>())
        .unwrap();
    button.into_base::<Obj>().delete().unwrap();
    let _ = button.view();
}
