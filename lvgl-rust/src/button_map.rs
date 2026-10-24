//! Button matrix maps: arrays of button texts that LVGL keeps and walks up
//! to their terminator.

use crate::sys;
use core::ffi::{CStr, c_char};

/// One entry of a [`ButtonMap`]: a button text, or [`MapEntry::NEWLINE`].
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct MapEntry(*const c_char);

// SAFETY: a `'static` string, never written.
unsafe impl Sync for MapEntry {}

impl MapEntry {
    /// Starts a new row.
    pub const NEWLINE: Self = Self(c"\n".as_ptr());
    /// The terminator (an empty string, `lv_buttonmatrix.c:836`).
    pub const END: Self = Self(c"".as_ptr());

    /// A button with this text. Empty texts end the map in LVGL, so
    /// [`ButtonMap::new`] refuses them before the end.
    pub const fn new(text: &'static CStr) -> Self {
        Self(text.as_ptr())
    }

    const fn text(self) -> &'static [u8] {
        // SAFETY: built from a `&'static CStr`.
        unsafe { CStr::from_ptr(self.0) }.to_bytes()
    }
}

/// The buttons of a button matrix (`lv_buttonmatrix_set_map`), which keeps
/// the pointer: made by [`button_map!`](crate::button_map), so it is
/// `'static` and terminated.
pub struct ButtonMap {
    entries: &'static [MapEntry],
    buttons: u32,
}

impl ButtonMap {
    /// A map of `entries`, which must end with [`MapEntry::END`] and contain
    /// no other empty text (LVGL stops at the first one).
    ///
    /// # Panics
    ///
    /// If `entries` is not terminated as above (a compile error in a
    /// `static`).
    pub const fn new(entries: &'static [MapEntry]) -> Self {
        let Some((last, entries_before)) = entries.split_last() else {
            panic!("a button map must end with MapEntry::END");
        };
        assert!(
            last.text().is_empty(),
            "a button map must end with MapEntry::END"
        );
        let mut buttons = 0;
        let mut i = 0;
        while i < entries_before.len() {
            let text = entries_before[i].text();
            assert!(!text.is_empty(), "an empty button text ends the map");
            if !matches!(text, b"\n") {
                buttons += 1;
            }
            i += 1;
        }
        Self { entries, buttons }
    }

    /// The number of buttons (line breaks excluded).
    pub const fn button_count(&self) -> u32 {
        self.buttons
    }

    /// The map.
    pub const fn as_ptr(&self) -> *const *const c_char {
        self.entries.as_ptr().cast()
    }
}

/// A [`ButtonMap`] from button texts and `"\n"` row breaks, terminated
/// automatically.
///
/// ```no_run
/// use lvgl_rust::{ButtonMap, button_map};
///
/// static KEYPAD: ButtonMap = button_map![c"1", c"2", c"3", c"\n", c"0"];
/// ```
#[macro_export]
macro_rules! button_map {
    ($($text:expr),* $(,)?) => {
        $crate::ButtonMap::new(&[
            $($crate::MapEntry::new($text),)*
            $crate::MapEntry::END,
        ])
    };
}

#[cfg(lv_use_buttonmatrix)]
mod buttonmatrix {
    use super::*;
    use crate::{Error, enums::ButtonmatrixCtrl, widgets::ButtonMatrix};

    const _: () = assert!(
        core::mem::size_of::<ButtonmatrixCtrl>()
            == core::mem::size_of::<sys::lv_buttonmatrix_ctrl_t>()
    );

    impl ButtonMatrix {
        /// Sets the control flags of every button, one entry per button of
        /// the current map (LVGL copies that many,
        /// `lv_buttonmatrix.c:160`). Fails with [`Error::InvalidArg`] if
        /// `ctrl` is shorter.
        pub fn set_ctrl_map(&self, ctrl: &[ButtonmatrixCtrl]) -> Result<(), Error> {
            if (ctrl.len() as u64) < u64::from(self.count_buttons()) {
                return Err(Error::InvalidArg);
            }
            // SAFETY: shared access (as the generated setters); the slice
            // covers every button, and LVGL copies it.
            unsafe { sys::lv_buttonmatrix_set_ctrl_map(self.as_ptr(), ctrl.as_ptr().cast()) };
            Ok(())
        }

        /// Buttons in the current map, counted as LVGL does
        /// (`lv_buttonmatrix.c:836-844`): at least as many as it has
        /// allocated.
        fn count_buttons(&self) -> u32 {
            // SAFETY: a live button matrix, under the token; its map is
            // terminated by NULL or an empty string, as LVGL requires.
            unsafe {
                let map = sys::lv_buttonmatrix_get_map(self.as_ptr());
                let mut count = 0;
                let mut i = 0;
                while !map.is_null() {
                    let entry = *map.add(i);
                    if entry.is_null() || *entry == 0 {
                        break;
                    }
                    if CStr::from_ptr(entry).to_bytes() != b"\n" {
                        count += 1;
                    }
                    i += 1;
                }
                count
            }
        }
    }
}
