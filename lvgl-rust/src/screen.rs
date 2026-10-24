//! Screen load animations.

use lvgl_rust_sys as sys;

/// A screen load animation type (`lv_screen_load_anim_t`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct ScreenLoadAnim(u8);

#[allow(clippy::unnecessary_cast)] // the bindgen alias is `c_int` on MSVC
impl ScreenLoadAnim {
    pub const NONE: Self = Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_NONE as u8);
    pub const OVER_LEFT: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OVER_LEFT as u8);
    pub const OVER_RIGHT: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OVER_RIGHT as u8);
    pub const OVER_TOP: Self = Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OVER_TOP as u8);
    pub const OVER_BOTTOM: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OVER_BOTTOM as u8);
    pub const MOVE_LEFT: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_MOVE_LEFT as u8);
    pub const MOVE_RIGHT: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_MOVE_RIGHT as u8);
    pub const MOVE_TOP: Self = Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_MOVE_TOP as u8);
    pub const MOVE_BOTTOM: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_MOVE_BOTTOM as u8);
    pub const FADE_IN: Self = Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_FADE_IN as u8);
    pub const FADE_OUT: Self = Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_FADE_OUT as u8);
    pub const OUT_LEFT: Self = Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OUT_LEFT as u8);
    pub const OUT_RIGHT: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OUT_RIGHT as u8);
    pub const OUT_TOP: Self = Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OUT_TOP as u8);
    pub const OUT_BOTTOM: Self =
        Self(sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_OUT_BOTTOM as u8);
}

/// How to load a screen: animation type, duration and delay in milliseconds
/// (up to 65.5 s), and whether the previous screen is deleted once the load
/// completes. 6 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct ScreenAnim {
    kind: ScreenLoadAnim,
    auto_delete: bool,
    time_ms: u16,
    delay_ms: u16,
}

impl ScreenAnim {
    /// Load at once and keep the previous screen.
    pub const NONE: Self = Self::new(ScreenLoadAnim::NONE, 0, 0, false);

    pub const fn new(kind: ScreenLoadAnim, time_ms: u16, delay_ms: u16, auto_delete: bool) -> Self {
        Self {
            kind,
            auto_delete,
            time_ms,
            delay_ms,
        }
    }

    pub(crate) fn kind(self) -> u32 {
        self.kind.0 as u32
    }

    pub(crate) fn auto_delete(self) -> bool {
        self.auto_delete
    }

    pub(crate) fn time_ms(self) -> u32 {
        self.time_ms as u32
    }

    pub(crate) fn delay_ms(self) -> u32 {
        self.delay_ms as u32
    }
}

const _: () = assert!(size_of::<ScreenAnim>() == 6);
