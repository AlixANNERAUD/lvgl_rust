//! Tests of the core against the real LVGL, or against the Rust model of it
//! (`base/ffi_stub.rs`) under Miri and loom. Every test holds the LVGL lock.

extern crate std;

use crate::*;
use std::sync::{Mutex, MutexGuard};

#[cfg(not(lvgl_rust_views_only))]
use crate::base::registry::reg;

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every test's LVGL calls.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// The first test initializes LVGL (and a display); the others enter.
fn session(guard: &mut Guard) -> Session<'_> {
    if crate::base::registry::reg()
        .hdr
        .init
        .load(core::sync::atomic::Ordering::SeqCst)
        == 0
    {
        let session = init(guard).unwrap();
        platform::create_display();
        session
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

#[cfg(not(any(miri, loom)))]
mod platform {
    use crate::sys;
    use core::ptr;

    const WIDTH: i32 = 64;
    const HEIGHT: i32 = 64;
    const BUF_BYTES: usize = (WIDTH * HEIGHT * 4) as usize / 4;
    static mut BUF: [u8; BUF_BYTES] = [0; BUF_BYTES];

    unsafe extern "C" fn flush(disp: *mut sys::lv_display_t, _: *const sys::lv_area_t, _: *mut u8) {
        unsafe { sys::lv_display_flush_ready(disp) }
    }

    pub(super) fn create_display() {
        // SAFETY: called once, with the test lock held.
        unsafe {
            let display = sys::lv_display_create(WIDTH, HEIGHT);
            sys::lv_display_set_buffers(
                display,
                (&raw mut BUF).cast(),
                ptr::null_mut(),
                BUF_BYTES as u32,
                sys::lv_display_render_mode_t_LV_DISPLAY_RENDER_MODE_PARTIAL,
            );
            sys::lv_display_set_flush_cb(display, Some(flush));
        }
    }

    pub(super) fn top_layer() -> *mut sys::lv_obj_t {
        // SAFETY: test lock held.
        unsafe { sys::lv_layer_top() }
    }
}

#[cfg(any(miri, loom))]
mod platform {
    use crate::base::ffi;

    pub(super) fn create_display() {}

    pub(super) fn top_layer() -> *mut ffi::lv_obj_t {
        // SAFETY: test lock held.
        unsafe { ffi::display_layers(core::ptr::null_mut())[1] }
    }
}

/// Slots in use, to check that tests give back what they take.
#[cfg(not(lvgl_rust_views_only))]
fn used_slots() -> usize {
    let reg = reg();
    (0..reg.high_water_mark())
        .filter(|&i| reg.slot(i).state() != 0)
        .count()
}

fn top_layer(lv: &Lvgl) -> &Obj {
    // SAFETY: the top layer lives as long as the display.
    unsafe { Obj::from_raw(lv, core::ptr::NonNull::new(platform::top_layer()).unwrap()) }
}

#[test]
fn views_reach_children_and_parents() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let child = {
        let mut screen = lv.screen_active_mut().unwrap();
        let mut parent = Obj::create(&mut screen).unwrap();
        Obj::create(&mut parent).unwrap().view().as_ptr()
    };
    let parent = lv.screen_active().unwrap().child(-1).unwrap();
    assert_eq!(parent.child(0).unwrap().as_ptr(), child);
    assert_eq!(
        parent.child(0).unwrap().parent().unwrap().as_ptr(),
        parent.as_ptr()
    );
    assert!(parent.child(1).is_none());
    let parent = parent.as_ptr();
    lv.edit(|lv| lv.screen_active().and_then(|s| s.child(-1)))
        .filter(|o| o.view().as_ptr() == parent)
        .unwrap()
        .delete()
        .unwrap();
}

