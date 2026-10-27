//! Views: zero-sized references at an object's address. A view `&'a W`
//! borrows the [`Lvgl`] it came from, so it cannot outlive shared access,
//! and calls through it are plain FFI calls.

use super::{
    ffi::{self, lv_obj_class_t, lv_obj_t},
    registry::reg,
};
use crate::{Error, Lvgl, ObjMut};
use core::{cell::UnsafeCell, marker::PhantomData, marker::PhantomPinned, ptr::NonNull};

#[cfg(not(lvgl_rust_views_only))]
use super::{
    registry::{Kind, meta::*},
    structural,
    sync::Ordering,
};
#[cfg(not(lvgl_rust_views_only))]
use crate::{Handle, Owned};

/// A widget class with a view type.
///
/// # Safety
///
/// `Self` must be a zero-sized `#[repr(transparent)]` wrapper of `Base`
/// (ultimately of [`Obj`]), and every object whose class is [`class`] or a
/// subclass of it must support every safe method of `Self`.
///
/// [`class`]: Widget::class
pub unsafe trait Widget: 'static {
    /// The direct base class (`Obj` for `Obj` itself).
    type Base: Widget;

    /// The LVGL class descriptor.
    fn class() -> *const lv_obj_class_t;

    /// Every object is an `Obj`: class checks for `Obj` are skipped.
    #[doc(hidden)]
    const IS_OBJ: bool = false;
}

/// `Self` is `B` or a subclass of it.
///
/// # Safety
///
/// Every object of class `Self` must be a valid `B`.
pub unsafe trait IsA<B: Widget>: Widget {}

/// Class to check when resolving a handle to `W` (null: no check).
#[cfg(not(lvgl_rust_views_only))]
pub(crate) fn class_check<W: Widget>() -> *const lv_obj_class_t {
    if W::IS_OBJ {
        core::ptr::null()
    } else {
        W::class()
    }
}

/// The base object (`lv_obj_t`). Only ever used behind a reference.
#[repr(C)]
pub struct Obj {
    _o: UnsafeCell<[u8; 0]>,
    _m: PhantomData<(*mut u8, PhantomPinned)>,
}

// SAFETY: `Obj` is the root of the hierarchy.
unsafe impl Widget for Obj {
    type Base = Obj;
    fn class() -> *const lv_obj_class_t {
        ffi::obj_class()
    }
    const IS_OBJ: bool = true;
}

// SAFETY: reflexive.
unsafe impl IsA<Obj> for Obj {}

/// A view of the object at `p`.
///
/// # Safety
///
/// `p` must be a live object of class `W` (or a subclass) that is not being
/// deleted, and stay so for `'a` (invariant V).
pub(crate) unsafe fn view<'a, W: Widget>(p: *mut lv_obj_t) -> &'a W {
    // SAFETY: `W` is zero-sized: a non-null pointer makes a valid reference.
    unsafe { &*p.cast::<W>() }
}

/// The object a view stands for.
pub(crate) fn ptr_of<W: Widget>(w: &W) -> *mut lv_obj_t {
    (w as *const W).cast_mut().cast()
}

/// Whether `o` or one of its ancestors is being deleted. Only callbacks
/// can see such objects: LVGL deletes the children of an object after
/// marking it, and sends DELETE to each.
///
/// # Safety
///
/// `o` must be a live object, under the token.
pub(crate) unsafe fn in_deletion(mut o: *mut lv_obj_t) -> bool {
    while !o.is_null() {
        // SAFETY: caller contract; ancestors of a live object are live.
        unsafe {
            if ffi::obj_is_deleting(o) {
                return true;
            }
            o = ffi::obj_parent(o);
        }
    }
    false
}

/// The view of an object an exclusive call just created (also used by the
/// generated `create` functions): [`Error::OutOfMemory`] for NULL.
///
/// # Safety
///
/// `p` is NULL or a live object of class `W` just created, and the caller
/// holds access for `'a` (borrowing the parent's `ObjMut`, so that no
/// exclusive call runs while the view lives).
pub(crate) unsafe fn created<'a, W: Widget>(p: *mut lv_obj_t) -> Result<&'a W, Error> {
    if p.is_null() {
        return Err(Error::OutOfMemory);
    }
    // SAFETY: caller contract.
    Ok(unsafe { view(p) })
}

