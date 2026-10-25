//! Animations of objects: LVGL calls a Rust function with each new value,
//! from its animation timer, with shared access.

use crate::{
    AnimPath, Error, Lvgl, Obj, ObjMut, Widget,
    base::gen_support::getter_view,
    event::{UserData, decode, encode},
    sys,
};
use core::{marker::PhantomData, mem, ptr::NonNull};

/// An animation to start on an object with [`ObjMut::animate`]: built on
/// the stack (LVGL copies it), carrying a [`UserData`] value for its
/// callbacks.
///
/// ```no_run
/// use lvgl_rust::{Anim, AnimPath, ObjMut, Obj};
///
/// fn fade_in(object: &mut ObjMut<'_, Obj>) {
///     let anim = Anim::new(())
///         .values(0, 255)
///         .duration(300)
///         .path(AnimPath::EaseOut)
///         .exec(|ctx, (), value| {
///             if let Some(target) = ctx.target() {
///                 let _ = target; // set the opacity from `value`
///             }
///         });
///     object.animate(anim).unwrap();
/// }
/// ```
pub struct Anim<D: UserData = ()> {
    a: sys::lv_anim_t,
    _d: PhantomData<D>,
}

/// What animation callbacks get: shared access and the animated object.
pub struct AnimCtx<'cb> {
    a: NonNull<sys::lv_anim_t>,
    _p: PhantomData<(&'cb Lvgl, *mut ())>,
}

impl<'cb> AnimCtx<'cb> {
    /// Shared access, for the call's duration.
    pub fn lv(&self) -> &'cb Lvgl {
        // SAFETY: `Lvgl` is zero-sized; animation callbacks get shared access.
        unsafe { &*NonNull::<Lvgl>::dangling().as_ptr() }
    }

    /// The animated object (`None` while it is being deleted).
    pub fn target(&self) -> Option<&'cb Obj> {
        // SAFETY: a live animation; its `var` is the object it was started
        // on, and LVGL deletes an object's animations with it.
        getter_view(unsafe { (*self.a.as_ptr()).var }.cast())
    }
}

