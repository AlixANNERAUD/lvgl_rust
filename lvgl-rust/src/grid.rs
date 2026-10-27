//! Grid layouts: track templates that LVGL keeps and walks up to their
//! terminator, and the layout setters that keep a grid container's
//! templates in place.
//!
//! LVGL reads a template wherever it lays out the container, and trusts it:
//!
//! - an empty template gives zero tracks, and placing a child then reads
//!   `x[-1]` (`lv_grid.c:534-536, 573`);
//! - a grid container with no template of its own is a *subgrid*: LVGL
//!   copies `span` entries of its parent's template from `pos`, both read
//!   from the container's cell style without any bound
//!   (`lv_grid.c:321-325, 423-427`).
//!
//! [`GridTemplate`] always holds at least one track and its terminator, so
//! the first case cannot happen. LVGL clamps children positions and spans
//! to the track count (`lv_grid.c:528-558`), but tests `pos + span` in an
//! `int32_t`, which overflows for spans near `i32::MAX` and leaves the span
//! unclamped (then `x[pos + span - 1]` is read far out of bounds): spans are
//! bounded by [`GridTemplate::MAX_SPAN`] and templates to
//! [`GridTemplate::MAX_TRACKS`] tracks, so the sum stays far below it.
//! The subgrid path is made unreachable: the grid layout is only set by
//! [`ObjMut::set_grid_dsc_array`], together with both templates in the
//! object's own local style (which applies in every state), and every other
//! layout setter refuses `LV_LAYOUT_GRID`. The templates cannot be removed
//! without the layout ([`ObjMut::remove_local_style_prop`] refuses them;
//! removing the whole local style removes the layout too), and the style
//! setters of the templates, which accept NULL, stay in `sys`: a NULL
//! template in a style of higher precedence would hide the local one.

use crate::{Error, ObjMut, Selector, StyleMut, Widget, enums::GridAlign, sys};

/// `LV_COORD_MAX`: grid templates encode their special tracks below it.
const COORD_MAX: i32 = sys::LV_COORD_MAX as i32;
/// `LV_GRID_TEMPLATE_LAST`.
const LAST: i32 = COORD_MAX;
/// `LV_GRID_CONTENT`.
const CONTENT: i32 = COORD_MAX - 101;
/// `LV_GRID_FR(0)`: `IS_FR(x)` is `x >= COORD_MAX - 100` (`lv_grid.c:26`).
const FR_BASE: i32 = COORD_MAX - 100;

const _: () = assert!(LAST == sys::LV_GRID_TEMPLATE_LAST as i32);
const _: () = assert!(CONTENT == sys::LV_GRID_CONTENT as i32);

/// One column or row of a [`GridTemplate`].
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Track(i32);

impl Track {
    /// As large as the largest child placed in this track alone
    /// (`LV_GRID_CONTENT`).
    pub const CONTENT: Self = Self(CONTENT);

    /// The terminator (`LV_GRID_TEMPLATE_LAST`), appended by
    /// [`grid_template!`](crate::grid_template).
    #[doc(hidden)]
    pub const END: Self = Self(LAST);

    /// A track of `size` pixels.
    ///
    /// # Panics
    ///
    /// If `size` is negative or reaches the encoded tracks
    /// (`LV_GRID_CONTENT`, about 5 × 10⁸): a compile error in a `static`.
    pub const fn px(size: i32) -> Self {
        assert!(
            size >= 0 && size < CONTENT,
            "a grid track is 0 to LV_COORD_MAX - 102 pixels"
        );
        Self(size)
    }

    /// A share of the free space (`LV_GRID_FR(weight)`).
    ///
    /// # Panics
    ///
    /// If `weight` is 0 or above 99 (`LV_GRID_FR(100)` is the terminator):
    /// a compile error in a `static`.
    pub const fn fr(weight: u8) -> Self {
        assert!(weight >= 1 && weight <= 99, "a grid fraction is 1 to 99");
        Self(FR_BASE + weight as i32)
    }

    const fn pixels(self) -> Option<i32> {
        if self.0 < CONTENT { Some(self.0) } else { None }
    }
}

/// The columns or rows of a grid (`lv_obj_set_grid_dsc_array`), which LVGL
/// keeps and walks up to their terminator: made by
/// [`grid_template!`](crate::grid_template), so it is `'static`,
/// terminated and never empty.
pub struct GridTemplate {
    tracks: &'static [Track],
}

