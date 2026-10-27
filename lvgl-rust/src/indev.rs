//! Input devices: a Rust reader in an application static. Readers get no
//! token: they only report the device's state.

use crate::{Error, Lvgl, sys};
use core::{
    cell::{Cell, UnsafeCell},
    mem::MaybeUninit,
    ptr::{self, NonNull},
};

/// An input driver: reports the device's state when LVGL polls it.
pub trait Read: Send + 'static {
    /// Fills `data` with the current state.
    fn read(&mut self, data: &mut IndevData<'_>);
}

/// What kind of input device a reader is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum IndevKind {
    /// Touchpad or mouse: a point and pressed state.
    Pointer,
    /// Keyboard or keypad: a key and pressed state, sent to the focused
    /// object of the device's group.
    Keypad,
    /// Rotary encoder: steps and a pressed state.
    Encoder,
}

impl IndevKind {
    fn raw(self) -> sys::lv_indev_type_t {
        match self {
            IndevKind::Pointer => sys::lv_indev_type_t_LV_INDEV_TYPE_POINTER,
            IndevKind::Keypad => sys::lv_indev_type_t_LV_INDEV_TYPE_KEYPAD,
            IndevKind::Encoder => sys::lv_indev_type_t_LV_INDEV_TYPE_ENCODER,
        }
    }
}

/// The state a reader reports (`lv_indev_data_t`).
pub struct IndevData<'a>(&'a mut sys::lv_indev_data_t);

impl IndevData<'_> {
    /// Pressed or released.
    pub fn set_pressed(&mut self, pressed: bool) {
        self.0.state = if pressed {
            sys::lv_indev_state_t_LV_INDEV_STATE_PRESSED
        } else {
            sys::lv_indev_state_t_LV_INDEV_STATE_RELEASED
        };
    }

    /// The pointer's position (pointer devices).
    pub fn set_point(&mut self, x: i32, y: i32) {
        self.0.point = sys::lv_point_t { x, y };
    }

    /// The key (keypads): a character or an `LV_KEY_*` code.
    pub fn set_key(&mut self, key: u32) {
        self.0.key = key;
    }

    /// Steps since the last read (encoders).
    pub fn set_steps(&mut self, steps: i16) {
        self.0.enc_diff = steps;
    }

    /// Asks LVGL to read again right away (buffered input).
    pub fn set_continue_reading(&mut self, more: bool) {
        self.0.continue_reading = more;
    }
}

/// A reader: an application `static`, claimed by [`Indev::create`] until
/// the device is deleted.
pub struct IndevCell<R> {
    claimed: Cell<bool>,
    reader: UnsafeCell<MaybeUninit<R>>,
}

// SAFETY: the reader is used only by the read callback (under the token)
// and by `create` / `try_delete` (exclusive access).
unsafe impl<R: Send> Sync for IndevCell<R> {}

impl<R> IndevCell<R> {
    /// An unclaimed cell.
    pub const fn new() -> Self {
        Self {
            claimed: Cell::new(false),
            reader: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

impl<R> Default for IndevCell<R> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe extern "C" fn read_trampoline<R: Read>(
    indev: *mut sys::lv_indev_t,
    data: *mut sys::lv_indev_data_t,
) {
    // SAFETY: LVGL calls this for a device made by `Indev::create`, whose
    // driver data is this cell, under the token, with its data to fill.
    unsafe {
        let cell = &*sys::lv_indev_get_driver_data(indev).cast::<IndevCell<R>>();
        (*cell.reader.get())
            .assume_init_mut()
            .read(&mut IndevData(&mut *data));
    }
}

unsafe fn drop_reader<R>(cell: *const ()) {
    // SAFETY: called once by `try_delete`, for an initialised reader.
    unsafe {
        let cell = &*cell.cast::<IndevCell<R>>();
        (*cell.reader.get()).assume_init_drop();
        cell.claimed.set(false);
    }
}

/// An input device made by [`Indev::create`]. Dropping it keeps the
/// device (a leak, counted by `DebugCounters::leaked` with
/// `debug-counters`); deletion is explicit ([`Indev::delete`]).
pub struct Indev {
    indev: NonNull<sys::lv_indev_t>,
    cell: NonNull<()>,
    drop_reader: unsafe fn(*const ()),
}

// SAFETY: pointers to LVGL's device and a `'static` cell, used only with
// the token.
unsafe impl Send for Indev {}

/// Counts the leak (feature `debug-counters`); the device stays.
#[cfg(feature = "debug-counters")]
impl Drop for Indev {
    fn drop(&mut self) {
        crate::counters::count(crate::counters::Counter::Leaked);
    }
}

impl Indev {
    /// Creates an input device of `kind` reading through `reader`, whose
    /// state lives in `cell`, on the default display. Fails with
    /// [`Error::AlreadyOwned`] if the cell is in use.
    pub fn create<R: Read>(
        lv: &mut Lvgl,
        cell: &'static IndevCell<R>,
        reader: R,
        kind: IndevKind,
    ) -> Result<Indev, Error> {
        let _ = lv;
        if cell.claimed.get() {
            return Err(Error::AlreadyOwned);
        }
        // SAFETY: exclusive access.
        let indev = unsafe { sys::lv_indev_create() };
        let Some(indev) = NonNull::new(indev) else {
            return Err(Error::OutOfMemory);
        };
        cell.claimed.set(true);
        // SAFETY: the cell is claimed; the device was just created.
        unsafe {
            (*cell.reader.get()).write(reader);
            let i = indev.as_ptr();
            sys::lv_indev_set_type(i, kind.raw());
            sys::lv_indev_set_driver_data(i, ptr::from_ref(cell).cast_mut().cast());
            sys::lv_indev_set_read_cb(i, Some(read_trampoline::<R>));
        }
        Ok(Indev {
            indev,
            cell: NonNull::from(cell).cast(),
            drop_reader: drop_reader::<R>,
        })
    }

    /// Deletes the device, then drops the reader and releases the cell.
    /// Exclusive access at depth 0: no frame holds the device (input
    /// processing keeps it across events, `lv_indev.c:202-268`).
    pub fn delete(self, lv: &mut Lvgl) {
        let _ = lv;
        // SAFETY: exclusive access; the reader is dropped after LVGL stopped
        // using it.
        unsafe {
            sys::lv_indev_delete(self.indev.as_ptr());
            (self.drop_reader)(self.cell.as_ptr());
        }
        // Deleted: not a leak.
        #[cfg(feature = "debug-counters")]
        core::mem::forget(self);
    }

    /// The LVGL input device, for raw calls.
    pub fn as_ptr(&self) -> *mut sys::lv_indev_t {
        self.indev.as_ptr()
    }
}
