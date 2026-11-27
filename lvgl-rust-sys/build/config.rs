//! Pure build configuration logic.
//!
//! This module only depends on `std` so it can be unit tested from
//! `tests/build_config.rs` without running Cargo, `cc` or `bindgen`.

use std::{
    fmt,
    path::{Path, PathBuf},
};

/// Directory containing `lv_conf.h`.
pub const CONFIG_DIR_VAR: &str = "DEP_LV_CONFIG_PATH";
/// Legacy mode: generated font sources compiled into the LVGL archive.
pub const FONTS_DIR_VAR: &str = "LVGL_FONTS_DIR";
/// Extra comma-separated `NAME` or `NAME=VALUE` preprocessor definitions.
pub const CFLAGS_VAR: &str = "LVGL_CFLAGS";

/// Color depths selectable through the `color_depth_*` Cargo features.
pub const COLOR_DEPTHS: [u8; 5] = [1, 8, 16, 24, 32];

/// A C preprocessor definition (`NAME` or `NAME=VALUE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Define {
    pub name: String,
    pub value: Option<String>,
}

impl Define {
    pub fn new(name: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        Self {
            name: name.into(),
            value: value.map(Into::into),
        }
    }

    /// Parses `NAME` or `NAME=VALUE`, returning `None` for blank entries.
    pub fn parse(entry: &str) -> Option<Self> {
        let entry = entry.trim();
        if entry.is_empty() {
            return None;
        }
        Some(match entry.split_once('=') {
            Some((name, value)) => Self::new(name.trim(), Some(value.trim())),
            None => Self::new(entry, None::<String>),
        })
    }

    /// Renders the definition as a clang/gcc command line argument.
    pub fn to_compiler_arg(&self) -> String {
        format!("-D{self}")
    }
}

impl fmt::Display for Define {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.value {
            Some(value) => write!(f, "{}={}", self.name, value),
            None => f.write_str(&self.name),
        }
    }
}

/// Parses the comma-separated `LVGL_CFLAGS` syntax.
pub fn parse_cflags(raw: &str) -> Vec<Define> {
    raw.split(',').filter_map(Define::parse).collect()
}

/// Joins definitions with `separator`, as exported through Cargo metadata.
pub fn join_defines(defines: &[Define], separator: &str) -> String {
    defines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(separator)
}

/// Joins paths with `;`, as exported through Cargo metadata.
pub fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(";")
}

/// Selected color depth and every depth that was requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorDepth {
    pub selected: Option<u8>,
    pub requested: Vec<u8>,
}

impl ColorDepth {
    /// Selects the smallest requested depth. Cargo features are additive, so
    /// conflicting depths (e.g. under `--all-features`) are reported rather
    /// than rejected.
    pub fn from_features(is_enabled: impl Fn(u8) -> bool) -> Self {
        let requested: Vec<u8> = COLOR_DEPTHS
            .into_iter()
            .filter(|depth| is_enabled(*depth))
            .collect();
        Self {
            selected: requested.first().copied(),
            requested,
        }
    }

    pub fn is_conflicting(&self) -> bool {
        self.requested.len() > 1
    }

    /// Definitions selecting the depth. LVGL 9 configures the color format;
    /// `LV_COLOR_DEPTH` is set alongside so both always agree.
    pub fn defines(&self) -> Vec<Define> {
        self.selected
            .map(|depth| {
                vec![
                    Define::new("LV_COLOR_FORMAT_DEFAULT", Some(color_format(depth))),
                    Define::new("LV_COLOR_DEPTH", Some(depth.to_string())),
                ]
            })
            .unwrap_or_default()
    }
}

/// LVGL color format matching a color depth.
pub fn color_format(depth: u8) -> &'static str {
    match depth {
        1 => "LV_COLOR_FORMAT_I1",
        8 => "LV_COLOR_FORMAT_L8",
        16 => "LV_COLOR_FORMAT_RGB565",
        24 => "LV_COLOR_FORMAT_RGB888",
        32 => "LV_COLOR_FORMAT_XRGB8888",
        _ => panic!("Unsupported color depth {depth}"),
    }
}

