//! Compiles LVGL, generates its Rust bindings and exports the native build
//! configuration to dependent build scripts (see the README).
//!
//! The same include paths and definitions feed `cc`, `bindgen` and the
//! exported metadata so that the native library, the bindings and downstream
//! C code (e.g. external fonts) always agree on the ABI.

mod config;

use config::{CFLAGS_VAR, CONFIG_DIR_VAR, ColorDepth, Define, FONTS_DIR_VAR};
use std::{
    env,
    path::{Path, PathBuf},
};

fn main() {
    let paths = Paths::new();
    let native = NativeConfig::from_env(&paths);

    let linked = cargo_feature("BUILD_LVGL");
    if linked {
        compile_lvgl(&native);
    }
    let bindings = PathBuf::from(cargo_env("OUT_DIR")).join("bindings.rs");
    native.export_metadata(&bindings, linked);
    generate_bindings(&native, &bindings);
}

/// Locations inside this crate.
struct Paths {
    lvgl: PathBuf,
    lvgl_src: PathBuf,
    lvgl_include: PathBuf,
    shims: PathBuf,
    vendored_config: PathBuf,
    shadcn_theme: PathBuf,
}

impl Paths {
    fn new() -> Self {
        let manifest = canonicalize(&PathBuf::from(cargo_env("CARGO_MANIFEST_DIR")));
        let lvgl = manifest.join("lvgl");
        Self {
            lvgl_src: lvgl.join("src"),
            lvgl_include: lvgl.join("include"),
            lvgl,
            shims: manifest.join("shims"),
            vendored_config: manifest.join("conf"),
            shadcn_theme: manifest.join("lvgl_theme_shadcn"),
        }
    }
}

/// Everything needed to compile LVGL-compatible C code.
struct NativeConfig {
    target: String,
    host: String,
    /// Directories whose `.c` files are compiled into the LVGL archive.
    source_dirs: Vec<PathBuf>,
    /// Headers handed to bindgen.
    headers: Vec<PathBuf>,
    include_paths: Vec<PathBuf>,
    /// Definitions derived from the crate configuration.
    defines: Vec<Define>,
    /// User supplied definitions from `LVGL_CFLAGS`.
    custom_defines: Vec<Define>,
}

impl NativeConfig {
    fn from_env(paths: &Paths) -> Self {
        let config_dir = config_dir(paths);
        let fonts_dir = env_var(FONTS_DIR_VAR).map(|dir| canonicalize(Path::new(&dir)));
        let custom_defines = config::parse_cflags(&env_var(CFLAGS_VAR).unwrap_or_default());

        let mut config = Self {
            target: cargo_env("TARGET"),
            host: cargo_env("HOST"),
            source_dirs: vec![
                paths.lvgl_src.clone(),
                config_dir.clone(),
                paths.shims.clone(),
            ],
            headers: vec![paths.shims.join("lvgl_sys.h")],
            include_paths: vec![
                paths.lvgl_src.clone(),
                // LVGL 9.6 public headers: resolves `lvgl/...` includes such
                // as the default `LV_PROFILER_INCLUDE`.
                paths.lvgl_include.clone(),
                // LVGL root: resolves `lvgl.h` and `include/lvgl/...`.
                paths.lvgl.clone(),
                config_dir.clone(),
                paths.shims.clone(),
            ],
            defines: vec![Define::new("LV_CONF_INCLUDE_SIMPLE", Some("1"))],
            custom_defines,
        };

        rerun_if_changed(&config_dir.join("lv_conf.h"));
        rerun_if_changed(&paths.lvgl_src);
        rerun_if_changed(&paths.lvgl_include);
        rerun_if_changed(&paths.lvgl.join("lvgl.h"));
        rerun_if_changed(&paths.shims);

        let color_depth =
            ColorDepth::from_features(|depth| cargo_feature(&format!("COLOR_DEPTH_{depth}")));
        if color_depth.is_conflicting() {
            println!(
                "cargo:warning=Multiple color depth features enabled ({:?}); using {}",
                color_depth.requested,
                color_depth.selected.unwrap()
            );
        }
        config.defines.extend(color_depth.defines());

        // Legacy mode: fonts are compiled into the LVGL archive, so any font
        // change rebuilds LVGL and regenerates the bindings.
        if let Some(fonts_dir) = fonts_dir {
            rerun_if_changed(&fonts_dir);
            config.headers.extend(headers_in(&fonts_dir));
            config.source_dirs.insert(0, fonts_dir.clone());
            config.include_paths.push(fonts_dir);
        }

        if cargo_feature("SHADCN_THEME") {
            let theme = &paths.shadcn_theme;
            rerun_if_changed(&theme.join("src"));
            rerun_if_changed(&theme.join("include"));
            config.source_dirs.push(theme.join("src"));
            config.include_paths.push(theme.join("include"));
            config.headers.push(theme.join("include/lv_theme_shadcn.h"));
        }

        config
    }

