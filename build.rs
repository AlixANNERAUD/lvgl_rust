use cc::Build;
#[cfg(feature = "drivers")]
use std::collections::HashSet;
use std::{
    env,
    path::{Path, PathBuf},
};

static CONFIG_NAME: &str = "DEP_LV_CONFIG_PATH";

// See https://github.com/rust-lang/rust-bindgen/issues/687#issuecomment-450750547
#[cfg(feature = "drivers")]
#[derive(Debug)]
struct IgnoreMacros(HashSet<String>);
#[cfg(feature = "drivers")]
impl bindgen::callbacks::ParseCallbacks for IgnoreMacros {
    fn will_parse_macro(&self, name: &str) -> bindgen::callbacks::MacroParsingBehavior {
        if self.0.contains(name) {
            bindgen::callbacks::MacroParsingBehavior::Ignore
        } else {
            bindgen::callbacks::MacroParsingBehavior::Default
        }
    }
}

fn get_color_depth() -> Option<&'static str> {
    if std::env::var_os("CARGO_FEATURE_COLOR_DEPTH_1").is_some() {
        Some("1")
    } else if std::env::var_os("CARGO_FEATURE_COLOR_DEPTH_8").is_some() {
        Some("8")
    } else if std::env::var_os("CARGO_FEATURE_COLOR_DEPTH_16").is_some() {
        Some("16")
    } else if std::env::var_os("CARGO_FEATURE_COLOR_DEPTH_24").is_some() {
        Some("24")
    } else if std::env::var_os("CARGO_FEATURE_COLOR_DEPTH_32").is_some() {
        Some("32")
    } else {
        None
    }
}

