//! Views: zero-sized references at an object's address. A view `&'a W`
//! borrows the [`Lvgl`] it came from, so it cannot outlive shared access,
//! and calls through it are plain FFI calls.

use super::{
    ffi::{self, lv_obj_class_t, lv_obj_t},
    registry::reg,
};
use crate::{Error, Lvgl};
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

/// A view of a non-null object returned by an LVGL getter, unless the
/// object is being deleted (possible only inside callbacks).
pub(crate) fn getter_view<'a>(p: *mut lv_obj_t) -> Option<&'a Obj> {
    if p.is_null() {
        return None;
    }
    // SAFETY: LVGL returned a live object; outside callbacks no deletion is
    // in progress.
    if reg().hdr.depth.get() != 0 && unsafe { ffi::obj_is_deleting(p) } {
        return None;
    }
    // SAFETY: as above.
    Some(unsafe { view(p) })
}

impl Obj {
    /// Creates a base object (`lv_obj_create`).
    pub fn create(parent: &Obj) -> Result<&Obj, Error> {
        // SAFETY: shared access through the parent view; creation frees
        // nothing.
        let p = unsafe { ffi::obj_create(parent.as_ptr()) };
        if p.is_null() {
            return Err(Error::OutOfMemory);
        }
        // SAFETY: just created.
        Ok(unsafe { view(p) })
    }

    /// Creates a base object and takes ownership of it.
    #[cfg(not(lvgl_rust_views_only))]
    pub fn create_owned(parent: &Obj) -> Result<Owned<Obj>, Error> {
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
    /// `p` must be a live object that is not being deleted.
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
