# Your first LVGL application in Rust

In this tutorial you build a small LVGL program in Rust with lvgl-rust. It
shows a button and a label on a 320 × 240 screen, counts the clicks on the
button in the label, and checks what LVGL drew. Everything runs on your
desktop computer: the display renders into memory and a pretend mouse does
the clicking, so you need no screen and no board.

By the end you will have written a display driver, an input driver, a
screen with two widgets, an event handler and a main loop, which are the
parts of every lvgl-rust application.

## What you need

- Rust 1.89 or later (`rustc --version` prints it).
- Git and curl.
- A C compiler and libclang: lvgl-rust compiles LVGL from source and
  generates its bindings with bindgen. On Debian or Ubuntu, install the
  `clang` and `libclang-dev` packages.

## 1. Create the project

Create a new binary crate and go into it:

```sh
cargo new first-application
cd first-application
```

## 2. Add lvgl-rust

Open `Cargo.toml` and add lvgl-rust to the dependencies:

```toml
[dependencies]
lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust" }
```

Cargo fetches the repository and its LVGL submodule on the next build.

## 3. Give LVGL its configuration

LVGL is configured at compile time by a header, `lv_conf.h`. Download the
configuration that lvgl-rust uses for its own tests into a new
`lvgl-configuration` directory:

```sh
mkdir lvgl-configuration
curl -L -o lvgl-configuration/lv_conf.h https://raw.githubusercontent.com/AlixANNERAUD/lvgl_rust/main/lvgl-rust/conf/lv_conf.h
```

lvgl-rust keeps track of objects through LVGL's object ID hooks, so the
configuration must turn them on and leave them to lvgl-rust. Check that it
does:

```sh
grep -E 'define LV_(USE_OBJ_ID|OBJ_ID_AUTO_ASSIGN|USE_OBJ_ID_BUILTIN) ' lvgl-configuration/lv_conf.h
```

```text
#define LV_USE_OBJ_ID 1
#define LV_OBJ_ID_AUTO_ASSIGN 1
#define LV_USE_OBJ_ID_BUILTIN 0
```

The build finds the header through the `DEP_LV_CONFIG_PATH` environment
variable. Set it for every Cargo command of the project by creating
`.cargo/config.toml`:

```sh
mkdir .cargo
```

```toml
[env]
DEP_LV_CONFIG_PATH = { value = "lvgl-configuration", relative = true }
```

`relative = true` makes the path relative to the project directory, so the
build finds the header wherever you run Cargo from.

## 4. Build LVGL

Build the project as `cargo new` created it:

```sh
cargo build
```

The first build compiles LVGL and generates the bindings, which takes a few
minutes. Later builds reuse them. When it ends with `Finished`, LVGL and
lvgl-rust are ready.

Your project now looks like this:

```text
first-application/
├── .cargo/
│   └── config.toml
├── Cargo.toml
├── lvgl-configuration/
│   └── lv_conf.h
└── src/
    └── main.rs
```

In the next steps you replace the content of `src/main.rs` piece by piece.
Delete what `cargo new` put there, then add each piece below the previous
one. You run the program once every piece is in place.

## 5. Hold the LVGL lock

LVGL is not thread safe: only one part of a program may call it at a
time. lvgl-rust asks you to prove that you hold your lock before it lets
you call LVGL. The proof is a guard type that implements the `LvglAccess`
trait.

Start `src/main.rs` with the imports of the whole program and the lock:

```rust
use lvgl_rust::{
    Display, DisplayCell, DrawBuf, DrawBuffers, Error, Flush, FlushRequest, Handle, Indev,
    IndevCell, IndevData, IndevKind, Lvgl, LvglAccess, Read, RenderMode, WidgetExt,
    enums::{Align, EventCode},
    tick_inc,
    widgets::{Button, Label},
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed},
};

/// The LVGL lock: every LVGL call happens while its guard is alive.
static LVGL: Mutex<()> = Mutex::new(());

/// Proof that the program holds the LVGL lock.
struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex that serializes every LVGL call of the
// program.
unsafe impl LvglAccess for Guard {}
```

