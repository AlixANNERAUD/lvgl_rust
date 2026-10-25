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
    /// LVGL keeps pointers to the object: a display layer, or a child of a
    /// widget (see [`ObjMut`](crate::ObjMut)). The operation is refused.
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

#[cfg(feature = "fmt")]
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Error::Dead => "the object was deleted",
            Error::RegistryFull => "no registry slot is left",
            Error::OutOfMemory => "LVGL could not allocate memory",
            Error::InvalidArg => "invalid argument",
            Error::Busy => "LVGL still uses it; retried later",
            Error::Internal => "LVGL keeps pointers to it: refused",
            Error::Reentrant => "LVGL is already initialized, or a session or callback is active",
            Error::NotInitialized => "LVGL is not initialized",
            Error::AlreadyOwned => "already owned",
            Error::TooSmall => "buffer too small",
        })
    }
}

#[cfg(feature = "fmt")]
impl core::error::Error for Error {}
