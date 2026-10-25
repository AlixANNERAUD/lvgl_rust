//! Styles, fonts and colors: data LVGL keeps pointers to, so the safe API
//! only passes `'static` references to it.

use crate::{
    Lvgl,
    enums::{Part, State},
    sys,
};
use core::{cell::UnsafeCell, marker::PhantomData};

/// A style whose properties can change: a `static` of 12 bytes on 32-bit
/// targets (16 with `LV_USE_ASSERT_STYLE`), whose properties live on LVGL's
/// heap. Changed through [`Lvgl::style`], read by LVGL wherever it is added.
///
/// ```no_run
/// use lvgl_rust::{Lvgl, Selector, Style, color};
///
/// static CARD: Style = Style::new();
///
/// fn build(lv: &mut Lvgl) {
///     {
///         let mut card = lv.style(&CARD);
///         card.set_radius(8);
///         card.set_bg_color(color::hex(0x202020));
///     } // objects using `CARD` are refreshed here
///     let mut screen = lv.edit(|lv| lv.screen_active()).unwrap();
///     screen.add_style(&CARD, Selector::DEFAULT);
/// }
/// ```
///
/// Styles that never change are cheaper as a [`ConstStyle`].
#[repr(transparent)]
pub struct Style(UnsafeCell<sys::lv_style_t>);

// SAFETY: only LVGL calls read or write the style, under the token: reads
// with shared access, changes through `StyleMut`, which holds exclusive
// access.
unsafe impl Sync for Style {}

impl Style {
    /// An empty style, as `lv_style_init` makes it.
    #[cfg_attr(not(lv_use_assert_style), allow(unused_mut))]
    pub const fn new() -> Self {
        // SAFETY: all zeroes is a valid `lv_style_t` (a null pointer and
        // integers), and what `lv_style_init` writes.
        let mut style: sys::lv_style_t = unsafe { core::mem::zeroed() };
        #[cfg(lv_use_assert_style)]
        {
            style.sentinel = sys::LV_STYLE_SENTINEL_VALUE;
        }
        Self(UnsafeCell::new(style))
    }
}

impl Default for Style {
    fn default() -> Self {
        Self::new()
    }
}

/// Exclusive access to a [`Style`], from [`Lvgl::style`]: the generated
/// `set_*`, `remove_prop`, `copy`, `merge` and `reset` methods. When dropped,
/// the objects using the style are refreshed
/// (`lv_obj_report_style_change`).
pub struct StyleMut<'a> {
    style: &'static Style,
    _l: PhantomData<&'a mut Lvgl>,
}

impl StyleMut<'_> {
    /// The style (also used by the generated wrappers).
    pub(crate) fn as_raw(&self) -> *mut sys::lv_style_t {
        self.style.0.get()
    }
}

impl Drop for StyleMut<'_> {
    fn drop(&mut self) {
        // SAFETY: exclusive access; a refresh can run callbacks, which get
        // shared access only.
        unsafe { sys::lv_obj_report_style_change(self.as_raw()) }
    }
}

