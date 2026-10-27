//! Timer callbacks must be `Send`, zero-sized captures included.

use core::marker::PhantomData;
use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

struct NotSend(PhantomData<*mut ()>);

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let marker = NotSend(PhantomData);
    let _timer = lv.timer(100, (), move |_, ()| {
        let _ = &marker;
    });
}
