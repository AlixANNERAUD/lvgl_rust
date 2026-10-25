//! Restricted handlers: query and draw codes, sent in the middle of layout
//! or rendering.
//!
//! LVGL asks objects questions while it lays out or renders: how large is
//! your content (`GET_SELF_SIZE`), does this point click you (`HIT_TEST`),
//! do you cover this area (`COVER_CHECK`), how far do you draw outside
//! your area (`REFR_EXT_DRAW_SIZE`), and it reports the layers and draw
//! tasks it renders (`DRAW_*`). The frames sending these codes keep
//! pointers that ordinary shared calls could free or invalidate (a queued
//! label draw task keeps `label->text`, `lv_label.c:936`; the refresh keeps
//! its layers, `lv_refr.c`), so shared handlers never run for them
//! ([`handler_allowed`](crate::handler_allowed)).
//!
//! A restricted handler gets a [`QueryEvent`] instead of an
//! [`Event`](crate::Event): it can read the event's parameter, typed per
//! code, and write the answer the code expects, and nothing else. It has
//! no [`Lvgl`] access, so no view, handle resolution, setter, creation or
//! deletion is reachable from it; [`Lvgl::enter`] fails inside it (a
//! callback is running).
//!
//! ```compile_fail
//! use lvgl_rust::{Obj, QueryEvent, enums::QueryCode};
//! fn register(object: &Obj) {
//!     // A restricted handler has no shared access...
//!     let _ = object.on_query(QueryCode::GET_SELF_SIZE, |e: &QueryEvent<'_>| {
//!         let _ = e.lv();
//!     });
//! }
//! ```
//!
//! ```compile_fail
//! use lvgl_rust::{Obj, QueryEvent, enums::QueryCode};
//! fn register(object: &Obj) {
//!     // ...and cannot capture a view either.
//!     let _ = object.on_query(QueryCode::GET_SELF_SIZE, move |_: &QueryEvent<'_>| {
//!         let _ = object.as_ptr();
//!     });
//! }
//! ```
//!
//! ```no_run
//! use lvgl_rust::{Obj, QueryEvent, enums::QueryCode};
//! fn register(object: &Obj) {
//!     // Answers the query: content of 120 x 40.
//!     let _ = object.on_query(QueryCode::GET_SELF_SIZE, |e: &QueryEvent<'_>| {
//!         if let Some(size) = e.self_size() {
//!             size.extend(120, 40);
//!         }
//!     });
//! }
//! ```

use crate::{
    Error, Lvgl, Obj,
    enums::{EventCode, HANDLER_RESTRICTED, QueryCode},
    event::{HandlerResult, UserData, decode, encode},
    sys,
};
use core::{ffi::c_void, marker::PhantomData, mem, ptr, ptr::NonNull};

impl From<QueryCode> for EventCode {
    #[inline]
    fn from(code: QueryCode) -> Self {
        EventCode(code.0)
    }
}

/// Whether restricted handlers ([`Obj::on_query`]) run for `code`: the
/// codes `events.toml` gives the restricted tier (the constants of
/// [`QueryCode`]).
pub fn query_handler_allowed(code: EventCode) -> bool {
    let bits = code.bits();
    let word = bits as usize / 64;
    word < HANDLER_RESTRICTED.len() && (HANDLER_RESTRICTED[word] >> (bits % 64)) & 1 != 0
}

/// A query or draw event being delivered to a restricted handler.
///
/// It gives the event's parameter, typed per code, and nothing of LVGL
/// beyond it: no [`Lvgl`] access and no view. `!Send`, and borrowed for
/// the handler's duration only.
pub struct QueryEvent<'cb> {
    e: NonNull<sys::lv_event_t>,
    _p: PhantomData<(&'cb (), *mut ())>,
}

