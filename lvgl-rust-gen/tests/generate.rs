//! Golden and negative tests of the generator on a small synthetic input
//! covering every rule. Set `UPDATE_GOLDEN=1` to rewrite the golden files.

use lvgl_rust_gen::{generate_from, headers::Headers, parse::Bindings, tables::Tables};
use std::{fs, path::Path};

const BINDINGS: &str = r#"
pub const LVGL_VERSION_MAJOR: u32 = 9;
pub const LVGL_VERSION_MINOR: u32 = 6;
pub const LV_USE_SLIDER: u32 = 1;
pub const LV_USE_MENU: u32 = 0;
#[repr(C)] pub struct _lv_obj_t { _unused: [u8; 0] }
pub type lv_obj_t = _lv_obj_t;
#[repr(C)] pub struct lv_obj_class_t { _unused: [u8; 0] }
#[repr(C)] pub struct lv_timer_t { _unused: [u8; 0] }
#[repr(C)] pub struct lv_color_t { pub blue: u8, pub green: u8, pub red: u8 }
#[repr(C)] pub struct lv_area_t { pub x1: i32, pub y1: i32, pub x2: i32, pub y2: i32 }
pub type lv_opa_t = u8;
#[repr(C)] pub struct lv_style_t { pub values_and_props: *mut core::ffi::c_void, pub has_group: u32, pub prop_cnt: u8 }
#[repr(C)] pub struct lv_font_t { pub line_height: i32 }
#[repr(C)] pub struct lv_subject_t { pub type_: u32 }
#[repr(C)] pub struct _lv_observer_t { _unused: [u8; 0] }
pub type lv_observer_t = _lv_observer_t;
pub type lv_style_prop_t = u8;
pub type lv_style_selector_t = u32;
pub type _lv_style_id_t = core::ffi::c_uint;
pub const _lv_style_id_t_LV_STYLE_BG_COLOR: _lv_style_id_t = 73;
pub const _lv_style_id_t_LV_STYLE_RADIUS: _lv_style_id_t = 12;
pub const _lv_style_id_t_LV_STYLE_TEXT_FONT: _lv_style_id_t = 90;
pub type lv_event_code_t = core::ffi::c_uint;
pub const lv_event_code_t_LV_EVENT_CLICKED: lv_event_code_t = 10;
pub const lv_event_code_t_LV_EVENT_KEY: lv_event_code_t = 21;
pub const lv_event_code_t_LV_EVENT_DRAW_MAIN: lv_event_code_t = 33;
pub const lv_event_code_t_LV_EVENT_VALUE_CHANGED: lv_event_code_t = 39;
pub const lv_event_code_t_LV_EVENT_SCREEN_LOADED: lv_event_code_t = 70;
pub type lv_anim_enable_t = bool;
pub type lv_align_t = core::ffi::c_uint;
pub const lv_align_t_LV_ALIGN_CENTER: lv_align_t = 9;
pub const lv_align_t_LV_ALIGN_TOP_LEFT: lv_align_t = 1;
pub type lv_state_t = core::ffi::c_uint;
pub const lv_state_t_LV_STATE_DEFAULT: lv_state_t = 0;
pub const lv_state_t_LV_STATE_CHECKED: lv_state_t = 1;
pub const lv_state_t_LV_STATE_FOCUSED: lv_state_t = 2;
unsafe extern "C" {
    pub static lv_obj_class: lv_obj_class_t;
    pub static lv_bar_class: lv_obj_class_t;
    pub static lv_slider_class: lv_obj_class_t;
    pub static lv_label_class: lv_obj_class_t;
    pub static lv_image_class: lv_obj_class_t;
    pub static lv_textarea_class: lv_obj_class_t;
    pub static lv_msgbox_class: lv_obj_class_t;
    pub static lv_font_montserrat_14: lv_font_t;

    pub fn lv_style_set_bg_color(style: *mut lv_style_t, value: lv_color_t);
    pub fn lv_style_set_radius(style: *mut lv_style_t, value: i32);
    pub fn lv_style_set_text_font(style: *mut lv_style_t, value: *const lv_font_t);
    pub fn lv_style_set_bg_image_src(style: *mut lv_style_t, value: *const core::ffi::c_void);
    pub fn lv_image_set_src(obj: *mut lv_obj_t, src: *const core::ffi::c_void);
    pub fn lv_style_copy(dst: *mut lv_style_t, src: *const lv_style_t);
    pub fn lv_style_is_empty(style: *const lv_style_t) -> bool;
    pub fn lv_obj_add_style(obj: *mut lv_obj_t, style: *const lv_style_t, selector: lv_style_selector_t);
    pub fn lv_obj_remove_style(obj: *mut lv_obj_t, style: *const lv_style_t, selector: lv_style_selector_t);
    pub fn lv_obj_get_style_text_font(obj: *const lv_obj_t, part: u32) -> *const lv_font_t;
    pub fn lv_font_get_line_height(font: *const lv_font_t) -> i32;
    pub fn lv_font_get_default() -> *const lv_font_t;
    pub fn lv_font_set_kerning(font: *mut lv_font_t, kerning: u32);

    pub fn lv_obj_set_size(obj: *mut lv_obj_t, w: i32, h: i32);
    pub fn lv_obj_set_opa(obj: *mut lv_obj_t, opa: lv_opa_t);
    pub fn lv_obj_align_to(obj: *mut lv_obj_t, base: *const lv_obj_t, align: lv_align_t, x: i32, y: i32);
    pub fn lv_obj_add_state(obj: *mut lv_obj_t, state: lv_state_t);
    pub fn lv_obj_get_state(obj: *const lv_obj_t) -> lv_state_t;
    pub fn lv_obj_get_screen(obj: *const lv_obj_t) -> *mut lv_obj_t;
    pub fn lv_obj_set_user_data(obj: *mut lv_obj_t, user_data: *mut core::ffi::c_void);
    pub fn lv_obj_get_coords(obj: *const lv_obj_t, coords: *mut lv_area_t);
    pub fn lv_obj_set_ext_area(obj: *mut lv_obj_t, area: *const lv_area_t);
    pub fn lv_obj_set_id(obj: *mut lv_obj_t, id: *mut core::ffi::c_void);
    pub fn lv_obj_old(obj: *mut lv_obj_t);
    pub fn lv_obj_unlisted(obj: *mut lv_obj_t);
    pub fn lv_bar_create(parent: *mut lv_obj_t) -> *mut lv_obj_t;
    pub fn lv_bar_set_value(obj: *mut lv_obj_t, value: i32, anim: lv_anim_enable_t);
    pub fn lv_bar_set_segment(obj: *mut lv_obj_t, idx: u32, value: i32);
    pub fn lv_bar_select(obj: *mut lv_obj_t, sel_opt: u32);
    pub fn lv_slider_create(parent: *mut lv_obj_t) -> *mut lv_obj_t;
    pub fn lv_slider_get_value(obj: *const lv_obj_t) -> i32;
    pub fn lv_slider_bind_value(obj: *mut lv_obj_t, subject: *mut lv_subject_t) -> *mut lv_observer_t;
    pub fn lv_label_create(parent: *mut lv_obj_t) -> *mut lv_obj_t;
    pub fn lv_label_set_text(obj: *mut lv_obj_t, text: *const core::ffi::c_char);
    pub fn lv_label_set_text_static(obj: *mut lv_obj_t, text: *const core::ffi::c_char);
    pub fn lv_label_set_text_fmt(obj: *mut lv_obj_t, fmt: *const core::ffi::c_char, ...);
    pub fn lv_label_set_recolor_text(obj: *mut lv_obj_t, text: *const core::ffi::c_char);
    pub fn lv_label_get_text(obj: *const lv_obj_t) -> *mut core::ffi::c_char;
    pub fn lv_label_get_letter(obj: *const lv_obj_t) -> *mut core::ffi::c_char;
    pub fn lv_textarea_get_label(obj: *const lv_obj_t) -> *mut lv_obj_t;
    pub fn lv_textarea_set_placeholder_text(obj: *mut lv_obj_t, txt: *const core::ffi::c_char);
    pub fn lv_msgbox_close(mbox: *mut lv_obj_t);
    pub fn lv_msgbox_set_parent_or_null(mbox: *mut lv_obj_t, parent: *mut lv_obj_t);
    pub fn lv_screen_clean_all();
    pub fn lv_layer_count() -> u32;
    pub fn lv_color_make(r: u8, g: u8, b: u8) -> lv_color_t;
    pub fn lv_color_screen_of(c: lv_color_t) -> *mut lv_obj_t;
    pub fn lv_timer_set_period(timer: *mut lv_timer_t, period: u32);
    pub fn lv_timer_get_paused(timer: *mut lv_timer_t) -> bool;
    pub fn lv_tick_inc(tick_period: u32);
}
"#;

