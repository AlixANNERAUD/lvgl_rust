//! The generated wrappers against LVGL.

use lvgl_rust::{
    Lvgl, LvglAccess, Obj, Session,
    enums::{Align, LabelLongMode, State},
    functions, init,
    widgets::{Bar, Button, Dropdown, Label, Slider, Switch},
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
    let mut buf = [0u8; 16];
    assert_eq!(label.get_text_into(&mut buf), Ok(Some(c"temporary")));
    label.set_text_static(Some(c"static")).unwrap();
    drop(lv);
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    // Deletes the label's animations, which other frames may be running:
    // exclusive access.
    lv.edit(|lv| lv.screen_active()?.child(-1)?.downcast::<Label>())
        .unwrap()
        .set_long_mode(LabelLongMode::DOTS);
    let label = lv.screen_active().unwrap().child(-1).unwrap();
    assert_eq!(
        label.downcast::<Label>().unwrap().get_long_mode(),
        LabelLongMode::DOTS
    );
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
    {
        // Value setters may run animation callbacks: exclusive access.
        let mut edit = slider.get_mut(&mut lv).unwrap();
        edit.set_range(0, 100);
        edit.set_value(42, false);
    }
    let view = slider.get(&lv).unwrap();
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
    Slider::create(lv.screen_active().unwrap()).unwrap();
    fn last_slider(lv: &Lvgl) -> Option<&Slider> {
        lv.screen_active()?.child(-1)?.downcast()
    }
    {
        let mut edit = lv.edit(last_slider).unwrap();
        edit.set_range(0, 10);
        edit.set_value(7, false);
    }
    let bar: &Bar = last_slider(&lv).unwrap();
    assert_eq!(bar.get_value(), 7);
    let object = lv.screen_active().unwrap().child(-1).unwrap();
    assert!(object.downcast::<Bar>().is_some() && object.downcast::<Button>().is_none());
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}

/// Geometry, flags and states can reach LVGL code that shrinks children
/// arrays (through events), so they need exclusive access.
#[test]
fn objects_expose_geometry_flags_and_states() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(lv.screen_active().unwrap()).unwrap();
    fn last(lv: &Lvgl) -> Option<&Obj> {
        lv.screen_active()?.child(-1)
    }
    {
        let mut object = lv.edit(last).unwrap();
        object.set_size(40, 20);
        object.align(Align::CENTER, 0, 0);
        object.update_layout();
        object.set_hidden(true);
        object.add_state(State::CHECKED | State::FOCUSED);
        object.remove_state(State::CHECKED);
    }
    let object = last(&lv).unwrap();
    assert_eq!((object.get_width(), object.get_height()), (40, 20));
    assert_eq!(object.get_x(), (240 - 40) / 2);
    assert!(object.is_hidden());
    assert!(object.has_state(State::FOCUSED) && !object.has_state(State::CHECKED));
    assert_eq!(object.get_state(), State::FOCUSED);
    let screen = lv.screen_active().unwrap();
    assert_eq!(object.get_screen().unwrap().as_ptr(), screen.as_ptr());
    let switch = Switch::create(object).unwrap();
    assert_eq!(object.get_child_count(), 1);
    assert_eq!(switch.get_index(), 0);
    lv.edit(last).unwrap().delete().unwrap();
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

#[test]
fn text_is_copied_out_of_views_and_borrowed_under_exclusive_access() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let label = Label::create(lv.screen_active().unwrap()).unwrap();
    label.set_text(Some(c"hello"));
    let mut buf = [0u8; 8];
    assert_eq!(label.get_text_into(&mut buf), Ok(Some(c"hello")));
    let mut small = [0u8; 5]; // no room for the NUL
    assert_eq!(
        label.get_text_into(&mut small),
        Err(lvgl_rust::Error::TooSmall)
    );
    {
        fn last_label(lv: &Lvgl) -> Option<&Label> {
            lv.screen_active()?.child(-1)?.downcast()
        }
        let mut edit = lv.edit(last_label).unwrap();
        assert_eq!(edit.get_text(), Some(c"hello"));
    }
    // NULL comes back as `None`: a dropdown has no fixed text by default.
    let dropdown = Dropdown::create(lv.screen_active().unwrap()).unwrap();
    assert_eq!(dropdown.get_text_into(&mut buf), Ok(None));
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}

