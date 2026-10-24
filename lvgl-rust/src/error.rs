//! The crate's error type.

/// Why an operation failed. 1 byte.
#[repr(u8)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Error {
    /// The object (or resource) was deleted.
    Dead = 1,
    /// No registry slot is left (`LVGL_RUST_HANDLE_SLOTS`).
    RegistryFull,
    /// LVGL could not allocate memory.
    OutOfMemory,
    /// An argument is out of range or would make LVGL's state inconsistent
    /// (a parent cycle, loading a screen that has a parent, ...).
    InvalidArg,
    /// LVGL still uses the object (a screen being loaded); retried later.
    Busy,
    /// LVGL manages the object itself (a display layer, a widget's internal
    /// part): the operation is refused.
    Internal,
    /// LVGL is already initialized, or a session or a callback is active.
    Reentrant,
    /// LVGL is not initialized, or was deinitialized.
    NotInitialized,
    /// The object already has an owner.
    AlreadyOwned,
    /// A buffer is too small.
    TooSmall,
}
