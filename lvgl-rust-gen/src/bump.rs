//! `lvgl-rust-gen bump`: what the tables need after an LVGL bump (or a
//! change of `lvgl-rust/conf`), computed from the bindings and sources.

use crate::{Diagnostics, parse::Bindings, tables::Tables};
use std::collections::BTreeMap;

/// The header of `universe.txt`.
const UNIVERSE_HEADER: &str = "\
# Every LVGL {version} function of the configurations lvgl-rust supports (here:
# lvgl-rust/conf/lv_conf.h). Table entries for functions missing from a
# build's bindings are fine if they are listed here.
";

/// `universe.txt` for these bindings: every LVGL function, in byte order.
pub fn universe(bindings: &Bindings, version: &str) -> String {
    let mut out = UNIVERSE_HEADER.replace("{version}", version);
    // LVGL and its shims (not the C library's `imaxabs`, ...).
    let lvgl = |name: &&String| name.starts_with("lv_") || name.starts_with("lvgl_sys_");
    for name in bindings.functions.keys().filter(lvgl) {
        out.push_str(name);
        out.push('\n');
    }
    out
}

/// The base of every class defined in `sources` (`const lv_obj_class_t
/// lv_<x>_class = { ... .base_class = &lv_<y>_class, ... }`), as
/// `x -> y`.
pub fn class_bases<'a>(sources: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, String> {
    let mut bases = BTreeMap::new();
    for source in sources {
        let mut rest = source;
        while let Some(start) = rest.find("lv_obj_class_t lv_") {
            rest = &rest[start + "lv_obj_class_t ".len()..];
            let Some(name) = rest
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .next()
            else {
                continue;
            };
            let Some(class) = name
                .strip_prefix("lv_")
                .and_then(|n| n.strip_suffix("_class"))
            else {
                continue;
            };
            // Only definitions: `name = {`, up to the closing `};`.
            let after = rest[name.len()..].trim_start();
            let Some(body) = after.strip_prefix('=').map(str::trim_start) else {
                continue;
            };
            if !body.starts_with('{') {
                continue;
            }
            let end = body.find("};").unwrap_or(body.len());
            let body = &body[..end];
            if let Some(base) = body
                .split(".base_class")
                .nth(1)
                .and_then(|b| b.split("&lv_").nth(1))
                .and_then(|b| b.split("_class").next())
            {
                bases.insert(class.to_owned(), base.trim().to_owned());
            }
        }
    }
    bases
}

/// Differences between the classes found in the sources and the
/// `[hierarchy]` of `classes.toml` (which lists only bases other than
/// `obj`), as lines to review.
pub fn hierarchy_diff(
    found: &BTreeMap<String, String>,
    table: &BTreeMap<String, String>,
    in_bindings: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut diff = Vec::new();
    for (class, base) in found {
        if base == "obj" || !in_bindings(class) {
            continue;
        }
        match table.get(class) {
            None => diff.push(format!("missing: {class} = \"{base}\"")),
            Some(listed) if listed != base => diff.push(format!(
                "wrong: {class} = \"{listed}\", sources say \"{base}\""
            )),
            Some(_) => {}
        }
    }
    for class in table.keys() {
        if found.get(class).is_none_or(|b| b == "obj") {
            diff.push(format!("not a subclass in the sources: {class}"));
        }
    }
    diff
}

/// Strict validation of the tables against these bindings: every error and
/// stale entry (functions or parameters that no longer exist).
pub fn stale_entries(tables: &Tables, bindings: &Bindings) -> Vec<String> {
    let mut diags = Diagnostics::default();
    tables.validate(bindings, &mut diags);
    diags.errors.into_iter().chain(diags.stale).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_class_bases_and_reports_differences() {
        let sources = [
            r#"
            const lv_obj_class_t lv_slider_class = {
                .constructor_cb = lv_slider_constructor,
                .base_class = &lv_bar_class,
                .name = "lv_slider",
            };
            extern const lv_obj_class_t lv_bar_class;
            "#,
            r#"
            const lv_obj_class_t lv_bar_class = {
                .base_class = &lv_obj_class,
            };
            const lv_obj_class_t lv_spinner_class = { .base_class = &lv_arc_class };
            "#,
        ];
        let found = class_bases(sources);
        assert_eq!(found["slider"], "bar");
        assert_eq!(found["bar"], "obj");
        assert_eq!(found["spinner"], "arc");
        let table = BTreeMap::from([
            ("slider".to_owned(), "bar".to_owned()),
            ("spinner".to_owned(), "label".to_owned()),
            ("gone".to_owned(), "obj".to_owned()),
        ]);
        let diff = hierarchy_diff(&found, &table, |_| true);
        assert_eq!(
            diff,
            [
                "wrong: spinner = \"label\", sources say \"arc\"",
                "not a subclass in the sources: gone",
            ]
        );
        let found = BTreeMap::from([("keyboard".to_owned(), "buttonmatrix".to_owned())]);
        assert_eq!(
            hierarchy_diff(&found, &BTreeMap::new(), |_| true),
            ["missing: keyboard = \"buttonmatrix\""]
        );
        assert!(hierarchy_diff(&found, &BTreeMap::new(), |_| false).is_empty());
    }
}
