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

#[cfg(lv_use_buttonmatrix)]
impl crate::enums::ButtonmatrixCtrl {
    /// Both flags (`self | other`), usable in constants: a width and
    /// [`POPOVER`](Self::POPOVER), say.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A keyboard layout (`lv_keyboard_set_map`): buttons and one control
/// entry per button, which LVGL keeps together and reads as many control
/// entries as the map has buttons (`lv_keyboard.c:498-504`). Both are
/// `'static`, and the lengths are checked when it is made.
#[cfg(lv_use_keyboard)]
pub struct KeyboardMap {
    map: ButtonMap,
    ctrl: &'static [crate::enums::ButtonmatrixCtrl],
}

#[cfg(lv_use_keyboard)]
impl KeyboardMap {
    /// A layout of `map` with the control flags `ctrl`, one per button
    /// (widths, [`POPOVER`](crate::enums::ButtonmatrixCtrl::POPOVER), ...).
    ///
    /// # Panics
    ///
    /// If `ctrl` does not have one entry per button of `map` (a compile
    /// error in a `static`).
    pub const fn new(map: ButtonMap, ctrl: &'static [crate::enums::ButtonmatrixCtrl]) -> Self {
        assert!(
            ctrl.len() == map.button_count() as usize,
            "a keyboard map needs one control entry per button"
        );
        Self { map, ctrl }
    }

    /// The buttons.
    pub const fn map(&self) -> &ButtonMap {
        &self.map
    }

    /// The control map.
    pub const fn ctrl_ptr(&self) -> *const sys::lv_buttonmatrix_ctrl_t {
        self.ctrl.as_ptr().cast()
    }
}

#[cfg(lv_use_keyboard)]
mod keyboard {
    use super::KeyboardMap;
    use crate::{enums::KeyboardMode, sys, widgets::Keyboard};

    impl Keyboard {
        /// Uses `map` for `mode`, in every keyboard: LVGL keeps one layout
        /// per mode for all of them (`lv_keyboard.c:188-215, 291-292`).
        /// This keyboard shows its current mode's layout again; the others
        /// pick up the new one when they next change mode.
        ///
        /// `lv_keyboard_set_map`.
        pub fn set_map(&self, mode: KeyboardMode, map: &'static KeyboardMap) {
            // SAFETY: shared access (the audit's), live keyboard; `mode` is
            // one of LVGL's modes (below the 10 entries of `kb_map`), the
            // map and its control entries are `'static`, terminated and of
            // matching lengths.
            unsafe {
                sys::lv_keyboard_set_map(
                    self.as_ptr(),
                    mode.bits() as sys::lv_keyboard_mode_t,
                    map.map().as_ptr(),
                    map.ctrl_ptr(),
                )
            };
        }

        /// Shows popovers on press (or not), for the layout of the current
        /// mode.
        ///
        /// LVGL applies the mode's control map to the buttons the keyboard
        /// shows, copying one entry per button (`lv_keyboard.c:491-516`):
        /// if the keyboard shows another map (a button matrix map, or the
        /// mode's previous layout after [`set_map`](Self::set_map) on
        /// another keyboard), that reads past the control map. So the
        /// mode's layout is shown again first (by switching modes and
        /// back), which also replaces a map set through the button matrix.
        ///
        /// `lv_keyboard_set_popovers`.
        pub fn set_popovers(&self, enabled: bool) {
            let keyboard = self.as_ptr();
            // SAFETY: shared access (the audit's, for each call), live
            // keyboard; every mode's map and control map have matching
            // lengths (LVGL's, or a `KeyboardMap`), and switching modes
            // applies both (`lv_keyboard.c:259-267, 479-485`).
            unsafe {
                let mode = sys::lv_keyboard_get_mode(keyboard);
                let other = if mode == sys::lv_keyboard_mode_t_LV_KEYBOARD_MODE_TEXT_LOWER {
                    sys::lv_keyboard_mode_t_LV_KEYBOARD_MODE_TEXT_UPPER
                } else {
                    sys::lv_keyboard_mode_t_LV_KEYBOARD_MODE_TEXT_LOWER
                };
                sys::lv_keyboard_set_mode(keyboard, other);
                sys::lv_keyboard_set_mode(keyboard, mode);
                sys::lv_keyboard_set_popovers(keyboard, enabled);
            }
        }
    }
}
