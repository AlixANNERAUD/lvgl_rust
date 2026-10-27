//! Boxed closures and what they capture cross threads with the token: they
//! must be `Send`.

use lvgl_rust::{Lvgl, LvglAccess, enums::EventCode};
use std::rc::Rc;

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let shared = Rc::new(0u32);
    let _ = lv
        .screen_active()
        .unwrap()
        .on_boxed(EventCode::CLICKED, move |_| {
            let _ = *shared;
        });
}
