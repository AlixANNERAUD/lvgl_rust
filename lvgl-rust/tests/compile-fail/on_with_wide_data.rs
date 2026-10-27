//! `on_with` data lives in the 4-byte user data of 32-bit targets: a `u64` is
//! refused on 64-bit hosts too, so host builds accept what firmware does.

use lvgl_rust::{Lvgl, LvglAccess, enums::EventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let _ = lv
        .screen_active()
        .unwrap()
        .on_with(EventCode::CLICKED, 3u64, |_, _| ());
}
