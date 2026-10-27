# API overview

The public API of `lvgl-rust`, by area. Each table gives the item, what
it is, the access it needs and, where it has one, its cost on 32-bit
targets. Measured costs (time and firmware sizes) are in
[Overhead measurements](overhead.md); why the API is shaped this way is in
[Access model](../explanation/access-model.md) and
[Object lifetime](../explanation/object-lifetime.md).

Access values:

| Value | Meaning |
| --- | --- |
| none | No token: callable from any thread or interrupt. |
| shared | `&Lvgl`, or a view (`&Obj`, `&Label`, ...), which borrows one. Event handlers, timer functions, animation callbacks and observers get shared access. |
| exclusive | `&mut Lvgl`, or an `ObjMut`, which borrows one. Never available in callbacks. |
| restricted | A `QueryEvent`: no `Lvgl` and no view. |

Items marked "handles" do not exist with `LVGL_RUST_HANDLE_SLOTS=0`
(views-only mode, cfg `lvgl_rust_views_only`). Items marked with a
feature exist only with that Cargo feature ([Cargo features](features.md)).
Widget-specific items exist only when the widget is enabled in
`lv_conf.h` (cfg `lv_use_<widget>`, see
[Build configuration](build-configuration.md)).

## Sessions and access

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `unsafe trait LvglAccess` | Proof of exclusive, non-recursive access to LVGL, implemented by the application on its own lock guard type. The end of one value's lifetime must synchronise-with the start of the next (Release/Acquire or stronger). No value may be borrowed for `'static` by `init` or `Lvgl::enter`. | — | — |
| `init(&mut guard) -> Result<Session, Error>` | Calls `lv_init`, installs the crate's tick source and returns the first session. `Error::Reentrant` if LVGL is initialised or a session is active, `Error::NotInitialized` after `Lvgl::deinit`. | guard | — |
| `Lvgl` | Shared (`&Lvgl`) or exclusive (`&mut Lvgl`) access. Zero-sized, `!Send`, `!Sync`. | — | 0 B |
| `Session<'g>` | Access for as long as the guard is borrowed; dereferences to `Lvgl`. Dropping it ends the session. | — | 0 B |
| `Lvgl::enter(&mut guard) -> Result<Session, Error>` | Starts a session, then runs the drain (deferred requests, posted functions, deferred frees). `Error::NotInitialized` before `init` or after `deinit`, `Error::Reentrant` if a session is active or a callback is running. | guard | — |
| `Lvgl::timer_handler(&mut self) -> u32` | Drain, `lv_timer_handler`, drain again. Returns the milliseconds until the next timer. | exclusive | — |
| `tick_inc(ms: u32)` | Advances the crate's tick counter. `fetch_add` where the target has 32-bit atomic read-modify-write; a load and a store otherwise (`thumbv6m`, `riscv32imc`), where concurrent callers may lose ticks. | none | — |
| `Lvgl::set_tick_cb(&mut self, f: extern "C" fn() -> u32)` | Replaces the crate's tick source. | exclusive | — |
| `Lvgl::deinit(session) -> Result<(), Error>` | Drain, then `lv_deinit` (deletes every object). Terminal: `init` and `enter` fail afterwards. | session | — |
| `unsafe Lvgl::assume<'a>() -> &'a mut Lvgl` | Exclusive access without a session, for interop with raw code. | caller's guarantee | — |
| `unsafe Lvgl::enter_callback() -> Option<DepthGuard>` | Enters a callback level for raw C callbacks that call safe code. `None` at `LVGL_RUST_MAX_CALLBACK_DEPTH` (counted as `skipped` with `debug-counters`). | caller's guarantee | — |
| `DepthGuard` | One callback level; `lv()` gives shared access. Zero-sized, `!Send`; dropping it leaves the level. | — | 0 B |

## Views and widgets

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `Obj` | The base object (`lv_obj_t`), used only behind a reference. A view `&Obj` is the object's address and borrows the `Lvgl` it came from. | shared | 4 B (a pointer) |
| `widgets::*` | One `#[repr(transparent)]` view type per widget class enabled in `lv_conf.h`, dereferencing to its base class. With `lvgl-rust/conf/lv_conf.h`: `Animimg` (base `Image`), `Arc`, `Arclabel`, `Bar`, `Button`, `ButtonMatrix`, `Calendar`, `Canvas` (base `Image`), `Chart`, `Checkbox`, `Dropdown`, `Image`, `ImageButton`, `Keyboard` (base `ButtonMatrix`), `Label`, `Led`, `Line`, `MsgBox`, `Roller`, `Scale`, `Slider` (base `Bar`), `SpanGroup`, `Spinbox` (base `TextArea`), `Spinner` (base `Arc`), `Switch`, `Table`, `TabView`, `TextArea`, `TileView`; the others have base `Obj`. | shared | 4 B |
| `unsafe trait Widget` | A widget class with a view type: `type Base`, `fn class()`. | — | — |
| `unsafe trait IsA<B>` | `Self` is `B` or a subclass of it; used by `upcast` and `into_base`. | — | — |
| Generated view methods | Every function the access audit marks shared: a `&self` method on the view of its class (`obj.set_ext_click_area(v)`, `label.set_text(Some(c"..."))`). A shared function is a view method only: from an `ObjMut`, call it through `.view()`. | shared | Plain FFI call; see [Overhead measurements](overhead.md) |
| Generated `Lvgl` methods | Functions without an object: `&self` on `Lvgl` when shared (`anim_count_running`, `area_get_width`, ...), `&mut self` when exclusive (`anim_delete_all`, `anim_refr_now`, ...). | shared or exclusive | — |
| `functions::*` | Pure or interrupt-safe functions, as free functions (`color_hex`, `color_make`, `color_mix`, `pct` with `lvgl-rust/conf`). | none | — |
| `enums::*` | One `#[repr(transparent)]` newtype per LVGL enum, with one constant per value (`Align::CENTER`, `EventCode::CLICKED`, `Part::MAIN`, `State::PRESSED`, ...). | — | 4 B |
| `Obj::as_ptr(&self) -> *mut lv_obj_t` | The raw pointer, valid until the next exclusive call. | shared | — |
| `unsafe Obj::from_raw(&Lvgl, NonNull<lv_obj_t>) -> &Obj` | A view of a raw object, which must be live and not being deleted. | shared | — |
| `Obj::downcast::<W>(&self) -> Option<&W>` | This object as a `W`, if its class is `W` or a subclass (`lv_obj_has_class`). | shared | — |
| `Obj::child(&self, index: i32) -> Option<&Obj>` | The child at `index` (negative: from the end). | shared | — |
| `Obj::parent(&self) -> Option<&Obj>` | The parent; `None` for a screen or a layer. | shared | — |
| `Lvgl::screen_active(&self) -> Option<&Obj>` | The active screen of the default display. | shared | — |
| `Lvgl::edit(&mut self, select) -> Option<ObjMut<W>>` | Exclusive access to the object that `select` chooses among the views of shared access. | exclusive | — |
| Borrowed text getters | A string getter whose result points into the object becomes `x_into(&self, buf: &mut [u8])` on the view (copies the text out; `Error::TooSmall` if it does not fit with its NUL) and `x(&mut self)` on `ObjMut` (borrows it). Example: `Label::get_text_into`. | shared / exclusive | — |