#[test]
fn labels_take_str_and_numbers() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let label = Label::create(lv.screen_active().unwrap()).unwrap();
    let mut buf = [0u8; 128];
    let mut text = |label: &Label| {
        label
            .get_text_into(&mut buf)
            .unwrap()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    };
    // Multi-byte characters across the 31-byte chunk boundaries.
    let long = "héllo wörld, ça va très bien — 日本語のテキスト 🎉🎉 fin";
    label.set_text_str(long).unwrap();
    assert_eq!(text(label), long);
    label.set_text_str("").unwrap();
    assert_eq!(text(label), "");
    assert_eq!(
        label.set_text_str("a\0b"),
        Err(lvgl_rust::Error::InvalidArg)
    );
    // Replaces static text too.
    label.set_text_static(Some(c"static")).unwrap();
    label.set_text_str("dynamic").unwrap();
    assert_eq!(text(label), "dynamic");
    label.set_text_i32(i32::MIN);
    assert_eq!(text(label), "-2147483648");
    label.set_text_u32(u32::MAX);
    assert_eq!(text(label), "4294967295");
    label.set_text_i32(0);
    assert_eq!(text(label), "0");
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}

/// Composite deletion paths, for ASan (`mise run test-asan`): an open
/// dropdown deletes the list it put on the top layer, a closed message box
/// deletes its parts (and its backdrop when it has one).
#[test]
fn composites_free_their_parts_when_deleted() {
    use lvgl_rust::widgets::MsgBox;
    let mut guard = guard();
    let mut lv = session(&mut guard);
    // The dropdown puts its list on the screen, after itself.
    let index = Dropdown::create(lv.screen_active().unwrap())
        .unwrap()
        .get_index();
    fn dropdown(lv: &Lvgl, index: i32) -> Option<&Dropdown> {
        lv.screen_active()?.child(index)?.downcast()
    }
    let children = lv.screen_active().unwrap().get_child_count();
    lv.edit(|lv| dropdown(lv, index)).unwrap().open();
    assert!(dropdown(&lv, index).unwrap().get_list().is_some());
    lv.edit(|lv| dropdown(lv, index)).unwrap().delete().unwrap();
    lv.timer_handler();
    assert_eq!(lv.screen_active().unwrap().get_child_count(), children - 2);

    fn last_msgbox(lv: &Lvgl) -> Option<&MsgBox> {
        lv.screen_active()?.child(-1)?.downcast()
    }
    MsgBox::create(lv.screen_active().unwrap()).unwrap();
    // SAFETY: session held, live message box: adds a header and footer.
    unsafe {
        let mbox = last_msgbox(&lv).unwrap().as_ptr();
        lvgl_rust::sys::lv_msgbox_add_title(mbox, c"Title".as_ptr());
        lvgl_rust::sys::lv_msgbox_add_close_button(mbox);
        lvgl_rust::sys::lv_msgbox_add_footer_button(mbox, c"OK".as_ptr());
    }
    let children = lv.screen_active().unwrap().get_child_count();
    lv.edit(last_msgbox).unwrap().close();
    lv.timer_handler();
    assert_eq!(lv.screen_active().unwrap().get_child_count(), children - 1);
}

#[cfg(feature = "fmt")]
#[test]
fn labels_take_formatted_text() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let label = Label::create(lv.screen_active().unwrap()).unwrap();
    let value = 3.5f32;
    label
        .set_text_fmt(format_args!("{} élans, {value:.2} — {:>8}|", 42, "日本"))
        .unwrap();
    let mut buf = [0u8; 64];
    let text = label
        .get_text_into(&mut buf)
        .unwrap()
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(text, "42 élans, 3.50 —       日本|");
    assert_eq!(
        label.set_text_fmt(format_args!("a{}b", '\0')),
        Err(lvgl_rust::Error::InvalidArg)
    );
    assert_eq!(lvgl_rust::Error::Dead.to_string(), "the object was deleted");
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}
