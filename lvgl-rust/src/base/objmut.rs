//! Exclusive access to one object ([`ObjMut`]) and the screen-level
//! operations of [`Lvgl`].

use super::{
    ffi::{self, lv_obj_t},
    structural,
    view::{Widget, getter_view, ptr_of, view},
};
use crate::{Error, Lvgl, Obj, ScreenAnim};
use core::{marker::PhantomData, ptr::NonNull};

#[cfg(not(lvgl_rust_views_only))]
use super::{
    registry::{Kind, index, reg},
    view::class_check,
};
#[cfg(not(lvgl_rust_views_only))]
use crate::{Handle, Owned, WidgetExt};

/// Exclusive access to an object: it borrows `&mut Lvgl`, so no view exists
/// while it does, and structural operations (delete, clean, reparent) are
/// safe.
///
/// Widgets keep pointers to some of their children, so these operations
/// are refused with [`Error::Internal`] on display layers and wherever a
/// widget could hold such a pointer: an object can be deleted, owned or
/// moved only if all its ancestors are plain objects (`lv_obj`), cleaned
/// only if it is one too, and moved only under a plain object.
pub struct ObjMut<'s, W> {
    p: NonNull<lv_obj_t>,
    _l: PhantomData<&'s mut Lvgl>,
    _w: PhantomData<fn() -> W>,
}

impl<W: Widget> ObjMut<'_, W> {
    /// # Safety
    ///
    /// `p` must be a live object of class `W` that is not being deleted, and
    /// the caller must hold exclusive access for the returned lifetime.
    pub(crate) unsafe fn new(p: NonNull<lv_obj_t>) -> Self {
        Self {
            p,
            _l: PhantomData,
            _w: PhantomData,
        }
    }

    /// The object (also used by generated exclusive wrappers).
    pub(crate) fn as_raw(&self) -> *mut lv_obj_t {
        self.p.as_ptr()
    }

    /// The same object as a base class, for the exclusive methods of that
    /// class (`ObjMut<'_, Slider>` to `ObjMut<'_, Bar>`).
    pub fn as_base<B: Widget>(&mut self) -> ObjMut<'_, B>
    where
        W: crate::IsA<B>,
    {
        // SAFETY: same object, a `B` by `IsA`; the reborrow keeps
        // exclusive access.
        unsafe { ObjMut::new(self.p) }
    }

    /// A view of the object, for the shared API.
    pub fn view(&self) -> &W {
        // SAFETY: live while `self` holds exclusive access.
        unsafe { view(self.as_raw()) }
    }

    /// Deletes the object and its children. Refuses display layers
    /// ([`Error::Internal`]) and screens LVGL is loading ([`Error::Busy`]).
    pub fn delete(self) -> Result<(), Error> {
        structural::delete(self.as_raw())
    }

    /// Deletes the children of the object.
    pub fn clean(&mut self) -> Result<(), Error> {
        structural::clean(self.as_raw())
    }

    /// Moves the object to `index` among its siblings (negative: from the
    /// end; out of range: no change). Refused where moving it to another
    /// parent would be ([`Error::Internal`]): widgets find some children
    /// by index.
    pub fn move_to_index(&mut self, index: i32) -> Result<(), Error> {
        structural::move_to_index(self.as_raw(), index)
    }

    /// Moves the object under `parent`. Refuses a cycle
    /// ([`Error::InvalidArg`]) and display layers ([`Error::Internal`]).
    #[cfg(not(lvgl_rust_views_only))]
    pub fn set_parent(&mut self, parent: Handle<Obj>) -> Result<(), Error> {
        let parent = reg().resolve(parent.raw(), Kind::Obj, class_check::<Obj>())?;
        structural::set_parent(self.as_raw(), parent.as_ptr().cast())
    }

    /// Moves the object under the parent chosen by `select`
    /// ([`Error::Dead`] when it chooses none).
    pub fn set_parent_to(
        &mut self,
        select: impl for<'a> FnOnce(&'a Lvgl) -> Option<&'a Obj>,
    ) -> Result<(), Error> {
        // SAFETY: zero-sized; `self` holds exclusive access, of which the
        // selector borrows a shared part that ends before the call below.
        let lv = unsafe { &*NonNull::<Lvgl>::dangling().as_ptr() };
        let parent = select(lv).ok_or(Error::Dead)?.as_ptr();
        structural::set_parent(self.as_raw(), parent)
    }

    /// Loads the object as the active screen. It must have no parent
    /// ([`Error::InvalidArg`]).
    pub fn load_screen(self, anim: ScreenAnim) -> Result<(), Error> {
        structural::load_screen(self.as_raw(), anim)
    }
}

