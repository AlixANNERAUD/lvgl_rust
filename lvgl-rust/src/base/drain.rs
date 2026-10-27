//! Deferred requests: dropped `Owned` handles, `delete_later`,
//! `clean_later` and `load_screen_later`, carried out at depth 0 by
//! [`Lvgl::enter`](crate::Lvgl::enter) and after
//! [`Lvgl::timer_handler`](crate::Lvgl::timer_handler).

#[cfg(lvgl_rust_views_only)]
pub(crate) fn drain() {
    super::post::run_posted();
}

#[cfg(not(lvgl_rust_views_only))]
pub(crate) use slots::*;

#[cfg(not(lvgl_rust_views_only))]
mod slots {
    use crate::Error;
    use crate::base::{
        ffi,
        registry::{meta::*, reg},
        structural,
        sync::{Ordering, fence},
    };
    use crate::counters::{Counter, count};

    /// Carries out pending requests. Token held, depth 0.
    pub(crate) fn drain() {
        let reg = reg();
        if reg.hdr.any_req.load(Ordering::Relaxed) {
            reg.hdr.any_req.store(false, Ordering::Relaxed);
            // Paired with the fence of `Registry::request_drop`: a drop whose
            // `any_req` store precedes the clear above has its `req` visible
            // to the scan below; a later one leaves `any_req` set.
            fence(Ordering::SeqCst);
            for index in 0..reg.high_water_mark() {
                // `Busy` sets `any_req` again: the next drain retries.
                let _ = act(index);
            }
        }
        apply_pending_load();
        crate::base::post::run_posted();
    }

    /// Carries out the requests of one slot. Returns `Err(Busy)` when the
    /// structural checks asked for a retry (the request stays pending and
    /// `any_req` is set again).
    pub(crate) fn act(index: usize) -> Result<(), Error> {
        let reg = reg();
        let slot = reg.slot(index);
        if slot.is_timer() || slot.is_group() {
            act_resource(index);
            return Ok(());
        }
        let dropped = slot.has(OWNED) && slot.req.load(Ordering::Relaxed) != 0;
        let delete = dropped || slot.has(P_DELETE);
        if !delete && !slot.has(P_CLEAN) {
            return Ok(());
        }
        let live = slot.state() == LIVE;
        let o = slot.ptr().cast::<ffi::lv_obj_t>();

        // A permanent refusal drops the request; `Busy` keeps it.
        let allowed = if !live {
            false
        } else {
            let check = if delete {
                structural::check_delete(o)
            } else {
                structural::check_clean(o)
            };
            match check {
                Ok(()) => true,
                Err(Error::Busy) => {
                    reg.hdr.any_req.store(true, Ordering::Relaxed);
                    return Err(Error::Busy);
                }
                Err(_) => {
                    count(Counter::Refused);
                    false
                }
            }
        };
        if allowed {
            count(Counter::Drained);
        }

        if dropped {
            // (a) The `Owned` is gone. With OWNED still set, the hook only
            // marks the slot DEAD, so it is freed here exactly once.
            if allowed {
                // SAFETY: token held, depth 0, checks passed.
                unsafe { ffi::obj_delete(o) };
            }
            slot.clear(OWNED | P_DELETE | P_CLEAN | TEMP);
            // Clears the id of an object left alive by a refusal.
            reg.release(index);
        } else if slot.has(P_DELETE) {
            // Read before the delete: in case (c) the hook frees the slot, and
            // a callback run by the deletion may reuse it.
            let owned = slot.has(OWNED);
            if allowed {
                // (b) owned: the hook leaves the slot DEAD and OWNED until the
                // `Owned` is dropped. (c) not owned: the hook frees it.
                // SAFETY: as above.
                unsafe { ffi::obj_delete(o) };
            }
            if owned || !allowed {
                slot.clear(P_DELETE | P_CLEAN);
                release_temp(index);
            }
        } else {
            if allowed {
                // SAFETY: as above.
                unsafe { ffi::obj_clean(o) };
            }
            slot.clear(P_CLEAN);
            release_temp(index);
        }
        Ok(())
    }

    /// Deletes a timer or a group whose `Owned` was dropped or that asked
    /// to be deleted (`TimerCtx::delete_later`).
    fn act_resource(index: usize) {
        let reg = reg();
        let slot = reg.slot(index);
        let dropped = slot.has(OWNED) && slot.req.load(Ordering::Relaxed) != 0;
        if !dropped && !slot.has(P_DELETE) {
            return;
        }
        if slot.state() == LIVE {
            count(Counter::Drained);
            // SAFETY: token held, depth 0; a LIVE slot points to a timer or
            // a group the crate created and nothing else deletes.
            unsafe {
                if slot.is_timer() {
                    #[cfg(feature = "heap")]
                    let boxed = slot
                        .has(TIMER_BOXED)
                        .then(|| ffi::timer_user_data(slot.ptr().cast()));
                    ffi::timer_delete(slot.ptr().cast());
                    // The closure goes after its timer.
                    #[cfg(feature = "heap")]
                    if let Some(block) = boxed {
                        let header = block.cast::<crate::timer::BoxedTimer>();
                        ((*header).free)(block);
                    }
                } else {
                    ffi::group_delete(slot.ptr().cast());
                }
            }
        }
        if dropped {
            reg.free(index);
        } else {
            reg.resource_deleted(index);
        }
    }

    /// Releases a TEMP slot once its request completed.
    fn release_temp(index: usize) {
        let reg = reg();
        let slot = reg.slot(index);
        if slot.has(TEMP) && !slot.has(P_DELETE | P_CLEAN) {
            reg.release(index);
        }
    }

    /// Applies the pending `load_screen_later`, if its screen is still alive.
    fn apply_pending_load() {
        let hdr = &reg().hdr;
        let Some(index) = (hdr.load.get() as usize).checked_sub(1) else {
            return;
        };
        hdr.load.set(0);
        let slot = reg().slot(index);
        // Skip a screen that is dead or about to be deleted.
        if slot.state() != LIVE || slot.has(P_DELETE) || slot.req.load(Ordering::Relaxed) != 0 {
            return;
        }
        let screen = slot.ptr().cast();
        // A refused load is discarded, like a load of a dead screen.
        match structural::load_screen(screen, hdr.load_anim.get()) {
            Ok(()) => count(Counter::Drained),
            Err(_) => count(Counter::Refused),
        }
        release_temp(index);
    }
}
