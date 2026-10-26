//! Calendars. The arrow header writes `month_names[month - 1]` for the
//! shown month without checking it (`lv_calendar_header_arrow.c:148, 164`;
//! the `LV_ASSERT_FORMAT_MSG` before it compiles to nothing without
//! `LV_USE_ASSERT`), so the shown month must stay in `1..=12`.
//!
//! The dropdown header stays raw (`audit.toml`), and the part tables keep
//! the button matrix and the headers' children in place (`classes.toml`).

use crate::{Error, ObjMut, sys, widgets::Calendar};

impl ObjMut<'_, Calendar> {
    /// Shows `month` (1 to 12, [`Error::InvalidArg`] otherwise) of `year`.
    ///
    /// `lv_calendar_set_month_shown`.
    pub fn set_month_shown(&mut self, year: u32, month: u32) -> Result<(), Error> {
        if !(1..=12).contains(&month) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: exclusive access (the headers' handlers run), live
        // calendar, a valid month.
        unsafe { sys::lv_calendar_set_month_shown(self.as_raw(), year, month) };
        Ok(())
    }

    /// Shows `month` (1 to 12, [`Error::InvalidArg`] otherwise) of the
    /// shown year.
    ///
    /// `lv_calendar_set_shown_month`.
    pub fn set_shown_month(&mut self, month: u32) -> Result<(), Error> {
        if !(1..=12).contains(&month) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: as above.
        unsafe { sys::lv_calendar_set_shown_month(self.as_raw(), month) };
        Ok(())
    }
}