impl GridTemplate {
    /// The most tracks a template has.
    pub const MAX_TRACKS: u32 = u16::MAX as u32;

    /// The largest cell span (larger spans mean the same: LVGL clamps
    /// them to the track count).
    pub const MAX_SPAN: i32 = u16::MAX as i32;

    /// A template of `tracks`, which must end with the terminator, have
    /// at least one track before it (at most [`MAX_TRACKS`](Self::MAX_TRACKS))
    /// and no other terminator, and add up to
    /// at most `LV_COORD_MAX` fixed pixels (LVGL sums them in an `int32_t`,
    /// `lv_grid.c:361-375`).
    ///
    /// # Panics
    ///
    /// If `tracks` is not as above (a compile error in a `static`).
    pub const fn new(tracks: &'static [Track]) -> Self {
        let Some((last, before)) = tracks.split_last() else {
            panic!("a grid template must end with its terminator");
        };
        assert!(
            last.0 == LAST,
            "a grid template must end with its terminator"
        );
        assert!(!before.is_empty(), "a grid template needs a track");
        assert!(
            before.len() <= Self::MAX_TRACKS as usize,
            "a grid template has at most 65535 tracks"
        );
        let mut pixels: i64 = 0;
        let mut i = 0;
        while i < before.len() {
            assert!(before[i].0 != LAST, "the terminator ends the template");
            if let Some(size) = before[i].pixels() {
                pixels += size as i64;
            }
            i += 1;
        }
        assert!(
            pixels <= COORD_MAX as i64,
            "the fixed tracks of a grid template exceed LV_COORD_MAX"
        );
        Self { tracks }
    }

    /// The number of tracks (the terminator excluded): at least 1.
    pub const fn track_count(&self) -> u32 {
        (self.tracks.len() - 1) as u32
    }

    /// The template.
    pub const fn as_ptr(&self) -> *const i32 {
        self.tracks.as_ptr().cast()
    }
}

/// A [`GridTemplate`] from tracks ([`Track::px`], [`Track::fr`],
/// [`Track::CONTENT`]), terminated automatically.
///
/// ```no_run
/// use lvgl_rust::{GridTemplate, Track, grid_template};
///
/// static COLUMNS: GridTemplate = grid_template![Track::px(48), Track::fr(1)];
/// static ROWS: GridTemplate = grid_template![Track::CONTENT, Track::fr(2), Track::fr(1)];
/// ```
#[macro_export]
macro_rules! grid_template {
    ($($track:expr),+ $(,)?) => {
        $crate::GridTemplate::new(&[$($track,)+ $crate::Track::END])
    };
}

const COLUMN_PROP: sys::lv_style_prop_t =
    sys::_lv_style_id_t_LV_STYLE_GRID_COLUMN_DSC_ARRAY as sys::lv_style_prop_t;
const ROW_PROP: sys::lv_style_prop_t =
    sys::_lv_style_id_t_LV_STYLE_GRID_ROW_DSC_ARRAY as sys::lv_style_prop_t;

/// Whether `prop` is a grid template (`remove_local_style_prop` refuses
/// them).
pub(crate) const fn is_template_prop(prop: sys::lv_style_prop_t) -> bool {
    prop == COLUMN_PROP || prop == ROW_PROP
}

/// Whether LVGL can clamp `span` without overflowing.
pub(crate) const fn span_ok(span: i32) -> bool {
    span <= GridTemplate::MAX_SPAN
}