#[test]
fn reentry_is_refused() {
    struct Other;
    // SAFETY: only used while the test lock is held.
    unsafe impl LvglAccess for Other {}

    let mut guard = guard();
    let lv = session(&mut guard);
    assert_eq!(init(&mut Other).err(), Some(Error::Reentrant));
    assert_eq!(Lvgl::enter(&mut Other).err(), Some(Error::Reentrant));
    drop(lv);
}

#[test]
fn callback_depth_is_bounded_and_blocks_sessions() {
    struct Other;
    // SAFETY: only used while the test lock is held.
    unsafe impl LvglAccess for Other {}

    let mut guard = guard();
    drop(session(&mut guard));
    let mut levels = std::vec::Vec::new();
    // SAFETY: test-only nesting, no LVGL call made through the guards.
    while let Some(level) = unsafe { Lvgl::enter_callback() } {
        levels.push(level);
    }
    assert_eq!(levels.len(), crate::config::MAX_CALLBACK_DEPTH as usize);
    assert_eq!(Lvgl::enter(&mut Other).err(), Some(Error::Reentrant));
    drop(levels);
    drop(Lvgl::enter(&mut Other).unwrap());
}

#[test]
fn layers_cannot_be_deleted() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let layer = top_layer(&lv).as_ptr();
    let error = lv
        .edit(|lv| Some(top_layer(lv)))
        .unwrap()
        .delete()
        .unwrap_err();
    assert_eq!(error, Error::Internal);
    assert_eq!(top_layer(&lv).as_ptr(), layer);
}

#[test]
fn reparenting_refuses_cycles() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let parent = Obj::create(&mut lv.screen_active_mut().unwrap())
        .unwrap()
        .view()
        .as_ptr();
    fn select_parent(lv: &Lvgl) -> Option<&Obj> {
        lv.screen_active()?.child(-1)
    }
    {
        let mut edit = lv.edit(select_parent).unwrap();
        assert_eq!(edit.view().as_ptr(), parent);
        // The object itself, then its child.
        assert_eq!(edit.set_parent_to(select_parent), Err(Error::InvalidArg));
    }
    Obj::create(&mut lv.edit(select_parent).unwrap()).unwrap();
    let mut edit = lv.edit(select_parent).unwrap();
    assert_eq!(
        edit.set_parent_to(|lv: &Lvgl| select_parent(lv)?.child(0)),
        Err(Error::InvalidArg)
    );
    edit.delete().unwrap();
}

#[test]
fn only_parentless_objects_load_as_screens() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let edit = lv.edit(|lv| lv.screen_active()?.child(-1)).unwrap();
    assert_eq!(edit.load_screen(ScreenAnim::NONE), Err(Error::InvalidArg));
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}

#[cfg(not(lvgl_rust_views_only))]
mod handles {
    use super::*;