impl Lvgl {
    /// Changes `style`. LVGL may be reading it from any object it was
    /// added to, so this needs exclusive access; the objects are refreshed
    /// once, when the returned [`StyleMut`] is dropped.
    pub fn style(&mut self, style: &'static Style) -> StyleMut<'_> {
        StyleMut {
            style,
            _l: PhantomData,
        }
    }
}

/// A style that never changes, entirely in `.rodata`: no RAM, no heap.
/// Made with [`const_style!`](crate::const_style).
///
/// LVGL refuses to change a constant style, and the functions that would
/// write to it anyway (`lv_style_init`, `lv_style_reset`) only take a
/// [`StyleMut`].
#[repr(transparent)]
pub struct ConstStyle(sys::lv_style_t);

// SAFETY: never written (see above); its properties are `'static`.
unsafe impl Sync for ConstStyle {}

impl ConstStyle {
    /// A style with these properties, which must end with
    /// [`ConstProp::END`] (and contain it only there): LVGL reads them up
    /// to it. [`const_style!`](crate::const_style) appends it.
    ///
    /// # Panics
    ///
    /// If `props` is not terminated as above (a compile error in a
    /// `static`).
    #[cfg_attr(not(lv_use_assert_style), allow(unused_mut))]
    pub const fn new(props: &'static [ConstProp]) -> Self {
        let Some((last, props_before)) = props.split_last() else {
            panic!("a constant style must end with ConstProp::END");
        };
        assert!(
            last.is_end(),
            "a constant style must end with ConstProp::END"
        );
        let mut i = 0;
        while i < props_before.len() {
            assert!(
                !props_before[i].is_end(),
                "ConstProp::END hides the properties after it"
            );
            i += 1;
        }
        // SAFETY: as in `Style::new`.
        let mut style: sys::lv_style_t = unsafe { core::mem::zeroed() };
        // `LV_STYLE_CONST_INIT` (`lv_style.h`).
        style.values_and_props = props.as_ptr().cast_mut().cast();
        style.has_group = 0xFFFF_FFFF;
        style.prop_cnt = 255;
        #[cfg(lv_use_assert_style)]
        {
            style.sentinel = sys::LV_STYLE_SENTINEL_VALUE;
        }
        Self(style)
    }
}

/// One property of a [`ConstStyle`], from the constructors of
/// [`props`](crate::props).
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ConstProp(sys::lv_style_const_prop_t);

// SAFETY: plain values, or pointers to `'static` data that is never written
// (fonts).
unsafe impl Sync for ConstProp {}

impl ConstProp {
    /// The terminator (`LV_STYLE_CONST_PROPS_END`).
    pub const END: Self = Self::new(
        sys::_lv_style_id_t_LV_STYLE_PROP_INV as sys::lv_style_prop_t,
        sys::lv_style_value_t { num: 0 },
    );

    pub(crate) const fn new(prop: sys::lv_style_prop_t, value: sys::lv_style_value_t) -> Self {
        Self(sys::lv_style_const_prop_t { prop, value })
    }

    const fn is_end(&self) -> bool {
        self.0.prop == sys::_lv_style_id_t_LV_STYLE_PROP_INV as sys::lv_style_prop_t
    }
}

/// A constant style from property constructors of [`props`](crate::props),
/// terminated automatically. Every argument must be a constant expression;
/// colors come from [`color`], fonts from their `const` constructors.
///
/// ```no_run
/// use lvgl_rust::{ConstStyle, Font, color, const_style, enums::Align};
///
/// static CARD: ConstStyle = const_style![
///     radius(8),
///     bg_color(color::hex(0x202020)),
///     align(Align::CENTER),
///     text_font(Font::montserrat_14()),
/// ];
/// ```
#[macro_export]
macro_rules! const_style {
    ($($prop:ident ( $($arg:expr),* $(,)? )),* $(,)?) => {
        $crate::ConstStyle::new(&[
            $($crate::props::$prop($($arg),*),)*
            $crate::ConstProp::END,
        ])
    };
}

/// A style LVGL may keep a pointer to: a `&'static` [`Style`] or
/// [`ConstStyle`]. Taken by `add_style`, `remove_style`, `replace_style`
/// and the style copy functions.
#[derive(Clone, Copy)]
pub struct StyleRef(*const sys::lv_style_t);

impl StyleRef {
    /// The style.
    pub fn as_ptr(self) -> *const sys::lv_style_t {
        self.0
    }
}

impl From<&'static Style> for StyleRef {
    fn from(style: &'static Style) -> Self {
        Self(style.0.get())
    }
}

impl From<&'static ConstStyle> for StyleRef {
    fn from(style: &'static ConstStyle) -> Self {
        Self(&style.0)
    }
}

/// Which part of an object, in which states, a style applies to
/// (`lv_style_selector_t`): `Part::MAIN`, `State::PRESSED`,
/// `Part::INDICATOR | State::CHECKED`, or [`Selector::DEFAULT`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Selector(u32);

impl Selector {
    /// The main part in the default state.
    pub const DEFAULT: Self = Self(0);

    /// `part` in `state`.
    pub const fn new(part: Part, state: State) -> Self {
        Self(part.bits() | state.bits())
    }

    /// The raw value.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl From<Part> for Selector {
    fn from(part: Part) -> Self {
        Self(part.bits())
    }
}

impl From<State> for Selector {
    fn from(state: State) -> Self {
        Self(state.bits())
    }
}

impl core::ops::BitOr<State> for Part {
    type Output = Selector;
    fn bitor(self, state: State) -> Selector {
        Selector::new(self, state)
    }
}

impl core::ops::BitOr<State> for Selector {
    type Output = Selector;
    fn bitor(self, state: State) -> Selector {
        Self(self.0 | state.bits())
    }
}

