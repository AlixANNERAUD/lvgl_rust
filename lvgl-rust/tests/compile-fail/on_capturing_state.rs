//! `on` stores no closure state (the handler is the closure's type): a
//! closure capturing a value is refused when `on` is instantiated, and
//! `on_with` or `on_static` named instead.

use lvgl_rust::{Lvgl, LvglAccess, enums::EventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let count = 3u32;
    let _ = lv
        .screen_active()
        .unwrap()
        .on(EventCode::CLICKED, move |_| {
            let _ = count + 1;
        });
}