Getters that return objects (`child`, `parent`, `screen_active`,
`Group::focused`, ...) return `None` inside callbacks for an object whose
ancestor is being deleted.

## Exclusive access to an object: `ObjMut`

`ObjMut<'s, W>` borrows `&mut Lvgl`, so no view exists while it does. 4 B
(a pointer). Generated functions the access audit marks exclusive are
`ObjMut` methods only; when a later audit moves a function from exclusive
to shared, its method moves to the view.

| Item | What it is | Errors |
| --- | --- | --- |
| `view(&self) -> &W` | A view of the object, for the shared API. | — |
| `into_view(self) -> &'s W` | Gives up exclusive access for a view lasting as long as that access was borrowed. | — |
| `into_base::<B>(self) -> ObjMut<'s, B>` | The same object as a base class (`W: IsA<B>`); takes `self`. | — |
| `delete(self)` | Deletes the object and its children. | `Internal`, `Busy` |
| `clean(&mut self)` | Deletes the children. | `Internal` |
| `move_to_index(&mut self, index: i32)` | Moves the object among its siblings (negative: from the end; out of range: no change). | `InvalidArg` (a screen), `Internal` |
| `set_parent(&mut self, parent: Handle<Obj>)` (handles) | Moves the object under `parent`. | `Dead`, `InvalidArg` (a cycle or a screen), `Internal` |
| `set_parent_to(&mut self, select)` | Moves the object under the parent `select` chooses. | `Dead` (none chosen), `InvalidArg`, `Internal` |
| `load_screen(self, anim: ScreenAnim)` | Loads the object as the active screen; it must have no parent. | `InvalidArg`, `Internal`, `Busy` |
| `send_event(self, code: SendableEventCode) -> Result<Self, Error>` | Sends `code` with a NULL parameter; consumes the access and gives it back only if the object survived. | `Dead` |
| `animate(&mut self, anim: Anim<D>)` | Starts an animation (see [Animations](#animations)). | `OutOfMemory` |
| `delete_animations(&mut self) -> bool` | Deletes every animation of the object; whether there was one. | — |
| `remove_from_group(&mut self)` | Removes the object from its focus group. | — |
| `set_layout(&mut self, layout: u32)` | `lv_obj_set_layout`; `LV_LAYOUT_GRID` refused. | `InvalidArg` |
| `set_style_layout(&mut self, value, selector)` | `lv_obj_set_style_layout`; `LV_LAYOUT_GRID` refused. | `InvalidArg` |
| `remove_local_style_prop(&mut self, prop, selector) -> Result<bool, Error>` | `lv_obj_remove_local_style_prop`; the grid templates refused. `Ok(false)` if the property was not set. | `InvalidArg` |
| `set_grid_dsc_array(&mut self, columns, rows)` | The only way to set the grid layout: both `&'static GridTemplate`s go into the object's local style for every state, then the layout. | `OutOfMemory` |
| `set_grid_cell(&mut self, column_align, column, column_span, row_align, row, row_span)` | Places the object in its parent's grid; positions and spans beyond the grid are clamped by LVGL. | `InvalidArg` (span above `GridTemplate::MAX_SPAN`) |
| `set_style_grid_cell_column_span`, `set_style_grid_cell_row_span` | Style properties, bounded by `GridTemplate::MAX_SPAN`. | `InvalidArg` |

Structural rules, checked before LVGL is called (`base/structural.rs`):

- Display layers cannot be deleted, owned or reparented (`Internal`).
- Widgets keep pointers to some of their children and find others by
  index. The part tables of `lvgl-rust-gen/tables/lvgl-9.6/classes.toml`
  list them. A part, or an ancestor of one, cannot be deleted, owned or
  moved; a widget with parts cannot be cleaned; nothing can be moved into
  an indexed widget. Inside a widget class the tables do not list, nothing
  can be deleted, owned, moved, reordered or cleaned, and no child can be
  created. Refusals are `Internal`.
- A closed widget (the tile view) gets children only from its own API; a
  pinned one (a calendar header) never changes parent.
- A screen being loaded, or the previous screen of an animated load,
  cannot be deleted yet (`Busy`). The previous screen cannot be loaded
  (`Busy`).
- `set_parent` refuses cycles and screens (`InvalidArg`). Only parentless
  objects can be loaded as screens (`InvalidArg`).

## Handles and ownership (handles)

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `Handle<W>` | A reference that can be stored and sent between threads: slot index and 16-bit generation. `Copy`, `Eq`, `Hash`. Never keeps its object alive; reports a deleted object as `Error::Dead`, whatever deleted it. A handle kept after its object died is reported dead until its slot has been reused 65,536 times. | — | 4 B; `Option<Handle<W>>` 4 B |
| `Handle::get(self, &Lvgl) -> Result<&W, Error>` | A view, or `Dead`. Checks the class unless `W` is `Obj`. | shared | See [Overhead measurements](overhead.md) |
| `Handle::get_mut(self, &mut Lvgl) -> Result<ObjMut<W>, Error>` | Exclusive access, or `Dead`. | exclusive | — |
| `Handle::upcast::<B>(self) -> Handle<B>` | The same handle as a base class. | none | — |
| `Handle::downcast::<D>(self, &Lvgl) -> Option<Handle<D>>` | The same handle as a subclass, if the object's class is `D` or a subclass. | shared | — |
| `Owned<W: Tracked>` | The single owner of an object, a timer or a group. Not `Clone`. Dropping it records a deletion, carried out at the next drain, from any thread and without a session. | — | 4 B; `Option<Owned<W>>` 4 B |
| `Owned::handle(&self) -> Handle<W>` | A handle to what it owns. | none | — |
| `Owned::get`, `Owned::get_mut` | As for `Handle`. | shared / exclusive | — |
| `Owned::delete(self, &mut Lvgl) -> Result<(), Error>` | Deletes at once. A screen LVGL is still loading gives `Busy` and stays pending, retried at each drain. A permanent structural refusal (`Internal`, only reachable after raw code moved the object) is returned: the object stays alive, released from its owner, and its handles report `Dead`. If a pending `Busy` deletion is later refused for good, the drain releases it the same way (counted as `refused` with `debug-counters`). What is already dead is released and gives `Ok(())`. | exclusive | — |
| `Owned::upcast::<B>(self) -> Owned<B>` | The same owner as a base class. | none | — |
| `HandleCell<W>` | An optional handle in a `static`: `new()` (const), `get() -> Option<Handle<W>>`, `set(Option<Handle<W>>)`. Atomic loads and stores only. | none | 4 B |
| `trait WidgetExt` | Implemented for every widget. `handle(&self) -> Result<Handle<Self>, Error>` takes a registry slot the first time (`RegistryFull`); `own(&self) -> Result<Owned<Self>, Error>` (`AlreadyOwned`, `Internal`, `RegistryFull`). | shared | one registry slot |
| `trait Tracked` | What an `Owned` can own: every widget, `Timer`, `Group`. Sealed. | — | — |
| `trait Component` | A widget made of other widgets, as a plain struct: `type Root: Widget`, `fn root(&self) -> Handle<Self::Root>`, provided `fn delete(self, &mut Lvgl) -> Result<(), Error>` (deletes the root and so the parts; `Dead` if the root is gone, otherwise the errors of `ObjMut::delete`). Nothing is allocated beyond the objects and the registry slots of the handles. | exclusive (`delete`) | — |

Registry: a static array of 8-byte slots (16 B on 64-bit hosts) plus a
24-byte header, `8·N + 24` bytes of `.bss` on 32-bit targets with N =
`LVGL_RUST_HANDLE_SLOTS` (default 32, rounded up to a power of two). In
views-only mode only the 8-byte header remains. Each LVGL object costs
4 B for `lv_obj_t::id` (`LV_USE_OBJ_ID`), whether or not it has a handle.

## Creation

Creating an object needs exclusive access: creation can reach code that
moves or deletes other objects. The new object is lent as a view
borrowing the parent's `ObjMut`; exclusive calls on it go through a handle
(or `Lvgl::edit`) once that borrow ends. Callbacks create objects through
`Lvgl::post`.

| Item | What it is | Errors |
| --- | --- | --- |
| `Obj::create(parent: &mut ObjMut<P>) -> Result<&Obj, Error>` | `lv_obj_create` under `parent`. | `Internal` (a parent whose children a widget indexes or casts), `OutOfMemory` |
| `<Widget>::create(parent: &mut ObjMut<P>) -> Result<&Widget, Error>` | Generated for every widget class, same shape (`Label::create(&mut parent)?`). | as above |
| `Obj::create_owned`, `<Widget>::create_owned(parent) -> Result<Owned<W>, Error>` (handles) | Creates and takes ownership. | as above, plus `RegistryFull`, `AlreadyOwned` |
| `TileView::create(parent)` | Hand-written: replaces LVGL's scroll-end handler with a checked one (see [Widget-specific checks](#widget-specific-checks)). | `Internal`, `OutOfMemory` |
| `Lvgl::create_screen(&mut self)` | A screen (no parent): `Owned<Obj>` with handles, `ObjMut<Obj>` in views-only mode. | `OutOfMemory`, `RegistryFull` |
| `Lvgl::screen_active_mut(&mut self) -> Option<ObjMut<Obj>>` | Exclusive access to the active screen, to create objects on it. | — |

## Screens

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `Lvgl::load_screen(&mut self, screen: Handle<Obj>, anim: ScreenAnim)` (handles) | Loads `screen`. Errors: `Dead`, `InvalidArg` (has a parent), `Internal` (a layer), `Busy` (the previous screen of an animated load). | exclusive | — |
| `Lvgl::load_screen_later(&self, screen: Handle<Obj>, anim)` (handles) | Loads `screen` at the next drain; a later request replaces a pending one; a screen deleted meanwhile is skipped; a refused load is discarded (`refused` with `debug-counters`). | shared | — |
| `ScreenAnim` | `new(kind: ScreenLoadAnim, time_ms: u16, delay_ms: u16, auto_delete: bool)`, `ScreenAnim::NONE`. | — | 6 B |
| `ScreenLoadAnim` | `NONE`, `OVER_*`, `MOVE_*`, `FADE_IN`, `FADE_OUT`, `OUT_*` (`LEFT`, `RIGHT`, `TOP`, `BOTTOM`). | — | 1 B |

## Styles

Styles and fonts are always `'static`: LVGL keeps pointers to them.

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `ConstStyle` | A style that never changes, entirely in `.rodata`: no RAM, no heap. `ConstStyle::new(&'static [ConstProp])` panics (a compile error in a `static`) unless the slice ends with `ConstProp::END` and contains it only there. | — | `lv_style_t` and its properties in `.rodata` |
| `const_style![prop(args), ...]` | A `ConstStyle` from `props` constructors, terminated automatically. Every argument must be a constant expression. | — | — |
| `props::*` | One `const fn` per style setter whose value fits a constant (`radius`, `bg_color`, `align`, `text_font`, ...), returning a `ConstProp`. `props::layout` panics on `LV_LAYOUT_GRID`; `props::grid_cell_column_span` and `grid_cell_row_span` panic above `GridTemplate::MAX_SPAN`. | — | — |
| `ConstProp` | One property of a `ConstStyle`; `ConstProp::END` is the terminator. | — | — |
| `Style` | A style whose properties can change: a `static` (`Style::new()` is `const`), properties on LVGL's heap. | — | 12 B of `.bss` (16 B with `LV_USE_ASSERT_STYLE`) |
| `Lvgl::style(&mut self, &'static Style) -> StyleMut` | Changes a style. The objects using it are refreshed once, when the `StyleMut` is dropped (`lv_obj_report_style_change`). | exclusive | — |
| `StyleMut` | The generated `lv_style_*` functions as `&mut self` methods (`set_*`, `remove_prop`, `copy`, `merge`, `reset`), always exclusive. `set_layout` refuses `LV_LAYOUT_GRID`; `set_grid_cell_column_span` and `set_grid_cell_row_span` refuse spans above `GridTemplate::MAX_SPAN` (`InvalidArg`). | exclusive | — |
| `StyleRef` | A `&'static Style` or `&'static ConstStyle`, taken by `add_style`, `remove_style`, `replace_style` and the style copy functions. `as_ptr()`. | — | — |
| `Selector` | `lv_style_selector_t`: `Selector::DEFAULT`, `Selector::new(part, state)`, `Part::INDICATOR \| State::CHECKED`, `From<Part>`, `From<State>`, `bits()`. | — | 4 B |
| `StyleProp` | A style property constant per property (`StyleProp::BG_COLOR`, ...), `StyleProp::END`. | — | — |
| `style_props![BG_COLOR, ...]` | A `&'static [StyleProp]` terminated with `StyleProp::END`, for `TransitionDsc`. | — | — |
| `color::hex(0xRRGGBB)`, `color::rgb(r, g, b)` | `const fn` colors (`lv_color_t`). | none | — |

## Fonts

| Item | What it is | Access |
| --- | --- | --- |
| `Font` | `lv_font_t`, reachable only as `&'static Font`. | — |
| `Font::<name>()` | A `const fn` per `lv_font_t` static of the bindings (`Font::montserrat_14()` with `lvgl-rust/conf`). | none |
| `unsafe Font::from_raw(&'static lv_font_t) -> &'static Font` | A font defined elsewhere (`lv_font_conv` output, a font crate). It must never be written afterwards. | none |
| `Font::as_ptr(&self)` | The raw font. | none |
| `const lv_font_t *` parameters | Take a `&'static Font`. `lv_font_t *` (writable) parameters stay raw. | — |

## Retained data

Data LVGL keeps a pointer to and reads later. All of it is `const` and
`'static`, so it can stay in flash, and its shape is checked when it is
made (a compile error in a `static`).

| Item | What it is | Checked |
| --- | --- | --- |
| `ImageData<const N: usize>(pub [u8; N])` | Image bytes aligned to 4 for in-place reads of 16- and 32-bit pixels. | — |
| `ImageDsc::new(format, width: u16, height: u16, stride: u16, &'static ImageData<N>)` (little-endian targets) | A constant image (`lv_image_dsc_t`). `stride` 0 means packed rows. | stride at least a row; data holds the palette (indexed formats) and `height` rows, and is not longer than that |
| `unsafe ImageDsc::from_raw(&'static lv_image_dsc_t)` | A descriptor defined elsewhere; must describe its data correctly and never be written. | caller |
| `PixelFormat` | `L8`, `A1`, `A2`, `A4`, `A8`, `I1`, `I2`, `I4`, `I8`, `Rgb565`, `Argb8565`, `Al88`, `Rgb888`, `Argb8888`, `Xrgb8888`; `bits_per_pixel()`, `palette_len()`. | — |
| `ImageSrc` | `image(&'static ImageDsc)`, `path(&'static CStr) -> Option` (first byte printable ASCII), `symbol(&'static CStr) -> Option` (first byte `0x80` or more), `From<&'static ImageDsc>`. | first byte, as LVGL tells sources apart by it |
| `ImageColorkey::new(low, high)` | `lv_image_colorkey_t`. | — |
| `button_map![c"1", c"2", c"\n", c"0"]` | A `ButtonMap`, terminated automatically; `button_count()`. `MapEntry::new`, `MapEntry::NEWLINE`, `MapEntry::END`. | terminated; no empty text before the end |
| `ButtonMatrix::set_ctrl_map(&self, &[ButtonmatrixCtrl])` | Control flags, one per button of the current map. | `InvalidArg` if shorter than the button count |
| `ButtonmatrixCtrl::union(self, other)` | Both flags, usable in constants. | — |
| `KeyboardMap::new(map: ButtonMap, ctrl: &'static [ButtonmatrixCtrl])` | A keyboard layout: a button map with one control entry per button. | one control entry per button |
| `Keyboard::set_map(&self, mode, &'static KeyboardMap)` | Uses the layout for `mode` in every keyboard (LVGL keeps one layout per mode). | — |
| `Keyboard::set_popovers(&self, enabled)` | Shows the current mode's layout again, then sets popovers. | — |
| `grid_template![Track::px(48), Track::fr(1), Track::CONTENT]` | A `GridTemplate`, terminated automatically, never empty; `track_count()`. | at least one and at most `GridTemplate::MAX_TRACKS` (65,535) tracks; fixed tracks sum to at most `LV_COORD_MAX` |
| `Track` | `px(size)` (0 to `LV_COORD_MAX - 102`), `fr(weight)` (1 to 99), `Track::CONTENT`. | ranges |
| `GridTemplate::MAX_SPAN` | 65,535: the largest cell span accepted. | — |
| `scale_labels![c"0", c"50", c"100"]` | `ScaleLabels`, NULL-terminated, no NULL before the end; `label_count()`. | terminated |
| `Scale::set_text_src(&self, Option<&'static ScaleLabels>)` | Labels the major ticks (`None`: their values). | — |
| `GradDsc::new(dir: GradDir, stops: &[lv_grad_stop_t])` | A horizontal or vertical gradient (`GradDir::Horizontal`, `GradDir::Vertical`). | 1 to `LV_GRADIENT_MAX_STOPS` stops |
| `TransitionDsc::new(props: &'static [StyleProp], path: AnimPath, time_ms, delay_ms)` | A style transition. | `props` ends with `StyleProp::END`, only there |
| `ColorFilter::new(filter)`, `ColorFilter::shade()` | A color filter; `shade()` is LVGL's `lv_color_filter_shade`. The filter may run on a draw thread. | — |

Grid layouts: the grid layout is set only by
`ObjMut::set_grid_dsc_array`, together with both templates, and every
other layout setter refuses `LV_LAYOUT_GRID`, so LVGL's grid code stays
within the templates. The style setters of the templates stay in `sys`.
Grid templates and scale labels passed to other functions stay raw.

## Text

| Item | What it is | Access | Errors |
| --- | --- | --- | --- |
| `Label::set_text(&self, Option<&CStr>)` (generated) | Text LVGL copies (`None`: refreshes the current text). | shared | — |
| `Label::set_text_str(&self, &str)` | Appended in chunks of up to 31 bytes cut at character boundaries (`lv_label_ins_text`). | shared | `InvalidArg` (interior NUL), `OutOfMemory` (label left empty) |
| `Label::set_text_fmt(&self, format_args!(..))` (`fmt`) | Formatted output streamed into the label in 32-byte chunks; no other buffer. | shared | `InvalidArg`, `OutOfMemory` |
| `Label::set_text_i32(&self, i32)`, `set_text_u32(&self, u32)` | Decimal numbers without `core::fmt`. | shared | — |
| `Label::set_text_static(&self, Option<&'static CStr>)` | Text LVGL keeps without copying (`None`: refreshes the current text). | shared | `Internal` on the label of a textarea |
| `Label::get_text_into(&self, buf)` / `ObjMut<Label>::get_text(&mut self)` | Copy-out from a view, borrow under exclusive access. | shared / exclusive | `TooSmall` (copy) |
| `Dropdown::set_options(&self, &CStr)`, `set_options_static(&self, &'static CStr)`, `clear_options(&self)` | Options copied or kept; the open list's label is pointed at the current options. | shared | — |

## Events

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `Obj::on(&self, code: EventCode, f)` | `f` is a closure without captures (zero-sized; checked at compile time), `Fn(&Event) -> R + Send + 'static`. | shared | LVGL's event descriptor only |
| `Obj::on_with(&self, code, data: D, f)` | `f(&Event, D)` with `data` stored in the handler's user data. | shared | LVGL's event descriptor only |
| `Obj::on_fn(&self, code, f: fn(&Event))` | A function pointer. | shared | LVGL's event descriptor only |
| `Obj::on_static(&self, code, &'static Handler<C>)` | `Handler { ctx: C, f: fn(&Event, &'static C) }` in a `static`. | shared | LVGL's event descriptor only |
| `Obj::on_removable(&self, code, data, f) -> Result<EventSub, Error>` (handles) | As `on_with`, returning an `EventSub`; takes a registry slot. | shared | `EventSub` 12 B |
| `EventSub::remove(self, &mut Lvgl)` (handles) | Removes the handler, unless its object was deleted. `#[must_use]`. | exclusive | — |
| `Obj::on_boxed(&self, code, f)` (`heap`) | A closure with captures, on LVGL's heap until the object is deleted; dropped then, the block freed at the next drain. | shared | LVGL heap block: 8 B header, then the closure |
| `unsafe trait UserData` | Values stored by value in a callback's user data: `Copy + Send + 'static`, at most 4 bytes, no padding. Implemented for `u8`, `u16`, `u32`, `i8`, `i16`, `i32`, `f32`, `bool`, `char`, `()`, `Handle<W>`, `Option<Handle<W>>`. | — | at most 4 B |
| `trait HandlerResult` | What a handler returns: `()` or `Result<(), Error>` (errors are dropped). Sealed. | — | — |
| `Event<'cb>` | `lv() -> &'cb Lvgl`, `code() -> EventCode`, `target()`, `current_target()` (`None` while being deleted), `target_is(Handle<Obj>)` (handles), `param_obj()` (`CHILD_CHANGED`, `CHILD_CREATED`), `key()` (`KEY`), `point()` (pointer events from an input device), `stop_bubbling()`, `stop_processing()`. | shared | — |
| `handler_allowed(code: EventCode) -> bool` | Whether shared handlers run for `code`: the codes `events.toml` gives the shared tier, plus codes registered with `lv_event_register_id`. Refused: query and draw codes, `CREATE`, display and input-device codes. | none | — |
| `EventCode` | Every code of `lv_event_code_t` in the configuration, plus `ALL`. | — | 4 B |
| `SafeEventCode` | Codes safe code may send with shared access (45 with `lvgl-rust/conf`, `READY` among them). No public constructor. | — | 4 B |
| `Obj::send_event(&self, SafeEventCode)` | `lv_obj_send_event` with a NULL parameter. | shared | `Dead` if LVGL reports the object deleted |
| `SendableEventCode` | Codes sendable with exclusive access, deleting ones included (62 with `lvgl-rust/conf`, `CLICKED` and `VALUE_CHANGED` among them). No public constructor. | — | 4 B |
| `ObjMut::send_event(self, SendableEventCode) -> Result<Self, Error>` | See [ObjMut](#exclusive-access-to-an-object-objmut). | exclusive | — |

Registration errors: `InvalidArg` for a code handlers never receive
(`EventCode::ALL` is accepted and means every allowed code); `Busy` when
called from a callback on an object LVGL is sending an event to;
`OutOfMemory`. A handler is skipped when `LVGL_RUST_MAX_CALLBACK_DEPTH`
callbacks are already running.

Handlers get shared access: anything that needs exclusive access
(creating, deleting, moving objects, setting subjects) is deferred with
`delete_later`, `clean_later` or `Lvgl::post`. Inside callbacks, views and
handles of objects under a deleting ancestor are refused.

### Restricted handlers

Query and draw codes are sent in the middle of layout or rendering. With
`lvgl-rust/conf` the `QueryCode` constants are `COVER_CHECK`, `DRAW_MAIN`,
`DRAW_MAIN_BEGIN`, `DRAW_MAIN_END`, `DRAW_POST`, `DRAW_POST_BEGIN`,
`DRAW_POST_END`, `DRAW_TASK_ADDED`, `GET_SELF_SIZE`, `HIT_TEST` and
`REFR_EXT_DRAW_SIZE`.

| Item | What it is | Access |
| --- | --- | --- |
| `Obj::on_query(&self, code: QueryCode, f)` | A closure without captures, `Fn(&QueryEvent) -> R + Send + 'static`. | shared (to register); the handler is restricted |
| `Obj::on_query_with(&self, code, data: D, f)` | With a `UserData` value. | as above |
| `Obj::on_query_fn(&self, code, f: fn(&QueryEvent))` | A function pointer. | as above |
| `QueryEvent<'cb>` | `code()`, `stop_bubbling()`, `stop_processing()` and one typed accessor per code, each `None` for other codes or a NULL parameter: `hit_test()`, `cover_check()`, `ext_draw_size()`, `self_size()`, `layer()`, `draw_task()`. No `Lvgl` access and no views; `!Send`. | restricted |
| `HitTest` | `point()`, `result()`, `set_result(clicks: bool)`. | restricted |
| `CoverCheck`, `CoverResult` | `area()`, `set_result(CoverResult)`. | restricted |
| `ExtDrawSize` | `get()`, `extend(size)`. | restricted |
| `SelfSize` | `get()`, `extend(width, height)`. | restricted |
| `Layer` | `clip_area()`, `buffer_area()`, `as_ptr()` (drawing into a layer stays raw). | restricted |
| `DrawTask` | `kind()`, `area()`, `part()`, `id1()`, `id2()`, `color()`, `set_color()`, `opacity()`, `set_opacity()`. | restricted |
| `query_handler_allowed(code: EventCode) -> bool` | Whether restricted handlers run for `code`. | none |

The compiler rejects anything else in a restricted handler: the closure
is `Send + 'static` and zero-sized, its data is a `UserData` value, and a
thread-local holding a `&'static Lvgl` would need a guard borrowed for
`'static`, which the `LvglAccess` contract forbids. `Lvgl::enter` fails
inside the handler.

## Deferred work

Deferred requests run at the next drain: `Lvgl::enter`, and before and
after `lv_timer_handler` in `Lvgl::timer_handler`.

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `Obj::delete_later(&self)` (handles) | Deletes the object at the next drain. Permanent structural refusals fail now (`Internal`); `Busy` is retried by the drain. Takes a registry slot until then if the object has none (`RegistryFull`). | shared | a registry slot |
| `Obj::clean_later(&self)` (handles) | Deletes the children at the next drain, as above. | shared | a registry slot |
| `Lvgl::post(&self, f, data: D)` | Runs `f(&mut Lvgl, data)` at the next drain with exclusive access. `f` is a function item or a closure without captures. `Busy` when the queue is full. Entries posted while the queue runs wait for the next drain. | shared | 8 B per entry; queue `8·P + 4` B of `.bss`, P = `LVGL_RUST_POST_SLOTS` (default 4) |
| `Lvgl::load_screen_later` | See [Screens](#screens). | shared | — |
| `Drop for Owned` | Records a deletion (no session needed). | none | — |
| `TimerCtx::delete_later(&self)` | Deletes the timer at the next drain. | shared | — |

## Timers (handles)

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `Lvgl::timer(&self, period_ms, data: D, f) -> Result<Owned<Timer>, Error>` | Calls `f(&TimerCtx, data)` every `period_ms` from `timer_handler`. `f` is a function item or a closure without captures. Auto-delete is off: a repeat count that runs out pauses the timer. Dropping the `Owned` deletes it at the next drain. | shared | LVGL's timer and a registry slot; no other allocation |
| `Lvgl::timer_boxed(&self, period_ms, f)` (`heap`) | A closure with captures, on LVGL's heap until the timer is deleted. | shared | LVGL heap block: 4 B header, then the closure |
| `Timer` | The type of `Owned<Timer>` and `Handle<Timer>`. | — | — |
| `TimerCtx<'cb>` | `lv()`, `handle() -> Option<Handle<Timer>>` (a scan of the registry), `delete_later()`. | shared | — |
| `Handle<Timer>::pause`, `resume`, `set_period(ms)`, `set_repeat_count(count: i32)` (`-1`: forever), `ready`, `reset` | Each takes `&Lvgl`; `Dead` once the timer is deleted. | shared | — |

## Animations

| Item | What it is | Access |
| --- | --- | --- |
| `Anim<D: UserData = ()>` | An animation built on the stack (LVGL copies it): `new(data)` (0 to 100 over 500 ms), `values(start, end)`, `duration(ms)`, `delay(ms)`, `reverse(ms)`, `repeat(Option<u32>)` (`None`: forever), `repeat_delay(ms)`, `path(AnimPath)`, `early_apply(bool)`, `exec(f)` (`f(&AnimCtx, D, i32)`), `on_completed(f)` (`f(&AnimCtx, D)`). Callbacks are function items or closures without captures. | none |
| `ObjMut::animate(&mut self, Anim<D>)` | Starts the animation on the object, next to its running ones; LVGL deletes it with the object. `OutOfMemory`. | exclusive |
| `ObjMut::delete_animations(&mut self) -> bool` | Deletes every animation of the object. | exclusive |
| `AnimCtx<'cb>` | `lv()`, `target()` (`None` while being deleted). | shared |
| `AnimPath` | `Linear`, `EaseIn`, `EaseOut`, `EaseInOut`, `Overshoot`, `Bounce`, `Step`. | — |

## Subjects

| Item | What it is | Access |
| --- | --- | --- |
| `IntSubject`, `ColorSubject` | `static VOLUME: IntSubject = IntSubject::new(50);`. A subject initialises itself on first use, so LVGL never sees one of the wrong type. | — |
| `get(&'static self, &Lvgl)`, `previous(&'static self, &Lvgl)` | The current and previous values. | shared |
| `set(&'static self, &mut Lvgl, value)` | Sets the value and notifies observers and bound objects. | exclusive |
| `observe(&'static self, &Lvgl, data, f)` | Calls `f(&ObserverCtx, D, value)` now and on every change, forever. `OutOfMemory`. | shared |
| `observe_obj(&'static self, &Obj, data, f)` | As `observe`, until the object is deleted. `Busy`, `OutOfMemory`. | shared |
| `ObserverCtx<'cb>` | `lv()`, `target()`. | shared |
| `bind_value(&mut self, &'static IntSubject)` (generated, `ObjMut` of `Arc`, `Slider`, ...), `bind_checked` (generated) | Binds a widget to a subject. `OutOfMemory`. | exclusive |
| `ObjMut<Label>::bind_text(&mut self, &'static IntSubject)` | Shows the subject as a decimal number. `OutOfMemory`. | exclusive |

## Displays

Drivers get no token: they cannot touch objects.

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `trait Flush: Send + 'static` | `fn flush(&mut self, req: FlushRequest)`. Returning (or dropping `req`) frees the buffer. | — | — |
| `FlushRequest` | `area()`, `stride()`, `pixels() -> &mut [u8]`, `defer() -> FlushDone`. | — | — |
| `FlushDone` | A deferred completion, `Send` and interrupt-safe: `ready()` (or drop) is one Release store. LVGL's own flag is not touched. Complete from an interrupt or another thread, never later on the thread running `timer_handler`. | none | — |
| `DisplayCell<D>` | The driver and the crate's state of its display: an application `static` (`DisplayCell::new()` is `const`), claimed while in use. | — | — |
| `DrawBuf<const BYTES: usize>` | A draw buffer `static`, aligned to `LV_DRAW_BUF_ALIGN`, claimed by the display using it. | — | `BYTES` and a claim flag |
| `DrawBuffers` | `single(&'static DrawBuf<N>)`, `double(&'static DrawBuf<N>, &'static DrawBuf<M>)` (LVGL uses the smaller size). | — | — |
| `RenderMode` | `Partial`, `Direct`, `Full`. | — | — |
| `Rotation` | `Degrees0`, `Degrees90`, `Degrees180`, `Degrees270`. | — | — |
| `DisplayFormat` | `Rgb565`, `Rgb565Swapped`, `Rgb888`, `Xrgb8888`, `Argb8888`, `L8`, `Al88`, `A8`. | — | — |
| `Display::create(&mut Lvgl, &'static DisplayCell<D>, driver, width, height, buffers, mode)` | The first display becomes the default one. `AlreadyOwned` (cell or buffer in use, or both buffers the same), `InvalidArg` (zero size, above `LV_COORD_MAX`, or a buffer too small for `mode`: one line in partial mode, the screen otherwise), `OutOfMemory`. | exclusive | `Display` 8 B |
| `Display::create_boxed(&mut Lvgl, driver, width, height, buffers, mode)` (`heap`) | The driver and the crate's state in one LVGL heap block instead of a cell, freed when the display is deleted. | exclusive | LVGL heap block |
| `set_default(&self, &mut Lvgl)`, `resolution(&self, &Lvgl) -> (u32, u32)` | Default display; resolution after rotation. | exclusive / shared | — |
| `set_resolution(&self, &mut Lvgl, w, h)`, `set_rotation`, `set_color_format(DisplayFormat)`, `set_render_mode(RenderMode)` | Check that the draw buffers hold what LVGL will render (a row in partial mode, the screen in full and direct modes): `TooSmall` otherwise; `InvalidArg` for sizes that would wrap LVGL's 32-bit size arithmetic (and zero sizes). | exclusive | — |
| `set_tile_count(&self, &mut Lvgl, count)` | `InvalidArg` outside `1..=255` or above the draw buffer size in bytes. | exclusive | — |
| `try_delete(self, &mut Lvgl) -> Result<(), (Display, Error)>` | Deletes the display, its screens and objects, drops the driver, releases the cell or frees the block and the buffers. `Busy` (giving the display back) while a flush is pending. | exclusive | — |
| `as_ptr(&self)` | The raw display. | — | — |

Dropping a `Display` keeps the display (a leak, counted as `leaked` with
`debug-counters`).

## Input devices

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `trait Read: Send + 'static` | `fn read(&mut self, data: &mut IndevData)`. | — | — |
| `IndevData` | `set_pressed(bool)`, `set_point(x, y)`, `set_key(u32)`, `set_button(id: u32)`, `set_steps(i16)`, `set_continue_reading(bool)`. | — | — |
| `IndevKind` | `Pointer`, `Keypad`, `Encoder`, `Button`. | — | — |
| `IndevCell<R>` | The reader in an application `static`, claimed while in use. | — | — |
| `Indev::create(&mut Lvgl, &'static IndevCell<R>, reader, kind)` | On the default display. `AlreadyOwned`, `InvalidArg` for `IndevKind::Button`, `OutOfMemory`. | exclusive | `Indev` 12 B |
| `Indev::create_button(&mut Lvgl, cell, reader, &'static [lv_point_t; N])` | A button device: the reader reports a button id and LVGL presses `points[id]`; an id past the array reads as released. `N > 0` checked at compile time. | exclusive | — |
| `Indev::create_boxed(&mut Lvgl, reader, kind)`, `create_button_boxed(&mut Lvgl, reader, points)` (`heap`) | The reader in an LVGL heap block, freed when the device is deleted. | exclusive | LVGL heap block |
| `Indev::set_group(&mut self, &mut Lvgl, Option<Handle<Group>>)` (handles) | Sends keys or encoder steps to a group. `Dead`. | exclusive | — |
| `Indev::delete(self, &mut Lvgl)` | Deletes the device, drops the reader, releases the cell or frees the block. | exclusive | — |
| `Indev::as_ptr(&self)` | The raw device. | — | — |

Dropping an `Indev` keeps the device (a leak, counted as `leaked` with
`debug-counters`).

## Focus groups (handles)

| Item | What it is | Access |
| --- | --- | --- |
| `Lvgl::create_group(&self) -> Result<Owned<Group>, Error>` | Takes a registry slot; dropping the `Owned` deletes the group at the next drain. `OutOfMemory`, `RegistryFull`. | shared |
| `Handle<Group>::add_obj(self, &mut Lvgl, Handle<Obj>)`, `remove_all`, `focus_next`, `focus_prev`, `set_editing(bool)` | `Dead` once the group or object is deleted. | exclusive |
| `Handle<Group>::set_wrap(self, &Lvgl, bool)`, `set_default`, `focused() -> Option<&Obj>`, `obj_count() -> u32` | `Dead`. | shared |
| `ObjMut::remove_from_group(&mut self)` | Removes the object from its group. | exclusive |

## Async event queues (`async`)

No executor dependency, no atomics: queues are touched only under the
token.

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `StaticQueue<E, const N: usize>` | A queue of `N` events (1 to 255) in a `static`, filled through `capture: fn(&Event) -> Option<E>`, which runs in the handler. Never freed; deleting the object closes it. Overflow drops the newest event. | — | in the `static` |
| `StaticQueue::receiver(&'static self, &Lvgl) -> Option<StaticRx<E, N>>` | The single receiver (`None` once taken). | shared | — |
| `StaticQueue::take_overflow(&'static self, &Lvgl) -> bool` | Whether events were dropped; clears the flag. | shared | — |
| `StaticRx<E, N>` | The receiver; not `Clone`. | — | 4 B |
| `trait Receiver` | `type Item`, `try_recv(&self, &Lvgl) -> Recv<Item>`, `register(&self, &Lvgl, &Waker)`. | shared | — |
| `Recv<E>` | `Event(E)`, `Empty`, `Closed` (sticky). | — | — |
| `Obj::subscribe(&self, options, &'static StaticQueue<E, N>)` | Records the selected events through the queue's `capture`. | shared | LVGL event descriptors |
| `SubscribeOptions` | `new(code)`, `bubble_children()` (the object's children get `EVENT_BUBBLE` now and when created; cannot be undone, stays for the object's lifetime), `From<EventCode>`. | — | — |
| `next_event(lock, &rx).await -> (A, Option<Item>)` | Takes the async lock with `lock`, starts a session and pops an event, or registers the task's waker, releases the lock and waits. Returns the guard with the event (`None` once closed, or if a session could not start). | guard | — |
| `Obj::subscribe_owned::<E, N>(&self, options, capture) -> Result<OwnedQueue<E, N>, Error>` (`async` and `heap`, handles) | The queue on LVGL's heap; takes or shares the object's registry slot. `OutOfMemory`, `RegistryFull`. | shared | LVGL heap block: 4 B header, then the queue; `OwnedQueue` 8 B |
| `OwnedQueue::close(self, &mut Lvgl)` | Detaches the queue; the block is freed at the next drain. `bubble_children` stays. Dropping an `OwnedQueue` leaves the queue attached until the object is deleted. | exclusive | — |
| `OwnedQueue::take_overflow(&self, &Lvgl)`, `Receiver for OwnedQueue` | As for the static queue. | shared | — |

## LVGL heap types (`heap`)

| Item | What it is | Access | Cost |
| --- | --- | --- | --- |
| `Obj::on_boxed` | See [Events](#events). | shared | 8 B header |
| `Lvgl::timer_boxed` | See [Timers](#timers-handles). | shared | 4 B header |
| `ObjMut<Image>::set_src_owned(&mut self, image: T)` | Moves an `ImageDsc` or `ImageBuffer<N>` into an LVGL heap block owned by the image. Retired when a later `set_src_owned` replaces it or the image is deleted (LVGL's image caches drop it), freed at the next drain. `OutOfMemory`, `InvalidArg` (LVGL refuses the image). | exclusive | 4 B header, then the image |
| `trait OwnedImageSource` | What `set_src_owned` takes: `ImageDsc`, `ImageBuffer<N>`. Sealed. | — | — |
| `ImageBuffer<const N: usize>::new(format, width, height, stride, data: [u8; N])` (little-endian targets) | A descriptor and its pixels, checked like `ImageDsc::new` (`InvalidArg`). Goes through the stack on its way to the heap. | none | — |
| `ObjMut<Image>::set_src_buffer::<N>(&mut self, format, width, height, stride, fill)` (little-endian targets) | Allocates the block first, zeroes it, then `fill(&mut [u8; N])` writes the pixels in place: a large image never goes through the stack. | exclusive | 4 B header, then the image |
| `Obj::subscribe_owned` | See [Async event queues](#async-event-queues-async). | shared | 4 B header |
| `Display::create_boxed`, `Indev::create_boxed`, `Indev::create_button_boxed` | Drivers on LVGL's heap, freed when the display or device is deleted (a display never while a flush is pending). | exclusive | — |
| Deferred-free list | Blocks LVGL may still reach (owned image sources, owned queues, boxed handlers) are freed at the next drain. | — | 4 B of `.bss` |

LVGL's allocator adds its own header to each block.

## Allocator (`allocator`)

Needs `#define LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM` in `lv_conf.h`
(checked at build time).

| Item | What it is |
| --- | --- |
| `unsafe trait LvglAllocator: Sync` | `alloc(&self, size) -> *mut u8`, `unsafe realloc(&self, ptr, new_size)`, `unsafe free(&self, ptr)`. Blocks aligned to a pointer. |
| `SizedHeader<A>(pub A)` | Adapts a `GlobalAlloc`: each block starts with a pointer-sized header holding its size. |
| `lvgl_allocator!(HEAP)` | Defines the functions LVGL calls with `LV_STDLIB_CUSTOM` (`lv_malloc_core`, `lv_realloc_core`, `lv_free_core`, `lv_mem_init`, `lv_mem_deinit`, `lv_mem_add_pool`, `lv_mem_remove_pool`, `lv_mem_monitor_core`, `lv_mem_test_core`) over a `static` implementing `LvglAllocator`. Use it once, in the binary. |

## Diagnostics

| Item | What it is | Feature | Cost |
| --- | --- | --- | --- |
| `Error` | `#[non_exhaustive]`, `Copy`, `Eq`, `Hash`, `Debug`; see [Errors](errors.md). | — | 1 B |
| `impl Display for Error`, `impl core::error::Error for Error` | Messages for each variant. | `fmt` | links `core::fmt` |
| `defmt::Format` | For `Error`, `Handle`, `Owned`, the event codes and the other public enums and plain types. | `defmt` | — |
| `DebugCounters::read() -> DebugCounters` | Four 16-bit counters, wrapping at 65,536, readable from any thread: `drained` (deferred requests the drain carried out, and `Owned::delete`), `refused` (deferred requests dropped by a permanent structural refusal, not the refusals `Owned::delete` returns; refused `load_screen_later` loads), `skipped` (callbacks skipped at `LVGL_RUST_MAX_CALLBACK_DEPTH`), `leaked` (`Display`s and `Indev`s dropped without being deleted). | `debug-counters` | 8 B of `.bss` |
| `DebugCounters::reset()` | Sets every counter to zero. | `debug-counters` | — |

Both features cost nothing when off.

## Widget-specific checks

| Widget | Items | Checks |
| --- | --- | --- |
| Chart | `Chart::add_series(&self, color, axis) -> Result<ChartSeries, Error>`, `remove_series(&self, ChartSeries)`, `hide_series`, `set_series_color`, `get_series_color`, `set_x_start_point`, `get_x_start_point`, `get_point_pos_by_id`, `set_all_values`, `set_next_value`, `set_next_value2`, `set_series_values`, `set_series_values2`, `set_series_value_by_id`, `set_series_value_by_id2`, `get_series_y_values_into`, `get_series_x_values_into`, `set_point_count`, `set_type` | `ChartSeries` is a token (one pointer, 4 B, not `Clone`) checked against the chart's series list on every call: a series of another chart or already removed is `InvalidArg` (one of a deleted chart too, unless a newer series of the same chart reuses its address). Point counts whose arrays would wrap the address space are refused (`InvalidArg`). X points of scatter charts read as `LV_CHART_POINT_NONE` until written. External point arrays, cursors and `lv_chart_get_series_next` stay raw. |
| Table | `set_row_count`, `set_column_count`, `set_column_width`, `set_cell_value`, `set_cell_ctrl`, `clear_cell_ctrl`, `get_cell_value_into`, `get_cell_user_data`, `get_selected_cell`; `ObjMut<Table>::get_cell_value` | Getters check rows and columns against the table (`InvalidArg`); setters and counts check the size against `TABLE_CELL_LIMIT` (`u32::MAX / size_of::<*mut c_void>()` cells) and keep at least one row and one column (`InvalidArg`). Cell text is copied out of views (`TooSmall`) and borrowed under `ObjMut`. |
| Tile view | `TileView::create`, `create_owned`, `add_tile` (generated), `get_tile_active(&self) -> Option<&Obj>`, `ObjMut<TileView>::set_tile(&mut self, Handle<Obj>, anim)` | Children come only from `add_tile`. `get_tile_active` forgets a deleted tile (unless a newer tile reuses its address). `set_tile` takes only one of its tiles (`InvalidArg`). LVGL's scroll-end handler runs only while the content size it divides by is positive, checked in the same event descriptor (no extra cost). |
| Calendar | `ObjMut<Calendar>::set_month_shown(&mut self, year, month)`, `set_shown_month(&mut self, month)` | The shown month stays in `1..=12` (`InvalidArg`). Calendar headers keep their parts and their calendar (part tables; a header is pinned to its parent). |
| Scale | `Scale::set_range`, `set_min_value`, `set_max_value`; `set_text_src` | `min == max` refused (`InvalidArg`). |
| Keyboard | `ObjMut<Keyboard>::set_textarea(&mut self, Option<Handle<TextArea>>)` (handles) | The keyboard forgets the textarea when it is deleted. Takes a registry slot for the keyboard. |
| Label | `set_text_static` | Refused on a textarea's label (`Internal`). |
| Button matrix, keyboard | `set_ctrl_map`, `KeyboardMap`, `set_map`, `set_popovers` | See [Retained data](#retained-data). |

## What the compiler refuses

Each program of `lvgl-rust/tests/compile-fail/` must fail to build with
the error recorded next to it (`mise run test-compile-fail`), among them:
a view used after `timer_handler` or after an exclusive call, two
`ObjMut` at once, a view next to exclusive access, a view kept past its
session, in a `static` or sent to another thread, a session kept past its
guard, a capturing closure given to `on`, `timer` or `post`, data wider
than 4 bytes or borrowed given to `on_with`, an exclusive call or object
access from a handler, a cloned `Owned` or `StaticRx`, an `Owned` used
after `delete`, a handle to the wrong class, borrowed text kept across a
setter, non-`'static` text for `set_text_static`, a deleting code sent
with shared access, and the parent used while a child's view lives.

## Raw access

| Item | What it is |
| --- | --- |
| `lvgl_rust::sys` | `lvgl-rust-sys`, re-exported. Raw and safe code can live side by side. |
| `as_ptr()` | On views, `Display`, `Indev`, `Layer`, `Font`, `ImageDsc`, `ImageSrc`, `ButtonMap`, `GridTemplate`, `ScaleLabels`, `GradDsc`, `TransitionDsc`, `ColorFilter`, `ImageColorkey`, `StyleRef`. |
| `Obj::from_raw`, `Lvgl::assume`, `Lvgl::enter_callback` | Entry points from raw code (`unsafe`). |
| `$OUT_DIR/coverage.txt` | What happened to every bound function: wrapped (and how), raw, manual or skipped. See [Build configuration](build-configuration.md#generated-files). |