/// A view of a non-null object returned by an LVGL getter, unless the
/// object or an ancestor is being deleted (possible only inside callbacks).
pub(crate) fn getter_view<'a>(p: *mut lv_obj_t) -> Option<&'a Obj> {
    if p.is_null() {
        return None;
    }
    // SAFETY: LVGL returned a live object; outside callbacks no deletion is
    // in progress.
    if reg().hdr.depth.get() != 0 && unsafe { in_deletion(p) } {
        return None;
    }
    // SAFETY: as above.
    Some(unsafe { view(p) })
}

impl Obj {
    /// Creates a base object (`lv_obj_create`) under `parent`, with
    /// exclusive access: creation can reach code that moves or deletes
    /// other objects (the access table marks `lv_obj_create` exclusive).
    /// Refuses parents whose children a widget indexes or casts
    /// ([`Error::Internal`]), like [`ObjMut::set_parent`]. Callbacks, which
    /// have shared access only, create objects through
    /// [`Lvgl::post`](crate::Lvgl::post).
    ///
    /// [`ObjMut::set_parent`]: crate::ObjMut
    ///
    /// The new object is lent as a view borrowing the parent's `ObjMut`,
    /// like every object an exclusive call returns: an `ObjMut` of the
    /// child would allow exclusive calls (sending a deleting event, ...)
    /// that delete the parent while its `ObjMut` is still in scope. For
    /// exclusive calls on the child, take a handle (or use
    /// [`Lvgl::edit`]) once the parent's borrow ends.
    ///
    /// The child is a view, never an `ObjMut`:
    ///
    /// ```compile_fail
    /// # use lvgl_rust::{Error, Obj, ObjMut};
    /// fn f(parent: &mut ObjMut<'_, Obj>) -> Result<(), Error> {
    ///     let child: ObjMut<'_, Obj> = Obj::create(parent)?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// and the parent stays borrowed while it lives:
    ///
    /// ```compile_fail
    /// # use lvgl_rust::{Error, Obj, ObjMut};
    /// fn f(parent: &mut ObjMut<'_, Obj>) -> Result<(), Error> {
    ///     let child = Obj::create(parent)?;
    ///     parent.clean()?;
    ///     let _ = child.as_ptr();
    ///     Ok(())
    /// }
    /// ```
    ///
    /// The generated `create` of every widget has the same shape:
    ///
    /// ```compile_fail
    /// # use lvgl_rust::{Error, Obj, ObjMut, widgets::Button};
    /// fn f(parent: &mut ObjMut<'_, Obj>) -> Result<(), Error> {
    ///     let button: ObjMut<'_, Button> = Button::create(parent)?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// # use lvgl_rust::{Error, Obj, ObjMut, widgets::Button};
    /// fn f(parent: &mut ObjMut<'_, Obj>) -> Result<(), Error> {
    ///     let button = Button::create(parent)?;
    ///     parent.clean()?;
    ///     let _ = button.as_ptr();
    ///     Ok(())
    /// }
    /// ```
    ///
    /// What compiles:
    ///
    /// ```no_run
    /// # use lvgl_rust::{Error, Obj, ObjMut, widgets::Button};
    /// fn f(parent: &mut ObjMut<'_, Obj>) -> Result<(), Error> {
    ///     let child = Obj::create(parent)?;
    ///     let _ = child.as_ptr();
    ///     let button: &Button = Button::create(parent)?;
    ///     let _ = button.as_ptr();
    ///     parent.clean()
    /// }
    /// ```
    pub fn create<'p, P: Widget>(parent: &'p mut ObjMut<'_, P>) -> Result<&'p Obj, Error> {
        super::structural::check_create(parent.as_raw())?;
        // SAFETY: exclusive access through the parent's `ObjMut`.
        let p = unsafe { ffi::obj_create(parent.as_raw()) };
        // SAFETY: just created, of this class.
        unsafe { created(p) }
    }

