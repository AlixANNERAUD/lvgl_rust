//! Posted work runs at the next drain, maybe on the thread holding the lock
//! then: what `post` is given must be `Send`.

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
    let _ = lv.post(
        move |_, ()| {
            let _ = &marker;
        },
        (),
    );
}