impl QueryEvent<'_> {
    fn raw(&self) -> *mut sys::lv_event_t {
        self.e.as_ptr()
    }

    /// The code (`lv_event_get_code`).
    pub fn code(&self) -> EventCode {
        // SAFETY: a live event, during its dispatch.
        EventCode(unsafe { sys::lv_event_get_code(self.raw()) } as u32)
    }

    /// Stops the event from bubbling to the parents.
    pub fn stop_bubbling(&self) {
        // SAFETY: a live event; sets a flag of the event.
        unsafe { sys::lv_event_stop_bubbling(self.raw()) }
    }

    /// Stops the remaining handlers of this object.
    pub fn stop_processing(&self) {
        // SAFETY: as above (restricted codes are never `DELETE`).
        unsafe { sys::lv_event_stop_processing(self.raw()) }
    }

    /// The parameter, if the code is one of `codes` and the parameter is
    /// not NULL. The check runs here, never through `LV_USE_CHECK_ARG`.
    fn param<T>(&self, codes: &[EventCode]) -> Option<NonNull<T>> {
        if !codes.contains(&self.code()) {
            return None;
        }
        // SAFETY: a live event.
        NonNull::new(unsafe { sys::lv_event_get_param(self.raw()) }.cast::<T>())
    }

    /// The hit test of a `HIT_TEST` event (`None` for any other code).
    pub fn hit_test(&self) -> Option<HitTest<'_>> {
        // LVGL sends HIT_TEST only from `lv_obj_hit_test`, with a
        // `lv_hit_test_info_t` on its stack (`lv_obj_pos.c:1244-1247`); safe
        // code cannot send it (`events.toml`: `sendable = "no"`).
        self.param(&[EventCode::HIT_TEST]).map(|info| HitTest {
            info,
            _p: PhantomData,
        })
    }

    /// The cover check of a `COVER_CHECK` event (`None` for any other
    /// code).
    pub fn cover_check(&self) -> Option<CoverCheck<'_>> {
        // Sent with a `lv_cover_check_info_t` on the sender's stack
        // (`lv_refr.c:483-486, 1222-1225`); never sendable by safe code.
        self.param::<c_void>(&[EventCode::COVER_CHECK])
            .map(|_| CoverCheck {
                e: self.e,
                _p: PhantomData,
            })
    }

    /// The extra draw size of a `REFR_EXT_DRAW_SIZE` event (`None` for any
    /// other code).
    pub fn ext_draw_size(&self) -> Option<ExtDrawSize<'_>> {
        // Sent with an `int32_t` on the sender's stack
        // (`lv_obj_draw.c:431-432`); never sendable by safe code.
        self.param(&[EventCode::REFR_EXT_DRAW_SIZE])
            .map(|size| ExtDrawSize {
                size,
                _p: PhantomData,
            })
    }

    /// The content size of a `GET_SELF_SIZE` event (`None` for any other
    /// code).
    pub fn self_size(&self) -> Option<SelfSize<'_>> {
        // Sent with a `lv_point_t` on the sender's stack
        // (`lv_obj_pos.c:649-659`); never sendable by safe code.
        self.param(&[EventCode::GET_SELF_SIZE])
            .map(|point| SelfSize {
                point,
                _p: PhantomData,
            })
    }

    /// The layer being drawn by a `DRAW_MAIN*` or `DRAW_POST*` event
    /// (`None` for any other code, or when the event was sent without one:
    /// safe code may send the `*_BEGIN` and `DRAW_POST_END` codes with a
    /// NULL parameter).
    pub fn layer(&self) -> Option<Layer<'_>> {
        use EventCode as C;
        // The refresh and snapshots send these codes with a live layer
        // (`lv_refr.c:131-133, 165-260, 1159-1161`, `lv_snapshot.c:178-180`).
        self.param(&[
            C::DRAW_MAIN_BEGIN,
            C::DRAW_MAIN,
            C::DRAW_MAIN_END,
            C::DRAW_POST_BEGIN,
            C::DRAW_POST,
            C::DRAW_POST_END,
        ])
        .map(|layer| Layer {
            layer,
            _p: PhantomData,
        })
    }

    /// The draw task of a `DRAW_TASK_ADDED` event (`None` for any other
    /// code, or when it was sent without one, as `lv_bar.c:549` does).
    pub fn draw_task(&self) -> Option<DrawTask<'_>> {
        // Sent before the task is queued for a draw unit
        // (`lv_draw.c:146-162`, `lv_draw_mask.c:69-76`): LVGL's renderers do
        // not read it yet, and descriptor fields written here are what they
        // will read.
        self.param(&[EventCode::DRAW_TASK_ADDED])
            .map(|task| DrawTask {
                task,
                _p: PhantomData,
            })
    }
}

/// The parameter of a `HIT_TEST` event: may `point` click the object?
pub struct HitTest<'e> {
    info: NonNull<sys::lv_hit_test_info_t>,
    _p: PhantomData<&'e QueryEvent<'e>>,
}

impl HitTest<'_> {
    /// The point to test, in absolute coordinates.
    pub fn point(&self) -> sys::lv_point_t {
        // SAFETY: a live hit test whose point is the caller's (`lv_obj_pos.c:1245`).
        unsafe { sys::lvgl_sys_hit_test_point(self.info.as_ptr()) }
    }

    /// The current answer (`true` unless an earlier handler said no).
    pub fn result(&self) -> bool {
        // SAFETY: as above.
        unsafe { sys::lvgl_sys_hit_test_result(self.info.as_ptr()) }
    }

    /// Answers: whether the point clicks the object.
    pub fn set_result(&self, clicks: bool) {
        // SAFETY: as above; writes a `bool` of the sender's stack.
        unsafe { sys::lvgl_sys_hit_test_set_result(self.info.as_ptr(), clicks) }
    }
}

