//! Real LVGL coverage for class descriptors owned by an embedding host.

use core::{mem::MaybeUninit, ptr::NonNull};
use lvgl_rust::{
    Error, ExternalClassPolicy, Lvgl, LvglAccess, Obj, ScreenAnim, ScreenLoadAnim, WidgetExt, sys,
    tick_inc, widgets::MsgBox,
};

// Only the test below accesses these descriptors, under its LVGL session.
static mut WINDOW: MaybeUninit<sys::lv_obj_class_t> = MaybeUninit::uninit();
static mut UNKNOWN: MaybeUninit<sys::lv_obj_class_t> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
fn lvgl_rust_external_class_policy(class: *const sys::lv_obj_class_t) -> ExternalClassPolicy {
    if class == (&raw const WINDOW).cast() {
        ExternalClassPolicy::ChildrenOnly
    } else {
        ExternalClassPolicy::Protected
    }
}

struct Guard;

// SAFETY: this binary has one test and one guard, held for all calls to LVGL.
unsafe impl LvglAccess for Guard {}

#[test]
fn external_objects_keep_their_host_ownership() {
    let mut guard = Guard;
    let mut lv = lvgl_rust::init(&mut guard).unwrap();

    // This fixture deliberately requires private API bindings for construction.
    // The production policy only compares class pointers.
    unsafe {
        (&raw mut WINDOW).write(MaybeUninit::new(sys::lv_obj_class_t {
            base_class: &raw const sys::lv_obj_class,
            ..Default::default()
        }));
        (&raw mut UNKNOWN).write(MaybeUninit::new(sys::lv_obj_class_t {
            // Inheriting a known widget must not bypass external protection.
            base_class: &raw const sys::lv_button_class,
            ..Default::default()
        }));
        sys::lv_display_create(240, 160);
        let screen = sys::lv_screen_active();
        create_external((&raw const WINDOW).cast(), screen);
        let container = sys::lv_obj_create(screen);
        create_external((&raw const WINDOW).cast(), container);
        let unknown = create_external((&raw const UNKNOWN).cast(), screen);
        sys::lv_obj_create(unknown);
    }

    let top: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(0);
    let container: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(1);
    let nested: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(1)?.child(0);
    let unknown: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(2);

    assert_protected(&mut lv, top);
    assert_protected(&mut lv, nested);
    assert_protected(&mut lv, unknown);
    assert_protected(&mut lv, container);
    assert_eq!(lv.edit(container).unwrap().clean(), Err(Error::Internal));
    assert_eq!(
        lv.screen_active_mut().unwrap().clean(),
        Err(Error::Internal)
    );
    assert_eq!(lv.edit(unknown).unwrap().clean(), Err(Error::Internal));
    assert_eq!(
        Obj::create(&mut lv.edit(unknown).unwrap()).err(),
        Some(Error::Internal),
    );

    {
        let mut window = lv.edit(top).unwrap();
        Obj::create(&mut window).unwrap();
        Obj::create(&mut window).unwrap();
    }
    let content: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(0)?.child(1);
    let owned = content(&lv).unwrap().own().unwrap();
    lv.edit(content).unwrap().move_to_index(0).unwrap();
    owned
        .get_mut(&mut lv)
        .unwrap()
        .set_parent_to(Lvgl::screen_active)
        .unwrap();
    owned.delete(&mut lv).unwrap();

    lv.edit(top).unwrap().clean().unwrap();
    assert!(top(&lv).unwrap().child(0).is_none());

    assert_nested_windows(&mut lv);
    assert_message_box_close(&mut lv);
    assert_pending_screen_loads(&mut lv);

    let screen = lv.create_screen().unwrap();
    let auto_delete = ScreenAnim::new(ScreenLoadAnim::NONE, 0, 0, true);
    assert_eq!(
        lv.load_screen(screen.handle(), auto_delete),
        Err(Error::Internal)
    );
    lv.load_screen(screen.handle(), ScreenAnim::NONE).unwrap();

    Lvgl::deinit(lv).unwrap();
}