const HEADERS: &str = r#"
/**
 * Set the size of an object.
 * @param obj   pointer to an object
 */
void lv_obj_set_size(lv_obj_t * obj, int32_t w, int32_t h);

/**
 * Old API.
 */
LV_DEPRECATED("use the new one")
void lv_obj_old(lv_obj_t * obj);

/**
 * Change the parent of a message box.
 * @param mbox      the message box
 * @param parent    the new parent @nullable
 */
void lv_msgbox_set_parent_or_null(lv_obj_t * mbox, lv_obj_t * parent);

/**
 * Remove a style from an object.
 * @param style     the style, NULL for every style @nullable
 */
void lv_obj_remove_style(lv_obj_t * obj, const lv_style_t * style, lv_style_selector_t selector);
"#;

const ACCESS: &str = r#"
[fn.lv_obj_set_size]
access = "shared"
[fn.lv_obj_set_opa]
access = "shared"
[fn.lv_obj_align_to]
access = "shared"
[fn.lv_obj_add_state]
access = "shared"
[fn.lv_obj_get_state]
access = "shared"
[fn.lv_obj_get_screen]
access = "shared"
[fn.lv_obj_set_user_data]
access = "shared"
[fn.lv_obj_get_coords]
access = "shared"
[fn.lv_obj_set_ext_area]
access = "shared"
[fn.lv_obj_old]
access = "shared"
[fn.lv_bar_create]
access = "exclusive"
[fn.lv_bar_set_value]
access = "shared"
[fn.lv_bar_set_segment]
access = "shared"
[fn.lv_bar_select]
access = "exclusive"
[fn.lv_slider_create]
access = "shared"
[fn.lv_slider_get_value]
access = "shared"
[fn.lv_slider_bind_value]
access = "exclusive"
[fn.lv_label_create]
access = "shared"
[fn.lv_label_set_text]
access = "shared"
[fn.lv_label_set_text_static]
access = "shared"
[fn.lv_label_set_text_fmt]
access = "shared"
[fn.lv_label_set_recolor_text]
access = "shared"
[fn.lv_label_get_text]
access = "shared"
[fn.lv_label_get_letter]
access = "shared"
[fn.lv_textarea_get_label]
access = "shared"
[fn.lv_textarea_set_placeholder_text]
access = "raw"
why = "kept by the textarea"
[fn.lv_msgbox_close]
access = "exclusive"
consumes = true
[fn.lv_msgbox_set_parent_or_null]
access = "exclusive"
[fn.lv_screen_clean_all]
access = "exclusive"
[fn.lv_layer_count]
access = "shared"
[fn.lv_color_make]
access = "pure"
[fn.lv_color_screen_of]
access = "pure"
[fn.lv_timer_set_period]
access = "shared"
[fn.lv_timer_get_paused]
access = "shared"
[fn.lv_tick_inc]
access = "isr"
[fn.lv_style_set_bg_color]
access = "shared"
[fn.lv_style_set_radius]
access = "pure"
[fn.lv_style_set_text_font]
access = "shared"
[fn.lv_style_set_bg_image_src]
access = "shared"
[fn.lv_image_set_src]
access = "exclusive"
[fn.lv_style_copy]
access = "shared"
[fn.lv_style_is_empty]
access = "shared"
[fn.lv_obj_add_style]
access = "exclusive"
[fn.lv_obj_remove_style]
access = "exclusive"
[fn.lv_obj_get_style_text_font]
access = "shared"
[fn.lv_font_get_line_height]
access = "shared"
[fn.lv_font_get_default]
access = "pure"
[fn.lv_font_set_kerning]
access = "shared"
"#;

