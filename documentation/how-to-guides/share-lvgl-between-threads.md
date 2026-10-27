# Share LVGL between threads

LVGL is not thread safe. lvgl-rust never locks: you bring the lock, and
prove you hold it by implementing `LvglAccess` for its guard. This guide
shows correct implementations for a hosted program, bare-metal firmware,
an RTOS and async code, then how the other threads and interrupts talk to
the graphics code without the lock.

## What you need

- lvgl-rust configured as in [Configure LVGL](configure-lvgl.md).
- A lock that serializes every LVGL call of your program: one, used by
  all the code that touches LVGL.

## Steps

### 1. Wrap the lock's guard in a type of its own

The guard type must prove exclusive, non-recursive access to LVGL. Use a
newtype dedicated to it, never a guard type that other mutexes share (a
bare `std::sync::MutexGuard<'_, ()>` is also the guard of every other
`Mutex<()>` of the program). Keep its field private and give it one
constructor that takes the LVGL lock:

```rust
use lvgl_rust::LvglAccess;
use std::sync::{Mutex, MutexGuard, PoisonError};

static LVGL: Mutex<()> = Mutex::new(());

/// Exists only while the LVGL mutex is held.
pub struct Guard(MutexGuard<'static, ()>);

impl Guard {
    pub fn lock() -> Guard {
        Guard(LVGL.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

// SAFETY: a guard of the only mutex that serializes LVGL calls; unlocking
// a std mutex synchronizes with the next lock (Release/Acquire).
unsafe impl LvglAccess for Guard {}
```

The `LvglAccess` contract also requires that the end of one guard's
lifetime synchronizes-with the start of the next one, on any thread.
Mutexes provide this; a flag you set and clear yourself with relaxed
stores does not.

### 2. Start a session from the guard for each batch of work

Every LVGL call goes through a `Session`, which borrows the guard:

```rust
use lvgl_rust::Lvgl;

fn frame() -> u32 {
    let mut guard = Guard::lock();
    let mut lv = Lvgl::enter(&mut guard).expect("LVGL is initialised");
    lv.timer_handler() // milliseconds until the next timer
} // the session ends, then the guard unlocks
```

`lvgl_rust::init(&mut guard)` starts the first session. `Lvgl::enter`
returns `Error::Reentrant` if a session is already active or a callback
is running, and `Error::NotInitialized` before `init` or after
`Lvgl::deinit`. Starting a session also carries out the requests other
threads made since the previous one (step 5).

### 3. Pick the lock for your platform

**Bare-metal firmware with one main loop.** If only the main loop calls
LVGL and interrupt handlers never do, a zero-sized guard created once in
the main function is enough:

```rust
mod gui {
    use lvgl_rust::{Lvgl, LvglAccess};

    /// Created once, by `run`, and never stored in a `static`.
    struct Guard(());

    // SAFETY: single-threaded firmware: only `run` calls LVGL, and
    // interrupt handlers never do.
    unsafe impl LvglAccess for Guard {}

    pub fn run() -> ! {
        let mut guard = Guard(());
        let lv = lvgl_rust::init(&mut guard).unwrap();
        // ... create the display and the screens ...
        drop(lv);
        loop {
            if let Ok(mut lv) = Lvgl::enter(&mut guard) {
                lv.timer_handler();
            }
            // wait for the next tick
        }
    }
}
```

This is how the size-gate firmware
([`lvgl-rust/size-gate/src/main.rs`](../../lvgl-rust/size-gate/src/main.rs))
does it. Keep the guard type private to that module so that nothing else
can make one.

**Several tasks on an RTOS.** Wrap the RTOS mutex guard exactly as in
step 1. The RTOS mutex must give Release/Acquire ordering between unlock
and lock, which mutexes of FreeRTOS, Zephyr and similar kernels do on
their supported cores.

**Async code.** `next_event` takes the lock with an async function and
returns the guard (see
[Receive events asynchronously](receive-events-asynchronously.md)). With
an async mutex such as the one of `embassy-sync`, wrap its guard:

```rust
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::{Mutex, MutexGuard}};
use lvgl_rust::LvglAccess;

static LVGL: Mutex<CriticalSectionRawMutex, ()> = Mutex::new(());

pub struct Guard(MutexGuard<'static, CriticalSectionRawMutex, ()>);

impl Guard {
    pub async fn lock() -> Guard {
        Guard(LVGL.lock().await)
    }
}

// SAFETY: a guard of the only mutex that serializes LVGL calls.
unsafe impl LvglAccess for Guard {}
```

### 4. Drive LVGL's clock

LVGL needs a millisecond tick. Pick one source:

- `lvgl_rust::tick_inc(ms)` needs no guard: call it from a timer
  interrupt or a dedicated thread. On targets with 32-bit atomic
  read-modify-write it is a `fetch_add`. On `thumbv6m` and `riscv32imc`,
  which have none, it is a load and a store, so two concurrent callers
  can lose ticks: call it from one context only there.
- `lv.set_tick_cb(f)` makes an `extern "C" fn() -> u32` returning
  milliseconds LVGL's clock instead, for example on a hosted system:

  ```rust
  use std::{sync::OnceLock, time::Instant};

  static START: OnceLock<Instant> = OnceLock::new();

  extern "C" fn millis() -> u32 {
      START.get_or_init(Instant::now).elapsed().as_millis() as u32
  }

  lv.set_tick_cb(millis);
  ```

### 5. Let other threads change the interface

Views (`&Obj`), `ObjMut`, `Session` and `Lvgl` are `!Send`: they never
leave the thread that holds the guard. Three things do cross threads:

- **Handles.** A `Handle<W>` is 4 bytes, `Copy`, `Send` and `Sync`. Store
  one in a worker, then take the lock when you need the object:

  ```rust
  use lvgl_rust::{Error, Handle, Lvgl, widgets::Label};

  fn show_progress(label: Handle<Label>, percent: u32) -> Result<(), Error> {
      let mut guard = Guard::lock();
      let lv = Lvgl::enter(&mut guard)?;
      label.get(&lv)?.set_text_u32(percent);
      Ok(())
  }
  ```

  A handle never keeps its object alive: once the object is deleted,
  `get` returns `Error::Dead`. To share a handle through a `static`, use
  `HandleCell<W>` (an atomic of 4 bytes with `get` and `set`).

- **Owned values.** Dropping an `Owned<W>` needs no guard: it records the
  request with two atomic stores, and the object (or timer, or group) is
  deleted at the next drain, which runs in `Lvgl::enter` and in
  `Lvgl::timer_handler`. Drop it on any thread.

- **Plain data.** For values the interface reads often (sensor readings,
  touch state), keep them in atomics and read them from a timer callback
  or an input driver, with no lock on the producer's side.

## Pitfalls

- **Never borrow a guard for `'static`.** No guard may be leaked
  (`Box::leak`) or kept in a `static` and then passed to `init` or
  `Lvgl::enter`. The session would give a `&'static Lvgl`, which a
  thread-local could carry into a restricted handler running in the middle
  of layout or rendering. Drop each guard when its access ends.
- **Do not lock inside callbacks.** Event handlers, timers and animation
  callbacks run inside `timer_handler`, with the lock already held:
  locking a std mutex there deadlocks. Use the shared access they receive
  (`e.lv()`, `ctx.lv()`), and `Lvgl::post` for exclusive work.
- **Do not implement `LvglAccess` for a guard shared with other locks**,
  and do not make two different mutexes produce the same guard type: the
  guard would no longer prove anything.
- **Interrupt handlers do not call LVGL.** They may call `tick_inc`,
  complete a deferred flush (`FlushDone::ready`) and update atomics read by
  drivers.
- **No compare-and-swap.** The crate itself uses only loads and stores
  outside the lock, so it builds for `thumbv6m` and `riscv32imc`; if your
  lock needs compare-and-swap there, take it from a critical-section
  implementation or your RTOS.

## See also

- [Access model](../explanation/access-model.md)
- [Object lifetime](../explanation/object-lifetime.md)
- [API overview](../reference/api-overview.md)