    #[test]
    fn handles_are_deduplicated_and_die_with_their_object() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let mut screen = lv.screen_active_mut().unwrap();
        let object = Obj::create(&mut screen).unwrap();
        let first = object.view().handle().unwrap();
        assert_eq!(object.view().handle().unwrap(), first);
        assert_eq!(used_slots(), before + 1);
        first.get_mut(&mut lv).unwrap().delete().unwrap();
        assert_eq!(first.get(&lv).err(), Some(Error::Dead));
        assert_eq!(used_slots(), before);
    }

    #[test]
    fn deleting_a_parent_kills_the_handles_of_its_children() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let parent = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        let child = Obj::create(&mut parent.get_mut(&mut lv).unwrap())
            .unwrap()
            .view()
            .handle()
            .unwrap();
        let grandchild = Obj::create_owned(&mut child.get_mut(&mut lv).unwrap()).unwrap();
        parent.delete(&mut lv).unwrap();
        assert_eq!(child.get(&lv).err(), Some(Error::Dead));
        assert_eq!(grandchild.get(&lv).err(), Some(Error::Dead));
        // The owned grandchild's slot waits for its `Owned`.
        assert_eq!(used_slots(), before + 1);
        drop(grandchild);
        drop(lv);
        let lv = session(&mut guard);
        assert_eq!(used_slots(), before);
        drop(lv);
    }

    #[test]
    fn dropped_owned_objects_are_deleted_at_the_next_drain() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        let handle = owned.handle();
        let object = owned.get(&lv).unwrap().as_ptr();
        drop(owned);
        // Already unreachable through handles, deleted by the drain.
        assert_eq!(handle.get(&lv).err(), Some(Error::Dead));
        assert_eq!(
            lv.screen_active().unwrap().child(-1).unwrap().as_ptr(),
            object
        );
        lv.timer_handler();
        assert!(
            lv.screen_active()
                .unwrap()
                .child(-1)
                .is_none_or(|last| last.as_ptr() != object)
        );
        assert_eq!(used_slots(), before);
    }

    #[test]
    fn owned_objects_can_be_dropped_on_another_thread() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        let handle = owned.handle();
        drop(lv);
        std::thread::spawn(move || drop(owned)).join().unwrap();
        let lv = session(&mut guard);
        assert_eq!(handle.get(&lv).err(), Some(Error::Dead));
        assert_eq!(used_slots(), before);
        drop(lv);
    }

    #[test]
    fn objects_have_at_most_one_owner() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        assert_eq!(
            owned.get(&lv).unwrap().own().err(),
            Some(Error::AlreadyOwned)
        );
        assert_eq!(top_layer(&lv).own().err(), Some(Error::Internal));
        owned.delete(&mut lv).unwrap();
    }

    #[test]
    fn delete_later_runs_at_the_drain_and_releases_its_slot() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
        let object = lv.screen_active().unwrap().child(-1).unwrap();
        let ptr = object.as_ptr();
        object.delete_later().unwrap();
        assert_eq!(used_slots(), before + 1);
        assert_eq!(top_layer(&lv).delete_later(), Err(Error::Internal));
        lv.timer_handler();
        assert!(
            lv.screen_active()
                .unwrap()
                .child(-1)
                .is_none_or(|last| last.as_ptr() != ptr)
        );
        assert_eq!(used_slots(), before);
    }

    #[test]
    fn clean_later_keeps_the_object_and_releases_its_slot() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        let mut parent = owned.get_mut(&mut lv).unwrap();
        Obj::create(&mut parent).unwrap();
        let child = Obj::create(&mut parent).unwrap().view().handle().unwrap();
        parent.view().clean_later().unwrap();
        lv.timer_handler();
        let parent = owned.get(&lv).unwrap();
        assert!(parent.child(0).is_none());
        assert_eq!(child.get(&lv).err(), Some(Error::Dead));
        assert_eq!(used_slots(), before + 1);
        owned.delete(&mut lv).unwrap();
        assert_eq!(used_slots(), before);
    }

    #[test]
    fn delete_later_on_an_owned_object_pins_the_slot_until_the_drop() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        owned.get(&lv).unwrap().delete_later().unwrap();
        lv.timer_handler();
        assert_eq!(owned.get(&lv).err(), Some(Error::Dead));
        // Other slots come and go meanwhile.
        let other = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .handle()
            .unwrap();
        other.get_mut(&mut lv).unwrap().delete().unwrap();
        assert_eq!(used_slots(), before + 1);
        drop(owned);
        lv.timer_handler();
        assert_eq!(used_slots(), before);
    }

    #[test]
    fn stale_handles_stay_dead_when_their_slot_is_reused() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let stale = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .handle()
            .unwrap();
        stale.get_mut(&mut lv).unwrap().delete().unwrap();
        // Take every free slot, so the stale handle's slot is reused.
        let mut handles = std::vec::Vec::new();
        while let Ok(handle) = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .handle()
        {
            handles.push(handle);
        }
        assert_eq!(stale.get(&lv).err(), Some(Error::Dead));
        assert!(handles.iter().all(|h| h.get(&lv).is_ok()));
        // The object created by the failed iteration has no handle.
        lv.edit(|lv| lv.screen_active()?.child(-1))
            .unwrap()
            .delete()
            .unwrap();
        for handle in handles {
            handle.get_mut(&mut lv).unwrap().delete().unwrap();
        }
    }

    #[test]
    fn handles_downcast_and_upcast_through_the_same_slot() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let handle = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .handle()
            .unwrap();
        let same: Handle<Obj> = handle.downcast::<Obj>(&lv).unwrap().upcast();
        assert_eq!(same, handle);
        handle.get_mut(&mut lv).unwrap().delete().unwrap();
        assert!(handle.downcast::<Obj>(&lv).is_none());
    }

    #[test]
    fn handle_cells_store_optional_handles() {
        static CELL: HandleCell<Obj> = HandleCell::new();
        let mut guard = guard();
        let mut lv = session(&mut guard);
        assert_eq!(CELL.get(), None);
        let handle = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .handle()
            .unwrap();
        CELL.set(Some(handle));
        assert_eq!(CELL.get(), Some(handle));
        CELL.get()
            .unwrap()
            .get_mut(&mut lv)
            .unwrap()
            .delete()
            .unwrap();
        CELL.set(None);
        assert_eq!(CELL.get(), None);
    }

    #[test]
    fn untracking_an_object_through_set_id_kills_its_handles() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let object = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .as_ptr();
        let handle = lv
            .screen_active()
            .unwrap()
            .child(-1)
            .unwrap()
            .handle()
            .unwrap();
        // SAFETY: live object; NULL is the only id the hook accepts.
        unsafe { crate::sys::lv_obj_set_id(object, core::ptr::null_mut()) };
        assert_eq!(handle.get(&lv).err(), Some(Error::Dead));
        assert_eq!(used_slots(), before);
        lv.edit(|lv| lv.screen_active()?.child(-1))
            .unwrap()
            .delete()
            .unwrap();
    }

    #[test]
    fn load_screen_later_keeps_the_last_request_and_skips_dead_screens() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let original = lv.screen_active().unwrap().handle().unwrap();
        let first = lv.create_screen().unwrap();
        let second = lv.create_screen().unwrap();
        lv.load_screen_later(first.handle(), ScreenAnim::NONE)
            .unwrap();
        lv.load_screen_later(second.handle(), ScreenAnim::NONE)
            .unwrap();
        lv.timer_handler();
        assert_eq!(
            lv.screen_active().unwrap().as_ptr(),
            second.get(&lv).unwrap().as_ptr()
        );
        lv.load_screen_later(first.handle(), ScreenAnim::NONE)
            .unwrap();
        first.delete(&mut lv).unwrap();
        lv.timer_handler();
        assert_eq!(
            lv.screen_active().unwrap().as_ptr(),
            second.get(&lv).unwrap().as_ptr()
        );
        lv.load_screen(original, ScreenAnim::NONE).unwrap();
        second.delete(&mut lv).unwrap();
    }
}

