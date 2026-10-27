//! Two `ObjMut` cannot be alive together: deleting one could delete the
//! other's object.

use lvgl_rust::{Lvgl, LvglAccess, Obj, WidgetExt};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let parent = Obj::create(&mut lv.screen_active_mut().unwrap())
        .unwrap()
        .handle()
        .unwrap();
    let child = Obj::create(&mut parent.get_mut(&mut lv).unwrap())
        .unwrap()
        .handle()
        .unwrap();
    let parent = parent.get_mut(&mut lv).unwrap();
    let mut child = child.get_mut(&mut lv).unwrap();
    parent.delete().unwrap();
    child.clean().unwrap();
}
