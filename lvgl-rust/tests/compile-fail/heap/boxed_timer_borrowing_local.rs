//! `timer_boxed` keeps its closure until the timer is deleted: the closure
//! cannot borrow a local.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let ticks = vec![1u32, 2, 3];
    let _timer = lv.timer_boxed(100, |_| {
        let _ = ticks.len();
    });
}
