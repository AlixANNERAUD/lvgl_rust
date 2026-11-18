# Host-owned object identity

Goal: give some objects a host-defined identity (an integer a guest or
another process can hold) without a handle registry, at no cost for the
objects that never get one.

Handles ([Host several clients](host-several-clients.md)) cost a registry
slot per tracked object. On a board with a few hundred kilobytes of RAM you
may want none of that: borrowed views for native code, and one 4 B word per
object that LVGL already has when `LV_USE_OBJ_ID` is on.

## Steps

1. Select views-only mode: the `views-only` feature of `lvgl-rust`, or
   `LVGL_RUST_HANDLE_SLOTS=0`. The crate then defines no OBJ_ID hooks.
2. In `lv_conf.h`, set `LV_USE_OBJ_ID 1`, `LV_OBJ_ID_AUTO_ASSIGN 1` and
   `LV_USE_OBJ_ID_BUILTIN 0`. The build script warns that the host must
   define the five hooks.
3. Define `lv_obj_assign_id` (a no-op), `lv_obj_free_id`, `lv_obj_set_id`,
   `lv_obj_id_compare` and `lv_obj_stringify_id` with `#[unsafe(no_mangle)]`.
   `lv_obj_free_id` runs at the end of the destructor of every object, on
   every deletion path (`delete`, a parent's deletion, `clean`, a deleted
   screen, `Lvgl::deinit`), while the object's memory and its `id` are still
   valid.
4. Keep the table that names your objects as a sorted array of
   `(id, pointer)` pairs, allocated to its exact size. Binary search gives
   lookups in O(log n) and costs 8 B per named object on 32-bit targets,
   nothing for the others.
5. Write the `id` field from one function only, which also updates the table.
   Remove entries in `lv_obj_free_id`, keyed by pointer: read the object's
   `id`, find it, and compare the stored pointer before removing, so an
   identical number held for another table is left alone.

## Pitfalls

- Never dereference an `id` as a pointer, and never pass an `id` a guest
  supplied to `lv_obj_find_by_id`: with a null root it searches every
  object, whatever table it belongs to.
- Issue identifiers from a counter that is never reset, and fail creations
  when it runs out. An address LVGL reuses after a deletion then never
  carries an old identity.
- A build either uses the registry (`lvgl-rust` stores slot indices in the
  `id` field and defines the hooks) or this setup. Two definitions of the
  hooks fail at link time.

`lvgl-rust/tests/host_object_ids.rs` is a complete host.