    fn all_defines(&self) -> impl Iterator<Item = &Define> {
        self.defines.iter().chain(&self.custom_defines)
    }

    fn is_cross_compiling(&self) -> bool {
        self.target != self.host
    }

    /// Publishes the configuration as `DEP_LVGL_RUST_SYS_*` variables.
    fn export_metadata(&self, bindings: &Path, linked: bool) {
        println!(
            "cargo:INCLUDE_PATHS={}",
            config::join_paths(&self.include_paths)
        );
        println!("cargo:DEFINES={}", config::join_defines(&self.defines, ";"));
        println!(
            "cargo:CFLAGS={}",
            config::join_defines(&self.custom_defines, ",")
        );
        println!("cargo:TARGET={}", self.target);
        println!("cargo:BINDINGS={}", bindings.display());
        println!("cargo:LINKED={}", u8::from(linked));
    }

    /// Arguments making clang see the headers exactly as the C compiler does.
    fn clang_args(&self) -> Vec<String> {
        let mut args: Vec<String> = self
            .include_paths
            .iter()
            .map(|path| format!("-I{}", path.display()))
            .collect();
        args.extend(self.all_defines().map(Define::to_compiler_arg));
        args.push("-fvisibility=default".into());

        if self.is_cross_compiling() {
            args.push(format!("--target={}", self.target));
        }
        if self.target.ends_with("emscripten") {
            let emsdk = env_var("EMSDK").expect(
                "The EMSDK environment variable is not set. Has emscripten been properly initialized?",
            );
            let system = Path::new(&emsdk).join("upstream/emscripten/system");
            for include in [
                "include/libc",
                "lib/libc/musl/arch/emscripten",
                "include/SDL",
            ] {
                args.push(format!("-I{}", system.join(include).display()));
            }
        }
        args
    }
}

fn config_dir(paths: &Paths) -> PathBuf {
    let vendored = (cargo_feature("USE_VENDORED_CONFIG") || env_var("DOCS_RS").is_some())
        .then(|| paths.vendored_config.clone());
    let dir = config::resolve_config_dir(env_var(CONFIG_DIR_VAR).map(PathBuf::from), vendored)
        .and_then(|dir| config::validate_config_dir(&dir).map(|()| dir))
        .unwrap_or_else(|error| panic!("{error}"));
    canonicalize(&dir)
}

fn compile_lvgl(native: &NativeConfig) {
    let mut build = cc::Build::new();
    build.warnings(true);
    for dir in &native.source_dirs {
        let sources = config::collect_files(dir, "c")
            .unwrap_or_else(|error| panic!("Cannot read {}: {error}", dir.display()));
        build.files(sources);
    }
    build.includes(&native.include_paths);
    for define in native.all_defines() {
        build.define(&define.name, define.value.as_deref());
    }
    build.compile("lvgl");
}

fn generate_bindings(native: &NativeConfig, output: &Path) {
    native
        .headers
        .iter()
        .fold(bindgen::Builder::default(), |builder, header| {
            builder.header(header.display().to_string())
        })
        .generate_comments(false)
        .derive_default(true)
        .layout_tests(false)
        .use_core()
        .ctypes_prefix("core::ffi")
        .clang_args(native.clang_args())
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(output)
        .expect("Unable to write bindings");
}

/// Top-level headers of `dir` (generated font headers are not nested).
fn headers_in(dir: &Path) -> Vec<PathBuf> {
    let mut headers: Vec<PathBuf> = dir
        .read_dir()
        .unwrap_or_else(|error| panic!("Cannot read {}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "h"))
        .collect();
    headers.sort();
    headers
}

/// Reads an environment variable (empty means unset) and tells Cargo to rerun
/// when it changes.
fn env_var(name: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={name}");
    env::var(name).ok().filter(|value| !value.is_empty())
}

/// Reads a variable that Cargo always sets for build scripts.
fn cargo_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("Cargo did not set {name}"))
}

/// Whether the Cargo feature with the given (upper snake case) name is enabled.
fn cargo_feature(name: &str) -> bool {
    env::var_os(format!("CARGO_FEATURE_{name}")).is_some()
}

fn rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

fn canonicalize(path: &Path) -> PathBuf {
    config::canonicalize(path)
        .unwrap_or_else(|error| panic!("Cannot resolve {}: {error}", path.display()))
}
