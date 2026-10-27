//! The view a creation lends borrows the parent's `ObjMut`: the parent
//! cannot be deleted (or used otherwise) while the child's view is.

use lvgl_rust::{Lvgl, LvglAccess, Obj};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let mut parent = lv.edit(|lv| lv.screen_active()?.child(0)).unwrap();
    let child = Obj::create(&mut parent).unwrap();
    parent.delete().unwrap();
    let _ = child.as_ptr();
}