`unsafe impl` is your promise that a `Guard` exists only while the mutex is
locked. [The access model](../explanation/access-model.md) explains what
lvgl-rust builds on top of it.

## 6. Write the display driver

LVGL draws into a buffer, then hands each finished area to a display
driver, which sends it to the screen. Here the "screen" is an array in
memory, `FRAMEBUFFER`, and the driver also counts how many areas it
received.

A driver is a type that implements `Flush`. lvgl-rust keeps it in a
`DisplayCell` static while the display exists, and LVGL renders into a
`DrawBuf` static, here 24 rows of the screen:

```rust
const WIDTH: usize = 320;
const HEIGHT: usize = 240;
/// `lv_conf.h` renders in RGB565: 2 bytes per pixel.
const BYTES_PER_PIXEL: usize = 2;

/// The pixels of the screen, as a panel would hold them.
static FRAMEBUFFER: Mutex<[u8; WIDTH * HEIGHT * BYTES_PER_PIXEL]> =
    Mutex::new([0; WIDTH * HEIGHT * BYTES_PER_PIXEL]);
/// How many areas LVGL has sent to the display.
static FLUSHES: AtomicU32 = AtomicU32::new(0);

/// The display driver: copies each area LVGL renders into `FRAMEBUFFER`.
struct Panel;

impl Flush for Panel {
    fn flush(&mut self, mut request: FlushRequest<'_>) {
        let area = request.area();
        let stride = request.stride();
        let row_bytes = (area.x2 - area.x1 + 1) as usize * BYTES_PER_PIXEL;
        let mut framebuffer = FRAMEBUFFER.lock().unwrap();
        for (y, row) in request.pixels().chunks(stride).enumerate() {
            let start = ((area.y1 as usize + y) * WIDTH + area.x1 as usize) * BYTES_PER_PIXEL;
            let n = row_bytes.min(row.len());
            if let Some(out) = framebuffer.get_mut(start..start + n) {
                out.copy_from_slice(&row[..n]);
            }
        }
        FLUSHES.fetch_add(1, Relaxed);
    }
}

/// Where the display driver lives while the display exists.
static PANEL: DisplayCell<Panel> = DisplayCell::new();
/// LVGL renders into this buffer: 24 rows of the screen at a time.
static DRAW_BUFFER: DrawBuf<{ WIDTH * BYTES_PER_PIXEL * 24 }> = DrawBuf::new();
```

The configuration you downloaded renders 16-bit RGB565 pixels, hence 2
bytes per pixel. `FlushRequest` gives the area of the screen, the bytes
between two rows (`stride`) and the pixels.

## 7. Write the input driver

LVGL polls its input devices regularly. An input driver implements `Read` and
fills in where the pointer is and whether it is pressed. This one reports a
pretend mouse that the program moves through three atomics:

```rust
/// The position and state of a pretend mouse, moved by `click`.
static POINTER_X: AtomicI32 = AtomicI32::new(0);
static POINTER_Y: AtomicI32 = AtomicI32::new(0);
static POINTER_DOWN: AtomicBool = AtomicBool::new(false);

/// The input driver: reports the pretend mouse to LVGL.
struct Pointer;

impl Read for Pointer {
    fn read(&mut self, data: &mut IndevData<'_>) {
        data.set_point(POINTER_X.load(Relaxed), POINTER_Y.load(Relaxed));
        data.set_pressed(POINTER_DOWN.load(Relaxed));
    }
}

/// Where the input driver lives while the input device exists.
static POINTER: IndevCell<Pointer> = IndevCell::new();
```

## 8. Count the clicks

When the button is clicked, its event handler runs. A handler runs while
LVGL is busy processing the input, so it gets shared access to LVGL only.
Setting a label's text needs no more than that, but creating, deleting or
moving objects needs exclusive access, which a handler never has. The way
out is `post`: the handler queues a function, which runs once
`timer_handler` has finished its work, with exclusive access. This program
uses `post` for the count, so the pattern is in place when your handlers
need more.

Add the function the handler will post. It counts the click and shows the
count in a label:

```rust
/// The number of clicks so far.
static CLICKS: AtomicU32 = AtomicU32::new(0);

/// Counts a click and shows the new count in `label`. The button's
/// handler posts it: it runs once the handler has returned, with
/// exclusive access to LVGL.
fn count_click(lv: &mut Lvgl, label: Handle<Label>) {
    let clicks = CLICKS.fetch_add(1, Relaxed) + 1;
    if let Ok(label) = label.get(lv) {
        label.set_text_u32(clicks);
    }
}
```

The label arrives as a `Handle<Label>`, a 4-byte reference that stays
valid to store: `label.get(lv)` returns the label, or an error if it was
deleted in the meantime.

## 9. Build the screen

Now create the widgets on the active screen: a label at the top, showing
`0`, and a button in the middle with the text "Click me". The button's
handler, registered with `on_with`, posts `count_click` with the label's
handle on every `CLICKED` event:

```rust
/// Builds the screen: a label showing the count, and a button below
/// it. Returns the label's handle.
fn build(lv: &mut Lvgl) -> Result<Handle<Label>, Error> {
    let (count, button) = {
        let mut screen = lv.screen_active_mut().expect("the display has a screen");
        let count = Label::create(&mut screen)?;
        count.set_text_u32(0);
        let count = count.handle()?;
        let button = Button::create(&mut screen)?.handle()?;
        (count, button)
    };
    count.get_mut(lv)?.align(Align::TOP_MID, 0, 40);

    let mut button = button.get_mut(lv)?;
    button.set_size(120, 50);
    button.center();
    Label::create(&mut button)?.set_text_str("Click me")?;
    button
        .view()
        .on_with(EventCode::CLICKED, count, |event, count| {
            let _ = event.lv().post(count_click, count);
        })?;
    Ok(count)
}
```

Creating a widget needs exclusive access to its parent, an `ObjMut`. The
new widget comes back as a view (`&Label`), which you can use right away
to set its text, and `handle()` turns it into a `Handle` you can keep.
Moving and resizing go through the widget's own `ObjMut`, which
`get_mut(lv)` gives. [Object lifetime](../explanation/object-lifetime.md)
explains views, handles and `ObjMut`.

## 10. Let time pass and click

LVGL needs two things from the main loop: the time that has passed, given
with `tick_inc`, and regular calls to `timer_handler`, which reads the
input devices, runs the handlers and the posted functions, and redraws
what changed. `timer_handler` is a method of the session that
`Lvgl::enter` opens with the guard; `tick_inc` needs no session, so a
timer interrupt can call it.

Add a function that lets some milliseconds pass, a function that clicks
with the pretend mouse, and one that reads a pixel back from the
framebuffer:

```rust
/// Lets `ms` milliseconds pass: LVGL reads the pointer, runs the
/// handlers and the posted functions, and redraws what changed.
fn frame(ms: u32) {
    let mut guard = Guard(LVGL.lock().unwrap());
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    tick_inc(ms);
    lv.timer_handler();
}

/// Presses and releases the pretend mouse at (`x`, `y`).
fn click(x: i32, y: i32) {
    POINTER_X.store(x, Relaxed);
    POINTER_Y.store(y, Relaxed);
    POINTER_DOWN.store(true, Relaxed);
    frame(40);
    POINTER_DOWN.store(false, Relaxed);
    frame(40);
}

/// The RGB565 pixel at (`x`, `y`) of the framebuffer.
fn pixel(x: usize, y: usize) -> u16 {
    let framebuffer = FRAMEBUFFER.lock().unwrap();
    let start = (y * WIDTH + x) * BYTES_PER_PIXEL;
    u16::from_le_bytes([framebuffer[start], framebuffer[start + 1]])
}
```

`click` keeps the pointer pressed for 40 ms, then releases it: LVGL sends
`CLICKED` to the object under the pointer on release.

## 11. Put it together in `main`

Last, `main` starts LVGL with `lvgl_rust::init`, creates the display and
the input device, builds the screen, clicks the button three times, and
checks the result:

```rust
fn main() -> Result<(), Error> {
    let count = {
        let mut guard = Guard(LVGL.lock().unwrap());
        let mut lv = lvgl_rust::init(&mut guard)?;
        // Dropping a `Display` or an `Indev` keeps the device.
        let _display = Display::create(
            &mut lv,
            &PANEL,
            Panel,
            WIDTH as u32,
            HEIGHT as u32,
            DrawBuffers::single(&DRAW_BUFFER),
            RenderMode::Partial,
        )?;
        let _pointer = Indev::create(&mut lv, &POINTER, Pointer, IndevKind::Pointer)?;
        build(&mut lv)?
    };

    // Draw the screen once, click the button three times, and let LVGL
    // draw the new count.
    frame(40);
    for _ in 0..3 {
        click(WIDTH as i32 / 2, HEIGHT as i32 / 2);
    }
    for _ in 0..5 {
        frame(40);
    }

    let mut guard = Guard(LVGL.lock().unwrap());
    let lv = Lvgl::enter(&mut guard)?;
    let mut text = [0u8; 12];
    let shown = count.get(&lv)?.get_text_into(&mut text)?;
    // Inside the button, left of its text, and the screen's corner.
    let button_pixel = pixel(WIDTH / 2 - 50, HEIGHT / 2);
    let background_pixel = pixel(4, HEIGHT - 4);
    println!(
        "{} clicks, the label shows {:?}, {} flushes, button pixel {:#06x}, background pixel {:#06x}",
        CLICKS.load(Relaxed),
        shown,
        FLUSHES.load(Relaxed),
        button_pixel,
        background_pixel
    );
    assert_eq!(CLICKS.load(Relaxed), 3, "three clicks");
    assert_eq!(shown, Some(c"3"), "the label shows the count");
    assert!(FLUSHES.load(Relaxed) > 0, "LVGL sent pixels to the display");
    assert_ne!(button_pixel, background_pixel, "the button is drawn");
    Ok(())
}
```

The two pixels are read inside the button, left of its text, and in the
bottom-left corner of the screen. If LVGL drew the button, they have
different colours.

## 12. Run it

```sh
cargo run
```

The program prints one line:

```text
3 clicks, the label shows Some("3"), 28 flushes, button pixel 0x24be, background pixel 0xf7be
```

The number of areas LVGL sent to the display and the two pixel values
depend on the LVGL version and its default theme, so yours may differ.
The assertions pass: the button was clicked three times, the
label shows `3`, LVGL sent pixels to the display and the button is drawn.

You have written a complete LVGL application in Rust. On a real device,
the same program shape stays: the display driver sends the pixels to a
panel instead of an array, the input driver reads a touch controller, and
the main loop calls `tick_inc` and `timer_handler` forever.

The same program is part of the repository, as
[`lvgl-rust/examples/first_application.rs`](../../lvgl-rust/examples/first_application.rs)
(wrapped in a module there, because the repository also builds it in a
configuration without handles). In a checkout of the repository,
`mise run example` runs it, then the larger desktop example.

## What next

- Connect a real screen and touch controller:
  [write a display driver](../how-to-guides/write-a-display-driver.md) and
  [write an input driver](../how-to-guides/write-an-input-driver.md).
- React to more events, and pass data to handlers:
  [handle events](../how-to-guides/handle-events.md).
- Adapt `lv_conf.h` to your application:
  [configure LVGL](../how-to-guides/configure-lvgl.md).
- Move the program to a microcontroller:
  [build for embedded targets](../how-to-guides/build-for-embedded-targets.md)
  and [reduce the footprint](../how-to-guides/reduce-the-footprint.md).
- Run the main loop and the rest of the application on different threads:
  [share LVGL between threads](../how-to-guides/share-lvgl-between-threads.md).
- Understand why handlers get shared access only:
  [event handlers](../explanation/event-handlers.md).
- See what the checks cost: on the host of the reference measurements, resolving a
  `Handle<Label>` takes 3.1 ns, and on `thumbv6m-none-eabi` the crate adds
  4498 bytes of code and 188 bytes of `.bss` with 16 handle slots ([overhead measurements](../reference/overhead.md),
  [where the overhead comes from](../explanation/overhead.md)).
