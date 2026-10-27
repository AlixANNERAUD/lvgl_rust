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
///
/// Handles carry a 16-bit generation of their registry slot. A handle kept
/// after its object died is reported dead until that slot has been reused
/// 65,536 times; after that it may resolve to the slot's current object (of
/// a compatible class: a logic error, never undefined behaviour). Drop
/// handles to dead objects rather than keeping them for days.
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

#[cfg(feature = "defmt")]
impl<W> defmt::Format for Handle<W> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Handle({=u32})", self.raw.get())
    }
}

impl<W> Handle<W> {
    pub(crate) fn raw(self) -> u32 {
        self.raw.get()
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

/// What an [`Owned`] can own: every widget, and [`Timer`](crate::Timer).
pub trait Tracked: private::Sealed + 'static {}

impl<W: Widget> Tracked for W {}
impl<W: Widget> private::Sealed for W {}

pub(crate) mod private {
    pub trait Sealed {}
}

/// The single owner of an object (or a timer): dropping it deletes it.
///
/// Dropping needs no session: the deletion is recorded and carried out at the
/// next drain ([`Lvgl::enter`] or after [`Lvgl::timer_handler`]), so an
/// `Owned` can be dropped on any thread. 4 bytes.
#[repr(transparent)]
pub struct Owned<W: Tracked>(Handle<W>);

impl<W: Tracked> Owned<W> {
    /// # Safety
    ///
    /// `raw` must be the handle of a slot just marked OWNED for this value,
    /// tracking a `W`.
    pub(crate) unsafe fn from_raw(raw: u32) -> Self {
        Self(Handle {
            // Raw handles hold `index + 1` in their low half: never zero.
            raw: NonZeroU32::new(raw).unwrap_or(NonZeroU32::MIN),
            _w: PhantomData,
        })
    }

    /// A handle to what this value owns.
    pub fn handle(&self) -> Handle<W> {
        self.0
    }

    /// Deletes it now. An object that is a screen LVGL is still loading
    /// ([`Error::Busy`]) stays pending and is retried at each drain; if the
    /// checks later refuse it for good, the drain releases it as a refused
    /// drop (counted in `DebugCounters::refused`). What is already dead is
    /// just released.
    ///
    /// An object the structural checks refuse to delete for good (moved by
    /// raw code where a widget keeps pointers to it, for instance) is
    /// released as a refused drop would release it, and their error
    /// returned ([`Error::Internal`]): it stays alive, unowned, its
    /// handles report [`Error::Dead`], and it goes with its parent.
    pub fn delete(self, lv: &mut Lvgl) -> Result<(), Error> {
        let _ = lv;
        let index = index(self.0.raw.get());
        mem::forget(self);
        // The token is held: no other thread reads `req` meanwhile.
        reg().slot(index).req.store(1, Ordering::Relaxed);
        drain::act(index)
    }
}

impl<W: Widget> Owned<W> {
    /// A view of the object, or [`Error::Dead`] if it was deleted through
    /// another path (its parent deleted, for instance).
    pub fn get<'a>(&self, lv: &'a Lvgl) -> Result<&'a W, Error> {
        self.0.get(lv)
    }

    /// Exclusive access to the object.
    pub fn get_mut<'s>(&self, lv: &'s mut Lvgl) -> Result<ObjMut<'s, W>, Error> {
        self.0.get_mut(lv)
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

impl<W: Tracked> Drop for Owned<W> {
    fn drop(&mut self) {
        reg().request_drop(index(self.0.raw.get()));
    }
}

impl<W: Tracked> fmt::Debug for Owned<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Owned").field(&self.0.raw).finish()
    }
}

#[cfg(feature = "defmt")]
impl<W: Tracked> defmt::Format for Owned<W> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Owned({=u32})", self.0.raw.get())
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
