//! A text getter that points into the object borrows its exclusive access:
//! a setter, which may free that text, cannot run while the text is used.

use lvgl_rust::{Lvgl, LvglAccess, widgets::TextArea};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.screen_active().unwrap();
    let _ = TextArea::create(screen).unwrap();
    let mut textarea = lv
        .edit(|lv| lv.screen_active()?.child(-1)?.downcast::<TextArea>())
        .unwrap();
    let text = textarea.get_text();
    textarea.set_text(c"replaced");
    let _ = text;
}