/// Cases the real LVGL cannot set up deterministically: a screen held by an
/// animated load, objects of an unrelated class.
#[cfg(all(any(miri, loom), not(lvgl_rust_views_only)))]
mod model {
    use super::*;
    use crate::base::ffi;

    #[test]
    fn loading_screens_are_deleted_once_released() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let screen = lv.create_screen().unwrap();
        let handle = screen.handle();
        let p = screen.get(&lv).unwrap().as_ptr();
        ffi::set_loading_screens(p, core::ptr::null_mut());
        assert_eq!(handle.get_mut(&mut lv).unwrap().delete(), Err(Error::Busy));
        // `Owned::delete` keeps the request pending across drains.
        assert_eq!(screen.delete(&mut lv), Err(Error::Busy));
        lv.timer_handler();
        assert_eq!(used_slots(), before + 1);
        ffi::set_loading_screens(core::ptr::null_mut(), core::ptr::null_mut());
        lv.timer_handler();
        assert_eq!(used_slots(), before);
    }

    #[test]
    fn handles_check_the_class_of_their_object() {
        struct Other(Obj);
        // SAFETY: test widget over the stub's second class.
        unsafe impl Widget for Other {
            type Base = Obj;
            fn class() -> *const ffi::lv_obj_class_t {
                ffi::other_class()
            }
        }
        // SAFETY: as above.
        unsafe impl IsA<Obj> for Other {}

        let mut guard = guard();
        let mut lv = session(&mut guard);
        Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
        let plain = lv.screen_active().unwrap().child(-1).unwrap();
        assert!(plain.downcast::<Other>().is_none());
        let plain_ptr = plain.as_ptr();
        let plain = plain_ptr;
        // Children of a widget class are out of reach of structural calls.
        {
            // SAFETY: test lock held, live parent.
            let widget = unsafe { ffi::obj_create_other(plain) };
            let widget = core::ptr::NonNull::new(widget).unwrap();
            // Nothing is created inside it either; a widget makes its own.
            // SAFETY: live object, made just above.
            let mut edit = lv
                .edit(|lv| Some(unsafe { Obj::from_raw(lv, widget) }))
                .unwrap();
            assert_eq!(Obj::create(&mut edit).err(), Some(Error::Internal));
            let widget = unsafe { Obj::from_raw(&lv, widget) };
            // SAFETY: test lock held, live parent.
            let part = unsafe { ffi::obj_create(widget.as_ptr()) };
            let part = unsafe { Obj::from_raw(&lv, core::ptr::NonNull::new(part).unwrap()) };
            assert_eq!(part.own().err(), Some(Error::Internal));
            assert_eq!(part.delete_later(), Err(Error::Internal));
            assert_eq!(widget.clean_later(), Err(Error::Internal));
        }
        let plain = lv.screen_active().unwrap().child(-1).unwrap();
        assert_eq!(plain.as_ptr(), plain_ptr);
        let plain = plain.handle().unwrap();
        assert!(plain.downcast::<Other>(&lv).is_none());
        // SAFETY: test lock held, live parent.
        let other = unsafe { ffi::obj_create_other(lv.screen_active().unwrap().as_ptr()) };
        let other = unsafe { Obj::from_raw(&lv, core::ptr::NonNull::new(other).unwrap()) };
        let typed = other.downcast::<Other>().unwrap().handle().unwrap();
        assert_eq!(typed.upcast::<Obj>().downcast::<Other>(&lv), Some(typed));
        typed.get_mut(&mut lv).unwrap().delete().unwrap();
        plain.get_mut(&mut lv).unwrap().delete().unwrap();
    }
}