    /// Creates a base object and takes ownership of it.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn create_owned<P: Widget>(parent: &mut ObjMut<'_, P>) -> Result<Owned<Obj>, Error> {
        use crate::WidgetExt;
        Self::create(parent)?.own()
    }

    /// The raw pointer, valid until the next exclusive call.
    pub fn as_ptr(&self) -> *mut lv_obj_t {
        ptr_of(self)
    }

    /// A view of a raw object.
    ///
    /// # Safety
    ///
    /// `p` must be a live object that is not being deleted. Cross-object
    /// pointers set up through `sys` (a keyboard's textarea, a widget given a
    /// user object as part) are the caller's responsibility: safe code may
    /// delete the objects they point to.
    pub unsafe fn from_raw(lv: &Lvgl, p: NonNull<lv_obj_t>) -> &Obj {
        let _ = lv;
        // SAFETY: caller contract.
        unsafe { view(p.as_ptr()) }
    }

    /// This object as a `W`, if its class is `W` or a subclass
    /// (`lv_obj_has_class`).
    pub fn downcast<W: Widget>(&self) -> Option<&W> {
        // SAFETY: live object (V).
        let ok = W::IS_OBJ || unsafe { ffi::obj_has_class(self.as_ptr(), W::class()) };
        // SAFETY: class checked.
        ok.then(|| unsafe { view(self.as_ptr()) })
    }

    /// The child at `index` (negative: from the end).
    pub fn child(&self, index: i32) -> Option<&Obj> {
        // SAFETY: live object (V).
        getter_view(unsafe { ffi::obj_child(self.as_ptr(), index) })
    }

    /// The parent, `None` for a screen or a layer.
    pub fn parent(&self) -> Option<&Obj> {
        // SAFETY: live object (V).
        getter_view(unsafe { ffi::obj_parent(self.as_ptr()) })
    }

    /// Deletes this object at the next drain (next
    /// [`Lvgl::enter`](crate::Lvgl::enter) or after
    /// [`Lvgl::timer_handler`](crate::Lvgl::timer_handler)). Usable from
    /// callbacks. Takes a registry slot until then if the object has none.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn delete_later(&self) -> Result<(), Error> {
        self.request(P_DELETE, structural::check_delete)
    }

    /// Deletes the children of this object at the next drain, as
    /// [`delete_later`](Obj::delete_later) does.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn clean_later(&self) -> Result<(), Error> {
        self.request(P_CLEAN, structural::check_clean)
    }

    #[cfg(not(lvgl_rust_views_only))]
    fn request(&self, bit: u8, check: fn(*mut lv_obj_t) -> Result<(), Error>) -> Result<(), Error> {
        let o = self.as_ptr();
        // Permanent refusals fail now; `Busy` is retried by the drain.
        match check(o) {
            Ok(()) | Err(Error::Busy) => {}
            Err(error) => return Err(error),
        }
        let reg = reg();
        let minted = reg.mint(o.cast(), Kind::Obj)?;
        let slot = reg.slot(minted.index);
        if minted.fresh {
            slot.set(TEMP);
        }
        slot.set(bit);
        reg.hdr.any_req.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// Keeping objects: handles and ownership. Implemented for every widget.
#[cfg(not(lvgl_rust_views_only))]
pub trait WidgetExt: Widget + private::Sealed {
    /// A handle to this object, taking a registry slot the first time
    /// ([`Error::RegistryFull`] when none is left).
    fn handle(&self) -> Result<Handle<Self>, Error>
    where
        Self: Sized;

    /// Takes ownership: the object is deleted when the [`Owned`] is dropped.
    /// Fails with [`Error::AlreadyOwned`] for an object that has an owner and
    /// [`Error::Internal`] for objects LVGL manages (display layers).
    fn own(&self) -> Result<Owned<Self>, Error>
    where
        Self: Sized;
}

#[cfg(not(lvgl_rust_views_only))]
impl<W: Widget> WidgetExt for W {
    fn handle(&self) -> Result<Handle<W>, Error> {
        let reg = reg();
        let minted = reg.mint(ptr_of(self).cast(), Kind::Obj)?;
        reg.slot(minted.index).clear(TEMP);
        // SAFETY: a fresh raw handle of a slot tracking a `W`.
        Ok(unsafe { Handle::from_raw(minted.raw) })
    }

    fn own(&self) -> Result<Owned<W>, Error> {
        let o = ptr_of(self);
        structural::check_own(o)?;
        let reg = reg();
        let minted = reg.mint(o.cast(), Kind::Obj)?;
        let slot = reg.slot(minted.index);
        if slot.has(OWNED) {
            return Err(Error::AlreadyOwned);
        }
        slot.set(OWNED);
        slot.clear(TEMP);
        // SAFETY: as above; the slot is now owned by this value only.
        Ok(unsafe { Owned::from_raw(minted.raw) })
    }
}

#[cfg(not(lvgl_rust_views_only))]
mod private {
    pub trait Sealed {}
    impl<W: super::Widget> Sealed for W {}
}
