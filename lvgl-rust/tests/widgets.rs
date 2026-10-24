//! The generated wrappers against LVGL.

use lvgl_rust::{
    Lvgl, LvglAccess, Obj, Session,
    enums::{Align, LabelLongMode, State},
    functions, init,
    widgets::{Bar, Button, Label, Slider, Switch},
};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<bool> = Mutex::new(false);

struct Guard(MutexGuard<'static, bool>);

// SAFETY: a guard of the mutex serializing every test's LVGL calls.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|p| p.into_inner()))
}

fn session(guard: &mut Guard) -> Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        let lv = init(guard).unwrap();
        // SAFETY: session held.
        unsafe { lvgl_rust::sys::lv_display_create(240, 160) };
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

#[test]
fn labels_take_copied_and_static_text() {
    let mut guard = guard();
    let lv = session(&mut guard);
    let label = Label::create(lv.screen_active().unwrap()).unwrap();
    {
        let text = std::ffi::CString::new("temporary").unwrap();
        label.set_text(Some(&text));
    } // freed: LVGL keeps its own copy
    // SAFETY: session held, live label.
    let stored =
        unsafe { core::ffi::CStr::from_ptr(lvgl_rust::sys::lv_label_get_text(label.as_ptr())) };
    assert_eq!(stored, c"temporary");
    label.set_text_static(Some(c"static"));
    label.set_long_mode(LabelLongMode::DOTS);
    assert_eq!(label.get_long_mode(), LabelLongMode::DOTS);
    drop(lv);
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}

#[test]
#[cfg(not(lvgl_rust_views_only))]
fn sliders_are_bars() {
    use lvgl_rust::WidgetExt;
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let slider = Slider::create_owned(lv.screen_active().unwrap()).unwrap();
    let view = slider.get(&lv).unwrap();
    view.set_range(0, 100);
    view.set_value(42, false);
    assert_eq!(view.get_value(), 42);
    // Bar methods through `Deref`.
    let bar: &Bar = view;
    assert_eq!(bar.get_max_value(), 100);
    assert!(!view.is_dragged());
    // Handles keep the class and check it on the way down.
    let as_bar = slider.handle().upcast::<Bar>();
    let as_obj = slider.handle().upcast::<Obj>();
    assert_eq!(as_obj.downcast::<Slider>(&lv), Some(slider.handle()));
    assert_eq!(as_bar.get(&lv).unwrap().get_value(), 42);
    let button = Button::create(lv.screen_active().unwrap())
        .unwrap()
        .handle()
        .unwrap();
    assert!(button.upcast::<Obj>().downcast::<Slider>(&lv).is_none());
    assert!(
        lv.screen_active()
            .unwrap()
            .child(-1)
            .unwrap()
            .downcast::<Button>()
            .is_some()
    );
    button.get_mut(&mut lv).unwrap().delete().unwrap();
    slider.delete(&mut lv).unwrap();
}

#[test]
fn slider_views_are_bar_views() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let slider = Slider::create(lv.screen_active().unwrap()).unwrap();
    slider.set_range(0, 10);
    slider.set_value(7, false);
    let bar: &Bar = slider;
    assert_eq!(bar.get_value(), 7);
    let object = lv.screen_active().unwrap().child(-1).unwrap();
    assert!(object.downcast::<Bar>().is_some() && object.downcast::<Button>().is_none());
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}

#[test]
fn objects_expose_geometry_flags_and_states() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let screen = lv.screen_active().unwrap();
    let object = Obj::create(screen).unwrap();
    object.set_size(40, 20);
    object.align(Align::CENTER, 0, 0);
    object.update_layout();
    assert_eq!((object.get_width(), object.get_height()), (40, 20));
    assert_eq!(object.get_x(), (240 - 40) / 2);
    object.set_hidden(true);
    assert!(object.is_hidden());
    object.add_state(State::CHECKED | State::FOCUSED);
    assert!(object.has_state(State::CHECKED));
    object.remove_state(State::CHECKED);
    assert_eq!(object.get_state(), State::FOCUSED);
    assert_eq!(object.get_screen().unwrap().as_ptr(), screen.as_ptr());
    let switch = Switch::create(object).unwrap();
    assert_eq!(object.get_child_count(), 1);
    assert_eq!(switch.get_index(), 0);
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}

#[test]
fn pure_functions_need_no_token() {
    let red = functions::color_hex(0xff0000);
    let made = functions::color_make(0xff, 0, 0);
    assert_eq!(
        (red.red, red.green, red.blue),
        (made.red, made.green, made.blue)
    );
    assert!(functions::pct(50) != 50);
}
