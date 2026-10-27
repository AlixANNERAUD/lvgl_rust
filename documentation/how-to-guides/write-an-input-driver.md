# Write an input driver

This guide connects a touch panel, a mouse, a keypad, a rotary encoder or
hardware buttons to LVGL through a `Read` driver.

## What you need

- A display created as in [Write a display driver](write-a-display-driver.md):
  input devices attach to the default display.
- A way to read the device's state. Interrupt handlers usually store it
  in atomics that the driver reads.

## Steps

### 1. Implement `Read`

LVGL polls each input device from `Lvgl::timer_handler` (every
`LV_DEF_REFR_PERIOD` milliseconds by default) and calls `Read::read`,
which fills an `IndevData`:

```rust
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use lvgl_rust::{IndevData, Read};

// Written by the touch controller's interrupt handler.
static TOUCH_X: AtomicI32 = AtomicI32::new(0);
static TOUCH_Y: AtomicI32 = AtomicI32::new(0);
static TOUCH_DOWN: AtomicBool = AtomicBool::new(false);

struct Touch;

impl Read for Touch {
    fn read(&mut self, data: &mut IndevData<'_>) {
        data.set_point(TOUCH_X.load(Relaxed), TOUCH_Y.load(Relaxed));
        data.set_pressed(TOUCH_DOWN.load(Relaxed));
    }
}
```

What each kind of device reports:

| `IndevKind` | Setters |
| --- | --- |
| `Pointer` (touch panel, mouse) | `set_point(x, y)`, `set_pressed(bool)` |
| `Keypad` (keyboard, keys) | `set_key(u32)` (a character or an `LV_KEY_*` code), `set_pressed(bool)` |
| `Encoder` (rotary knob) | `set_steps(i16)` (steps since the last read), `set_pressed(bool)` |
| `Button` (hardware buttons) | `set_button(id)`, `set_pressed(bool)` |

For buffered input, `set_continue_reading(true)` asks LVGL to call
`read` again right away. The driver must be `Send + 'static`.

### 2. Create the device with a static cell

```rust
use lvgl_rust::{Indev, IndevCell, IndevKind};

static TOUCH: IndevCell<Touch> = IndevCell::new();

let touch = Indev::create(&mut lv, &TOUCH, Touch, IndevKind::Pointer)?;
```

`Indev::create` fails with `Error::AlreadyOwned` if the cell is in use,
`Error::OutOfMemory` if LVGL cannot allocate the device, and
`Error::InvalidArg` for `IndevKind::Button`, which needs its points
(step 4). Dropping the `Indev` keeps the device; `touch.delete(&mut lv)`
deletes it and releases the cell.

### 3. Route keys and encoder steps to a focus group

Keypads and encoders send their input to the focused object of a group:

```rust
use lvgl_rust::{Indev, IndevCell, IndevData, IndevKind, Read, WidgetExt, sys};

struct Keys;

impl Read for Keys {
    fn read(&mut self, data: &mut IndevData<'_>) {
        // From your key scanning: here, Enter held down.
        data.set_key(sys::lv_key_t_LV_KEY_ENTER);
        data.set_pressed(true);
    }
}

static KEYS: IndevCell<Keys> = IndevCell::new();

let group = lv.create_group()?; // Owned<Group>: keep it alive
let mut keypad = Indev::create(&mut lv, &KEYS, Keys, IndevKind::Keypad)?;
keypad.set_group(&mut lv, Some(group.handle()))?;

// Add objects one by one...
group.handle().add_obj(&mut lv, button)?; // button: Handle<Obj>
// ...or make it the default group: focusable widgets created later join it.
group.handle().set_default(&lv)?;
```

`Handle<Group>` also has `focus_next`, `focus_prev`, `set_wrap`,
`set_editing` (for encoders: steps edit the focused object instead of
moving the focus), `focused`, `obj_count` and `remove_all`. Dropping the
`Owned<Group>` deletes the group at the next drain and detaches its
objects and devices. `ObjMut::remove_from_group` removes one object.

### 4. Map hardware buttons to points of the screen

A button device presses a point of the screen for each button id. LVGL
keeps the points, so they are a `'static` array:

```rust
use lvgl_rust::{Indev, IndevCell, IndevData, Read, sys};

static POINTS: [sys::lv_point_t; 2] = [
    sys::lv_point_t { x: 40, y: 220 },  // button 0: "Back"
    sys::lv_point_t { x: 280, y: 220 }, // button 1: "Next"
];

struct Buttons;

impl Read for Buttons {
    fn read(&mut self, data: &mut IndevData<'_>) {
        let (id, pressed) = read_gpio(); // your code
        data.set_button(id);
        data.set_pressed(pressed);
    }
}

static BUTTONS: IndevCell<Buttons> = IndevCell::new();

Indev::create_button(&mut lv, &BUTTONS, Buttons, &POINTS)?;
```

An id past the array reads as button 0 released, so a wrong id never
makes LVGL read past `POINTS`. An empty array does not compile.

### 5. Keep the driver on LVGL's heap instead (optional, feature `heap`)

`Indev::create_boxed(&mut lv, reader, kind)` and
`Indev::create_button_boxed(&mut lv, reader, &POINTS)` take no cell: the
driver goes in a block of LVGL's heap, freed when the device is deleted.
They fail with `Error::OutOfMemory` if the block cannot be allocated.

## Pitfalls

- **Read is called with the lock held.** It runs inside `timer_handler`:
  keep it short, do not block on a bus transfer, and never take the LVGL
  lock in it. Sample the hardware elsewhere and read the latest state.
- **Delete devices outside callbacks.** `Indev::delete` takes `&mut Lvgl`
  at depth 0, because LVGL keeps the device across the events it sends
  while processing input.
- **Groups need handles.** Groups, `add_obj` and `set_group` do not exist
  in views-only mode (`LVGL_RUST_HANDLE_SLOTS=0`). The group takes one
  registry slot, and each object added to it needs a handle.
- **Raw access.** `Indev::as_ptr` returns the `lv_indev_t` for calls that have
  no safe wrapper (`lv_indev_set_long_press_time`, `lv_indev_set_cursor`,
  ...), made under the lock.

## See also

- [Handle events](handle-events.md)
- [API overview](../reference/api-overview.md)
