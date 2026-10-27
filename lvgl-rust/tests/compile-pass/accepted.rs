//! The accepted side of the compile-fail cases: what they change to be
//! refused compiles here. Run without LVGL initialized, so `enter` fails
//! and nothing touches LVGL; building it is the test.

use core::marker::PhantomData;
use lvgl_rust::{Event, Lvgl, LvglAccess, Obj, TimerCtx, enums::EventCode};

struct Guard;
// SAFETY: the only guard of this program.
unsafe impl LvglAccess for Guard {}

/// Zero-sized and `Send`: captures of this kind are accepted (and leaked).
#[derive(Clone, Copy)]
struct Marker(PhantomData<fn()>);

fn count(_: &Event<'_>) {}

fn register(lv: &Lvgl, object: &Obj) {
    let marker = Marker(PhantomData);
    let _ = object.on(EventCode::CLICKED, move |_| {
        let _ = &marker;
    });
    let _ = object.on(EventCode::CLICKED, |event| event.stop_bubbling());
    let _ = object.on_fn(EventCode::CLICKED, count);
    let _ = object.on_with(EventCode::CLICKED, 3u32, |_, data| {
        let _ = data;
    });
    let _ = lv.timer(100, 3u16, |_: &TimerCtx<'_>, data| {
        let _ = data;
    });
    let _ = lv.post(
        |lv, ()| {
            lv.timer_handler();
        },
        (),
    );
}

#[cfg(feature = "async")]
mod queues {
    use lvgl_rust::{Event, StaticQueue};

    fn capture(event: &Event<'_>) -> Option<u32> {
        Some(event.code().bits())
    }

    pub static SMALLEST: StaticQueue<u32, 1> = StaticQueue::new(capture);
    pub static LARGEST: StaticQueue<u32, 255> = StaticQueue::new(capture);
}

fn main() {
    let mut guard = Guard;
    // LVGL is not initialized: `enter` fails before any LVGL call.
    if let Ok(mut lv) = Lvgl::enter(&mut guard) {
        if let Some(screen) = lv.screen_active() {
            register(&lv, screen);
        }
        lv.timer_handler();
    }
    #[cfg(feature = "async")]
    let _ = (&queues::SMALLEST, &queues::LARGEST);
}