/// Whether `target` has no C library, so LVGL needs the freestanding headers
/// in `shims/freestanding` (clang's `inttypes.h` defers to the C library's).
pub fn is_freestanding(target: &str) -> bool {
    target.starts_with("wasm") && target.ends_with("-unknown-unknown")
}

/// Selects the RISC-V architecture and calling convention for C compilation.
/// Clang's normalized target triple loses the Rust architecture extensions,
/// and GNU bare-metal defaults can select the soft-float calling convention.
pub fn riscv_compiler_flags(target: &str, target_calling_convention: &str) -> Vec<String> {
    let architecture = target.split('-').next().unwrap_or_default();
    let (width, extensions) = if let Some(extensions) = architecture.strip_prefix("riscv32") {
        (32, extensions)
    } else if let Some(extensions) = architecture.strip_prefix("riscv64") {
        (64, extensions)
    } else {
        return Vec::new();
    };

    if !extensions.starts_with(['i', 'e', 'g'])
        || !extensions.bytes().all(|extension| {
            matches!(
                extension,
                b'i' | b'e' | b'm' | b'a' | b'f' | b'd' | b'c' | b'g'
            )
        })
    {
        return Vec::new();
    }

    let calling_convention = match target_calling_convention {
        "ilp32" | "ilp32e" | "ilp32f" | "ilp32d" | "lp64" | "lp64f" | "lp64d" => {
            target_calling_convention
        }
        _ => match (width, extensions) {
            (32, extensions) if extensions.starts_with('e') => "ilp32e",
            (32, extensions) if extensions.contains(['g', 'd']) => "ilp32d",
            (32, extensions) if extensions.contains('f') => "ilp32f",
            (32, _) => "ilp32",
            (_, extensions) if extensions.contains(['g', 'd']) => "lp64d",
            (_, extensions) if extensions.contains('f') => "lp64f",
            _ => "lp64",
        },
    };

    vec![
        format!("-march=rv{width}{extensions}"),
        format!("-mabi={calling_convention}"),
    ]
}

/// Selects `-mlongcalls` for Xtensa C compilation so calls from flash reach
/// ROM functions beyond the 18-bit `call8` range.
pub fn xtensa_compiler_flags(target: &str, target_arch: &str) -> Vec<String> {
    if target_arch == "xtensa" || target.split('-').next() == Some("xtensa") {
        vec!["-mlongcalls".into()]
    } else {
        Vec::new()
    }
}

/// Picks the configuration directory: the explicit one if given, otherwise the
/// vendored fallback when it is allowed.
pub fn resolve_config_dir(
    explicit: Option<PathBuf>,
    vendored: Option<PathBuf>,
) -> Result<PathBuf, String> {
    explicit.or(vendored).ok_or_else(|| {
        format!(
            "The environment variable {CONFIG_DIR_VAR} must point to a directory containing \
             lv_conf.h (or enable the `use-vendored-config` feature)"
        )
    })
}

/// Checks that `dir` is a directory containing `lv_conf.h`.
pub fn validate_config_dir(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!(
            "{} referenced by {CONFIG_DIR_VAR} must be an existing directory",
            dir.display()
        ));
    }
    if !dir.join("lv_conf.h").is_file() {
        return Err(format!(
            "{} referenced by {CONFIG_DIR_VAR} must contain lv_conf.h",
            dir.display()
        ));
    }
    Ok(())
}

/// Recursively collects files with `extension` under `dir`, sorted so that the
/// archive layout does not depend on directory iteration order.
pub fn collect_files(dir: &Path, extension: &str) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_files_into(dir, extension, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_into(
    dir: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for entry in dir.read_dir()? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files_into(&path, extension, files)?;
        } else if path.extension().is_some_and(|ext| ext == extension) {
            files.push(path);
        }
    }
    Ok(())
}

/// Canonicalizes `path`, stripping the Windows verbatim prefix that C
/// compilers do not understand.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = path.canonicalize()?;
    let canonical = canonical.to_string_lossy();
    Ok(PathBuf::from(
        canonical.strip_prefix(r"\\?\").unwrap_or(&canonical),
    ))
}