fn main() {
    let project_dir = canonicalize(PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()));
    let shims_dir = project_dir.join("shims");
    let vendor = project_dir.join("vendor");
    let lvgl_src = project_dir.join("lvgl").join("src");

    println!(
        "cargo:rerun-if-changed={}",
        shims_dir.join("lvgl_sys.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        shims_dir.join("lvgl_sys.c").display()
    );

    #[cfg(feature = "rust_timer")]
    let timer_shim = vendor.join("include").join("timer");

    println!("cargo:rerun-if-env-changed=LVGL_FONTS_DIR");
    let font_extra_src = env::var("LVGL_FONTS_DIR")
        .ok()
        .map(|path| canonicalize(PathBuf::from(path)));

    // Some basic defaults; SDL2 is the only driver enabled in the provided
    // driver config by default
    #[cfg(feature = "drivers")]
    let incl_extra =
        env::var("LVGL_INCLUDE").unwrap_or("/usr/include,/usr/local/include".to_string());

    let cflags_extra_string = env::var("LVGL_CFLAGS").unwrap_or_default();

    let cflags_extra = if cflags_extra_string.is_empty() {
        None
    } else {
        Some(cflags_extra_string.split(','))
    };

    #[cfg(feature = "drivers")]
    let link_extra = env::var("LVGL_LINK").unwrap_or("SDL2".to_string());

    #[cfg(feature = "drivers")]
    let drivers = vendor.join("lv_drivers");

    let lv_config_dir = {
        let conf_path = env::var(CONFIG_NAME)
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                match std::env::var("DOCS_RS") {
                    Ok(_) => {
                        // We've detected that we are building for docs.rs
                        // so let's use the vendored `lv_conf.h` file.
                        vendor.join("include")
                    }
                    Err(_) => {
                        #[cfg(not(feature = "use-vendored-config"))]
                        panic!(
                            "The environment variable {} is required to be defined",
                            CONFIG_NAME
                        );

                        #[cfg(feature = "use-vendored-config")]
                        vendor.join("include")
                    }
                }
            });

        if !conf_path.exists() {
            panic!(
                "Directory {} referenced by {} needs to exist",
                conf_path.to_string_lossy(),
                CONFIG_NAME
            );
        }
        if !conf_path.is_dir() {
            panic!("{} needs to be a directory", CONFIG_NAME);
        }
        if !conf_path.join("lv_conf.h").exists() {
            panic!(
                "Directory {} referenced by {} needs to contain a file called lv_conf.h",
                conf_path.to_string_lossy(),
                CONFIG_NAME
            );
        }
        #[cfg(feature = "drivers")]
        if !conf_path.join("lv_drv_conf.h").exists() {
            panic!(
                "Directory {} referenced by {} needs to contain a file called lv_drv_conf.h",
                conf_path.to_string_lossy(),
                CONFIG_NAME
            );
        }

        if let Some(p) = &font_extra_src {
            println!("cargo:rerun-if-changed={}", p.to_str().unwrap())
        }

        println!(
            "cargo:rerun-if-changed={}",
            conf_path.join("lv_conf.h").to_str().unwrap()
        );
        #[cfg(feature = "drivers")]
        println!(
            "cargo:rerun-if-changed={}",
            conf_path.join("lv_drv_conf.h").to_str().unwrap()
        );
        conf_path
    };

    #[cfg(feature = "drivers")]
    {
        println!("cargo:rerun-if-env-changed=LVGL_INCLUDE");
        println!("cargo:rerun-if-env-changed=LVGL_LINK");
    }

    let mut native_includes = vec![
        lvgl_src.clone(),
        vendor.clone(),
        lv_config_dir.clone(),
        shims_dir.clone(),
    ];
    if let Some(font_dir) = &font_extra_src {
        native_includes.push(font_dir.clone());
    }
    let mut native_defines = vec![("LV_CONF_INCLUDE_SIMPLE".to_string(), Some("1".to_string()))];

    #[cfg(feature = "shadcn-theme")]
    {
        let theme = project_dir.join("lvgl_theme_shadcn");
        native_includes.push(theme.join("include"));
        native_includes.push(project_dir.join("lvgl"));
    }
    #[cfg(feature = "rust_timer")]
    native_includes.push(timer_shim.clone());
    #[cfg(feature = "drivers")]
    {
        native_includes.push(drivers.clone());
        native_includes.extend(incl_extra.split(',').map(PathBuf::from));
    }

    if let Some(ref cflags_extra) = cflags_extra {
        cflags_extra.clone().for_each(|entry| {
            let mut it = entry.splitn(2, '=');
            native_defines.push((
                it.next().unwrap().to_string(),
                it.next().map(str::to_string),
            ));
        });
    }

    if let Some(depth) = get_color_depth() {
        native_defines.push(("LV_COLOR_DEPTH".to_string(), Some(depth.to_string())));
    }

    let mut cfg = Build::new();
    if let Some(p) = &font_extra_src {
        add_c_files(&mut cfg, p)
    }
    add_c_files(&mut cfg, &lvgl_src);
    add_c_files(&mut cfg, &lv_config_dir);
    add_c_files(&mut cfg, &shims_dir);
    #[cfg(feature = "shadcn-theme")]
    {
        let theme = project_dir.join("lvgl_theme_shadcn");
        println!("cargo:rerun-if-changed={}", theme.join("src").display());
        println!("cargo:rerun-if-changed={}", theme.join("include").display());
        add_c_files(&mut cfg, &theme.join("src"));
    }
    #[cfg(feature = "drivers")]
    add_c_files(&mut cfg, &drivers);

    cfg.warnings(true);
    for include in &native_includes {
        cfg.include(include);
    }
    for (name, value) in &native_defines {
        cfg.define(name, value.as_deref());
    }
    // Bindgen is useful independently of the native library build (for
    // example, when another crate provides or builds LVGL itself).
    if env::var_os("CARGO_FEATURE_BUILD_LVGL").is_some() {
        cfg.compile("lvgl");
    }

    println!("cargo:rerun-if-env-changed=LVGL_CFLAGS");
    println!(
        "cargo:INCLUDE_PATHS={}",
        native_includes
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(";")
    );
    println!(
        "cargo:DEFINES={}",
        native_defines
            .iter()
            .map(|(name, value)| match value {
                Some(value) => format!("{name}={value}"),
                None => name.clone(),
            })
            .collect::<Vec<_>>()
            .join(";")
    );
    println!("cargo:CFLAGS={}", cflags_extra_string);
    println!(
        "cargo:TARGET={}",
        env::var("TARGET").expect("Cargo build scripts always have TARGET")
    );

    let mut cc_args = vec![
        "-DLV_CONF_INCLUDE_SIMPLE=1",
        "-I",
        lv_config_dir.to_str().unwrap(),
        "-I",
        vendor.to_str().unwrap(),
        "-fvisibility=default",
    ];

    // Set correct target triple for bindgen when cross-compiling
    let target = env::var("TARGET").expect("Cargo build scripts always have TARGET");
    let host = env::var("HOST").expect("Cargo build scripts always have HOST");
    if target != host {
        cc_args.push("-target");
        cc_args.push(target.as_str());
    }

    let mut additional_args = Vec::new();
    if target.ends_with("emscripten") {
        match env::var("EMSDK") {
            Ok(em_path) =>
        {
            additional_args.push("-I".to_string());
            additional_args.push(format!(
                "{}/upstream/emscripten/system/include/libc",
                em_path
            ));
            additional_args.push("-I".to_string());
            additional_args.push(format!(
                "{}/upstream/emscripten/system/lib/libc/musl/arch/emscripten",
                em_path
            ));
            additional_args.push("-I".to_string());
            additional_args.push(format!(
                "{}/upstream/emscripten/system/include/SDL",
                em_path
            ));
        }
        Err(_) => panic!("The EMSDK environment variable is not set. Has emscripten been properly initialized?")
        }
    }

    #[cfg(feature = "drivers")]
    let ignored_macros = IgnoreMacros(
        vec![
            "FP_INFINITE".into(),
            "FP_NAN".into(),
            "FP_NORMAL".into(),
            "FP_SUBNORMAL".into(),
            "FP_ZERO".into(),
            "IPPORT_RESERVED".into(),
        ]
        .into_iter()
        .collect(),
    );

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let bindings =
        bindgen::Builder::default().header(shims_dir.join("lvgl_sys.h").to_str().unwrap());
    let bindings = add_font_headers(bindings, &font_extra_src);
    #[cfg(feature = "shadcn-theme")]
    let bindings = bindings
        .header(
            project_dir
                .join("lvgl_theme_shadcn/include/lv_theme_shadcn.h")
                .to_str()
                .unwrap(),
        )
        .clang_arg(format!("-I{}", project_dir.join("lvgl").display()));
    #[cfg(feature = "drivers")]
    let bindings = bindings
        .header(shims_dir.join("lvgl_drv.h").to_str().unwrap())
        .parse_callbacks(Box::new(ignored_macros));
    #[cfg(feature = "rust_timer")]
    let bindings = bindings.header(shims_dir.join("rs_timer.h").to_str().unwrap());

    let bindings = bindings
        .generate_comments(false)
        .derive_default(true)
        .layout_tests(false)
        .use_core()
        .ctypes_prefix("core::ffi")
        .clang_args(&cc_args)
        .clang_args(&additional_args)
        .clang_args(
            cflags_extra
                .map(|s| s.collect::<Vec<_>>())
                .unwrap_or(Vec::new()),
        )
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Can't write bindings!");

    #[cfg(feature = "drivers")]
    link_extra.split(',').for_each(|a| {
        println!("cargo:rustc-link-lib={a}");
        //println!("cargo:rustc-link-search=")
    })
}