/// An answer to `COVER_CHECK`, from weakest to strongest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
#[allow(clippy::unnecessary_cast)] // the bindgen enum type is `i32` on some targets
pub enum CoverResult {
    /// The area is fully covered by the object.
    Cover = sys::lv_cover_res_t_LV_COVER_RES_COVER as u32,
    /// The area is not fully covered.
    NotCover = sys::lv_cover_res_t_LV_COVER_RES_NOT_COVER as u32,
    /// The area is masked out: nothing is drawn there.
    Masked = sys::lv_cover_res_t_LV_COVER_RES_MASKED as u32,
}

/// The parameter of a `COVER_CHECK` event: does the object cover the area?
pub struct CoverCheck<'e> {
    e: NonNull<sys::lv_event_t>,
    _p: PhantomData<&'e QueryEvent<'e>>,
}

impl CoverCheck<'_> {
    /// The area to check, in absolute coordinates.
    pub fn area(&self) -> sys::lv_area_t {
        // SAFETY: a COVER_CHECK event with a non-NULL parameter (checked by
        // `cover_check`); its area is the sender's (`lv_refr.c:485, 1224`).
        unsafe { *sys::lv_event_get_cover_area(self.e.as_ptr()) }
    }

    /// Answers; LVGL keeps only the strongest answer
    /// (`lv_event_set_cover_res`).
    pub fn set_result(&self, result: CoverResult) {
        // SAFETY: as above.
        unsafe { sys::lv_event_set_cover_res(self.e.as_ptr(), result as sys::lv_cover_res_t) }
    }
}

/// The parameter of a `REFR_EXT_DRAW_SIZE` event: how far the object draws
/// outside its area.
pub struct ExtDrawSize<'e> {
    size: NonNull<i32>,
    _p: PhantomData<&'e QueryEvent<'e>>,
}

impl ExtDrawSize<'_> {
    /// The size other handlers asked for so far.
    pub fn get(&self) -> i32 {
        // SAFETY: a live `int32_t` of the sender's stack.
        unsafe { self.size.read() }
    }

    /// Asks for at least `size` pixels (keeps the largest request, as
    /// `lv_event_set_ext_draw_size` does).
    pub fn extend(&self, size: i32) {
        // SAFETY: as above.
        unsafe { self.size.write(self.get().max(size)) }
    }
}

/// The parameter of a `GET_SELF_SIZE` event: the size of the object's
/// content.
pub struct SelfSize<'e> {
    point: NonNull<sys::lv_point_t>,
    _p: PhantomData<&'e QueryEvent<'e>>,
}

impl SelfSize<'_> {
    /// The size other handlers answered so far (`x` is the width, `y` the
    /// height; LVGL asks for one at a time and starts the other at
    /// `LV_COORD_MIN`).
    pub fn get(&self) -> sys::lv_point_t {
        // SAFETY: a live `lv_point_t` of the sender's stack.
        unsafe { self.point.read() }
    }

    /// Answers a content of at least `width` x `height` (keeps the larger
    /// value of each, as LVGL's widgets do).
    pub fn extend(&self, width: i32, height: i32) {
        let mut point = self.get();
        point.x = point.x.max(width);
        point.y = point.y.max(height);
        // SAFETY: as above.
        unsafe { self.point.write(point) }
    }
}

/// The layer of a `DRAW_MAIN*` or `DRAW_POST*` event.
///
/// Read-only: drawing into it (`sys::lv_draw_*`) stays raw. Those calls add
/// draw tasks, may send `DRAW_TASK_ADDED` and dispatch the renderer, and
/// the access audit found frames of the refresh that keep layers across
/// these sends (`events.toml`); descriptors that point to data (label
/// text, image sources) also need it to outlive rendering.
pub struct Layer<'e> {
    layer: NonNull<sys::lv_layer_t>,
    _p: PhantomData<&'e QueryEvent<'e>>,
}

