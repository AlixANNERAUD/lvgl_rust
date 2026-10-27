//! Calls involving two objects against LVGL.
#![cfg(not(lvgl_rust_views_only))]

use lvgl_rust::{
    Error, Lvgl, LvglAccess, Obj, Session, WidgetExt,
    enums::Align,
    widgets::{Keyboard, MsgBox, TabView, TextArea},
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
        let lv = lvgl_rust::init(guard).unwrap();
        // SAFETY: session held.
        unsafe { lvgl_rust::sys::lv_display_create(240, 160) };
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

#[test]
fn a_keyboard_forgets_a_deleted_textarea() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let keyboard = Keyboard::create(&mut screen)
        .unwrap()
        .into_view()
        .handle()
        .unwrap();
    let textarea = TextArea::create(&mut screen)
        .unwrap()
        .into_view()
        .handle()
        .unwrap();
    {
        let mut kb = keyboard.get_mut(&mut lv).unwrap();
        kb.set_textarea(Some(textarea)).unwrap();
        // Binding again does not stack a second watcher.
        kb.set_textarea(Some(textarea)).unwrap();
    }
    assert!(keyboard.get(&lv).unwrap().get_textarea().is_some());
    assert_eq!(textarea.get(&lv).unwrap().get_event_count(), {
        let mut screen = lv.screen_active_mut().unwrap();
        let other = TextArea::create(&mut screen).unwrap().into_view();
        let count = other.get_event_count() + 1;
        other.delete_later().unwrap();
        count
    });
    textarea
        .upcast::<Obj>()
        .get_mut(&mut lv)
        .unwrap()
        .delete()
        .unwrap();
    // The keyboard no longer points to it (ASan: no read of it).
    assert!(keyboard.get(&lv).unwrap().get_textarea().is_none());
    // A deleted keyboard leaves a harmless watcher on the next textarea.
    let other = TextArea::create(&mut lv.screen_active_mut().unwrap())
        .unwrap()
        .view()
        .handle()
        .unwrap();
    keyboard
        .get_mut(&mut lv)
        .unwrap()
        .set_textarea(Some(other))
        .unwrap();
    keyboard.get_mut(&mut lv).unwrap().delete().unwrap();
    other
        .upcast::<Obj>()
        .get_mut(&mut lv)
        .unwrap()
        .delete()
        .unwrap();
    lv.timer_handler();
}

#[test]
fn exclusive_calls_take_handles_and_lend_views() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let base = Obj::create(&mut screen)
        .unwrap()
        .into_view()
        .handle()
        .unwrap();
    let object = Obj::create(&mut screen)
        .unwrap()
        .into_view()
        .handle()
        .unwrap();
    object
        .get_mut(&mut lv)
        .unwrap()
        .align_to(Some(base), Align::OUT_BOTTOM_MID, 0, 5)
        .unwrap();
    // A dead handle is refused before LVGL is called.
    base.get_mut(&mut lv).unwrap().delete().unwrap();
    assert_eq!(
        object
            .get_mut(&mut lv)
            .unwrap()
            .align_to(Some(base), Align::CENTER, 0, 0),
        Err(Error::Dead)
    );
    object.get_mut(&mut lv).unwrap().delete().unwrap();

    // Objects created by an exclusive call are lent as views.
    let msgbox = MsgBox::create(&mut lv.screen_active_mut().unwrap())
        .unwrap()
        .view()
        .handle()
        .unwrap();
    {
        let mut edit = msgbox.get_mut(&mut lv).unwrap();
        let title = edit.add_title(Some(c"Hello")).unwrap();
        assert!(title.parent().is_some());
    }
    msgbox.get_mut(&mut lv).unwrap().delete().unwrap();
    let tabview = TabView::create(&mut lv.screen_active_mut().unwrap())
        .unwrap()
        .view()
        .handle()
        .unwrap();
    let page = {
        let mut edit = tabview.get_mut(&mut lv).unwrap();
        edit.add_tab(Some(c"One")).unwrap().handle().unwrap()
    };
    // A page takes the application's objects.
    Obj::create(&mut page.get_mut(&mut lv).unwrap()).unwrap();
    tabview.get_mut(&mut lv).unwrap().delete().unwrap();
}