/// `Owned::drop` on another thread against the drain: no lost request, no
/// slot freed twice, never the wrong object deleted.
#[cfg(all(loom, not(lvgl_rust_views_only)))]
mod protocol {
    use super::*;
    use crate::base::drain::drain;

    /// Every slot handed out by the free list is distinct.
    fn assert_free_list_consistent() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let mut handles = std::vec::Vec::new();
        while let Ok(handle) = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .handle()
        {
            assert!(!handles.contains(&handle));
            handles.push(handle);
        }
        drop(lv);
    }

    #[test]
    fn dropped_owned_objects_are_deleted_exactly_once() {
        loom::model(|| {
            let mut guard = guard();
            let mut lv = session(&mut guard);
            let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
            let bystander = Obj::create(&mut lv.screen_active_mut().unwrap())
                .unwrap()
                .view()
                .handle()
                .unwrap();
            let thread = loom::thread::spawn(move || drop(owned));
            drain();
            thread.join().unwrap();
            drain();
            assert_eq!(used_slots(), 1);
            assert!(bystander.get(&lv).is_ok());
            drop(lv);
            drop(guard);
            assert_free_list_consistent();
        });
    }

    #[test]
    fn a_drop_racing_a_pending_delete_frees_the_slot_once() {
        loom::model(|| {
            let mut guard = guard();
            let mut lv = session(&mut guard);
            let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
            owned.get(&lv).unwrap().delete_later().unwrap();
            let reused = Obj::create(&mut lv.screen_active_mut().unwrap())
                .unwrap()
                .view()
                .handle()
                .unwrap();
            let thread = loom::thread::spawn(move || drop(owned));
            drain();
            reused.get(&lv).unwrap();
            thread.join().unwrap();
            drain();
            assert_eq!(used_slots(), 1);
            assert!(reused.get(&lv).is_ok());
            drop(lv);
            drop(guard);
            assert_free_list_consistent();
        });
    }
}

