//! Diagnostic counters (feature `debug-counters`): what the crate did or
//! refused silently. Without the feature, [`count`] compiles to nothing and
//! no static exists.

/// The events counted. The discriminant is the counter's index.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "debug-counters"), allow(dead_code))]
pub(crate) enum Counter {
    Drained = 0,
    Refused = 1,
    Skipped = 2,
    Leaked = 3,
}

/// Counts one `counter` event (wrapping at 65,536).
#[inline(always)]
pub(crate) fn count(counter: Counter) {
    #[cfg(feature = "debug-counters")]
    enabled::count(counter);
    #[cfg(not(feature = "debug-counters"))]
    let _ = counter;
}

#[cfg(feature = "debug-counters")]
pub use enabled::DebugCounters;

#[cfg(feature = "debug-counters")]
mod enabled {
    // Core's atomics even under loom: the counters take no part in the
    // request protocol, and a loom atomic cannot live in a `static`.
    use core::sync::atomic::{AtomicU16, Ordering::Relaxed};

    use super::Counter;

    /// 8 bytes of `.bss`, zero at start.
    static COUNTERS: [AtomicU16; 4] = [const { AtomicU16::new(0) }; 4];

    pub(super) fn count(counter: Counter) {
        let c = &COUNTERS[counter as usize];
        // A load and a store, no read-modify-write (`thumbv6m` and
        // `riscv32imc` have no compare-and-swap). Only drops of a `Display`
        // or an `Indev` count without the token: an increment racing one
        // made on another thread can be lost.
        c.store(c.load(Relaxed).wrapping_add(1), Relaxed);
    }

    /// The crate's diagnostic counters (feature `debug-counters`): 8 bytes
    /// of `.bss`, four 16-bit counters that wrap at 65,536. They report what
    /// the crate does or refuses without returning an error, to debug
    /// deferred deletions and callbacks that do not run.
    ///
    /// Read them from any thread, with or without a session:
    ///
    /// ```
    /// # #[cfg(feature = "debug-counters")] {
    /// let counters = lvgl_rust::DebugCounters::read();
    /// if counters.refused != 0 {
    ///     // A deferred deletion was refused: the object is still alive.
    /// }
    /// # }
    /// ```
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[non_exhaustive]
    pub struct DebugCounters {
        /// Deferred requests the drain carried out: objects deleted or
        /// cleaned (dropped [`Owned`](crate::Owned)s, `delete_later`,
        /// `clean_later`), timers and groups deleted, screens loaded
        /// (`load_screen_later`). [`Owned::delete`](crate::Owned::delete)
        /// runs the same step at once and counts here too. Requests for
        /// objects that were already dead are not counted.
        pub drained: u16,
        /// Deferred requests dropped because the structural checks refused
        /// them for good (a display layer, a widget's part, a child whose
        /// parent indexes its children), leaving the object alive, and
        /// `load_screen_later` loads refused when the drain applied them (a
        /// display layer, an object with a parent, the previous screen of a
        /// load still animating). `Busy` refusals of deletions are retried
        /// and not counted, nor are refusals
        /// [`Owned::delete`](crate::Owned::delete) returns.
        pub refused: u16,
        /// Callbacks (event handlers, timers, animations, observers) whose
        /// Rust code was skipped because `LVGL_RUST_MAX_CALLBACK_DEPTH`
        /// nested callbacks were already running.
        pub skipped: u16,
        /// [`Display`](crate::Display)s and [`Indev`](crate::Indev)s
        /// dropped without being deleted: LVGL keeps them, and their cells
        /// stay claimed.
        pub leaked: u16,
    }

    impl DebugCounters {
        /// The current values.
        pub fn read() -> DebugCounters {
            let get = |c: Counter| COUNTERS[c as usize].load(Relaxed);
            DebugCounters {
                drained: get(Counter::Drained),
                refused: get(Counter::Refused),
                skipped: get(Counter::Skipped),
                leaked: get(Counter::Leaked),
            }
        }

        /// Sets every counter to zero.
        pub fn reset() {
            for c in &COUNTERS {
                c.store(0, Relaxed);
            }
        }
    }
}
