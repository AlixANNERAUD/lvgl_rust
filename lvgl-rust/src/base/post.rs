//! Work posted from callbacks ([`Lvgl::post`]): run at the next drain with
//! exclusive access, which callbacks never have.

use crate::{Error, Lvgl, config::POST_SLOTS, event::UserData};
use core::{cell::Cell, ffi::c_void, mem, ptr::NonNull};

/// A posted call: a trampoline over the zero-sized function, and its data.
/// 8 bytes on 32-bit targets.
#[derive(Clone, Copy)]
struct Entry {
    run: unsafe fn(&mut Lvgl, *mut c_void),
    data: *mut c_void,
}

/// A ring of entries in `.bss`.
struct Queue {
    entries: [Cell<Option<Entry>>; POST_SLOTS],
    head: Cell<u8>,
    len: Cell<u8>,
    /// Set while the drain runs entries: a posted function that drains
    /// again (`timer_handler`) does not run the rest out of order.
    running: Cell<bool>,
}

// SAFETY: only used under the token (posting needs `&Lvgl`, running happens
// in the drain).
unsafe impl Sync for Queue {}

static QUEUE: Queue = Queue {
    entries: [const { Cell::new(None) }; POST_SLOTS],
    head: Cell::new(0),
    len: Cell::new(0),
    running: Cell::new(false),
};

unsafe fn run<D, F>(lv: &mut Lvgl, data: *mut c_void)
where
    D: UserData,
    F: Fn(&mut Lvgl, D) + Send + 'static,
{
    // SAFETY: `F` is zero-sized (checked by `post`).
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    // SAFETY: stored by `post` with `encode::<D>`.
    f(lv, unsafe { crate::event::decode::<D>(data) });
}

impl Lvgl {
    /// Runs `f(lv, data)` at the next drain (the next
    /// [`Lvgl::enter`](crate::Lvgl::enter) or after
    /// [`Lvgl::timer_handler`](crate::Lvgl::timer_handler)), with exclusive
    /// access: for callbacks, which get shared access only. `f` is a
    /// function item or a closure without captures. Fails with
    /// [`Error::Busy`] when the queue (`LVGL_RUST_POST_SLOTS`, default 4)
    /// is full.
    pub fn post<D, F>(&self, f: F, data: D) -> Result<(), Error>
    where
        D: UserData,
        F: Fn(&mut Lvgl, D) + Send + 'static,
    {
        const {
            assert!(
                mem::size_of::<F>() == 0,
                "post takes a function item or a closure without captures"
            )
        };
        mem::forget(f);
        let len = QUEUE.len.get() as usize;
        let tail = wrap(QUEUE.head.get() as usize + len);
        match QUEUE.entries.get(tail) {
            Some(cell) if len < POST_SLOTS => {
                cell.set(Some(Entry {
                    run: run::<D, F>,
                    data: crate::event::encode(data),
                }));
                QUEUE.len.set(len as u8 + 1);
                Ok(())
            }
            _ => Err(Error::Busy),
        }
    }
}

/// `i` modulo the queue size, without a division (`i < 2 · POST_SLOTS`).
fn wrap(i: usize) -> usize {
    if i >= POST_SLOTS { i - POST_SLOTS } else { i }
}

/// Runs the entries posted before this call, in order. Token held, depth 0.
pub(crate) fn run_posted() {
    if POST_SLOTS == 0 || QUEUE.running.get() {
        return;
    }
    QUEUE.running.set(true);
    // Entries posted while these run wait for the next drain.
    for _ in 0..QUEUE.len.get() {
        let head = QUEUE.head.get() as usize;
        let entry = QUEUE.entries.get(head).and_then(Cell::take);
        QUEUE.head.set(wrap(head + 1) as u8);
        QUEUE.len.set(QUEUE.len.get().saturating_sub(1));
        if let Some(entry) = entry {
            // SAFETY: token held at depth 0: exclusive access.
            let lv = unsafe { Lvgl::assume() };
            // SAFETY: the trampoline matches its data (`post`).
            unsafe { (entry.run)(lv, entry.data) };
        }
    }
    QUEUE.running.set(false);
}