/// Structural refusals that need real widgets and animations.
#[cfg(all(not(any(miri, loom)), not(lvgl_rust_views_only)))]
mod lvgl_only {
    use super::*;
    use crate::sys;

    #[test]
    fn children_of_widgets_are_refused() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        // SAFETY: session held, live parent. The textarea keeps a pointer to
        // its label child (`ta->label`).
        let textarea = unsafe { sys::lv_textarea_create(screen.as_ptr()) };
        let textarea = unsafe { Obj::from_raw(&lv, core::ptr::NonNull::new(textarea).unwrap()) };
        let handle = textarea.handle().unwrap();
        let label = textarea.child(0).unwrap();
        assert_eq!(label.own().err(), Some(Error::Internal));
        assert_eq!(label.delete_later(), Err(Error::Internal));
        assert_eq!(textarea.clean_later(), Err(Error::Internal));
        assert_eq!(
            handle.get_mut(&mut lv).unwrap().clean(),
            Err(Error::Internal)
        );
        // The label, once `other` below is the screen's last child.
        fn in_textarea(lv: &Lvgl) -> Option<&Obj> {
            lv.screen_active()?.child(-2)?.child(0)
        }
        let other = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .view()
            .handle()
            .unwrap();
        // The label cannot move; the textarea, which keeps its label by
        // pointer, can take other children.
        assert_eq!(
            lv.edit(in_textarea).unwrap().set_parent(other),
            Err(Error::Internal)
        );
        assert_eq!(lv.edit(in_textarea).unwrap().delete(), Err(Error::Internal));
        // The widget itself can go, with its parts.
        handle.get_mut(&mut lv).unwrap().delete().unwrap();
        other.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    fn from_raw(lv: &Lvgl, p: *mut sys::lv_obj_t) -> &Obj {
        // SAFETY: test helper for objects just created through `sys`.
        unsafe { Obj::from_raw(lv, core::ptr::NonNull::new(p).unwrap()) }
    }