impl Layer<'_> {
    /// The area new draw tasks are clipped to, in absolute coordinates.
    pub fn clip_area(&self) -> sys::lv_area_t {
        // SAFETY: a live layer, during the dispatch.
        unsafe { (*self.layer.as_ptr())._clip_area }
    }

    /// The area of the layer's buffer, in absolute coordinates.
    pub fn buffer_area(&self) -> sys::lv_area_t {
        // SAFETY: as above.
        unsafe { (*self.layer.as_ptr()).buf_area }
    }

    /// The layer, for raw drawing with `sys::lv_draw_*`: using it is
    /// `unsafe`, and sound only for calls that free nothing other frames of
    /// the refresh hold, with descriptors whose data outlives rendering.
    pub fn as_ptr(&self) -> *mut sys::lv_layer_t {
        self.layer.as_ptr()
    }
}

/// The draw task of a `DRAW_TASK_ADDED` event, before any renderer reads
/// it: its kind, area and origin, and its color and opacity, which the
/// handler may change.
///
/// Only colors and opacities are writable: every value of them is valid.
/// Sizes, radii and widths stay raw (renderers trust them), and so do
/// pointers (text, image sources).
pub struct DrawTask<'e> {
    task: NonNull<sys::lv_draw_task_t>,
    _p: PhantomData<&'e QueryEvent<'e>>,
}

impl DrawTask<'_> {
    fn raw(&self) -> *mut sys::lv_draw_task_t {
        self.task.as_ptr()
    }

    /// The kind of task (`LV_DRAW_TASK_TYPE_*`).
    pub fn kind(&self) -> sys::lv_draw_task_type_t {
        // SAFETY: a live draw task, during the dispatch.
        unsafe { sys::lv_draw_task_get_type(self.raw()) }
    }

    /// The area the task draws, in absolute coordinates.
    pub fn area(&self) -> sys::lv_area_t {
        let mut area = sys::lv_area_t {
            x1: 0,
            y1: 0,
            x2: 0,
            y2: 0,
        };
        // SAFETY: as above.
        unsafe { sys::lv_draw_task_get_area(self.raw(), &mut area) };
        area
    }

    fn base(&self) -> *const sys::lv_draw_dsc_base_t {
        // SAFETY: a live task. Every draw descriptor starts with
        // `lv_draw_dsc_base_t` (LVGL writes `base->layer` through this cast,
        // `lv_draw.c:132-133`).
        unsafe { sys::lv_draw_task_get_draw_dsc(self.raw()).cast() }
    }

    /// The part of the widget the task draws (`LV_PART_*`).
    pub fn part(&self) -> u32 {
        // SAFETY: a live descriptor, read in place.
        unsafe { (*self.base()).part }
    }

    /// The widget-specific first id (a table row, a chart series, ...).
    pub fn id1(&self) -> u32 {
        // SAFETY: as above.
        unsafe { (*self.base()).id1 }
    }

    /// The widget-specific second id (a table column, a point, ...).
    pub fn id2(&self) -> u32 {
        // SAFETY: as above.
        unsafe { (*self.base()).id2 }
    }

    /// Pointers to the color and opacity of the descriptor, for the kinds
    /// that have both: fill, border, box shadow, label, line, arc and
    /// triangle.
    fn color_opa(&self) -> Option<(*mut sys::lv_color_t, *mut sys::lv_opa_t)> {
        let t = self.raw();
        macro_rules! kinds {
            ($($getter:ident),*) => { $(
                // SAFETY: a live task; the getter returns its descriptor
                // only when the task is of that kind (checked
                // unconditionally, e.g. `lv_draw_rect.c:72`).
                let dsc = unsafe { sys::$getter(t) };
                if !dsc.is_null() {
                    // SAFETY: a live descriptor of that type.
                    return Some(unsafe { (&raw mut (*dsc).color, &raw mut (*dsc).opa) });
                }
            )* };
        }
        kinds!(
            lv_draw_task_get_fill_dsc,
            lv_draw_task_get_border_dsc,
            lv_draw_task_get_box_shadow_dsc,
            lv_draw_task_get_label_dsc,
            lv_draw_task_get_line_dsc,
            lv_draw_task_get_arc_dsc,
            lv_draw_task_get_triangle_dsc
        );
        None
    }

    /// The color of a fill, border, box shadow, label, line, arc or
    /// triangle task (`None` for other kinds).
    pub fn color(&self) -> Option<sys::lv_color_t> {
        // SAFETY: a live field of the descriptor.
        self.color_opa().map(|(color, _)| unsafe { color.read() })
    }

    /// Changes the color of a fill, border, box shadow, label, line, arc or
    /// triangle task; `false` (and nothing changed) for other kinds.
    pub fn set_color(&self, color: sys::lv_color_t) -> bool {
        self.color_opa()
            // SAFETY: as above; no renderer reads the task yet.
            .map(|(field, _)| unsafe { field.write(color) })
            .is_some()
    }

    /// The opacity of a fill, border, box shadow, label, line, arc or
    /// triangle task (`None` for other kinds).
    pub fn opa(&self) -> Option<u8> {
        // SAFETY: a live field of the descriptor.
        self.color_opa().map(|(_, opa)| unsafe { opa.read() })
    }

    /// Changes the opacity (0 to 255) of a fill, border, box shadow, label,
    /// line, arc or triangle task; `false` (and nothing changed) for other
    /// kinds.
    pub fn set_opa(&self, opa: u8) -> bool {
        self.color_opa()
            // SAFETY: as above.
            .map(|(_, field)| unsafe { field.write(opa) })
            .is_some()
    }
}