const CLASSES: &str = r#"
generate = ["lv_timer_get_paused"]
manual = []
[hierarchy]
slider = "bar"
[names]
msgbox = "MsgBox"
[crate_owned]
skip = ["lv_obj_set_id"]
raw = ["lv_tick_*"]
[composite.textarea]
parts = ["lv_textarea_get_label"]
max_depth = 1
"#;

const RELATIONS: &str = r#"
[fn.lv_obj_align_to]
param = "base"
"#;

const OVERRIDES: &str = r#"
[type.lv_state_t]
flags = true
[fn.lv_label_set_text]
params.text = { kind = "copied_str" }
[fn.lv_obj_set_ext_area]
params.area = { kind = "copied" }
[fn.lv_textarea_get_label]
returns = { view = "label" }
[fn.lv_label_get_text]
returns = "borrowed_internal"
[fn.lv_bar_select]
params.sel_opt = { kind = "checked" }
[fn.lv_obj_get_coords]
params.coords = { kind = "out" }
[fn.lv_image_set_src]
params.src = { kind = "retained", rust = "ImageSrc" }
[fn.lv_slider_bind_value]
params.subject = { kind = "retained", rust = "IntSubject" }
[fn.lv_style_set_bg_image_src]
params.value = { kind = "retained", rust = "ImageSrc" }
"#;