    #[test]
    fn user_children_of_known_widgets_are_the_applications() {
        use crate::widgets::{Button, Label};
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let mut screen = lv.screen_active_mut().unwrap();
        let mut button = Button::create(&mut screen).unwrap();
        let text = Label::create_owned(&mut button).unwrap();
        let button = button.view().handle().unwrap();
        // A label put in a button by the application can be moved back
        // and deleted; the button can be cleaned.
        let screen = lv.screen_active().unwrap().handle().unwrap();
        text.get_mut(&mut lv).unwrap().set_parent(screen).unwrap();
        text.get_mut(&mut lv)
            .unwrap()
            .set_parent(button.upcast())
            .unwrap();
        button.get_mut(&mut lv).unwrap().clean().unwrap();
        assert_eq!(text.get(&lv).err(), Some(Error::Dead));
        drop(text);
        button.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn message_box_parts_are_protected_and_content_is_free() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        // SAFETY: session held, live parent; the message box creates its
        // content and, lazily, its header and title.
        let (msgbox, content, title) = unsafe {
            let mbox = sys::lv_msgbox_create(screen.as_ptr());
            let title = sys::lv_msgbox_add_title(mbox, c"Title".as_ptr());
            (mbox, sys::lv_msgbox_get_content(mbox), title)
        };
        let user =
            Obj::create_owned(&mut lv.edit(|lv| Some(from_raw(lv, content))).unwrap()).unwrap();
        let content = from_raw(&lv, content);
        assert_eq!(content.own().err(), Some(Error::Internal));
        assert_eq!(from_raw(&lv, title).delete_later(), Err(Error::Internal));
        assert_eq!(from_raw(&lv, msgbox).clean_later(), Err(Error::Internal));
        // The content's children are the application's.
        content.clean_later().unwrap();
        lv.timer_handler();
        assert_eq!(user.get(&lv).err(), Some(Error::Dead));
        drop(user);
        let msgbox = from_raw(&lv, msgbox).handle().unwrap();
        msgbox.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn tab_view_indexes_are_protected() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        // SAFETY: session held, live parent.
        let (tabview, page, bar, content) = unsafe {
            let tv = sys::lv_tabview_create(screen.as_ptr());
            let page = sys::lv_tabview_add_tab(tv, c"One".as_ptr());
            (
                tv,
                page,
                sys::lv_tabview_get_tab_bar(tv),
                sys::lv_tabview_get_content(tv),
            )
        };
        let user = Obj::create(&mut lv.edit(|lv| Some(from_raw(lv, page))).unwrap())
            .unwrap()
            .view()
            .handle()
            .unwrap();
        let page = from_raw(&lv, page);
        let bar = from_raw(&lv, bar);
        let button = bar.child(0).unwrap();
        let button_label = button.child(0).unwrap();
        for part in [bar, from_raw(&lv, content), page, button, button_label] {
            assert_eq!(part.delete_later(), Err(Error::Internal));
        }
        assert_eq!(bar.clean_later(), Err(Error::Internal));
        assert_eq!(button.clean_later(), Err(Error::Internal));
        // Nothing moves into the tab view, its bar or its content.
        let content = from_raw(&lv, content).handle().unwrap();
        let tabview = from_raw(&lv, tabview).handle().unwrap();
        for target in [tabview, content] {
            assert_eq!(
                user.get_mut(&mut lv).unwrap().set_parent(target),
                Err(Error::Internal)
            );
        }
        // Page content is the application's.
        user.get_mut(&mut lv).unwrap().delete().unwrap();
        tabview.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn widgets_without_a_part_table_keep_their_children() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        // SAFETY: session held; the calendar is not in the part tables.
        let calendar = unsafe { sys::lv_calendar_create(screen.as_ptr()) };
        let calendar = from_raw(&lv, calendar);
        let inside = calendar.child(0).unwrap();
        assert_eq!(inside.delete_later(), Err(Error::Internal));
        assert_eq!(calendar.clean_later(), Err(Error::Internal));
        let calendar = calendar.handle().unwrap();
        calendar.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn message_box_header_buttons_stay_in_place() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        // SAFETY: session held, live parent. The close button's callback
        // deletes parent(parent(button)) as the message box.
        let (msgbox, close) = unsafe {
            let mbox = sys::lv_msgbox_create(screen.as_ptr());
            (mbox, sys::lv_msgbox_add_close_button(mbox))
        };
        let screen = screen.handle().unwrap();
        let close = from_raw(&lv, close).handle().unwrap();
        let mut edit = close.get_mut(&mut lv).unwrap();
        assert_eq!(edit.set_parent(screen), Err(Error::Internal));
        assert_eq!(edit.move_to_index(0), Err(Error::Internal));
        assert_eq!(edit.delete(), Err(Error::Internal));
        let msgbox = from_raw(&lv, msgbox).handle().unwrap();
        msgbox.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn children_are_created_and_reordered_only_where_no_index_shifts() {
        use crate::widgets::Label;
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        // SAFETY: session held, live parent.
        let (tabview, page, content, bar, calendar) = unsafe {
            let tv = sys::lv_tabview_create(screen.as_ptr());
            let page = sys::lv_tabview_add_tab(tv, c"One".as_ptr());
            let content = sys::lv_tabview_get_content(tv);
            let bar = sys::lv_tabview_get_tab_bar(tv);
            (
                tv,
                page,
                content,
                bar,
                sys::lv_calendar_create(screen.as_ptr()),
            )
        };
        // The tab view indexes its pages; the calendar is not in the tables.
        for parent in [tabview, content, calendar] {
            let mut parent = lv.edit(|lv| Some(from_raw(lv, parent))).unwrap();
            assert_eq!(Obj::create(&mut parent).err(), Some(Error::Internal));
            assert_eq!(Label::create(&mut parent).err(), Some(Error::Internal));
        }
        Obj::create(&mut lv.edit(|lv| Some(from_raw(lv, page))).unwrap()).unwrap();
        // Siblings in a plain object can be reordered.
        let mut screen = lv.screen_active_mut().unwrap();
        let mut parent = Obj::create(&mut screen).unwrap();
        let parent_handle = parent.view().handle().unwrap();
        Obj::create(&mut parent).unwrap();
        let second = Obj::create(&mut parent).unwrap();
        let second_ptr = second.view().as_ptr();
        let second = second.view().handle().unwrap();
        second.get_mut(&mut lv).unwrap().move_to_index(0).unwrap();
        let parent = parent_handle.get(&lv).unwrap();
        assert_eq!(parent.child(0).unwrap().as_ptr(), second_ptr);
        // Tab buttons are found by index.
        let button = from_raw(&lv, bar).child(0).unwrap().handle().unwrap();
        assert_eq!(
            button.get_mut(&mut lv).unwrap().move_to_index(-1),
            Err(Error::Internal)
        );
        parent_handle.get_mut(&mut lv).unwrap().delete().unwrap();
        for widget in [tabview, calendar] {
            let widget = from_raw(&lv, widget).handle().unwrap();
            widget.get_mut(&mut lv).unwrap().delete().unwrap();
        }
    }

    #[test]
    fn screens_cannot_be_reparented() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.create_screen().unwrap();
        let mut edit = screen.get_mut(&mut lv).unwrap();
        assert_eq!(
            edit.set_parent_to(|lv| lv.screen_active()),
            Err(Error::InvalidArg)
        );
        screen.delete(&mut lv).unwrap();
    }

