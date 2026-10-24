//! Persistent references to objects: [`Handle`] (copyable, never deletes)
//! and [`Owned`] (the single owner, deletes on drop).

use super::{
    drain,
    registry::{Kind, index, reg},
    sync::Ordering,
    view::{IsA, Widget, class_check, view},
};
use crate::{Error, Lvgl, ObjMut};
use core::{fmt, hash, marker::PhantomData, mem, num::NonZeroU32, sync::atomic::AtomicU32};

/// A reference to an object that can be stored and sent between threads.
/// 4 bytes; `Option<Handle<W>>` too.
///
/// A handle never keeps its object alive: once the object is deleted, by any
/// path, [`get`](Handle::get) returns [`Error::Dead`].
#[repr(transparent)]
pub struct Handle<W> {
    raw: NonZeroU32,
    _w: PhantomData<fn() -> W>,
}

impl<W> Clone for Handle<W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for Handle<W> {}

impl<W> PartialEq for Handle<W> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<W> Eq for Handle<W> {}

impl<W> hash::Hash for Handle<W> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state)
    }
}

impl<W> fmt::Debug for Handle<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Handle").field(&self.raw).finish()
    }
}

impl<W: Widget> Handle<W> {
    /// # Safety
    ///
    /// `raw` must come from the registry for a slot tracking a `W`.
    pub(crate) unsafe fn from_raw(raw: u32) -> Self {
        Self {
            // Raw handles hold `index + 1` in their low half: never zero.
            raw: NonZeroU32::new(raw).unwrap_or(NonZeroU32::MIN),
            _w: PhantomData,
        }
    }

    pub(crate) fn raw(self) -> u32 {
        self.raw.get()
    }

    /// A view of the object, or [`Error::Dead`] if it was deleted.
    pub fn get(self, lv: &Lvgl) -> Result<&W, Error> {
        let _ = lv;
        let p = reg().resolve(self.raw(), Kind::Obj, class_check::<W>())?;
        // SAFETY: resolved: live, not being deleted, class checked.
        Ok(unsafe { view(p.as_ptr().cast()) })
    }

    /// Exclusive access to the object, for structural operations.
    pub fn get_mut(self, lv: &mut Lvgl) -> Result<ObjMut<'_, W>, Error> {
        let _ = lv;
        let p = reg().resolve(self.raw(), Kind::Obj, class_check::<W>())?;
        // SAFETY: as above, under exclusive access.
        Ok(unsafe { ObjMut::new(p.cast()) })
    }

    /// The same handle as a base class.
    pub fn upcast<B: Widget>(self) -> Handle<B>
    where
        W: IsA<B>,
    {
        Handle {
            raw: self.raw,
            _w: PhantomData,
        }
    }

    /// The same handle as a subclass, if the object's class is `D` or a
    /// subclass of it.
    pub fn downcast<D: Widget + IsA<W>>(self, lv: &Lvgl) -> Option<Handle<D>> {
        let _ = lv;
        reg()
            .resolve(self.raw(), Kind::Obj, class_check::<D>())
            .ok()
            .map(|_| Handle {
                raw: self.raw,
                _w: PhantomData,
            })
    }
}

/// The single owner of an object: dropping it deletes the object.
///
/// Dropping needs no session: the deletion is recorded and carried out at the
/// next drain ([`Lvgl::enter`] or after [`Lvgl::timer_handler`]), so an
/// `Owned` can be dropped on any thread. 4 bytes.
#[repr(transparent)]
pub struct Owned<W: Widget>(Handle<W>);

impl<W: Widget> Owned<W> {
    /// # Safety
    ///
    /// `raw` must be the handle of a slot just marked OWNED for this value.
    pub(crate) unsafe fn from_raw(raw: u32) -> Self {
        // SAFETY: caller contract.
        Self(unsafe { Handle::from_raw(raw) })
    }

    /// A handle to the owned object.
    pub fn handle(&self) -> Handle<W> {
        self.0
    }

    /// A view of the object, or [`Error::Dead`] if it was deleted through
    /// another path (its parent deleted, for instance).
    pub fn get<'a>(&self, lv: &'a Lvgl) -> Result<&'a W, Error> {
        self.0.get(lv)
    }

    /// Exclusive access to the object.
    pub fn get_mut<'s>(&self, lv: &'s mut Lvgl) -> Result<ObjMut<'s, W>, Error> {
        self.0.get_mut(lv)
    }

    /// Deletes the object now. If it is a screen LVGL is still loading
    /// ([`Error::Busy`]), the deletion stays pending and is retried at each
    /// drain. An object that is already dead is just released.
    pub fn delete(self, lv: &mut Lvgl) -> Result<(), Error> {
        let _ = lv;
        let index = index(self.0.raw());
        mem::forget(self);
        // The token is held: no other thread reads `req` meanwhile.
        reg().slot(index).req.store(1, Ordering::Relaxed);
        drain::act(index)
    }

    /// The same owner, typed as a base class.
    pub fn upcast<B: Widget>(self) -> Owned<B>
    where
        W: IsA<B>,
    {
        let handle = self.0.upcast();
        mem::forget(self);
        Owned(handle)
    }
}

impl<W: Widget> Drop for Owned<W> {
    fn drop(&mut self) {
        reg().request_drop(index(self.0.raw()));
    }
}

impl<W: Widget> fmt::Debug for Owned<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Owned").field(&self.0.raw).finish()
    }
}

/// An optional handle in a `static`: 4 bytes, loads and stores only.
pub struct HandleCell<W> {
    raw: AtomicU32,
    _w: PhantomData<fn() -> W>,
}

impl<W: Widget> HandleCell<W> {
    pub const fn new() -> Self {
        Self {
            raw: AtomicU32::new(0),
            _w: PhantomData,
        }
    }

    pub fn get(&self) -> Option<Handle<W>> {
        NonZeroU32::new(self.raw.load(core::sync::atomic::Ordering::Acquire)).map(|raw| Handle {
            raw,
            _w: PhantomData,
        })
    }

    pub fn set(&self, handle: Option<Handle<W>>) {
        let raw = handle.map_or(0, |h| h.raw.get());
        self.raw.store(raw, core::sync::atomic::Ordering::Release);
    }
}

impl<W: Widget> Default for HandleCell<W> {
    fn default() -> Self {
        Self::new()
    }
}
