//! A zero-sized capture is accepted (and never dropped), but handlers must be
//! `Send` like the rest of the state that crosses threads with the lock.

use core::marker::PhantomData;
use lvgl_rust::{Lvgl, LvglAccess, enums::EventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

struct NotSend(PhantomData<*mut ()>);

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let marker = NotSend(PhantomData);
    let _ = lv
        .screen_active()
        .unwrap()
        .on(EventCode::CLICKED, move |_| {
            let _ = &marker;
        });
}