impl<W: Widget> ObjMut<'_, W> {
    /// Places the object in the cell at `column`, `row` of its parent's
    /// grid, over `column_span` columns and `row_span` rows. Positions and
    /// spans beyond the grid are clamped to its last track; spans above
    /// [`GridTemplate::MAX_SPAN`] are refused ([`Error::InvalidArg`]).
    ///
    /// `lv_obj_set_grid_cell`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_grid_cell(
        &mut self,
        column_align: GridAlign,
        column: i32,
        column_span: i32,
        row_align: GridAlign,
        row: i32,
        row_span: i32,
    ) -> Result<(), Error> {
        if !span_ok(column_span) || !span_ok(row_span) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: exclusive access to a live object (the audit's); the
        // spans are bounded.
        unsafe {
            sys::lv_obj_set_grid_cell(
                self.as_raw(),
                column_align.bits() as sys::lv_grid_align_t,
                column,
                column_span,
                row_align.bits() as sys::lv_grid_align_t,
                row,
                row_span,
            )
        };
        Ok(())
    }

    /// Sets the `grid_cell_column_span` style property; values above
    /// [`GridTemplate::MAX_SPAN`] are refused ([`Error::InvalidArg`]).
    ///
    /// `lv_obj_set_style_grid_cell_column_span`.
    pub fn set_style_grid_cell_column_span(
        &mut self,
        value: i32,
        selector: impl Into<Selector>,
    ) -> Result<(), Error> {
        if !span_ok(value) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: as above.
        unsafe {
            sys::lv_obj_set_style_grid_cell_column_span(
                self.as_raw(),
                value,
                selector.into().bits(),
            )
        };
        Ok(())
    }

    /// Sets the `grid_cell_row_span` style property; values above
    /// [`GridTemplate::MAX_SPAN`] are refused ([`Error::InvalidArg`]).
    ///
    /// `lv_obj_set_style_grid_cell_row_span`.
    pub fn set_style_grid_cell_row_span(
        &mut self,
        value: i32,
        selector: impl Into<Selector>,
    ) -> Result<(), Error> {
        if !span_ok(value) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: as above.
        unsafe {
            sys::lv_obj_set_style_grid_cell_row_span(self.as_raw(), value, selector.into().bits())
        };
        Ok(())
    }

    /// Lays the children out in a grid of `columns` and `rows`, which LVGL
    /// keeps. Place children with `set_grid_cell`: positions and spans
    /// beyond the template are clamped to its last track.
    ///
    /// The templates go in the object's local style for every state, before
    /// the layout; if LVGL could not store one of them (out of memory), the
    /// layout is not set and [`Error::OutOfMemory`] is returned.
    ///
    /// `lv_obj_set_grid_dsc_array`.
    pub fn set_grid_dsc_array(
        &mut self,
        columns: &'static GridTemplate,
        rows: &'static GridTemplate,
    ) -> Result<(), Error> {
        let obj = self.as_raw();
        // SAFETY: exclusive access to a live object (the access the audit
        // gives `lv_obj_set_grid_dsc_array`, which makes these calls); the
        // templates are `'static` and terminated.
        unsafe {
            sys::lv_obj_set_style_grid_column_dsc_array(obj, columns.as_ptr(), 0);
            sys::lv_obj_set_style_grid_row_dsc_array(obj, rows.as_ptr(), 0);
            if !has_local(obj, COLUMN_PROP, columns) || !has_local(obj, ROW_PROP, rows) {
                return Err(Error::OutOfMemory);
            }
            sys::lv_obj_set_style_layout(obj, sys::lv_layout_t_LV_LAYOUT_GRID as u16, 0);
        }
        Ok(())
    }
}

impl StyleMut<'_> {
    /// Sets the `grid_cell_column_span` property; values above
    /// [`GridTemplate::MAX_SPAN`] are refused ([`Error::InvalidArg`]).
    ///
    /// `lv_style_set_grid_cell_column_span`.
    pub fn set_grid_cell_column_span(&mut self, value: i32) -> Result<(), Error> {
        if !span_ok(value) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: exclusive access to the style (as generated).
        unsafe { sys::lv_style_set_grid_cell_column_span(self.as_raw(), value) };
        Ok(())
    }

    /// Sets the `grid_cell_row_span` property; values above
    /// [`GridTemplate::MAX_SPAN`] are refused ([`Error::InvalidArg`]).
    ///
    /// `lv_style_set_grid_cell_row_span`.
    pub fn set_grid_cell_row_span(&mut self, value: i32) -> Result<(), Error> {
        if !span_ok(value) {
            return Err(Error::InvalidArg);
        }
        // SAFETY: as above.
        unsafe { sys::lv_style_set_grid_cell_row_span(self.as_raw(), value) };
        Ok(())
    }
}

/// Whether the local style of `obj` for every state holds `template` as
/// `prop`.
///
/// # Safety
///
/// `obj` is a live object, under the token.
unsafe fn has_local(
    obj: *mut sys::lv_obj_t,
    prop: sys::lv_style_prop_t,
    template: &GridTemplate,
) -> bool {
    let mut value = sys::lv_style_value_t { num: 0 };
    // SAFETY: as the caller guarantees; `value` is written on success.
    unsafe {
        sys::lv_obj_get_local_style_prop(obj, prop, &mut value, 0)
            == sys::lv_style_res_t_LV_STYLE_RES_FOUND
            && value.ptr == template.as_ptr().cast()
    }
}