const EVENTS: &str = r#"
[code.LV_EVENT_CLICKED]
value = 10
handler = "shared"
deleting = true
param = []
sendable = "exclusive"

[code.LV_EVENT_KEY]
value = 21
handler = "shared"
deleting = false
param = ["lv_event_get_param"]
sendable = "no"

[code.LV_EVENT_DRAW_MAIN]
value = 33
handler = "restricted"
deleting = false
param = ["lv_event_get_layer"]
sendable = "no"

[code.LV_EVENT_VALUE_CHANGED]
value = 39
handler = "shared"
deleting = false
param = []
sendable = "shared"

[code.LV_EVENT_SCREEN_LOADED]
value = 70
handler = "shared"
deleting = false
param = []
sendable = "shared"
"#;

fn inputs() -> (Bindings, Headers) {
    let bindings = Bindings::parse(BINDINGS).unwrap();
    let mut headers = Headers::default();
    headers.add_source(HEADERS);
    (bindings, headers)
}

fn load(access: &str, classes: &str, overrides: &str) -> Result<Tables, lvgl_rust_gen::Error> {
    let mut tables = Tables::parse(access, classes, RELATIONS, &[overrides], &[])?;
    tables.parse_events(EVENTS)?;
    Ok(tables)
}

fn check_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}: run with UPDATE_GOLDEN=1", path.display()));
    assert!(
        expected == actual,
        "{} differs from the generated output; rerun with UPDATE_GOLDEN=1 and review the diff",
        path.display()
    );
}

#[test]
fn generated_code_matches_golden() {
    let (bindings, headers) = inputs();
    let tables = load(ACCESS, CLASSES, OVERRIDES).unwrap();
    let output = generate_from(&bindings, &headers, &tables, true).unwrap();
    check_golden("synthetic.rs", &output.code);
    check_golden("synthetic.coverage.txt", &output.coverage);
    assert_eq!(
        output.cfgs,
        [
            ("lv_use_menu".to_owned(), false),
            ("lv_use_slider".to_owned(), true)
        ]
    );
}

#[test]
fn generated_code_is_valid_rust() {
    let (bindings, headers) = inputs();
    let tables = load(ACCESS, CLASSES, OVERRIDES).unwrap();
    let output = generate_from(&bindings, &headers, &tables, true).unwrap();
    syn::parse_file(&output.code).unwrap();
}

