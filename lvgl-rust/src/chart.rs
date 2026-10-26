//! Chart series: LVGL hands out `lv_chart_series_t *` pointers into the
//! chart's own list and frees them with the chart (`lv_chart.c:889-907`)
//! or with `lv_chart_remove_series` (529-540), without checking that a
//! series passed back belongs to the chart.
//!
//! A [`ChartSeries`] is a token for one series: it is not `Clone`,
//! [`Chart::remove_series`] consumes it, and every call first checks that
//! the series is still in the list of the chart it is used with, so a
//! series of a deleted chart, of another chart or already removed is
//! refused ([`Error::InvalidArg`]) instead of dereferenced. The token is
//! one pointer; LVGL's own list node is the only allocation.
//!
//! The audit finds no deletion and no hold of a series across event sends
//! in these functions (`access.toml`: all shared), so they take shared
//! access: draw events, during which the chart walks its series, are not
//! delivered to shared handlers.
//!
//! Left raw: external point arrays (`lv_chart_set_series_ext_*_array`: LVGL
//! writes them and `lv_chart_set_type` frees an external X array,
//! `lv_chart.c:125-131`), cursors (a cursor keeps a series pointer that
//! `lv_chart_remove_series` does not clear, 657-667) and
//! `lv_chart_get_series_next` (series without a token).
//!
//! LVGL allocates X arrays without initializing them, in
//! `lv_chart_add_series` on a scatter chart and in `lv_chart_set_type` when
//! a chart becomes one (`lv_chart.c:133-139, 476-482`; only Y points and
//! points added by `lv_chart_set_point_count` are filled). Both wrappers
//! fill them with `LV_CHART_POINT_NONE`, so the X points copied out are
//! always written values.
//!
//! Like the rest of LVGL, the point arrays assume allocations succeed: a
//! failed reallocation in `lv_chart_set_point_count` leaves a series with
//! fewer (or no) points than the chart's count (`lv_chart.c:1909-1937`).
//! `LV_USE_ASSERT_MALLOC`, on in both vendored configurations, stops in the
//! assert handler before that state is reached.

use crate::{Error, sys, widgets::Chart};
use core::ptr::NonNull;

/// One series of a chart, from [`Chart::add_series`]. Usable only with
/// that chart, until [`Chart::remove_series`] or the chart's deletion.
#[derive(Debug, PartialEq, Eq)]
pub struct ChartSeries(NonNull<sys::lv_chart_series_t>);

// SAFETY: the pointer is only compared with the series of a chart, under
// the token, before LVGL dereferences it.
unsafe impl Send for ChartSeries {}
// SAFETY: as above; shared references give no access.
unsafe impl Sync for ChartSeries {}

impl Chart {
    /// The series pointer, if `series` is still one of this chart's.
    fn series_ptr(&self, series: &ChartSeries) -> Result<*mut sys::lv_chart_series_t, Error> {
        let wanted = series.0.as_ptr();
        // SAFETY: shared access, live chart; the list is LVGL's own and
        // only compared against.
        let mut s = unsafe { sys::lv_chart_get_series_next(self.as_ptr(), core::ptr::null()) };
        while !s.is_null() {
            if s == wanted {
                return Ok(s);
            }
            // SAFETY: `s` is a series of this chart.
            s = unsafe { sys::lv_chart_get_series_next(self.as_ptr(), s) };
        }
        Err(Error::InvalidArg)
    }

    /// Sets the number of points of every series (0 counts as 1), keeping
    /// their newest points. Refused ([`Error::InvalidArg`]) when the point
    /// arrays would not fit in the address space: LVGL computes their size
    /// as `sizeof(int32_t) * cnt` in `size_t`, which wraps on 32-bit
    /// targets, then fills `cnt` points (`lv_chart.c:1910-1942`).
    ///
    /// `lv_chart_set_point_count`.
    pub fn set_point_count(&self, count: u32) -> Result<(), Error> {
        let bytes = usize::try_from(count).map(|c| c.checked_mul(core::mem::size_of::<i32>()));
        if !matches!(bytes, Ok(Some(_))) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: shared access, live chart; the array sizes do not wrap.
        unsafe { sys::lv_chart_set_point_count(self.as_ptr(), count) };
        Ok(())
    }