/// Runs `body` for the event `e` if its code is a restricted one and the
/// callback depth limit is not reached. Entering a callback level makes
/// [`Lvgl::enter`] fail inside the handler.
fn dispatch(e: *mut sys::lv_event_t, body: &mut dyn FnMut(&QueryEvent<'_>)) {
    let Some(e) = NonNull::new(e) else { return };
    let event = QueryEvent { e, _p: PhantomData };
    if !query_handler_allowed(event.code()) {
        return;
    }
    // SAFETY: LVGL runs handlers only inside calls made by the session
    // holder; the guard stays within this callback, and its shared access
    // is never handed to the handler.
    let Some(_depth) = (unsafe { Lvgl::enter_callback() }) else {
        return;
    };
    body(&event);
}

unsafe extern "C" fn fn_trampoline(e: *mut sys::lv_event_t) {
    // SAFETY: a live event during its dispatch.
    let p = unsafe { sys::lv_event_get_user_data(e) };
    // SAFETY: stored by `on_query_fn` from a `fn(&QueryEvent)`.
    let f = unsafe { mem::transmute::<*mut c_void, fn(&QueryEvent<'_>)>(p) };
    dispatch(e, &mut |event| f(event));
}

unsafe extern "C" fn zst_trampoline<F, R>(e: *mut sys::lv_event_t)
where
    F: Fn(&QueryEvent<'_>) -> R + Send + 'static,
    R: HandlerResult,
{
    // SAFETY: `F` is zero-sized (checked by `on_query`): a dangling pointer
    // is a valid reference to it, and no `F` value is created or dropped.
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    dispatch(e, &mut |event| {
        let _ = f(event);
    });
}

unsafe extern "C" fn data_trampoline<D, F, R>(e: *mut sys::lv_event_t)
where
    D: UserData,
    F: Fn(&QueryEvent<'_>, D) -> R + Send + 'static,
    R: HandlerResult,
{
    // SAFETY: as above.
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    // SAFETY: stored by `on_query_with` with `encode::<D>`.
    let d = unsafe { decode::<D>(sys::lv_event_get_user_data(e)) };
    dispatch(e, &mut |event| {
        let _ = f(event, d);
    });
}

impl Obj {
    /// Calls the restricted handler `f` for every `code` event of this
    /// object (see [`QueryEvent`]).
    pub fn on_query_fn(&self, code: QueryCode, f: fn(&QueryEvent<'_>)) -> Result<(), Error> {
        self.register_handler(code.into(), fn_trampoline, f as *mut c_void)
            .map(drop)
    }

    /// Calls the zero-sized closure `f` (no captures, or zero-sized ones)
    /// as a restricted handler for every `code` event. Captured values are
    /// never dropped.
    pub fn on_query<F, R>(&self, code: QueryCode, f: F) -> Result<(), Error>
    where
        F: Fn(&QueryEvent<'_>) -> R + Send + 'static,
        R: HandlerResult,
    {
        const { assert!(mem::size_of::<F>() == 0, "use on_query_with for state") };
        mem::forget(f);
        self.register_handler(code.into(), zst_trampoline::<F, R>, ptr::null_mut())
            .map(drop)
    }

    /// Calls the zero-sized closure `f` with `data` (a [`UserData`] value,
    /// stored in the handler: no allocation) as a restricted handler for
    /// every `code` event.
    pub fn on_query_with<D, F, R>(&self, code: QueryCode, data: D, f: F) -> Result<(), Error>
    where
        D: UserData,
        F: Fn(&QueryEvent<'_>, D) -> R + Send + 'static,
        R: HandlerResult,
    {
        const { assert!(mem::size_of::<F>() == 0, "UserData holds at most 4 bytes") };
        mem::forget(f);
        self.register_handler(code.into(), data_trampoline::<D, F, R>, encode(data))
            .map(drop)
    }
}
