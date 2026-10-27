//! `on_static` contexts are shared by every handler call, so they must be
//! `Sync`.

use core::cell::Cell;
use lvgl_rust::Handler;

static HANDLER: Handler<Cell<u32>> = Handler {
    ctx: Cell::new(0),
    f: |_, ctx| ctx.set(ctx.get() + 1),
};

fn main() {
    let _ = &HANDLER;
}
