# Errors

`lvgl_rust::Error` is the error type of every fallible operation of
`lvgl-rust`: a `#[repr(u8)]`, `#[non_exhaustive]` enum of 1 byte,
`Copy`, `Eq`, `Hash` and `Debug`. With feature `fmt` it implements
`Display` and `core::error::Error`; with `defmt`, `defmt::Format`.

Event handlers may return `Result<(), Error>`; LVGL has no way to report
the error, so it is dropped.

## Variants

| Variant | Meaning | `Display` (feature `fmt`) |
| --- | --- | --- |
| `Dead` | The object or resource was deleted, or is about to be. | the object was deleted |
| `RegistryFull` | No handle slot is left (`LVGL_RUST_HANDLE_SLOTS`, or 65,535 slots with `growable-registry`). | no registry slot is left |
| `OutOfMemory` | LVGL could not allocate memory, or a growable registry could not allocate a chunk. | LVGL could not allocate memory (out of memory with `growable-registry`) |
| `InvalidArg` | An argument is out of range or would make LVGL's state inconsistent. | invalid argument |
| `Busy` | LVGL still uses the object, or a queue is full; retrying later can succeed. | LVGL still uses it; retried later |
| `Internal` | LVGL keeps pointers to the object (a display layer, a part of a widget): the operation is refused. | LVGL keeps pointers to it: refused |
| `Reentrant` | LVGL is already initialised, or a session or a callback is active. | LVGL is already initialized, or a session or callback is active |
| `NotInitialized` | LVGL is not initialised, or was deinitialised. | LVGL is not initialized |
| `AlreadyOwned` | The object already has an owner, or a driver cell or draw buffer is in use. | already owned |
| `TooSmall` | A buffer is too small. | buffer too small |

## Dead

A handle resolves to `Dead` when its object was deleted by any path,
when its deletion is pending (its `Owned` was dropped, or `delete_later`
was requested), when the object's class is not the handle's type, and,
inside callbacks, when the object or an ancestor is being deleted.

| Returned by | Action |
| --- | --- |
| `Handle::get`, `Handle::get_mut`, `Owned::get`, `Owned::get_mut` | Drop the handle; recreate the object if it is still needed. |
| Calls that take a `Handle`: `ObjMut::set_parent`, `Lvgl::load_screen`, `Lvgl::load_screen_later`, `Handle<Group>::add_obj`, `Indev::set_group`, `ObjMut<TileView>::set_tile`, `ObjMut<Keyboard>::set_textarea`, generated exclusive calls taking another object | as above |
| `Handle<Timer>::pause`, `resume`, `set_period`, `set_repeat_count`, `ready`, `reset` | The timer was deleted (dropped `Owned<Timer>`, `TimerCtx::delete_later`). |
| `Handle<Group>` methods | The group was deleted. |
| `ObjMut::set_parent_to` | The selector returned `None`. |
| `Obj::send_event`, `ObjMut::send_event` | A handler deleted the object during the send. `ObjMut::send_event` consumes the access, so nothing refers to the object any more. |
| `Component::delete` | The root is already gone. |

`Handle::downcast` returns `None` instead.

## RegistryFull

| Returned by | Action |
| --- | --- |
| `WidgetExt::handle`, `WidgetExt::own`, `create_owned`, `Lvgl::create_screen` | Raise `LVGL_RUST_HANDLE_SLOTS`, or drop handles of objects no longer needed (a slot is freed when its object is deleted and, for owned objects, its `Owned` dropped). |
| `Obj::delete_later`, `Obj::clean_later` (object without a slot) | as above |
| `Obj::on_removable`, `Obj::subscribe_owned`, `ObjMut<Keyboard>::set_textarea` | as above |
| `Lvgl::timer`, `Lvgl::timer_boxed`, `Lvgl::create_group` | as above |

With feature `growable-registry`, these calls take a slot from a new chunk
instead, and return `RegistryFull` only once 65,535 slots are in use (the
limit of the 16-bit index in a handle). A failed allocation of the chunk
is `OutOfMemory` (below).

## OutOfMemory

