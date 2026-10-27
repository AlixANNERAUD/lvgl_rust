//! `timer` stores no closure state either: a capturing closure is refused
//! when `timer` is instantiated.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let period = 3u32;
    let _timer = lv.timer(100, (), move |_, ()| {
        let _ = period + 1;
    });
}
