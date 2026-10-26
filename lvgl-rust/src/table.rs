//! Table cells. LVGL checks `row` and `col` against the table's counts
//! only under `LV_USE_CHECK_ARG` (`lv_table.c:432, 501`), which
//! configurations turn off, and multiplies the counts in 32 bits before
//! allocating the cell array (`row_cnt * col_cnt * sizeof(...)`, 231, 246,
//! 271), so huge counts allocate a wrapped size that later accesses
//! overrun. The setters grow the table to fit `row + 1` and `col + 1`
//! (115-116), which wraps at `u32::MAX`.
//!
//! These wrappers check indexes against the current counts (getters) and
//! sizes against [`TABLE_CELL_LIMIT`] (setters and counts) before LVGL
//! sees them. Strings follow the rule of borrowed text: a copy out of a
//! view, since a shared setter can free a cell, and a borrow under
//! exclusive access.

use crate::{
    Error, ObjMut, enums::TableCellCtrl, sys, text::borrow_str, text::copy_str, widgets::Table,
};
use core::ffi::{CStr, c_void};

/// Most cells (rows times columns, and rows or columns alone) a table can
/// have: the cell array is `cells * size_of::<*mut _>()` bytes, computed by
/// LVGL in 32 bits.
pub const TABLE_CELL_LIMIT: u32 = u32::MAX / core::mem::size_of::<*mut c_void>() as u32;

/// Whether a table of `rows` by `columns` stays within [`TABLE_CELL_LIMIT`].
fn fits(rows: u32, columns: u32) -> bool {
    rows <= TABLE_CELL_LIMIT
        && columns <= TABLE_CELL_LIMIT
        && rows as u64 * columns as u64 <= TABLE_CELL_LIMIT as u64
}

impl Table {
    fn counts(&self) -> (u32, u32) {
        // SAFETY: shared access, live table.
        unsafe {
            (
                sys::lv_table_get_row_count(self.as_ptr()),
                sys::lv_table_get_column_count(self.as_ptr()),
            )
        }
    }

    /// Checks that cell `(row, col)` exists.
    fn check_cell(&self, row: u32, col: u32) -> Result<(), Error> {
        let (rows, columns) = self.counts();
        if row < rows && col < columns {
            Ok(())
        } else {
            Err(Error::InvalidArg)
        }
    }

    /// Checks that the table can grow to hold cell `(row, col)`, as the
    /// cell setters do.
    fn check_grow(&self, row: u32, col: u32) -> Result<(), Error> {
        let (rows, columns) = self.counts();
        let rows = rows.max(row.checked_add(1).ok_or(Error::InvalidArg)?);
        let columns = columns.max(col.checked_add(1).ok_or(Error::InvalidArg)?);
        if fits(rows, columns) {
            Ok(())
        } else {
            Err(Error::InvalidArg)
        }
    }

    /// Sets the number of rows (cells of removed rows are freed). Refused
    /// past [`TABLE_CELL_LIMIT`] cells ([`Error::InvalidArg`]).
    ///
    /// `lv_table_set_row_count`.
    pub fn set_row_count(&self, rows: u32) -> Result<(), Error> {
        if !fits(rows, self.counts().1) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: shared access, live table; the sizes fit in 32 bits.
        unsafe { sys::lv_table_set_row_count(self.as_ptr(), rows) };
        Ok(())
    }

    /// Sets the number of columns, as [`set_row_count`](Self::set_row_count).
    ///
    /// `lv_table_set_column_count`.
    pub fn set_column_count(&self, columns: u32) -> Result<(), Error> {
        if !fits(self.counts().0, columns) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: as above.
        unsafe { sys::lv_table_set_column_count(self.as_ptr(), columns) };
        Ok(())
    }

    /// Sets the width of column `col`, adding columns up to it.
    ///
    /// `lv_table_set_column_width`.
    pub fn set_column_width(&self, col: u32, width: i32) -> Result<(), Error> {
        self.check_grow(0, col)?;
        // SAFETY: as above; `col + 1` does not wrap and the grown table
        // fits.
        unsafe { sys::lv_table_set_column_width(self.as_ptr(), col, width) };
        Ok(())
    }

