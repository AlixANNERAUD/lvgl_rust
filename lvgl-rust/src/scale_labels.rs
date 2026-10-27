//! Custom labels of a scale's major ticks: an array of texts that LVGL
//! keeps and walks up to its NULL terminator.
//!
//! LVGL counts the labels once (`lv_scale.c:396-402`) and reads label
//! `n - 1` for the `n`-th major tick only while `n` is at most that count
//! (`lv_scale.c:1988-1997`); missing labels are drawn as none. The tick
//! index is a `uint16_t` there (`lv_scale.c:997, 1850, 1980`), but it cannot
//! wrap to 0 (which would read `txt_src[-1]`): the tick counts are 15-bit
//! fields (`lv_scale_private.h:74-75`), so a scale has at most 32767 major
//! ticks, whatever value the setters are given (they truncate). The
//! needle's division by `max - min` is refused by the range setters
//! (`manual.rs`).

use core::ffi::{CStr, c_char};

/// One entry of [`ScaleLabels`]: a label text, or the terminator.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ScaleLabel(*const c_char);

// SAFETY: a `'static` string (or NULL), never written.
unsafe impl Sync for ScaleLabel {}

impl ScaleLabel {
    /// The terminator (NULL), appended by
    /// [`scale_labels!`](crate::scale_labels).
    #[doc(hidden)]
    pub const END: Self = Self(core::ptr::null());

    /// A label with this text (empty texts are fine: an empty label).
    pub const fn new(text: &'static CStr) -> Self {
        Self(text.as_ptr())
    }
}

/// The labels of a scale's major ticks (`lv_scale_set_text_src`), which
/// LVGL keeps: made by [`scale_labels!`](crate::scale_labels), so it is
/// `'static` and NULL-terminated, with no NULL before the end.
pub struct ScaleLabels {
    labels: &'static [ScaleLabel],
}

impl ScaleLabels {
    /// Labels from `labels`, which must end with the terminator and
    /// contain it only there (LVGL stops at the first NULL).
    ///
    /// # Panics
    ///
    /// If `labels` is not terminated as above (a compile error in a
    /// `static`).
    pub const fn new(labels: &'static [ScaleLabel]) -> Self {
        let Some((last, before)) = labels.split_last() else {
            panic!("scale labels must end with their terminator");
        };
        assert!(
            last.0.is_null(),
            "scale labels must end with their terminator"
        );
        let mut i = 0;
        while i < before.len() {
            assert!(!before[i].0.is_null(), "the terminator ends the labels");
            i += 1;
        }
        Self { labels }
    }

    /// The number of labels (the terminator excluded).
    pub const fn label_count(&self) -> u32 {
        (self.labels.len() - 1) as u32
    }

    /// The labels.
    pub const fn as_ptr(&self) -> *const *const c_char {
        self.labels.as_ptr().cast()
    }
}

/// [`ScaleLabels`] from label texts, terminated automatically.
///
/// ```no_run
/// use lvgl_rust::{ScaleLabels, scale_labels};
///
/// static SPEEDS: ScaleLabels = scale_labels![c"0", c"50", c"100"];
/// ```
#[macro_export]
macro_rules! scale_labels {
    ($($text:expr),* $(,)?) => {
        $crate::ScaleLabels::new(&[
            $($crate::ScaleLabel::new($text),)*
            $crate::ScaleLabel::END,
        ])
    };
}

#[cfg(lv_use_scale)]
mod scale {
    use super::ScaleLabels;
    use crate::{sys, widgets::Scale};

    impl Scale {
        /// Labels the major ticks with `labels` (`None`: with their
        /// values), which LVGL keeps. Ticks beyond the last label get none.
        ///
        /// `lv_scale_set_text_src`.
        pub fn set_text_src(&self, labels: Option<&'static ScaleLabels>) {
            let labels = labels.map_or(core::ptr::null(), ScaleLabels::as_ptr);
            // SAFETY: shared access (the audit's), live scale; the labels
            // are `'static` and NULL-terminated. The parameter is not
            // `const` in C, but LVGL only reads the array.
            unsafe { sys::lv_scale_set_text_src(self.as_ptr(), labels.cast_mut()) };
        }
    }
}