fn assert_protected(lv: &mut Lvgl, object: impl for<'a> Fn(&'a Lvgl) -> Option<&'a Obj>) {
    assert_eq!(object(lv).unwrap().own().unwrap_err(), Error::Internal);
    let mut edit = lv.edit(&object).unwrap();
    assert_eq!(edit.move_to_index(0), Err(Error::Internal));
    assert_eq!(
        edit.set_parent_to(Lvgl::screen_active),
        Err(Error::Internal)
    );
    assert_eq!(edit.delete(), Err(Error::Internal));
}

unsafe fn create_external(
    class: *const sys::lv_obj_class_t,
    parent: *mut sys::lv_obj_t,
) -> *mut sys::lv_obj_t {
    // SAFETY: the fixture supplies initialized, static descriptors and a live parent.
    unsafe {
        let object = sys::lv_obj_class_create_obj(class, parent);
        assert!(!object.is_null());
        sys::lv_obj_class_init_obj(object);
        object
    }
}

/// `close` deletes the box, or the backdrop LVGL makes for a box without a
/// parent: either way it refuses to free a host-owned object inside.
fn assert_message_box_close(lv: &mut Lvgl) {
    let (plain, backdropped, host_owned) = unsafe {
        let plain = sys::lv_msgbox_create(sys::lv_screen_active());
        let backdropped = sys::lv_msgbox_create(core::ptr::null_mut());
        let first = create_external(
            (&raw const UNKNOWN).cast(),
            sys::lv_msgbox_get_content(plain),
        );
        let second = create_external(
            (&raw const UNKNOWN).cast(),
            sys::lv_msgbox_get_content(backdropped),
        );
        (
            NonNull::new(plain).unwrap(),
            NonNull::new(backdropped).unwrap(),
            [first, second],
        )
    };
    let top_layer = || unsafe { sys::lv_obj_get_child_count(sys::lv_layer_top()) };
    assert_eq!(
        top_layer(),
        1,
        "the second box has a backdrop on the top layer"
    );
    for message_box in [plain, backdropped] {
        let close = |lv: &mut Lvgl| {
            lv.edit(|lv| unsafe { Obj::from_raw(lv, message_box) }.downcast::<MsgBox>())
                .unwrap()
                .close()
        };
        assert_eq!(close(lv), Err(Error::Internal));
    }
    // Nothing was deleted: the host-owned objects and their boxes are live.
    assert_eq!(top_layer(), 1);
    for (message_box, object) in [plain, backdropped].into_iter().zip(host_owned) {
        let content = unsafe { sys::lv_msgbox_get_content(message_box.as_ptr()) };
        assert_eq!(unsafe { sys::lv_obj_get_child(content, 0) }, object);
    }

    // The host releases its objects, and the same calls now succeed.
    for object in host_owned {
        unsafe { sys::lv_obj_delete(object) };
    }
    for message_box in [plain, backdropped] {
        lv.edit(|lv| unsafe { Obj::from_raw(lv, message_box) }.downcast::<MsgBox>())
            .unwrap()
            .close()
            .unwrap();
    }
    assert_eq!(top_layer(), 0, "the backdrop went with its box");
}