fn add_font_headers(
    bindings: bindgen::Builder,
    dir: &Option<impl AsRef<Path>>,
) -> bindgen::Builder {
    if let Some(p) = dir {
        let mut temp = bindings;
        for e in p.as_ref().read_dir().unwrap() {
            let e = e.unwrap();
            let path = e.path();
            if !e.file_type().unwrap().is_dir()
                && path.extension().and_then(|s| s.to_str()) == Some("h")
            {
                temp = temp.header(path.to_str().unwrap());
            }
        }
        temp
    } else {
        bindings
    }
}

fn add_c_files(build: &mut cc::Build, path: impl AsRef<Path>) {
    for e in path.as_ref().read_dir().unwrap() {
        let e = e.unwrap();
        let path = e.path();
        if e.file_type().unwrap().is_dir() {
            add_c_files(build, e.path());
        } else if path.extension().and_then(|s| s.to_str()) == Some("c") {
            build.file(&path);
        }
    }
}

fn canonicalize(path: impl AsRef<Path>) -> PathBuf {
    let canonicalized = path.as_ref().canonicalize().unwrap();
    let canonicalized = &*canonicalized.to_string_lossy();

    PathBuf::from(canonicalized.strip_prefix(r"\\?\").unwrap_or(canonicalized))
}