/// A font LVGL can draw with (`lv_font_t`). Only reachable as
/// `&'static Font`: LVGL keeps pointers to the fonts it is given, and
/// fonts usually sit in flash. The fonts of the C library are constructors
/// (`Font::montserrat_14()`, ...).
#[repr(transparent)]
pub struct Font(sys::lv_font_t);

// SAFETY: never written while referenced (`from_raw`); read under the token.
unsafe impl Sync for Font {}

impl Font {
    /// A font defined outside this crate (by `lv_font_conv` output, a font
    /// crate, ...).
    ///
    /// # Safety
    ///
    /// `font` must be a valid LVGL font (its callbacks and data), usable
    /// from any thread holding the token, and nothing may write to it from
    /// now on (fonts that change, like `tiny_ttf` ones after
    /// `lv_tiny_ttf_set_size`, are raw only).
    pub const unsafe fn from_raw(font: &'static sys::lv_font_t) -> &'static Font {
        // SAFETY: `Font` is a transparent wrapper.
        unsafe { &*(font as *const sys::lv_font_t).cast::<Font>() }
    }

    /// The font.
    pub fn as_ptr(&self) -> *const sys::lv_font_t {
        &self.0
    }
}

/// A font returned by an LVGL getter, borrowed like a view.
pub(crate) fn font<'a>(p: *const sys::lv_font_t) -> Option<&'a Font> {
    // SAFETY: a font LVGL uses stays valid while it is set (raw setters
    // require it), and a shared borrow of LVGL prevents unsetting it.
    (!p.is_null()).then(|| unsafe { &*p.cast::<Font>() })
}

/// Colors as constants (`lv_color_hex` and `lv_color_make` are functions).
pub mod color {
    use crate::sys::lv_color_t;

    /// A color from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> lv_color_t {
        lv_color_t {
            red: (rgb >> 16) as u8,
            green: (rgb >> 8) as u8,
            blue: rgb as u8,
        }
    }

    /// A color from its components.
    pub const fn rgb(red: u8, green: u8, blue: u8) -> lv_color_t {
        lv_color_t { red, green, blue }
    }
}

/// A style property (`lv_style_prop_t`), for [`TransitionDsc`]: one
/// constant per property of these bindings (`StyleProp::BG_COLOR`, ...).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StyleProp(pub(crate) sys::lv_style_prop_t);

impl StyleProp {
    /// The terminator (`LV_STYLE_PROP_INV`).
    pub const END: Self = Self(sys::_lv_style_id_t_LV_STYLE_PROP_INV as sys::lv_style_prop_t);
}

/// The properties a [`TransitionDsc`] animates, terminated automatically:
/// `style_props![BG_COLOR, BORDER_WIDTH]`.
#[macro_export]
macro_rules! style_props {
    ($($prop:ident),* $(,)?) => {
        &[$($crate::StyleProp::$prop,)* $crate::StyleProp::END]
    };
}

/// A simple gradient (`lv_grad_dsc_t`, the `bg_grad` style property):
/// horizontal or vertical, between up to `LV_GRADIENT_MAX_STOPS` stops.
#[repr(transparent)]
pub struct GradDsc(sys::lv_grad_dsc_t);

// SAFETY: never written: LVGL writes `state` only for linear, radial and
// conical gradients (`lv_draw_sw_grad.c:275, 462, 537`), which `new` does
// not make.
unsafe impl Sync for GradDsc {}

/// The direction of a [`GradDsc`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GradDir {
    Horizontal,
    Vertical,
}

impl GradDsc {
    /// A gradient through `stops` (color, opacity and position from 0 to
    /// 255 each).
    ///
    /// # Panics
    ///
    /// Without stops or with more than `LV_GRADIENT_MAX_STOPS` (a compile
    /// error in a `static`): LVGL reads `stops[stops_count - 1]`.
    pub const fn new(dir: GradDir, stops: &[sys::lv_grad_stop_t]) -> Self {
        assert!(
            !stops.is_empty() && stops.len() <= sys::LV_GRADIENT_MAX_STOPS as usize,
            "a gradient needs 1 to LV_GRADIENT_MAX_STOPS stops"
        );
        // SAFETY: all zeroes is a valid `lv_grad_dsc_t` (integers, null
        // pointers, an empty union).
        let mut grad: sys::lv_grad_dsc_t = unsafe { core::mem::zeroed() };
        let mut i = 0;
        while i < stops.len() {
            grad.stops[i] = stops[i];
            i += 1;
        }
        grad.stops_count = stops.len() as u8;
        let dir = match dir {
            GradDir::Horizontal => sys::lv_grad_dir_t_LV_GRAD_DIR_HOR,
            GradDir::Vertical => sys::lv_grad_dir_t_LV_GRAD_DIR_VER,
        };
        // `dir : 4` then `extend : 3` (`lv_grad.h`); extend: pad (0).
        grad._bitfield_1 = sys::__BindgenBitfieldUnit::new([dir as u8]);
        Self(grad)
    }