| Returned by | Action |
| --- | --- |
| `Obj::create`, `<Widget>::create`, `create_owned`, `Lvgl::create_screen`, `TileView::create` | Free objects or raise LVGL's heap (`LV_MEM_SIZE` with the builtin allocator). `TileView::create` can also fail after the tile view was created: it then stays in the parent without a scroll handler. |
| `Obj::on*` registrations, `Obj::subscribe`, `subscribe_owned`, `observe`, `observe_obj`, `bind_value`, `bind_checked`, `bind_text` | as above |
| `Obj::on_boxed`, `Lvgl::timer_boxed`, `set_src_owned`, `set_src_buffer`, `Display::create_boxed`, `Indev::create_boxed`, `create_button_boxed` | as above |
| `Lvgl::timer`, `Lvgl::create_group`, `ObjMut::animate`, `Display::create`, `Indev::create`, `Indev::create_button`, `Chart::add_series` | as above |
| Every call listed under [RegistryFull](#registryfull), with `growable-registry` | Every slot is in use and the global allocator returned null for the next chunk (as many slots as the registry has, 8 B each on 32-bit targets). Nothing is taken: the object, timer or group is left as it was (a new timer or group is deleted again). Free memory, or drop handles so that slots come back. |
| `ObjMut::set_grid_dsc_array` | A template could not be stored; the layout is not set. |
| `Label::set_text_str`, `Label::set_text_fmt` | The label is left empty. |

## InvalidArg

| Returned by | Condition |
| --- | --- |
| `ObjMut::set_parent`, `set_parent_to` | A cycle (the new parent is the object or a descendant), or the object is a screen. |
| `ObjMut::move_to_index` | The object is a screen. |
| `ObjMut::load_screen`, `Lvgl::load_screen` | The object has a parent. |
| `Obj::on`, `on_with`, `on_fn`, `on_static`, `on_removable`, `on_boxed` | A code handlers never receive (`handler_allowed` is false). |
| `Label::set_text_str`, `Label::set_text_fmt` | A NUL byte in the text. |
| `ObjMut::set_layout`, `set_style_layout`, `StyleMut::set_layout` | `LV_LAYOUT_GRID` (use `ObjMut::set_grid_dsc_array`). |
| `ObjMut::remove_local_style_prop` | A grid template property. |
| `ObjMut::set_grid_cell`, `set_style_grid_cell_column_span`, `set_style_grid_cell_row_span`, `StyleMut::set_grid_cell_column_span`, `set_grid_cell_row_span` | A span above `GridTemplate::MAX_SPAN` (65,535). |
| `ButtonMatrix::set_ctrl_map` | Fewer entries than buttons in the current map. |
| `Scale::set_range`, `set_min_value`, `set_max_value` | Minimum equal to maximum. |
| `ObjMut<Calendar>::set_month_shown`, `set_shown_month` | A month outside `1..=12`. |
| `Chart` series calls | A series of another chart, or already removed. |
| `Chart::set_point_count` | Point arrays that would not fit in the address space. |
| `Chart::set_series_values2` | Arrays of different lengths. |
| `Table::set_row_count`, `set_column_count` | 0, or more than `TABLE_CELL_LIMIT` cells. |
| `Table` cell setters | A cell that would grow the table past `TABLE_CELL_LIMIT`. |
| `Table` cell getters | A cell outside the table. |
| `ObjMut<TileView>::set_tile` | Not one of this tile view's tiles. |
| `ImageBuffer::new`, `set_src_buffer` | Data shorter than the palette and the rows, or a stride shorter than a row. |
| `set_src_owned`, `set_src_buffer` | LVGL refused the image (it then shows nothing). |
| `Display::create`, `Display::create_boxed` | A zero size, a size above `LV_COORD_MAX`, or a buffer too small for the render mode (one line in partial mode, the screen otherwise). |
| `Display::set_resolution`, `set_rotation`, `set_color_format`, `set_render_mode` | A zero size, or sizes that would wrap LVGL's 32-bit size arithmetic. |
| `Display::set_tile_count` | Outside `1..=255`, or above the draw buffer size in bytes. |
| `Indev::create`, `Indev::create_boxed` | `IndevKind::Button` (use `create_button`, which takes the points). |

Constant data (`ConstStyle`, `ImageDsc`, `button_map!`, `KeyboardMap`,
`grid_template!`, `Track`, `scale_labels!`, `GradDsc`, `TransitionDsc`)
is checked by `const fn` assertions instead: a wrong shape is a compile
error in a `static`, a panic elsewhere.

## Busy

| Returned by | Condition | Action |
| --- | --- | --- |
| `ObjMut::delete` | The object is a screen being loaded, or the previous screen of an animated load. | Retry after the load, or use `delete_later`, which the drain retries. |
| `Owned::delete` | as above | The request stays pending and is retried at each drain. |
| `ObjMut::load_screen`, `Lvgl::load_screen` | The screen is the previous screen of an animated load. | Retry once the load completes. |
| `Lvgl::post` | The queue is full (`LVGL_RUST_POST_SLOTS`, default 4). | Raise `LVGL_RUST_POST_SLOTS`, or retry after the next drain. |
| `Obj::on*`, `Obj::subscribe`, `subscribe_owned`, `observe_obj` | Called from a callback on an object LVGL is sending an event to. | Register outside the handler, or post the registration with `Lvgl::post`. |
| `Display::try_delete` | A flush is pending. The `Display` is returned with the error. | Complete the flush (`FlushDone`), then retry. |

`delete_later`, `clean_later` and dropped `Owned`s never return `Busy`:
the drain retries them.

## Internal

Refused by the structural checks ([API overview](api-overview.md#exclusive-access-to-an-object-objmut)).

| Returned by | Condition |
| --- | --- |
| `ObjMut::delete`, `Obj::delete_later` | The object is a display layer, a part of a widget or contains one, an object a widget finds by index, or inside a widget class the part tables do not list. |
| `ObjMut::clean`, `Obj::clean_later` | The object has parts, contains a part of a widget above it, has children a widget counts, or is inside an unknown widget. |
| `Owned::delete` | As for `delete`, for an object raw code moved there (safe code cannot): the object is released, alive and unowned, as a refused drop is. |
| `WidgetExt::own`, `create_owned` | As for `delete`. |
| `ObjMut::set_parent`, `set_parent_to`, `move_to_index` | The object is a layer, protected as for `delete`, or pinned (a calendar header); or the new parent does not accept children (a closed or indexed widget, a part whose children are counted). |
| `Obj::create`, `<Widget>::create`, `TileView::create` | The parent does not accept children (for example a tile view: use `add_tile`). |
| `ObjMut::load_screen`, `Lvgl::load_screen` | The object is a display layer. |
| `Label::set_text_static` | The label belongs to a textarea. |
| `TileView::create` | LVGL's tile view has no handler to wrap (not expected with LVGL 9.6). |

A permanent refusal of a deferred request (`delete_later`, `clean_later`,
a dropped `Owned`) found by the drain drops the request and leaves the
object alive. With `debug-counters`, these are counted as `refused`; a
refusal `Owned::delete` returns is not, since the caller sees it.

## Reentrant and NotInitialized

| Returned by | `Reentrant` | `NotInitialized` |
| --- | --- | --- |
| `init` | LVGL is already initialised, or a session or a callback is active. | After `Lvgl::deinit`. |
| `Lvgl::enter` | A session is active, or a callback is running (a recursive lock). | Before `init`, or after `Lvgl::deinit`. |

`next_event` returns `None` with the guard when `Lvgl::enter` fails.

## AlreadyOwned

| Returned by | Condition |
| --- | --- |
| `WidgetExt::own`, `create_owned` | The object already has an `Owned`. |
| `Display::create` | The `DisplayCell` is in use, a draw buffer is in use, or both buffers are the same. |
| `Display::create_boxed` | A draw buffer is in use, or both buffers are the same. |
| `Indev::create`, `Indev::create_button` | The `IndevCell` is in use. |

A cell or buffer is released when its display or device is deleted
(`Display::try_delete`, `Indev::delete`); dropping a `Display` or `Indev`
without deleting it keeps it claimed.

## TooSmall

| Returned by | Condition |
| --- | --- |
| `*_into` getters (`Label::get_text_into`, `Table::get_cell_value_into`, ...) | The text does not fit in the buffer with its NUL. |
| `Display::set_resolution`, `set_rotation`, `set_color_format`, `set_render_mode` | The draw buffers cannot hold what LVGL would then render: a row in partial mode, the whole screen in full and direct modes. |
