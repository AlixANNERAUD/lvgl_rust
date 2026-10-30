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

    /// A widget whose class the part tables do not describe (a menu).
    #[cfg(not(lvgl_rust_views_only))]
    pub(super) fn unknown_widget(parent: *mut sys::lv_obj_t) -> *mut sys::lv_obj_t {
        // SAFETY: test lock held, live parent.
        unsafe { sys::lv_menu_create(parent) }
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

    /// A widget whose class the part tables do not describe.
    #[cfg(not(lvgl_rust_views_only))]
    pub(super) fn unknown_widget(parent: *mut ffi::lv_obj_t) -> *mut ffi::lv_obj_t {
        // SAFETY: test lock held, live parent.
        unsafe { ffi::obj_create_other(parent) }
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
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    // The new object's own `ObjMut`, once the screen's borrow ended.
    let child = Obj::create(&mut lv.edit(|lv| lv.screen_active()?.child(-1)).unwrap())
        .unwrap()
        .as_ptr();
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

/// The deferred-free list and `LvBox` (also under Miri, against the
/// stub's heap, whose leak check catches a block never freed).
#[cfg(feature = "heap")]
mod heap {
    use super::*;
    use crate::{
        base::{deferred, ffi},
        heap::LvBox,
    };
    use core::ffi::c_void;
    use std::sync::Arc;

    #[test]
    fn deferred_blocks_are_freed_at_the_next_drain() {
        let mut guard = guard();
        let lv = session(&mut guard);
        for size in [8, 24, 100] {
            // SAFETY: under the token; a fresh block, big enough for the
            // link, needing no drop.
            unsafe {
                let block = ffi::malloc(size);
                assert!(!block.is_null());
                deferred::defer_free(block);
            }
        }
        assert_eq!(deferred::queued(), 3);
        drop(lv);
        let _lv = Lvgl::enter(&mut guard).unwrap();
        assert_eq!(deferred::queued(), 0);
    }

    #[test]
    fn boxes_align_their_value_and_drop_it_once() {
        #[repr(align(32))]
        struct Aligned(Arc<()>);
        let mut guard = guard();
        let _lv = session(&mut guard);
        let shared = Arc::new(());
        let block = LvBox::<u8, Aligned>::new(7, Aligned(shared.clone()))
            .ok()
            .unwrap()
            .into_raw();
        let value = LvBox::<u8, Aligned>::value(block);
        assert_eq!(value.addr() % 32, 0);
        // SAFETY: the box's own header and value.
        unsafe {
            assert_eq!(*LvBox::<u8, Aligned>::header(block), 7);
            assert_eq!(Arc::strong_count(&(*value).0), 2);
            LvBox::<u8, Aligned>::free(block);
        }
        assert_eq!(Arc::strong_count(&shared), 1);
        // A box whose header is the link goes through the list.
        let block = LvBox::<*mut c_void, [u32; 3]>::new(core::ptr::null_mut(), [1, 2, 3])
            .ok()
            .unwrap()
            .into_raw();
        // SAFETY: as in `deferred_blocks_are_freed_at_the_next_drain`.
        unsafe { deferred::defer_free(block) };
        assert_eq!(deferred::queued(), 1);
        crate::base::drain::drain();
        assert_eq!(deferred::queued(), 0);
    }

    #[test]
    fn zeroed_boxes_are_built_in_place() {
        #[repr(align(16))]
        struct Wide([u8; 40]);
        let mut guard = guard();
        let _lv = session(&mut guard);
        // SAFETY: zero bytes are a valid `Wide`.
        let block = unsafe { LvBox::<*mut c_void, Wide>::new_zeroed(core::ptr::null_mut()) }
            .unwrap()
            .into_raw();
        let value = LvBox::<*mut c_void, Wide>::value(block);
        assert_eq!(value.addr() % 16, 0);
        // SAFETY: the box's own value, then a block needing no drop.
        unsafe {
            assert!((*value).0.iter().all(|&byte| byte == 0));
            (*value).0[39] = 1;
            deferred::defer_free(block);
        }
        crate::base::drain::drain();
        assert_eq!(deferred::queued(), 0);
    }
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
        let first = object.handle().unwrap();
        assert_eq!(object.handle().unwrap(), first);
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
    fn deleting_a_dead_owned_object_releases_it() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let parent = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        let child = Obj::create_owned(&mut parent.get_mut(&mut lv).unwrap()).unwrap();
        parent.delete(&mut lv).unwrap();
        assert_eq!(child.get(&lv).err(), Some(Error::Dead));
        // Already dead: released at once, without an error.
        assert_eq!(child.delete(&mut lv), Ok(()));
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
    #[cfg_attr(target_os = "wasi", ignore = "WASI has no threads")]
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
    fn owned_delete_reports_a_permanent_refusal() {
        use crate::base::ffi;
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let before = used_slots();
        let widget = platform::unknown_widget(lv.screen_active().unwrap().as_ptr());
        let owned = Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap();
        let handle = owned.handle();
        let object = owned.get(&lv).unwrap().as_ptr();
        // Raw code moves it into a widget the part tables do not describe:
        // its deletion is now refused for good.
        // SAFETY: test lock held, both objects live.
        unsafe { ffi::obj_set_parent(object, widget) };
        assert_eq!(owned.delete(&mut lv), Err(Error::Internal));
        // Released like a refused drop: alive, out of the registry.
        assert_eq!(handle.get(&lv).err(), Some(Error::Dead));
        assert_eq!(used_slots(), before);
        // SAFETY: test lock held; the widget is live, and deleted with the
        // object it kept.
        unsafe {
            assert_eq!(ffi::obj_child(widget, -1), object);
            ffi::obj_delete(widget);
        }
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
        let child = Obj::create(&mut parent).unwrap().handle().unwrap();
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
            .handle()
            .unwrap();
        let stale_index = crate::base::registry::index(stale.raw());
        stale.get_mut(&mut lv).unwrap().delete().unwrap();
        // Take slots, giving back the others at once, until the stale
        // handle's slot is reused (fresh slots first, then the free list in
        // order: at most every slot once).
        let reused = loop {
            let handle = Obj::create(&mut lv.screen_active_mut().unwrap())
                .unwrap()
                .handle()
                .unwrap();
            if crate::base::registry::index(handle.raw()) == stale_index {
                break handle;
            }
            handle.get_mut(&mut lv).unwrap().delete().unwrap();
        };
        assert_ne!(reused, stale);
        assert_eq!(stale.get(&lv).err(), Some(Error::Dead));
        assert!(reused.get(&lv).is_ok());
        reused.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn handles_downcast_and_upcast_through_the_same_slot() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let handle = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
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

/// The `growable-registry` feature: chunk arithmetic, then objects in
/// chunks, against LVGL or its model.
#[cfg(all(lvgl_rust_growable, not(loom)))]
mod growth {
    use super::*;
    use crate::base::registry::{
        CHUNKS, FAIL_GROWTH, Kind, LIMIT, capacity, chunk_len, index, locate, next_chunk,
    };
    use crate::config::HANDLE_SLOTS as N;
    use core::sync::atomic::Ordering;

    /// Chunks tile the indices from `2^shift` to `LIMIT - 1`, in order,
    /// each as large as everything before it except the last one.
    fn check_chunks(shift: u32) {
        let static_slots = 1usize << shift;
        let chunks = 16 - shift as usize;
        let mut expected = (0, 0);
        for index in static_slots..LIMIT {
            assert_eq!(locate(index, shift), expected, "index {index}");
            expected.1 += 1;
            if expected.1 == chunk_len(expected.0, shift) {
                expected = (expected.0 + 1, 0);
            }
        }
        assert_eq!(expected, (chunks, 0));
        let total: usize = (0..chunks).map(|chunk| chunk_len(chunk, shift)).sum();
        assert_eq!(static_slots + total, LIMIT);
        // `LIMIT` is just past the last chunk (`slot` checks it), larger
        // indices fall outside the chunk table.
        assert_eq!(
            locate(LIMIT, shift),
            (chunks - 1, chunk_len(chunks - 1, shift))
        );
        assert!(locate(LIMIT + 1, shift).0 >= chunks);
        assert!(locate(usize::MAX, shift).0 >= chunks);
    }

    #[test]
    fn chunks_double_the_capacity_up_to_the_handle_limit() {
        assert_eq!(CHUNKS, 16 - N.trailing_zeros() as usize);
        // One static slot, the default 32, and the largest static part.
        for shift in [0, N.trailing_zeros(), 15] {
            check_chunks(shift);
        }
        // With N = 32: 32, then 64, 128, ..., 32,768, then 65,535.
        for shift in [0, N.trailing_zeros(), 15] {
            let static_slots = 1usize << shift;
            assert_eq!(capacity(0, shift), static_slots);
            assert_eq!(capacity(static_slots, shift), static_slots);
            let mut total = static_slots;
            for chunk in 0..16 - shift as usize {
                assert_eq!(next_chunk(total, shift), Some(chunk));
                assert_eq!(chunk_len(chunk, shift), total.min(LIMIT - total));
                total += chunk_len(chunk, shift);
                assert_eq!(capacity(total, shift), total);
                assert_eq!(capacity(total - chunk_len(chunk, shift) + 1, shift), total);
            }
            assert_eq!(total, LIMIT);
            // At the limit, no chunk is left: the registry is full.
            assert_eq!(next_chunk(LIMIT, shift), None);
            assert_eq!(capacity(LIMIT, shift), LIMIT);
        }
        // 65,535 slots keep `index + 1` in the low half of a raw handle.
        assert_eq!(index(0x0001_FFFF), LIMIT - 1);
    }

    #[test]
    fn indices_of_no_chunk_resolve_to_a_free_slot() {
        let mut guard = guard();
        let lv = session(&mut guard);
        let reg = reg();
        // A raw handle whose index no chunk holds yet, and one with no index.
        for raw in [0x0003_FFFF, 0x0003_0000] {
            assert_eq!(reg.slot(index(raw)).state(), 0);
            assert_eq!(
                reg.resolve(raw, Kind::Obj, core::ptr::null()).err(),
                Some(Error::Dead)
            );
        }
        drop(lv);
    }

    /// Fills the slots, grows into several chunks, then gives everything
    /// back: allocation failure, resolution, `Owned` drops on both sides of
    /// chunk boundaries, reuse with new generations, `delete_later` and
    /// timers in chunks.
    #[test]
    fn objects_grow_into_chunks_and_give_their_slots_back() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let reg = reg();
        let before = used_slots();
        let mut handles = std::vec::Vec::new();
        fn new_handle(lv: &mut Lvgl) -> Result<Handle<Obj>, Error> {
            Obj::create(&mut lv.screen_active_mut().unwrap())
                .unwrap()
                .handle()
        }
        // Every slot in use, without growing.
        let start = reg.capacity();
        while used_slots() < start {
            handles.push(new_handle(&mut lv).unwrap());
        }
        assert_eq!(reg.capacity(), start);

        // The allocator fails: no slot, the object stays untracked.
        FAIL_GROWTH.store(true, Ordering::Relaxed);
        assert_eq!(new_handle(&mut lv).err(), Some(Error::OutOfMemory));
        assert_eq!(used_slots(), start);
        FAIL_GROWTH.store(true, Ordering::Relaxed);
        let last = lv.screen_active().unwrap().child(-1).unwrap();
        assert_eq!(last.delete_later(), Err(Error::OutOfMemory));
        assert_eq!(used_slots(), start);
        lv.edit(|lv| lv.screen_active()?.child(-1))
            .unwrap()
            .delete()
            .unwrap();
        assert_eq!(reg.capacity(), start);

        // Growth: the first slot of the new chunk, then the rest of it,
        // until at least two chunks follow the static part.
        let target = start.max(4 * N);
        while reg.capacity() < target || used_slots() < reg.capacity() {
            let capacity = reg.capacity();
            let handle = new_handle(&mut lv).unwrap();
            if used_slots() > capacity {
                assert_eq!(index(handle.raw()), capacity);
                assert_eq!(reg.capacity(), 2 * capacity);
            }
            handles.push(handle);
        }
        assert!(used_slots() >= 4 * N);
        for handle in &handles {
            let slot = reg.slot(index(handle.raw()));
            assert_eq!(slot.ptr(), handle.get(&lv).unwrap().as_ptr().cast());
        }

        // `Owned` on each side of the boundaries of the static part and of
        // chunk 0, dropped on another thread: those this test took (a slot
        // an earlier test left in use is not its to drop).
        let owned: std::vec::Vec<_> = [N - 1, N, 2 * N - 1, 2 * N]
            .iter()
            .filter_map(|&i| handles.iter().find(|h| index(h.raw()) == i))
            .map(|handle| handle.get(&lv).unwrap().own().unwrap())
            .collect();
        if before == 0 {
            assert_eq!(owned.len(), 4);
        }
        let boundaries: std::vec::Vec<_> = owned.iter().map(|o| index(o.handle().raw())).collect();
        let dropped: std::vec::Vec<_> = owned.iter().map(|o| o.handle()).collect();
        #[cfg(not(target_os = "wasi"))]
        std::thread::spawn(move || drop(owned)).join().unwrap();
        #[cfg(target_os = "wasi")]
        drop(owned);
        lv.timer_handler();
        assert!(
            dropped
                .iter()
                .all(|h| h.get(&lv).err() == Some(Error::Dead))
        );
        handles.retain(|h| !dropped.contains(h));
        assert_eq!(used_slots(), before + handles.len());

        // The freed slots come back from the free list (every other slot is
        // in use), with a new generation.
        let mut reused = std::vec::Vec::new();
        for _ in &boundaries {
            let handle = new_handle(&mut lv).unwrap();
            assert!(boundaries.contains(&index(handle.raw())));
            assert!(!dropped.contains(&handle));
            reused.push(handle);
        }
        assert!(
            dropped
                .iter()
                .all(|h| h.get(&lv).err() == Some(Error::Dead))
        );
        handles.extend(reused);

        // A temporary slot for `delete_later`, in a new chunk.
        let capacity = reg.capacity();
        Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
        let object = lv.screen_active().unwrap().child(-1).unwrap();
        object.delete_later().unwrap();
        let temp = reg.slot(capacity);
        assert_eq!(temp.ptr(), object.as_ptr().cast());
        assert!(temp.has(crate::base::registry::meta::TEMP));
        lv.timer_handler();
        assert_eq!(temp.state(), 0);
        assert_eq!(used_slots(), before + handles.len());

        // A timer slot in a chunk, found by its pointer.
        let timer = core::ptr::without_provenance_mut(0x1000);
        let minted = reg.mint(timer, Kind::Timer).unwrap();
        assert!(minted.index >= N);
        assert_eq!(reg.find_timer(timer), Some(minted.index));
        reg.free(minted.index);
        assert_eq!(reg.find_timer(timer), None);

        for handle in handles {
            handle.get_mut(&mut lv).unwrap().delete().unwrap();
        }
        assert_eq!(used_slots(), before);
    }
}

/// `Owned::drop` on another thread against the drain: no lost request, no
/// slot freed twice, never the wrong object deleted.
#[cfg(all(loom, not(lvgl_rust_views_only)))]
mod protocol {
    use super::*;
    use crate::base::drain::drain;

    /// Every slot handed out by the free list is distinct (up to the
    /// capacity, with growth).
    fn assert_free_list_consistent() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let mut handles = std::vec::Vec::new();
        #[cfg(lvgl_rust_growable)]
        let capacity = reg().capacity();
        while let Ok(handle) = Obj::create(&mut lv.screen_active_mut().unwrap())
            .unwrap()
            .handle()
        {
            assert!(!handles.contains(&handle));
            handles.push(handle);
            #[cfg(lvgl_rust_growable)]
            if used_slots() == capacity {
                break;
            }
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

    /// The token holder publishes a chunk and takes a slot in it while
    /// another thread drops an `Owned` of the static part and one of an
    /// earlier chunk (`LVGL_RUST_HANDLE_SLOTS=4`: chunk 0 holds slots 4 to
    /// 7, chunk 1 slots 8 to 15). It covers growth racing drops and the
    /// drain on other slots; a drop racing the publication of its own
    /// chunk cannot happen (the `Owned` is minted after it), so weakening
    /// the Release/Acquire pair of the chunk pointers would not fail here.
    #[cfg(lvgl_rust_growable)]
    #[test]
    fn drops_reach_the_drain_while_the_registry_grows() {
        loom::model(|| {
            let mut guard = guard();
            let mut lv = session(&mut guard);
            let mut owned = std::vec::Vec::new();
            for _ in 0..8 {
                owned.push(Obj::create_owned(&mut lv.screen_active_mut().unwrap()).unwrap());
            }
            assert_eq!(reg().capacity(), 8);
            let in_chunk = owned.pop().unwrap();
            let in_static = owned.remove(0);
            let dropped = [in_chunk.handle(), in_static.handle()];
            assert_eq!(
                dropped.map(|h| crate::base::registry::index(h.raw())),
                [7, 0]
            );
            let thread = loom::thread::spawn(move || {
                drop(in_chunk);
                drop(in_static);
            });
            let grown = Obj::create(&mut lv.screen_active_mut().unwrap())
                .unwrap()
                .handle()
                .unwrap();
            assert_eq!(crate::base::registry::index(grown.raw()), 8);
            drain();
            thread.join().unwrap();
            drain();
            assert!(
                dropped
                    .iter()
                    .all(|h| h.get(&lv).err() == Some(Error::Dead))
            );
            assert_eq!(used_slots(), owned.len() + 1);
            assert!(grown.get(&lv).is_ok());
            grown.get_mut(&mut lv).unwrap().delete().unwrap();
            drop(owned);
            drain();
            assert_eq!(used_slots(), 0);
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
        let button = Button::create(&mut screen).unwrap().handle().unwrap();
        let text = Label::create_owned(&mut button.get_mut(&mut lv).unwrap()).unwrap();
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
        // SAFETY: session held; the menu is not in the part tables.
        let menu = unsafe { sys::lv_menu_create(screen.as_ptr()) };
        let menu = from_raw(&lv, menu);
        let inside = menu.child(0).unwrap();
        assert_eq!(inside.delete_later(), Err(Error::Internal));
        assert_eq!(menu.clean_later(), Err(Error::Internal));
        let menu = menu.handle().unwrap();
        menu.get_mut(&mut lv).unwrap().delete().unwrap();
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
        let (tabview, page, content, bar, menu) = unsafe {
            let tv = sys::lv_tabview_create(screen.as_ptr());
            let page = sys::lv_tabview_add_tab(tv, c"One".as_ptr());
            let content = sys::lv_tabview_get_content(tv);
            let bar = sys::lv_tabview_get_tab_bar(tv);
            (tv, page, content, bar, sys::lv_menu_create(screen.as_ptr()))
        };
        // The tab view indexes its pages; the menu is not in the tables.
        for parent in [tabview, content, menu] {
            let mut parent = lv.edit(|lv| Some(from_raw(lv, parent))).unwrap();
            assert_eq!(Obj::create(&mut parent).err(), Some(Error::Internal));
            assert_eq!(Label::create(&mut parent).err(), Some(Error::Internal));
        }
        Obj::create(&mut lv.edit(|lv| Some(from_raw(lv, page))).unwrap()).unwrap();
        // Siblings in a plain object can be reordered.
        let mut screen = lv.screen_active_mut().unwrap();
        let parent_handle = Obj::create(&mut screen).unwrap().handle().unwrap();
        let mut parent = parent_handle.get_mut(&mut lv).unwrap();
        Obj::create(&mut parent).unwrap();
        let second = Obj::create(&mut parent).unwrap();
        let second_ptr = second.as_ptr();
        let second = second.handle().unwrap();
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
        for widget in [tabview, menu] {
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