#[test]
fn crate_owned_functions_cannot_be_exposed() {
    let (bindings, headers) = inputs();
    // Through access.toml.
    let access = format!("{ACCESS}\n[fn.lv_obj_set_id]\naccess = \"shared\"\n");
    let tables = load(&access, CLASSES, OVERRIDES).unwrap();
    let error = generate_from(&bindings, &headers, &tables, false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("[fn.lv_obj_set_id] is crate-owned"),
        "{error}"
    );
    // Through a glob pattern, and through an override.
    let overrides = format!("{OVERRIDES}\n[fn.lv_tick_inc]\naccess = \"exclusive\"\n");
    let tables = load(ACCESS, CLASSES, &overrides).unwrap();
    let error = generate_from(&bindings, &headers, &tables, false).unwrap_err();
    assert!(error.to_string().contains("[fn.lv_tick_inc]"), "{error}");
}

#[test]
fn stale_entries_fail_only_in_strict_mode() {
    let (bindings, headers) = inputs();
    let access = format!("{ACCESS}\n[fn.lv_gone]\naccess = \"shared\"\n");
    let overrides = format!("{OVERRIDES}\n[fn.lv_obj_set_size.params.depth]\nkind = \"out\"\n");
    let tables = load(&access, CLASSES, &overrides).unwrap();
    let output = generate_from(&bindings, &headers, &tables, false).unwrap();
    assert!(
        output.warnings.iter().any(|w| w.contains("lv_gone")),
        "{:?}",
        output.warnings
    );
    assert!(
        output.warnings.iter().any(|w| w.contains("`depth`")),
        "{:?}",
        output.warnings
    );
    let error = generate_from(&bindings, &headers, &tables, true).unwrap_err();
    assert!(error.to_string().contains("lv_gone"), "{error}");
    // Functions of other configurations (the universe) are not stale.
    let tables = Tables::parse(&access, CLASSES, RELATIONS, &[OVERRIDES], &["lv_gone"]).unwrap();
    generate_from(&bindings, &headers, &tables, true).unwrap();
}

