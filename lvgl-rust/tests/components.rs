//! A component built from widgets, handlers and handles.
#![cfg(not(lvgl_rust_views_only))]

use lvgl_rust::{
    Component, Error, Handle, Lvgl, LvglAccess, Obj, Session, WidgetExt,
    enums::EventCode,
    widgets::{Button, Label},
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

/// A button counting its clicks in a label.
struct Counter {
    button: Handle<Button>,
    label: Handle<Label>,
}

impl Counter {
    fn build(parent: Handle<Obj>, lv: &mut Lvgl) -> Result<Self, Error> {
        let button = Button::create(&mut parent.get_mut(lv)?)?.handle()?;
        let mut edit = button.get_mut(lv)?;
        let label = Label::create(&mut edit)?;
        label.set_text_i32(0);
        let label = label.handle()?;
        let button = button.get(lv)?;
        button.on_with(EventCode::CLICKED, label, |e, label| {
            if let Ok(label) = label.get(e.lv()) {
                let mut buf = [0u8; 12];
                let n: i32 = label
                    .get_text_into(&mut buf)
                    .ok()
                    .flatten()
                    .and_then(|t| t.to_str().ok()?.parse().ok())
                    .unwrap_or(0);
                label.set_text_i32(n + 1);
            }
        })?;
        Ok(Counter {
            button: button.handle()?,
            label,
        })
    }
}

impl Component for Counter {
    type Root = Button;
    fn root(&self) -> Handle<Button> {
        self.button
    }
}

#[test]
fn a_component_counts_clicks_and_goes_with_its_root() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let counter = Counter::build(lv.screen_active().unwrap().handle().unwrap(), &mut lv).unwrap();
    for _ in 0..3 {
        let button = counter.root().get(&lv).unwrap();
        // SAFETY: session held; CLICKED takes no param.
        unsafe {
            lvgl_rust::sys::lv_obj_send_event(
                button.as_ptr(),
                EventCode::CLICKED.bits() as _,
                core::ptr::null_mut(),
            )
        };
    }
    let mut buf = [0u8; 12];
    let label = counter.label.get(&lv).unwrap();
    assert_eq!(label.get_text_into(&mut buf), Ok(Some(c"3")));
    let label = counter.label;
    counter.delete(&mut lv).unwrap();
    assert_eq!(label.get(&lv).err(), Some(Error::Dead));
}
