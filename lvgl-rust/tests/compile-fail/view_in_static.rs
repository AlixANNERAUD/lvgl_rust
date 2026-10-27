//! Views cannot be stored for later: a `'static` view would outlive every
//! session.

use lvgl_rust::{Lvgl, LvglAccess, Obj};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

struct Keep {
    object: &'static Obj,
}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let _keep = Keep {
        object: lv.screen_active().unwrap(),
    };
}
