//! Input devices: a Rust reader in an application static (or on LVGL's
//! heap, with `Indev::create_boxed`). Readers get no token: they only
//! report the device's state.

#[cfg(feature = "heap")]
use crate::heap::LvBox;
use crate::{Error, Lvgl, layout::trailing, sys};
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
    /// Hardware buttons, each pressing a point of the screen: a button id
    /// and a pressed state. Made by [`Indev::create_button`], which takes
    /// the points.
    Button,
}

impl IndevKind {
    fn raw(self) -> sys::lv_indev_type_t {
        match self {
            IndevKind::Pointer => sys::lv_indev_type_t_LV_INDEV_TYPE_POINTER,
            IndevKind::Keypad => sys::lv_indev_type_t_LV_INDEV_TYPE_KEYPAD,
            IndevKind::Encoder => sys::lv_indev_type_t_LV_INDEV_TYPE_ENCODER,
            IndevKind::Button => sys::lv_indev_type_t_LV_INDEV_TYPE_BUTTON,
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

    /// The button that is pressed or released (button devices): an index
    /// into the points given to [`Indev::create_button`]. An id past them
    /// is read as button 0 released.
    pub fn set_button(&mut self, id: u32) {
        self.0.btn_id = id;
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
#[repr(C)]
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

#[cfg(test)]
impl<R> IndevCell<R> {
    pub(crate) fn reader_ptr(&self) -> *mut R {
        self.reader.get().cast()
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
    // SAFETY: LVGL calls this for a device made by `Indev::create` or
    // `create_boxed`, whose driver data is a claimed flag followed by an
    // `R` (a cell or a block), under the token, with its data to fill.
    unsafe {
        let base = sys::lv_indev_get_driver_data(indev)
            .cast::<Cell<bool>>()
            .cast_const();
        (*trailing::<Cell<bool>, R>(base)).read(&mut IndevData(&mut *data));
    }
}

/// The read callback of a button device with `N` points: LVGL indexes the
/// points with the id the reader reports, unchecked
/// (`lv_indev.c:1227-1228`).
unsafe extern "C" fn button_trampoline<R: Read, const N: usize>(
    indev: *mut sys::lv_indev_t,
    data: *mut sys::lv_indev_data_t,
) {
    // SAFETY: as `read_trampoline`; `data` is LVGL's to fill.
    unsafe {
        read_trampoline::<R>(indev, data);
        let data = &mut *data;
        if data.btn_id as usize >= N {
            data.btn_id = 0;
            data.state = sys::lv_indev_state_t_LV_INDEV_STATE_RELEASED;
        }
    }
}

/// Drops the reader of an [`IndevCell`] and releases the cell.
unsafe fn release_cell<R>(base: *const ()) {
    // SAFETY: called once by `delete`, for an initialised reader after
    // LVGL stopped using it; `base` points to the whole cell.
    unsafe {
        let claimed = base.cast::<Cell<bool>>();
        trailing::<Cell<bool>, R>(claimed).drop_in_place();
        (*claimed).set(false);
    }
}

/// Drops the reader of a boxed device and frees its block.
#[cfg(feature = "heap")]
unsafe fn release_boxed<R>(base: *const ()) {
    // SAFETY: called once by `delete`, after LVGL stopped using the block.
    unsafe { LvBox::<Cell<bool>, R>::free(base.cast_mut().cast()) }
}

/// An input device made by [`Indev::create`], [`Indev::create_button`] or
/// `Indev::create_boxed`. Dropping it keeps the device (a leak, counted
/// by `DebugCounters::leaked` with `debug-counters`); deletion is explicit
/// ([`Indev::delete`]).
pub struct Indev {
    indev: NonNull<sys::lv_indev_t>,
    /// The reader's cell or block.
    base: NonNull<()>,
    release: unsafe fn(*const ()),
}

// SAFETY: pointers to LVGL's device and the reader's cell or block, used
// only with the token.
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
    /// [`Error::AlreadyOwned`] if the cell is in use, and with
    /// [`Error::InvalidArg`] for [`IndevKind::Button`], which needs its
    /// points ([`Indev::create_button`]).
    pub fn create<R: Read>(
        lv: &mut Lvgl,
        cell: &'static IndevCell<R>,
        reader: R,
        kind: IndevKind,
    ) -> Result<Indev, Error> {
        if kind == IndevKind::Button {
            return Err(Error::InvalidArg);
        }
        Self::create_in_cell(lv, cell, reader, kind, read_trampoline::<R>, ptr::null())
    }

    /// Creates a button device reading through `reader`, whose state lives
    /// in `cell`, on the default display: the reader reports which button
    /// is pressed ([`IndevData::set_button`]) and LVGL presses
    /// `points[id]` of the screen. LVGL keeps the points, hence `'static`.
    /// Fails with [`Error::AlreadyOwned`] if the cell is in use.
    pub fn create_button<R: Read, const N: usize>(
        lv: &mut Lvgl,
        cell: &'static IndevCell<R>,
        reader: R,
        points: &'static [sys::lv_point_t; N],
    ) -> Result<Indev, Error> {
        const { assert!(N > 0, "a button device needs at least one point") };
        Self::create_in_cell(
            lv,
            cell,
            reader,
            IndevKind::Button,
            button_trampoline::<R, N>,
            points.as_ptr(),
        )
    }

    fn create_in_cell<R: Read>(
        lv: &mut Lvgl,
        cell: &'static IndevCell<R>,
        reader: R,
        kind: IndevKind,
        read: unsafe extern "C" fn(*mut sys::lv_indev_t, *mut sys::lv_indev_data_t),
        points: *const sys::lv_point_t,
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
        // The cell's pointer, so that the reader after the flag is in reach.
        let base = NonNull::from(cell).cast::<()>();
        // SAFETY: the cell is claimed: nothing else touches the reader,
        // which `trailing` finds at the cell's field (`repr(C)`); the device
        // was just created.
        unsafe {
            debug_assert_eq!(
                trailing::<Cell<bool>, R>(base.as_ptr().cast()),
                cell.reader.get().cast::<R>()
            );
            (*cell.reader.get()).write(reader);
            Self::install(indev, base, kind, read, points);
        }
        Ok(Indev {
            indev,
            base,
            release: release_cell::<R>,
        })
    }

    /// Like [`Indev::create`] (or [`Indev::create_button`] with `points`),
    /// with the reader in a block of LVGL's heap instead of an application
    /// `static`, freed when the device is deleted. Fails with
    /// [`Error::OutOfMemory`] if the block cannot be allocated, and with
    /// [`Error::InvalidArg`] for [`IndevKind::Button`] without points.
    #[cfg(feature = "heap")]
    pub fn create_boxed<R: Read>(
        lv: &mut Lvgl,
        reader: R,
        kind: IndevKind,
    ) -> Result<Indev, Error> {
        if kind == IndevKind::Button {
            return Err(Error::InvalidArg);
        }
        Self::create_in_block(lv, reader, kind, read_trampoline::<R>, ptr::null())
    }

    /// [`Indev::create_button`] with the reader in a block of LVGL's heap,
    /// freed when the device is deleted.
    #[cfg(feature = "heap")]
    pub fn create_button_boxed<R: Read, const N: usize>(
        lv: &mut Lvgl,
        reader: R,
        points: &'static [sys::lv_point_t; N],
    ) -> Result<Indev, Error> {
        const { assert!(N > 0, "a button device needs at least one point") };
        Self::create_in_block(
            lv,
            reader,
            IndevKind::Button,
            button_trampoline::<R, N>,
            points.as_ptr(),
        )
    }

    #[cfg(feature = "heap")]
    fn create_in_block<R: Read>(
        lv: &mut Lvgl,
        reader: R,
        kind: IndevKind,
        read: unsafe extern "C" fn(*mut sys::lv_indev_t, *mut sys::lv_indev_data_t),
        points: *const sys::lv_point_t,
    ) -> Result<Indev, Error> {
        let _ = lv;
        let Ok(block) = LvBox::new(Cell::new(true), reader) else {
            return Err(Error::OutOfMemory);
        };
        let block = block.into_raw();
        // SAFETY: exclusive access.
        let indev = unsafe { sys::lv_indev_create() };
        let (Some(indev), Some(base)) = (NonNull::new(indev), NonNull::new(block.cast::<()>()))
        else {
            // SAFETY: the block was never handed out.
            unsafe { LvBox::<Cell<bool>, R>::free(block) };
            return Err(Error::OutOfMemory);
        };
        // SAFETY: a fresh block holding the flag and an initialised reader;
        // the device was just created.
        unsafe { Self::install(indev, base, kind, read, points) };
        Ok(Indev {
            indev,
            base,
            release: release_boxed::<R>,
        })
    }

    /// Hands a new device to the reader at `base`.
    ///
    /// # Safety
    ///
    /// Exclusive access; `base` is a claimed flag followed by the reader
    /// `read` expects; `points` is NULL or a `'static` array of the length
    /// `read` checks ids against.
    unsafe fn install(
        indev: NonNull<sys::lv_indev_t>,
        base: NonNull<()>,
        kind: IndevKind,
        read: unsafe extern "C" fn(*mut sys::lv_indev_t, *mut sys::lv_indev_data_t),
        points: *const sys::lv_point_t,
    ) {
        let i = indev.as_ptr();
        // SAFETY: caller contract.
        unsafe {
            sys::lv_indev_set_type(i, kind.raw());
            if !points.is_null() {
                // After the type: LVGL keeps the points of button devices
                // only (`lv_indev.c:487-489`).
                sys::lv_indev_set_button_points(i, points);
            }
            sys::lv_indev_set_driver_data(i, base.as_ptr().cast());
            sys::lv_indev_set_read_cb(i, Some(read));
        }
    }

    /// Deletes the device, then drops the reader and releases the cell (or
    /// frees the block of a boxed device). Exclusive access at depth 0: no
    /// frame holds the device (input processing keeps it across events,
    /// `lv_indev.c:202-268`).
    pub fn delete(self, lv: &mut Lvgl) {
        let _ = lv;
        // SAFETY: exclusive access; the reader is dropped after LVGL stopped
        // using it.
        unsafe {
            sys::lv_indev_delete(self.indev.as_ptr());
            (self.release)(self.base.as_ptr());
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