/// Runs `body` with a context for `a`, under the callback depth limit.
fn with_ctx(a: *mut sys::lv_anim_t, body: &mut dyn FnMut(&AnimCtx<'_>, *mut core::ffi::c_void)) {
    let Some(a) = NonNull::new(a) else { return };
    // SAFETY: LVGL runs animations only from `lv_timer_handler` (or an
    // exclusive call), made by the session holder.
    let Some(_depth) = (unsafe { Lvgl::enter_callback() }) else {
        return;
    };
    // SAFETY: a live animation during its callback.
    let data = unsafe { sys::lv_anim_get_user_data(a.as_ptr()) };
    body(&AnimCtx { a, _p: PhantomData }, data);
}

unsafe extern "C" fn exec_trampoline<D, F>(a: *mut sys::lv_anim_t, value: i32)
where
    D: UserData,
    F: Fn(&AnimCtx<'_>, D, i32) + Send + 'static,
{
    // SAFETY: `F` is zero-sized (checked by `exec`).
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    // SAFETY: stored by `Anim::new` with `encode::<D>`.
    with_ctx(a, &mut |ctx, data| {
        f(ctx, unsafe { decode::<D>(data) }, value)
    });
}

unsafe extern "C" fn completed_trampoline<D, F>(a: *mut sys::lv_anim_t)
where
    D: UserData,
    F: Fn(&AnimCtx<'_>, D) + Send + 'static,
{
    // SAFETY: as above.
    let f = unsafe { NonNull::<F>::dangling().as_ref() };
    // SAFETY: as above.
    with_ctx(a, &mut |ctx, data| f(ctx, unsafe { decode::<D>(data) }));
}

impl<D: UserData> Anim<D> {
    /// A new animation from 0 to 100 over 500 ms (LVGL's defaults), with
    /// `data` for its callbacks.
    pub fn new(data: D) -> Self {
        // SAFETY: `lv_anim_init` writes every field.
        let mut a: sys::lv_anim_t = unsafe { mem::zeroed() };
        // SAFETY: a local animation descriptor.
        unsafe {
            sys::lv_anim_init(&mut a);
            sys::lv_anim_set_user_data(&mut a, encode(data));
        }
        Self { a, _d: PhantomData }
    }

    /// From `start` to `end`.
    pub fn values(mut self, start: i32, end: i32) -> Self {
        // SAFETY: a local descriptor.
        unsafe { sys::lv_anim_set_values(&mut self.a, start, end) };
        self
    }

    /// Time from start to end.
    pub fn duration(mut self, ms: u32) -> Self {
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_duration(&mut self.a, ms) };
        self
    }

    /// Time before starting.
    pub fn delay(mut self, ms: u32) -> Self {
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_delay(&mut self.a, ms) };
        self
    }

    /// Plays back from end to start, over `ms`.
    pub fn reverse(mut self, ms: u32) -> Self {
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_reverse_duration(&mut self.a, ms) };
        self
    }

    /// Repeats `count` times (`None`: forever).
    pub fn repeat(mut self, count: Option<u32>) -> Self {
        let count = count.unwrap_or(sys::LV_ANIM_REPEAT_INFINITE);
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_repeat_count(&mut self.a, count) };
        self
    }

    /// Time between repetitions.
    pub fn repeat_delay(mut self, ms: u32) -> Self {
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_repeat_delay(&mut self.a, ms) };
        self
    }

    /// How values move from start to end.
    pub fn path(mut self, path: AnimPath) -> Self {
        // SAFETY: as above; LVGL's built-in paths.
        unsafe { sys::lv_anim_set_path_cb(&mut self.a, path.callback()) };
        self
    }

    /// Applies the start value as soon as the animation starts, delay
    /// included.
    pub fn early_apply(mut self, early: bool) -> Self {
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_early_apply(&mut self.a, early) };
        self
    }

    /// Calls `f(ctx, data, value)` with each value. `f` is a function item
    /// or a closure without captures.
    pub fn exec<F>(mut self, f: F) -> Self
    where
        F: Fn(&AnimCtx<'_>, D, i32) + Send + 'static,
    {
        const {
            assert!(
                mem::size_of::<F>() == 0,
                "exec takes a function item or a closure without captures"
            )
        };
        mem::forget(f);
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_custom_exec_cb(&mut self.a, Some(exec_trampoline::<D, F>)) };
        self
    }

    /// Calls `f(ctx, data)` when the animation completes.
    pub fn on_completed<F>(mut self, f: F) -> Self
    where
        F: Fn(&AnimCtx<'_>, D) + Send + 'static,
    {
        const {
            assert!(
                mem::size_of::<F>() == 0,
                "on_completed takes a function item or a closure without captures"
            )
        };
        mem::forget(f);
        // SAFETY: as above.
        unsafe { sys::lv_anim_set_completed_cb(&mut self.a, Some(completed_trampoline::<D, F>)) };
        self
    }
}

impl<W: Widget> ObjMut<'_, W> {
    /// Starts `anim` on this object, next to its running animations (LVGL
    /// replaces animations only by their plain `exec_cb`, `lv_anim.c:843`:
    /// call [`delete_animations`](ObjMut::delete_animations) first to
    /// replace). LVGL copies it and deletes it with the object. Exclusive access: starting removes the animations it
    /// replaces, whose callbacks can reach deletions.
    pub fn animate<D: UserData>(&mut self, mut anim: Anim<D>) -> Result<(), Error> {
        let o = self.as_raw();
        // SAFETY: exclusive access; the descriptor is complete and its
        // callbacks and data stay valid (functions and values).
        let started = unsafe {
            sys::lv_anim_set_var(&mut anim.a, o.cast());
            sys::lv_anim_start(&anim.a)
        };
        if started.is_null() {
            Err(Error::OutOfMemory)
        } else {
            Ok(())
        }
    }

    /// Deletes every animation of this object. Returns whether there was
    /// one.
    pub fn delete_animations(&mut self) -> bool {
        // SAFETY: exclusive access; NULL matches every callback.
        unsafe { sys::lv_anim_delete(self.as_raw().cast(), None) }
    }
}