    /// Sets the text of cell `(row, col)`, which LVGL copies, adding rows
    /// and columns up to it.
    ///
    /// `lv_table_set_cell_value`.
    pub fn set_cell_value(&self, row: u32, col: u32, text: &CStr) -> Result<(), Error> {
        self.check_grow(row, col)?;
        // SAFETY: as above; LVGL copies the text (`lv_table.c:130-135`).
        unsafe { sys::lv_table_set_cell_value(self.as_ptr(), row, col, text.as_ptr()) };
        Ok(())
    }

    /// Adds `ctrl` to cell `(row, col)`, adding rows and columns up to it.
    ///
    /// `lv_table_set_cell_ctrl`.
    pub fn set_cell_ctrl(&self, row: u32, col: u32, ctrl: TableCellCtrl) -> Result<(), Error> {
        self.check_grow(row, col)?;
        #[allow(clippy::unnecessary_cast)] // `lv_table_cell_ctrl_t` is `c_int` on MSVC.
        // SAFETY: as above.
        unsafe {
            sys::lv_table_set_cell_ctrl(self.as_ptr(), row, col, ctrl.0 as _)
        };
        Ok(())
    }

    /// Removes `ctrl` from cell `(row, col)`, adding rows and columns up to
    /// it.
    ///
    /// `lv_table_clear_cell_ctrl`.
    pub fn clear_cell_ctrl(&self, row: u32, col: u32, ctrl: TableCellCtrl) -> Result<(), Error> {
        self.check_grow(row, col)?;
        #[allow(clippy::unnecessary_cast)] // `lv_table_cell_ctrl_t` is `c_int` on MSVC.
        // SAFETY: as above.
        unsafe {
            sys::lv_table_clear_cell_ctrl(self.as_ptr(), row, col, ctrl.0 as _)
        };
        Ok(())
    }

    /// Copies the text of cell `(row, col)` into `buf` (an empty cell
    /// gives `""`). [`Error::InvalidArg`] outside the table,
    /// [`Error::TooSmall`] if the text does not fit with its NUL.
    ///
    /// `lv_table_get_cell_value`.
    pub fn get_cell_value_into<'b>(
        &self,
        row: u32,
        col: u32,
        buf: &'b mut [u8],
    ) -> Result<&'b CStr, Error> {
        self.check_cell(row, col)?;
        // SAFETY: shared access, live table, a cell inside it.
        let text = unsafe { sys::lv_table_get_cell_value(self.as_ptr(), row, col) };
        copy_str(text, buf)?.ok_or(Error::InvalidArg)
    }

    /// The user data of cell `(row, col)` (NULL for a cell without any).
    /// [`Error::InvalidArg`] outside the table. LVGL never dereferences it.
    ///
    /// `lv_table_get_cell_user_data`.
    pub fn get_cell_user_data(&self, row: u16, col: u16) -> Result<*mut c_void, Error> {
        self.check_cell(row.into(), col.into())?;
        // SAFETY: shared access, live table, a cell inside it.
        Ok(unsafe { sys::lv_table_get_cell_user_data(self.as_ptr(), row, col) })
    }

    /// The selected cell as `(row, col)` (`LV_TABLE_CELL_NONE` for none).
    ///
    /// `lv_table_get_selected_cell`.
    pub fn get_selected_cell(&self) -> (u32, u32) {
        let (mut row, mut col) = (0, 0);
        // SAFETY: shared access, live table; LVGL writes both.
        unsafe { sys::lv_table_get_selected_cell(self.as_ptr(), &mut row, &mut col) };
        (row, col)
    }
}

impl ObjMut<'_, Table> {
    /// The text of cell `(row, col)`, borrowed while the exclusive access
    /// lasts (an empty cell gives `""`). [`Error::InvalidArg`] outside the
    /// table.
    ///
    /// `lv_table_get_cell_value`.
    pub fn get_cell_value(&mut self, row: u32, col: u32) -> Result<&CStr, Error> {
        self.view().check_cell(row, col)?;
        // SAFETY: exclusive access, live table, a cell inside it.
        borrow_str(unsafe { sys::lv_table_get_cell_value(self.as_raw(), row, col) })
            .ok_or(Error::InvalidArg)
    }
}
