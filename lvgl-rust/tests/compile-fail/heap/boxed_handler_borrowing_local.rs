//! `on_boxed` keeps its closure until the object is deleted: the closure
//! cannot borrow a local.

use lvgl_rust::{Lvgl, LvglAccess, enums::EventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let name = String::from("button");
    let _ = lv
        .screen_active()
        .unwrap()
        .on_boxed(EventCode::CLICKED, |_| {
            let _ = name.len();
        });
}
