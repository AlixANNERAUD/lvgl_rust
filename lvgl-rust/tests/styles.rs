//! Styles and fonts against LVGL.

use lvgl_rust::{
    ConstStyle, Font, Lvgl, LvglAccess, Obj, Selector, Session, Style, color, const_style,
    enums::{Align, Part, State},
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
        init_display(guard)
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

fn init_display(guard: &mut Guard) -> Session<'_> {
    let lv = lvgl_rust::init(guard).unwrap();
    // SAFETY: session held.
    unsafe { lvgl_rust::sys::lv_display_create(240, 160) };
    lv
}

fn last(lv: &Lvgl) -> Option<&Obj> {
    lv.screen_active()?.child(-1)
}

static RED: ConstStyle = const_style![
    bg_color(color::hex(0xff0000)),
    radius(23),
    align(Align::CENTER),
];

#[test]
fn constant_styles_apply_from_rodata() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    lv.edit(last).unwrap().add_style(&RED, Selector::DEFAULT);
    let object = last(&lv).unwrap();
    let bg = object.get_style_bg_color(Part::MAIN);
    assert_eq!((bg.red, bg.green, bg.blue), (0xff, 0, 0));
    assert_eq!(object.get_style_radius(Part::MAIN), 23);
    assert_eq!(object.get_style_align(Part::MAIN), Align::CENTER);
    assert!(lv.style_is_const(&RED));
    assert!(!lv.style_is_empty(&RED));
    {
        let mut object = lv.edit(last).unwrap();
        object.remove_style(Some((&RED).into()), Selector::DEFAULT);
    }
    assert_ne!(last(&lv).unwrap().get_style_radius(Part::MAIN), 23);
    // Only in the checked state.
    {
        let mut object = lv.edit(last).unwrap();
        object.add_style(&RED, Part::MAIN | State::CHECKED);
    }
    assert_ne!(last(&lv).unwrap().get_style_radius(Part::MAIN), 23);
    lv.edit(last).unwrap().add_state(State::CHECKED);
    assert_eq!(last(&lv).unwrap().get_style_radius(Part::MAIN), 23);
    lv.edit(last).unwrap().delete().unwrap();
}

static TEXT: ConstStyle = const_style![text_font(Font::montserrat_14())];

static CARD: Style = Style::new();

#[test]
fn mutable_styles_refresh_their_objects() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    assert!(lv.style_is_empty(&CARD));
    lv.style(&CARD).set_radius(3);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    lv.edit(last).unwrap().add_style(&CARD, Selector::DEFAULT);
    assert_eq!(last(&lv).unwrap().get_style_radius(Part::MAIN), 3);
    {
        let mut card = lv.style(&CARD);
        card.set_radius(9);
        card.set_bg_color(color::rgb(0, 0x80, 0));
    } // reported to the objects here
    let object = last(&lv).unwrap();
    assert_eq!(object.get_style_radius(Part::MAIN), 9);
    assert_eq!(object.get_style_bg_color(Part::MAIN).green, 0x80);
    // Copying a constant style replaces every property.
    lv.style(&CARD).copy(&RED);
    assert_eq!(last(&lv).unwrap().get_style_radius(Part::MAIN), 23);
    // Copying a style onto itself keeps it valid (empty: LVGL resets the
    // destination first).
    lv.style(&CARD).copy(&CARD);
    assert!(lv.style_is_empty(&CARD));
    lv.style(&CARD).reset();
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn fonts_are_static() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let font: &'static Font = Font::montserrat_14();
    assert!(lv.font_get_line_height(font) > 0);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    lv.edit(last)
        .unwrap()
        .set_style_text_font(font, Selector::DEFAULT);
    let got = last(&lv).unwrap().get_style_text_font(Part::MAIN).unwrap();
    assert_eq!(got.as_ptr(), font.as_ptr());
    static FONTED: Style = Style::new();
    lv.style(&FONTED).set_text_font(font);
    lv.edit(last).unwrap().add_style(&TEXT, Part::MAIN);
    let got = last(&lv).unwrap().get_style_text_font(Part::MAIN).unwrap();
    assert_eq!(got.as_ptr(), font.as_ptr());
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn constant_style_terminator_is_checked() {
    let unterminated = std::panic::catch_unwind(|| {
        static PROPS: [lvgl_rust::ConstProp; 1] = [lvgl_rust::props::radius(1)];
        ConstStyle::new(&PROPS)
    });
    assert!(unterminated.is_err());
}