/// A window inside a window (a shell hosting application windows): both are
/// host-owned, and both let safe code edit their children.
fn assert_nested_windows(lv: &mut Lvgl) {
    let nested_raw = unsafe {
        let outer = lv.screen_active().unwrap().child(0).unwrap().as_ptr();
        create_external((&raw const WINDOW).cast(), outer)
    };
    let outer: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(0);
    let nested: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(0)?.child(0);
    let inside: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(0)?.child(0)?.child(0);

    // Neither window can be freed, owned or moved from safe code, directly
    // or later.
    assert_protected(lv, outer);
    assert_protected(lv, nested);
    assert_eq!(nested(lv).unwrap().delete_later(), Err(Error::Internal));
    assert_eq!(outer(lv).unwrap().clean_later(), Err(Error::Internal));
    // The outer window holds a host-owned object: its content cannot be
    // cleaned, so the nested window cannot be freed through it.
    assert_eq!(lv.edit(outer).unwrap().clean(), Err(Error::Internal));

    // The nested window's content is the application's.
    Obj::create(&mut lv.edit(nested).unwrap()).unwrap();
    lv.edit(inside).unwrap().delete().unwrap();
    Obj::create(&mut lv.edit(nested).unwrap()).unwrap();
    Obj::create(&mut lv.edit(nested).unwrap()).unwrap();
    lv.edit(nested).unwrap().clean().unwrap();
    assert!(nested(lv).unwrap().child(0).is_none());

    // Objects move from the outer window into the nested one and back.
    Obj::create(&mut lv.edit(outer).unwrap()).unwrap();
    let sibling: fn(&Lvgl) -> Option<&Obj> = |lv| lv.screen_active()?.child(0)?.child(1);
    lv.edit(sibling).unwrap().set_parent_to(nested).unwrap();
    assert!(inside(lv).is_some());
    lv.edit(inside).unwrap().set_parent_to(outer).unwrap();
    lv.edit(sibling).unwrap().delete().unwrap();

    // An `Owned` made while the object was a plain one keeps it alive once
    // raw code puts a host-owned object in it: the drop is refused.
    Obj::create(&mut lv.edit(nested).unwrap()).unwrap();
    let guarded = inside(lv).unwrap().own().unwrap();
    let guarded_raw = inside(lv).unwrap().as_ptr();
    let host_owned = unsafe { create_external((&raw const UNKNOWN).cast(), guarded_raw) };
    drop(guarded);
    lv.timer_handler();
    // SAFETY: the object is still live (the checked drop was refused).
    assert_eq!(unsafe { sys::lv_obj_get_child(guarded_raw, 0) }, host_owned);
    assert_eq!(unsafe { sys::lv_obj_get_child_count(nested_raw) }, 1);

    // The host releases what it owns.
    unsafe { sys::lv_obj_delete(host_owned) };
    unsafe { sys::lv_obj_delete(nested_raw) };
    assert!(outer(lv).unwrap().child(0).is_none());
}

/// Loading a screen while another load is pending can delete the screens
/// involved, whatever the new load asks for.
fn assert_pending_screen_loads(lv: &mut Lvgl) {
    let original = lv.screen_active().unwrap().handle().unwrap();
    let old = lv.create_screen().unwrap();
    let next = lv.create_screen().unwrap();
    let last = lv.create_screen().unwrap();
    lv.load_screen(old.handle(), ScreenAnim::NONE).unwrap();
    let host_owned =
        unsafe { create_external((&raw const UNKNOWN).cast(), old.get(lv).unwrap().as_ptr()) };

    let slide = ScreenAnim::new(ScreenLoadAnim::MOVE_LEFT, 1000, 0, false);
    let slide_and_delete = ScreenAnim::new(ScreenLoadAnim::MOVE_LEFT, 1000, 0, true);
    lv.load_screen(next.handle(), slide).unwrap();
    // Before the animation starts, `old` is still the active screen, and
    // finishing this load first would delete it if it had asked to.
    for step in 0..2 {
        assert_eq!(
            lv.load_screen(last.handle(), ScreenAnim::NONE),
            Err(Error::Busy)
        );
        assert_eq!(
            lv.load_screen(last.handle(), slide_and_delete),
            Err(Error::Internal)
        );
        if step == 0 {
            // After the first timer run `old` is the previous screen.
            tick_inc(10);
            lv.timer_handler();
        }
    }
    for _ in 0..30 {
        tick_inc(100);
        lv.timer_handler();
    }
    // Nothing was deleted, and nothing is pending any more.
    assert!(old.get(lv).is_ok());
    lv.load_screen(last.handle(), ScreenAnim::NONE).unwrap();
    // A pending load of screens without host-owned objects still works.
    lv.load_screen(next.handle(), slide_and_delete).unwrap();
    for _ in 0..30 {
        tick_inc(100);
        lv.timer_handler();
    }
    assert!(
        last.get(lv).is_err(),
        "the old screen went with auto_delete"
    );

    unsafe { sys::lv_obj_delete(host_owned) };
    lv.load_screen(original, ScreenAnim::NONE).unwrap();
}
