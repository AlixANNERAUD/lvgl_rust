//! Plain-text scan of LVGL's headers for what bindgen drops: doc comments,
//! `LV_DEPRECATED(...)` before a declaration, `@nullable` parameters.

use std::{collections::BTreeMap, fs, io, path::Path};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderFn {
    pub deprecated: bool,
    /// The doc comment text, without comment markers.
    pub doc: String,
    /// Parameters documented as accepting NULL.
    pub nullable: Vec<String>,
}

#[derive(Default, Debug)]
pub struct Headers {
    pub functions: BTreeMap<String, HeaderFn>,
}

impl Headers {
    /// Scans every `.h` file under `dirs`, recursively.
    pub fn scan(dirs: &[&Path]) -> io::Result<Headers> {
        let mut headers = Headers::default();
        for dir in dirs {
            let mut files = Vec::new();
            collect(dir, &mut files)?;
            files.sort();
            for file in files {
                headers.add_source(&fs::read_to_string(&file)?);
            }
        }
        Ok(headers)
    }

    pub fn add_source(&mut self, source: &str) {
        let mut doc: Option<String> = None;
        let mut deprecated = false;
        let mut lines = source.lines().peekable();
        while let Some(line) = lines.next() {
            let trimmed = line.trim();
            if trimmed.starts_with("/**") {
                let mut text = String::new();
                let mut current = trimmed;
                loop {
                    text.push_str(strip_comment(current));
                    text.push('\n');
                    if current.contains("*/") {
                        break;
                    }
                    match lines.next() {
                        Some(next) => current = next.trim(),
                        None => break,
                    }
                }
                doc = Some(text.trim().to_owned());
                deprecated = false;
                continue;
            }
            if trimmed.starts_with("LV_DEPRECATED(") && !trimmed.contains(';') {
                deprecated = true;
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                if !trimmed.is_empty() {
                    doc = None;
                    deprecated = false;
                }
                continue;
            }
            if let Some(name) = declared_function(trimmed) {
                let doc_text = doc.take().unwrap_or_default();
                let entry = HeaderFn {
                    deprecated: deprecated
                        || trimmed.contains("LV_DEPRECATED(")
                        || doc_text.contains("@deprecated"),
                    nullable: nullable_params(&doc_text),
                    doc: doc_text,
                };
                self.functions.entry(name).or_insert(entry);
            } else {
                doc = None;
            }
            deprecated = false;
        }
    }
}

fn collect(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "h") {
            files.push(path);
        }
    }
    Ok(())
}

fn strip_comment(line: &str) -> &str {
    let line = line.trim_start_matches("/**").trim_end_matches("*/").trim();
    line.strip_prefix('*').map_or(line, str::trim_start)
}

/// The name of the function a top-level declaration line declares:
/// `void lv_label_set_text(lv_obj_t * obj, const char * text);`.
fn declared_function(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let head = line[..open].trim_end();
    if head.contains('=') || head.starts_with("return") || head.starts_with("typedef") {
        return None;
    }
    let name: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    // A return type must precede the name.
    let before = head[..head.len() - name.len()].trim();
    (name.starts_with("lv_") && !before.is_empty()).then_some(name)
}

/// Parameters whose `@param` text says they accept NULL.
fn nullable_params(doc: &str) -> Vec<String> {
    let mut nullable = Vec::new();
    let mut current: Option<(String, String)> = None;
    let mut finish = |entry: Option<(String, String)>| {
        if let Some((name, text)) = entry
            && (text.contains("@nullable")
                || text.contains("or NULL")
                || text.contains("can be NULL"))
        {
            nullable.push(name);
        }
    };
    for line in doc.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("@param") {
            finish(current.take());
            let mut words = rest.split_whitespace();
            if let Some(name) = words.next() {
                current = Some((name.to_owned(), words.collect::<Vec<_>>().join(" ")));
            }
        } else if line.starts_with('@') {
            finish(current.take());
        } else if let Some((_, text)) = current.as_mut() {
            text.push(' ');
            text.push_str(line);
        }
    }
    finish(current);
    nullable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_docs_deprecation_and_nullable_parameters() {
        let mut h = Headers::default();
        h.add_source(
            r#"
/**
 * Set a new text for a label.
 * @param obj           pointer to a label object
 * @param text          '\0' terminated character string. @nullable When NULL the label is
 *                      refreshed with its current text.
 */
void lv_label_set_text(lv_obj_t * obj, const char * text);

/**
 * Create a menu object
 */
LV_DEPRECATED(LV_MENU_DEPRECATED_MSG)
lv_obj_t * lv_menu_create(lv_obj_t * parent);

#define LV_FOO(x) lv_bar(x)
int32_t lv_obj_get_x(const lv_obj_t * obj);
"#,
        );
        let text = &h.functions["lv_label_set_text"];
        assert!(text.doc.starts_with("Set a new text for a label."));
        assert_eq!(text.nullable, ["text"]);
        assert!(!text.deprecated);
        assert!(h.functions["lv_menu_create"].deprecated);
        let x = &h.functions["lv_obj_get_x"];
        assert!(x.doc.is_empty() && !x.deprecated);
        assert!(!h.functions.contains_key("lv_bar"));
    }
}
