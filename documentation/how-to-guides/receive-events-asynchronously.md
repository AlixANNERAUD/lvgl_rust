# Receive events asynchronously

This guide records LVGL events into a queue and reads them from an async
task with `next_event`, which takes your async lock, returns the next
event with the guard, or waits. It needs no executor dependency and no
atomics.

## What you need

- The `async` feature:

  ```toml
  lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust", features = ["async"] }
  ```

- A guard type implementing `LvglAccess` whose lock can be taken with an
  async function, as in step 3 of
  [Share LVGL between threads](share-lvgl-between-threads.md). A blocking
  lock wrapped in `async { ... }` also works.
- An executor of your choice (embassy, a hosted runtime, a hand-written
  one).

## Steps

### 1. Write a capture function

The capture function runs inside the event handler, with shared access,
and chooses what to record: return `None` to skip an event. The recorded
type must be `Copy + Send + 'static`.

```rust
use lvgl_rust::{Event, enums::EventCode};

#[derive(Clone, Copy)]
enum Input {
    Clicked,
    LongPressed,
}

fn capture(e: &Event<'_>) -> Option<Input> {
    if e.code() == EventCode::CLICKED {
        Some(Input::Clicked)
    } else if e.code() == EventCode::LONG_PRESSED {
        Some(Input::LongPressed)
    } else {
        None
    }
}
```

### 2. Declare a static queue and subscribe

```rust
use lvgl_rust::{StaticQueue, SubscribeOptions};

static INPUT: StaticQueue<Input, 8> = StaticQueue::new(capture);

// With a session (`lv`) and a view of the button:
button.subscribe(EventCode::ALL, &INPUT)?;
let rx = INPUT.receiver(&lv).expect("taken once"); // StaticRx<Input, 8>
```

- A queue holds 1 to 255 events (`N`), checked at compile time. When it
  is full, the newest event is dropped and `INPUT.take_overflow(&lv)`
  returns `true` once.
- `subscribe` takes an `EventCode` (`EventCode::ALL` for every code a
  handler may receive) or `SubscribeOptions`. Several objects can
  subscribe to the same queue.
- `receiver` returns the single receiver, then `None`. `StaticRx` is
  `Send`: move it into the task that waits.
- The queue is never freed. Deleting a subscribed object closes it: the
  events already queued are still returned, then the receiver reports the
  queue closed. With several objects on one queue, deleting any of them
  closes it for all.

### 3. Wait for events in a task

```rust
use lvgl_rust::{Lvgl, StaticRx, next_event};

async fn input_task(rx: StaticRx<Input, 8>) {
    loop {
        let (mut guard, event) = next_event(Guard::lock, &rx).await;
        let Some(input) = event else {
            break; // the object was deleted, or no session could start
        };
        // The lock is held: handle the event with a session.
        if let Ok(lv) = Lvgl::enter(&mut guard) {
            match input {
                Input::Clicked => { /* update the interface through `lv` */ }
                Input::LongPressed => { /* ... */ }
            }
            drop(lv);
        }
    } // the guard is dropped at the end of each iteration
}
```

`next_event(lock, &rx)` calls `lock()` and awaits the guard, starts a
session and pops an event. If the queue is empty, it registers the task's
waker under the lock, releases the lock and waits; the handler that
records the next event wakes it. It returns the guard together with the
event, so the task can act on the event without locking again.

### 4. Record the events of an object's children (optional)

```rust
list.subscribe(
    SubscribeOptions::new(EventCode::CLICKED).bubble_children(),
    &INPUT,
)?;
```

The children get LVGL's `EVENT_BUBBLE` flag now, and children created
later get it when they are created, so their events reach the list's
queue. They then also reach the list's other handlers and the class
handlers of widgets. To tell the children apart in `capture`, read
`e.target()` (or compare with `e.target_is(handle)`).

### 5. Give each object its own queue on the heap (optional, features `async` and `heap`)

```rust
use lvgl_rust::OwnedQueue;

let rx: OwnedQueue<Input, 4> = button.subscribe_owned(EventCode::ALL, capture)?;
// ... `next_event(Guard::lock, &rx).await` as above ...
rx.close(&mut lv); // detach now; or let the object's deletion close it
```

The queue lives in a block of LVGL's heap (a 4-byte header, then the
queue) and goes with its object. `OwnedQueue` is 8 bytes on 32-bit
targets, not `Clone`, and takes the object's registry slot (or shares the
one it has). Dropping it leaves the queue attached until the object is
deleted; `close` detaches it at once. Either way the block is freed at the
next drain.

### 6. Poll without async (optional)

Both receivers implement `Receiver`: `rx.try_recv(&lv)` returns
`Recv::Event(e)`, `Recv::Empty` or `Recv::Closed` (sticky), for a main
loop that checks the queue after `timer_handler`.

## Pitfalls

- **`bubble_children` cannot be undone.** The flag and the
  `CHILD_CREATED` handler stay for the object's lifetime, after
  `OwnedQueue::close` too.
- **Keep `capture` short.** It runs inside the LVGL call that sent the
  event, with the lock held: no blocking, no lock.
- **`None` means stop.** `next_event` returns `None` once the queue is
  closed, and also if `Lvgl::enter` fails (before `init`, after `deinit`,
  or when called from inside a callback).
- **Overflow is silent.** Size `N` for bursts, or check `take_overflow`.
- **One waiter per queue.** A queue keeps one waker: the receiver is not
  `Clone`.

## See also

- [Handle events](handle-events.md)
- [Use the heap](use-the-heap.md)
- [Event handlers](../explanation/event-handlers.md)