    /// The gradient.
    pub const fn as_ptr(&self) -> *const sys::lv_grad_dsc_t {
        &self.0
    }
}

/// How an animated value moves from start to end (LVGL's built-in paths).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AnimPath {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Overshoot,
    Bounce,
    Step,
}

impl AnimPath {
    pub(crate) const fn callback(self) -> sys::lv_anim_path_cb_t {
        Some(match self {
            AnimPath::Linear => sys::lv_anim_path_linear,
            AnimPath::EaseIn => sys::lv_anim_path_ease_in,
            AnimPath::EaseOut => sys::lv_anim_path_ease_out,
            AnimPath::EaseInOut => sys::lv_anim_path_ease_in_out,
            AnimPath::Overshoot => sys::lv_anim_path_overshoot,
            AnimPath::Bounce => sys::lv_anim_path_bounce,
            AnimPath::Step => sys::lv_anim_path_step,
        })
    }
}

/// A style transition (`lv_style_transition_dsc_t`, the `transition` style
/// property): which properties animate when an object's state changes.
#[repr(transparent)]
pub struct TransitionDsc(sys::lv_style_transition_dsc_t);

// SAFETY: never written; its property list is `'static`.
unsafe impl Sync for TransitionDsc {}

impl TransitionDsc {
    /// Animates `props` (from [`style_props!`](crate::style_props)) along
    /// `path` for `time_ms`, after `delay_ms`.
    ///
    /// # Panics
    ///
    /// If `props` does not end with [`StyleProp::END`], or contains it
    /// earlier (a compile error in a `static`).
    pub const fn new(
        props: &'static [StyleProp],
        path: AnimPath,
        time_ms: u32,
        delay_ms: u32,
    ) -> Self {
        let Some((last, props_before)) = props.split_last() else {
            panic!("transition properties must end with StyleProp::END");
        };
        assert!(
            last.0 == StyleProp::END.0,
            "transition properties must end with StyleProp::END"
        );
        let mut i = 0;
        while i < props_before.len() {
            assert!(
                props_before[i].0 != StyleProp::END.0,
                "StyleProp::END hides the properties after it"
            );
            i += 1;
        }
        Self(sys::lv_style_transition_dsc_t {
            props: props.as_ptr().cast(),
            user_data: core::ptr::null_mut(),
            path_xcb: path.callback(),
            time: time_ms,
            delay: delay_ms,
        })
    }

    /// The transition.
    pub const fn as_ptr(&self) -> *const sys::lv_style_transition_dsc_t {
        &self.0
    }
}

/// A color filter (`lv_color_filter_dsc_t`, the `color_filter_dsc` style
/// property), applied to colors while drawing.
#[repr(transparent)]
pub struct ColorFilter(sys::lv_color_filter_dsc_t);

// SAFETY: never written; its callback is a plain function.
unsafe impl Sync for ColorFilter {}

impl ColorFilter {
    /// A filter calling `filter(dsc, color, opa)` for each color. It may run
    /// on a draw thread.
    pub const fn new(
        filter: extern "C" fn(
            *const sys::lv_color_filter_dsc_t,
            sys::lv_color_t,
            sys::lv_opa_t,
        ) -> sys::lv_color_t,
    ) -> Self {
        Self(sys::lv_color_filter_dsc_t {
            filter_cb: Some(filter),
            user_data: core::ptr::null_mut(),
        })
    }

    /// LVGL's shading filter (`lv_color_filter_shade`): darkens or lightens
    /// by the filter opacity.
    pub const fn shade() -> &'static ColorFilter {
        // SAFETY: a constant of the C library; `ColorFilter` is a
        // transparent wrapper.
        unsafe { &*core::ptr::addr_of!(sys::lv_color_filter_shade).cast::<ColorFilter>() }
    }

    /// The filter.
    pub const fn as_ptr(&self) -> *const sys::lv_color_filter_dsc_t {
        &self.0
    }
}
