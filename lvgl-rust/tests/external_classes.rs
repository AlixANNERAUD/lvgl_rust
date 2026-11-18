//! Real LVGL coverage for class descriptors owned by an embedding host.

use core::mem::MaybeUninit;
use lvgl_rust::{
    Error, ExternalClassPolicy, Lvgl, LvglAccess, Obj, ScreenAnim, ScreenLoadAnim, WidgetExt, sys,
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