impl<'s, W: Widget> ObjMut<'s, W> {
    /// Gives up exclusive access for a view of the object, for as long as
    /// that access was borrowed: what [`view`](ObjMut::view) gives, without
    /// keeping the `ObjMut`. Handy after creating an object:
    /// `Label::create(&mut parent)?.into_view()`.
    pub fn into_view(self) -> &'s W {
        // SAFETY: the exclusive borrow of `'s` becomes a shared one: the
        // object stays live and no exclusive call can run while the view
        // exists.
        unsafe { view(self.as_raw()) }
    }
}

impl Lvgl {
    /// Creates a screen (an object without a parent). A screen has no parent
    /// to own it, so it is returned owned. Like every creation, it needs
    /// exclusive access (`lv_obj_create` is exclusive in the access table).
    #[cfg(not(lvgl_rust_views_only))]
    pub fn create_screen(&mut self) -> Result<Owned<Obj>, Error> {
        let screen = create_screen()?;
        screen.view().own().inspect_err(|_| {
            // Nothing refers to the new screen yet: deleting it cannot leave
            // a dangling pointer.
            // SAFETY: live object, unreachable from any frame.
            unsafe { ffi::obj_delete(screen.as_raw()) }
        })
    }

    /// Creates a screen (an object without a parent). Like every creation,
    /// it needs exclusive access.
    #[cfg(lvgl_rust_views_only)]
    pub fn create_screen(&mut self) -> Result<ObjMut<'_, Obj>, Error> {
        create_screen()
    }

    /// Exclusive access to the active screen of the default display, to
    /// create objects on it.
    pub fn screen_active_mut(&mut self) -> Option<ObjMut<'_, Obj>> {
        self.edit(|lv| lv.screen_active())
    }

    /// The active screen of the default display.
    pub fn screen_active(&self) -> Option<&Obj> {
        // SAFETY: shared access.
        getter_view(unsafe { ffi::screen_active() })
    }

    /// Exclusive access to the object `select` chooses among the views of
    /// shared access.
    pub fn edit<W: Widget>(
        &mut self,
        select: impl for<'a> FnOnce(&'a Lvgl) -> Option<&'a W>,
    ) -> Option<ObjMut<'_, W>> {
        let p = ptr_of(select(self)?);
        // SAFETY: the selector's borrow ended and no LVGL call ran since, so
        // the view's object is still live; `self` is borrowed exclusively.
        Some(unsafe { ObjMut::new(NonNull::new_unchecked(p)) })
    }

    /// Loads `screen` as the active screen.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn load_screen(&mut self, screen: Handle<Obj>, anim: ScreenAnim) -> Result<(), Error> {
        let p = reg().resolve(screen.raw(), Kind::Obj, class_check::<Obj>())?;
        structural::load_screen(p.as_ptr().cast(), anim)
    }

    /// Loads `screen` at the next drain; usable from callbacks. A later
    /// request replaces a pending one; a screen deleted meanwhile is skipped.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn load_screen_later(&self, screen: Handle<Obj>, anim: ScreenAnim) -> Result<(), Error> {
        let reg = reg();
        reg.resolve(screen.raw(), Kind::Obj, class_check::<Obj>())?;
        reg.hdr.load_anim.set(anim);
        reg.hdr.load.set(index(screen.raw()) as u16 + 1);
        Ok(())
    }
}

/// A screen, with exclusive access: the caller holds `&mut Lvgl`.
fn create_screen<'a>() -> Result<ObjMut<'a, Obj>, Error> {
    // SAFETY: exclusive access (the caller holds `&mut Lvgl`).
    let p = unsafe { ffi::obj_create(core::ptr::null_mut()) };
    // SAFETY: just created.
    unsafe { created(p) }
}

/// The `ObjMut` of an object an exclusive call just created (also used by
/// the generated `create` functions): [`Error::OutOfMemory`] for NULL.
///
/// # Safety
///
/// `p` is NULL or a live object of class `W` just created, and the caller
/// holds exclusive access for `'a`.
pub(crate) unsafe fn created<'a, W: Widget>(p: *mut lv_obj_t) -> Result<ObjMut<'a, W>, Error> {
    let p = NonNull::new(p).ok_or(Error::OutOfMemory)?;
    // SAFETY: caller contract; a new object is not being deleted.
    Ok(unsafe { ObjMut::new(p) })
}
