//! `on_with` keeps its data until the object is deleted: data borrowing a
//! local is refused (`UserData` is only implemented for owned values).

use lvgl_rust::{Lvgl, LvglAccess, enums::EventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let local = 3u8;
    let _ = lv
        .screen_active()
        .unwrap()
        .on_with(EventCode::CLICKED, &local, |_, _| ());
}