#[test]
fn table_errors_always_fail() {
    let (bindings, headers) = inputs();
    let cases = [
        (
            ACCESS.to_owned(),
            format!("{OVERRIDES}\n[fn.lv_bar_set_value]\naccess = \"exclusive\"\n"),
            CLASSES.to_owned(),
            "conflicts with access.toml",
        ),
        (
            ACCESS.to_owned(),
            OVERRIDES.to_owned(),
            format!(
                "{CLASSES}\n[composite.msgbox]\nparts = [\"lv_label_get_text\"]\nmax_depth = 1\n"
            ),
            "is not a getter of msgbox",
        ),
        (
            ACCESS.to_owned(),
            OVERRIDES.to_owned(),
            format!("{CLASSES}\n[composite.roller]\nparts = [\"child:0\"]\nmax_depth = 0\n"),
            "max_depth must be at least 1",
        ),
        (
            ACCESS.to_owned(),
            format!(
                "{OVERRIDES}\n[fn.lv_obj_set_user_data]\nparams.user_data = {{ kind = \"retained\", rust = \"Anything\" }}\n"
            ),
            CLASSES.to_owned(),
            "unknown rust type `Anything`",
        ),
        (
            ACCESS.to_owned(),
            format!(
                "{OVERRIDES}\n[fn.lv_obj_set_user_data]\nparams.user_data = {{ kind = \"copied\", rust = \"ImageSrc\" }}\n"
            ),
            CLASSES.to_owned(),
            "`rust` needs kind = \"retained\"",
        ),
    ];
    for (access, overrides, classes, expected) in cases {
        let tables = load(&access, &classes, &overrides).unwrap();
        let error = generate_from(&bindings, &headers, &tables, false).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    let relations = "[fn.lv_obj_align_to]\nparam = \"base\"\nraw = true\n";
    let tables = Tables::parse(ACCESS, CLASSES, relations, &[OVERRIDES], &[]).unwrap();
    let error = generate_from(&bindings, &headers, &tables, false).unwrap_err();
    assert!(
        error.to_string().contains("either `param` or `raw = true`"),
        "{error}"
    );
}

#[test]
fn event_tables_are_checked_against_the_bindings() {
    let (bindings, headers) = inputs();
    let output = generate_from(
        &bindings,
        &headers,
        &load(ACCESS, CLASSES, OVERRIDES).unwrap(),
        true,
    )
    .unwrap();
    // CLICKED (10), KEY (21), VALUE_CHANGED (39) in the first word,
    // SCREEN_LOADED (70) in the second; DRAW_MAIN is restricted.
    let mask = (1u64 << 10) | (1 << 21) | (1 << 39);
    assert!(
        output.code.contains(&format!(
            "HANDLER_SHARED: [u64; 2] = [{mask}, {}]",
            1u64 << 6
        )),
        "{}",
        output.code
    );
    // DRAW_MAIN (33) alone gets the restricted context, with a query code.
    assert!(
        output.code.contains(&format!(
            "HANDLER_RESTRICTED: [u64; 2] = [{}, 0]",
            1u64 << 33
        )),
        "{}",
        output.code
    );
    assert!(output.code.contains("pub const DRAW_MAIN: Self"));
    let cases = [
        (
            "value = 10",
            "value = 11",
            "has value 11 but the bindings say 10",
        ),
        (
            "sendable = \"shared\"\n\n[code.LV_EVENT_SCREEN_LOADED]",
            "sendable = \"bogus\"\n\n[code.LV_EVENT_SCREEN_LOADED]",
            "cannot be sendable \"bogus\"",
        ),
        (
            "param = [\"lv_event_get_param\"]\nsendable = \"no\"",
            "param = [\"lv_event_get_param\"]\nsendable = \"shared\"",
            "[code.LV_EVENT_KEY] cannot be sendable \"shared\"",
        ),
        (
            "handler = \"restricted\"",
            "handler = \"maybe\"",
            "handler \"maybe\"",
        ),
    ];
    for (from, to, expected) in cases {
        let mut tables = Tables::parse(ACCESS, CLASSES, RELATIONS, &[OVERRIDES], &[]).unwrap();
        assert!(EVENTS.contains(from), "{from}");
        tables.parse_events(&EVENTS.replacen(from, to, 1)).unwrap();
        let error = generate_from(&bindings, &headers, &tables, false).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    // A code this configuration does not have is stale.
    let mut tables = load(ACCESS, CLASSES, OVERRIDES).unwrap();
    tables
        .parse_events(&format!(
            "{EVENTS}\n[code.LV_EVENT_GONE]\nvalue = 90\nhandler = \"shared\"\ndeleting = false\nsendable = \"shared\"\n"
        ))
        .unwrap();
    let output = generate_from(&bindings, &headers, &tables, false).unwrap();
    assert!(output.warnings.iter().any(|w| w.contains("LV_EVENT_GONE")));
    assert!(!output.code.contains("GONE"));
    assert!(generate_from(&bindings, &headers, &tables, true).is_err());
}

#[test]
fn tables_are_selected_by_lvgl_version() {
    let (bindings, _) = inputs();
    let root = lvgl_rust_gen::tables_root();
    assert!(
        Tables::directory(&root, &bindings)
            .unwrap()
            .ends_with("lvgl-9.6")
    );
    let other = Bindings::parse(
        "pub const LVGL_VERSION_MAJOR: u32 = 8;\npub const LVGL_VERSION_MINOR: u32 = 3;",
    )
    .unwrap();
    let error = Tables::directory(&root, &other).unwrap_err();
    assert!(
        error.to_string().contains("no tables for LVGL 8.3"),
        "{error}"
    );
}

#[test]
fn shipped_tables_parse() {
    let (bindings, _) = inputs();
    let dir = Tables::directory(&lvgl_rust_gen::tables_root(), &bindings).unwrap();
    let tables = Tables::load(&dir).unwrap();
    assert!(tables.universe.contains("lv_obj_create"));
    assert!(tables.access.functions.contains_key("lv_label_set_text"));
}