    #[test]
    fn the_previous_screen_of_an_animated_load_cannot_be_loaded() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let original = lv.screen_active().unwrap().handle().unwrap();
        let first = lv.create_screen().unwrap();
        let second = lv.create_screen().unwrap();
        lv.load_screen(first.handle(), ScreenAnim::NONE).unwrap();
        let slide = ScreenAnim::new(ScreenLoadAnim::MOVE_LEFT, 1000, 0, true);
        lv.load_screen(second.handle(), slide).unwrap();
        let first_ptr = first.get(&lv).unwrap().as_ptr();
        // The animation starts at the next timer run: `first` becomes the
        // previous screen, deleted by LVGL when the animation completes.
        for _ in 0..10 {
            tick_inc(10);
            lv.timer_handler();
        }
        // SAFETY: session held.
        let prev = unsafe { sys::lv_display_get_screen_prev(core::ptr::null_mut()) };
        assert_eq!(prev, first_ptr);
        assert_eq!(
            lv.load_screen(first.handle(), ScreenAnim::NONE),
            Err(Error::Busy)
        );
        lv.load_screen_later(first.handle(), ScreenAnim::NONE)
            .unwrap();
        for _ in 0..30 {
            tick_inc(100);
            lv.timer_handler();
        }
        assert_eq!(first.get(&lv).err(), Some(Error::Dead));
        assert_eq!(
            lv.screen_active().unwrap().as_ptr(),
            second.get(&lv).unwrap().as_ptr()
        );
        lv.load_screen(original, ScreenAnim::NONE).unwrap();
        drop(first);
        second.delete(&mut lv).unwrap();
        lv.timer_handler();
    }
}