    /// Adds a series drawn in `color` against `axis` (its points start as
    /// `LV_CHART_POINT_NONE`).
    ///
    /// `lv_chart_add_series`.
    pub fn add_series(
        &self,
        color: sys::lv_color_t,
        axis: crate::enums::ChartAxis,
    ) -> Result<ChartSeries, Error> {
        #[allow(clippy::unnecessary_cast)] // `lv_chart_axis_t` is `c_int` on MSVC.
        // SAFETY: shared access, live chart.
        let s = unsafe { sys::lv_chart_add_series(self.as_ptr(), color, axis.0 as _) };
        let s = NonNull::new(s).ok_or(Error::OutOfMemory)?;
        // SAFETY: a series of this chart, just added.
        unsafe { self.fill_x_points(s.as_ptr()) };
        Ok(ChartSeries(s))
    }

    /// Sets how the chart draws its series. A chart becoming a scatter
    /// chart gets X points for every series, `LV_CHART_POINT_NONE` until
    /// written; one leaving it frees them.
    ///
    /// `lv_chart_set_type`.
    pub fn set_type(&self, chart_type: crate::enums::ChartType) {
        let was_scatter = self.is_scatter();
        #[allow(clippy::unnecessary_cast)] // `lv_chart_type_t` is `c_int` on MSVC.
        // SAFETY: shared access, live chart (the audit finds no deletion
        // and no hold across event sends).
        unsafe {
            sys::lv_chart_set_type(self.as_ptr(), chart_type.0 as _)
        };
        if was_scatter || !self.is_scatter() {
            return;
        }
        // SAFETY: shared access, live chart; each `s` is one of its series.
        unsafe {
            let mut s = sys::lv_chart_get_series_next(self.as_ptr(), core::ptr::null());
            while !s.is_null() {
                self.fill_x_points(s);
                s = sys::lv_chart_get_series_next(self.as_ptr(), s);
            }
        }
    }

    fn is_scatter(&self) -> bool {
        // SAFETY: shared access, live chart.
        let chart_type = unsafe { sys::lv_chart_get_type(self.as_ptr()) };
        #[allow(clippy::unnecessary_cast)] // the bindgen alias may already match
        let scatter = chart_type as u32 == sys::lv_chart_type_t_LV_CHART_TYPE_SCATTER as u32;
        scatter
    }

    /// Fills the X points of `s`, if it has any, with
    /// `LV_CHART_POINT_NONE`.
    ///
    /// # Safety
    ///
    /// `s` is a series of this chart whose X array, if any, LVGL allocated
    /// for the point count (not an external array).
    unsafe fn fill_x_points(&self, s: *mut sys::lv_chart_series_t) {
        // SAFETY: caller contract.
        unsafe {
            let points = sys::lv_chart_get_series_x_array(self.as_ptr(), s);
            if points.is_null() {
                return;
            }
            let count = sys::lv_chart_get_point_count(self.as_ptr()) as usize;
            for i in 0..count {
                points.add(i).write(sys::LV_CHART_POINT_NONE as i32);
            }
        }
    }

    /// Removes `series` and frees its points.
    ///
    /// `lv_chart_remove_series`.
    pub fn remove_series(&self, series: ChartSeries) -> Result<(), Error> {
        let s = self.series_ptr(&series)?;
        // SAFETY: a series of this chart; the token is consumed, and a
        // cursor pointing at it cannot exist (cursors are raw).
        unsafe { sys::lv_chart_remove_series(self.as_ptr(), s) };
        Ok(())
    }

    /// Hides or shows `series`.
    ///
    /// `lv_chart_hide_series`.
    pub fn hide_series(&self, series: &ChartSeries, hide: bool) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart.
        unsafe { sys::lv_chart_hide_series(self.as_ptr(), s, hide) };
        Ok(())
    }

    /// Sets the color of `series`.
    ///
    /// `lv_chart_set_series_color`.
    pub fn set_series_color(
        &self,
        series: &ChartSeries,
        color: sys::lv_color_t,
    ) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart.
        unsafe { sys::lv_chart_set_series_color(self.as_ptr(), s, color) };
        Ok(())
    }

    /// The color of `series`.
    ///
    /// `lv_chart_get_series_color`.
    pub fn get_series_color(&self, series: &ChartSeries) -> Result<sys::lv_color_t, Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart.
        Ok(unsafe { sys::lv_chart_get_series_color(self.as_ptr(), s) })
    }

    /// Sets the index of the point drawn first (ignored past the point
    /// count, `lv_chart.c:581`).
    ///
    /// `lv_chart_set_x_start_point`.
    pub fn set_x_start_point(&self, series: &ChartSeries, id: u32) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart; LVGL checks `id`.
        unsafe { sys::lv_chart_set_x_start_point(self.as_ptr(), s, id) };
        Ok(())
    }

    /// The index of the point drawn first.
    ///
    /// `lv_chart_get_x_start_point`.
    pub fn get_x_start_point(&self, series: &ChartSeries) -> Result<u32, Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart.
        Ok(unsafe { sys::lv_chart_get_x_start_point(self.as_ptr(), s) })
    }

    /// The position of point `id` of `series`, relative to the chart (0, 0
    /// past the point count, `lv_chart.c:340-345`).
    ///
    /// `lv_chart_get_point_pos_by_id`.
    pub fn get_point_pos_by_id(
        &self,
        series: &ChartSeries,
        id: u32,
    ) -> Result<sys::lv_point_t, Error> {
        let s = self.series_ptr(series)?;
        let mut point = sys::lv_point_t { x: 0, y: 0 };
        // SAFETY: a series of this chart; LVGL checks `id` and writes the
        // point.
        unsafe { sys::lv_chart_get_point_pos_by_id(self.as_ptr(), s, id, &mut point) };
        Ok(point)
    }

    /// Sets every point of `series` to `value`.
    ///
    /// `lv_chart_set_all_values`.
    pub fn set_all_values(&self, series: &ChartSeries, value: i32) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart, whose points LVGL allocated for
        // the point count (external arrays are raw).
        unsafe { sys::lv_chart_set_all_values(self.as_ptr(), s, value) };
        Ok(())
    }

    /// Writes `value` at the next point of `series` and moves on (a ring
    /// of the point count).
    ///
    /// `lv_chart_set_next_value`.
    pub fn set_next_value(&self, series: &ChartSeries, value: i32) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: as for `set_all_values`; the next index stays below the
        // point count (`lv_chart.c:709-711`, and `set_point_count` resets
        // it).
        unsafe { sys::lv_chart_set_next_value(self.as_ptr(), s, value) };
        Ok(())
    }

    /// Writes `(x, y)` at the next point of `series` (scatter charts only:
    /// LVGL ignores the call otherwise, `lv_chart.c:722-725`).
    ///
    /// `lv_chart_set_next_value2`.
    pub fn set_next_value2(&self, series: &ChartSeries, x: i32, y: i32) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: as above; scatter series have X points (allocated by
        // `lv_chart_add_series` or `lv_chart_set_type`).
        unsafe { sys::lv_chart_set_next_value2(self.as_ptr(), s, x, y) };
        Ok(())
    }

    /// Appends `values` to `series`, as [`set_next_value`](Self::set_next_value)
    /// does for each.
    ///
    /// `lv_chart_set_series_values`.
    pub fn set_series_values(&self, series: &ChartSeries, values: &[i32]) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: as above; LVGL reads `values.len()` values.
        unsafe { sys::lv_chart_set_series_values(self.as_ptr(), s, values.as_ptr(), values.len()) };
        Ok(())
    }

    /// Appends `(x, y)` pairs to `series` (scatter charts only). The slices
    /// must have the same length ([`Error::InvalidArg`]).
    ///
    /// `lv_chart_set_series_values2`.
    pub fn set_series_values2(
        &self,
        series: &ChartSeries,
        x: &[i32],
        y: &[i32],
    ) -> Result<(), Error> {
        if x.len() != y.len() {
            return Err(Error::InvalidArg);
        }
        let s = self.series_ptr(series)?;
        // SAFETY: as above; LVGL reads `x.len()` values of each slice.
        unsafe {
            sys::lv_chart_set_series_values2(self.as_ptr(), s, x.as_ptr(), y.as_ptr(), x.len())
        };
        Ok(())
    }

    /// Sets point `id` of `series` (ignored past the point count,
    /// `lv_chart.c:765`).
    ///
    /// `lv_chart_set_series_value_by_id`.
    pub fn set_series_value_by_id(
        &self,
        series: &ChartSeries,
        id: u32,
        value: i32,
    ) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart; LVGL checks `id`.
        unsafe { sys::lv_chart_set_series_value_by_id(self.as_ptr(), s, id, value) };
        Ok(())
    }

    /// Sets point `id` of `series` to `(x, y)` (scatter charts only;
    /// ignored past the point count, `lv_chart.c:779-785`).
    ///
    /// `lv_chart_set_series_value_by_id2`.
    pub fn set_series_value_by_id2(
        &self,
        series: &ChartSeries,
        id: u32,
        x: i32,
        y: i32,
    ) -> Result<(), Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: as above.
        unsafe { sys::lv_chart_set_series_value_by_id2(self.as_ptr(), s, id, x, y) };
        Ok(())
    }

    /// Copies the Y points of `series` into `buf` (in storage order: the
    /// first drawn is at [`get_x_start_point`](Self::get_x_start_point))
    /// and returns how many were copied: the point count, at most
    /// `buf.len()`. A copy, since a shared call can reallocate them.
    ///
    /// `lv_chart_get_series_y_array`.
    pub fn get_series_y_values_into(
        &self,
        series: &ChartSeries,
        buf: &mut [i32],
    ) -> Result<usize, Error> {
        let s = self.series_ptr(series)?;
        // SAFETY: a series of this chart.
        let points = unsafe { sys::lv_chart_get_series_y_array(self.as_ptr(), s) };
        Ok(self.copy_points(points, buf))
    }

    /// Copies the X points of `series` (scatter charts; 0 points
    /// otherwise), as [`get_series_y_values_into`](Self::get_series_y_values_into).
    ///
    /// `lv_chart_get_series_x_array`.
    pub fn get_series_x_values_into(
        &self,
        series: &ChartSeries,
        buf: &mut [i32],
    ) -> Result<usize, Error> {
        let s = self.series_ptr(series)?;
        // X arrays follow the point count only in scatter charts: one left
        // by a failed `lv_chart_set_type` would be shorter
        // (`lv_chart.c:133-139, 158-160`).
        if !self.is_scatter() {
            return Ok(0);
        }
        // SAFETY: a series of this chart.
        let points = unsafe { sys::lv_chart_get_series_x_array(self.as_ptr(), s) };
        Ok(self.copy_points(points, buf))
    }

    fn copy_points(&self, points: *const i32, buf: &mut [i32]) -> usize {
        if points.is_null() {
            return 0;
        }
        // SAFETY: shared access, live chart.
        let count = unsafe { sys::lv_chart_get_point_count(self.as_ptr()) } as usize;
        let count = count.min(buf.len());
        // SAFETY: LVGL allocates `point_cnt` points per array
        // (`lv_chart.c:482-486`, kept in step by `lv_chart_set_point_count`
        // and `lv_chart_set_type`), all written (Y by LVGL, X by
        // `fill_x_points`); nothing runs during the copy.
        buf[..count].copy_from_slice(unsafe { core::slice::from_raw_parts(points, count) });
        count
    }
}
