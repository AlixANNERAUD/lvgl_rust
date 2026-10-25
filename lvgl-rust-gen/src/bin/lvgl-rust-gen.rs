//! Maintainer tool (feature `cli`).
//!
//! `lvgl-rust-gen audit [--check] [--verbose]` compiles the vendored LVGL
//! with `lvgl-rust/conf` to LLVM IR (for a fixed target, so the result does
//! not depend on the host), builds its call graph, runs the held-region
//! analysis and rewrites `tables/lvgl-<version>/access.toml` and
//! `events.toml` from them and `audit.toml`. `--check` compares instead
//! (CI); `--verbose` lists the event handlers and class callbacks that
//! reach a sink, the held-region hazards and the review lists (aliases,
//! frees without clear, unknown holds), for review.
//!
//! `lvgl-rust-gen bump [--check] [--bindings <bindings.rs>]` rewrites
//! `universe.txt` from the bindings of `lvgl-rust/conf` (built in
//! `target/bump` unless given), and reports differences between
//! `[hierarchy]` and the classes of the LVGL sources and stale table
//! entries. `--check` fails on any of them instead of writing.

use lvgl_rust_gen::audit::{
    events,
    graph::{AuditConfig, Graph, Verdict},
    held,
    ir::{self, Module},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("audit") => audit(
            args.iter().any(|a| a == "--check"),
            args.iter().any(|a| a == "--verbose"),
        ),
        Some("bump") => bump(
            args.iter().any(|a| a == "--check"),
            args.iter()
                .position(|a| a == "--bindings")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from),
        ),
        _ => Err("usage: lvgl-rust-gen audit [--check] [--verbose]\n       \
                  lvgl-rust-gen bump [--check] [--bindings <bindings.rs>]"
            .to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn audit(check: bool, verbose: bool) -> Result<(), String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let sys = workspace.join("lvgl-rust-sys");
    let lvgl = sys.join("lvgl");
    let (major, minor) = lvgl_version(&lvgl)?;
    let tables = lvgl_rust_gen::tables_root().join(format!("lvgl-{major}.{minor}"));
    let read = |name: &str| {
        fs::read_to_string(tables.join(name))
            .map_err(|e| format!("{}: {e}", tables.join(name).display()))
    };
    let config: AuditConfig =
        toml::from_str(&read("audit.toml")?).map_err(|e| format!("audit.toml: {e}"))?;
    let classes: lvgl_rust_gen::tables::Classes =
        toml::from_str(&read("classes.toml")?).map_err(|e| format!("classes.toml: {e}"))?;
    let owned = classes
        .crate_owned
        .skip
        .iter()
        .chain(&classes.crate_owned.raw);
    let universe: BTreeSet<String> = read("universe.txt")?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect();

    let src = lvgl.join("src");
    let out_dir = workspace.join("target/audit-ir");
    let _ = fs::remove_dir_all(&out_dir);
    let clang = std::env::var("CLANG").unwrap_or_else(|_| "clang".into());
    let options = ir::CompileOptions {
        clang: &clang,
        include_dirs: vec![
            workspace.join("lvgl-rust/conf"),
            src.clone(),
            lvgl.join("include"),
            lvgl.clone(),
            sys.join("shims"),
            // No C library headers (the target is fixed): LVGL's builtin
            // stdlib needs only clang's own headers and `inttypes.h`.
            sys.join("shims/freestanding"),
        ],
        defines: vec!["LV_CONF_INCLUDE_SIMPLE=1".into()],
        out_dir: out_dir.clone(),
    };
    eprintln!("compiling LVGL to LLVM IR in {}", out_dir.display());
    let shims = sys.join("shims");
    let mut sources = ir::c_sources(&src);
    sources.extend(ir::c_sources(&shims));
    let irs = ir::compile(&sources, &options)?;
    let literal: Vec<(String, String)> = config
        .literal_globals
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut module = Module::default();
    for (source, path) in sources.iter().zip(&irs) {
        let base = if source.starts_with(&src) {
            &src
        } else {
            &shims
        };
        let unit = source.strip_prefix(base).unwrap().with_extension("");
        let unit = unit.to_string_lossy().replace('\\', "/");
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        module.add_ir(&unit, &text, &literal);
    }
    let graph = Graph::build(&module, &config)?;

    // Access of every public function from the graph and the hand tables.
    struct Entry {
        access: String,
        why: Option<String>,
        consumes: bool,
    }
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut fixed: BTreeSet<String> = BTreeSet::new();
    let mut owned_set: BTreeSet<String> = BTreeSet::new();
    for name in &universe {
        if config
            .ignore
            .iter()
            .any(|i| lvgl_rust_gen::tables::glob(i, name))
        {
            continue;
        }
        let entry = if owned.clone().any(|p| lvgl_rust_gen::tables::glob(p, name)) {
            fixed.insert(name.clone());
            owned_set.insert(name.clone());
            Entry {
                access: "raw".to_owned(),
                why: Some("crate-owned (classes.toml)".to_owned()),
                consumes: false,
            }
        } else if let Some(exception) = config.exceptions.get(name) {
            fixed.insert(name.clone());
            Entry {
                access: exception.access.clone(),
                why: Some(exception.why.clone()),
                consumes: exception.consumes,
            }
        } else if module.functions.contains_key(name) {
            match graph.verdict(name) {
                Verdict::Shared => Entry {
                    access: "shared".to_owned(),
                    why: None,
                    consumes: false,
                },
                Verdict::Exclusive(path) => Entry {
                    access: "exclusive".to_owned(),
                    why: Some(path.join(" -> ")),
                    consumes: graph.deletes(name),
                },
                Verdict::Raw(path) => Entry {
                    access: "raw".to_owned(),
                    why: Some(format!("schedules a deletion: {}", path.join(" -> "))),
                    consumes: false,
                },
            }
        } else {
            continue; // not compiled in this configuration (or a macro)
        };
        if classes.manual.iter().any(|m| m == name) {
            fixed.insert(name.clone());
        }
        entries.insert(name.clone(), entry);
    }

    // Event codes, and the held-region analysis.
    let codes = ir::event_codes(&options)?;
    let real = events::real_codes(&codes);
    let code_names: BTreeMap<i64, String> = real.iter().map(|(n, v)| (*v, n.clone())).collect();
    let tiers_before: BTreeMap<i64, String> = real
        .iter()
        .map(|(n, v)| {
            let tier = match config.event_policy.get(n).and_then(|p| p.handler.clone()) {
                Some(h) => h,
                None if config.blockable_codes.contains(n) => "restricted".to_owned(),
                None => "shared".to_owned(),
            };
            (*v, tier)
        })
        .collect();
    let access: BTreeMap<String, String> = entries
        .iter()
        .map(|(n, e)| (n.clone(), e.access.clone()))
        .collect();
    let analysis = held::analyze(
        &module,
        &graph,
        &config,
        &held::Inputs {
            access: &access,
            fixed: &fixed,
            owned: &owned_set,
            handler: &tiers_before,
            code_names: &code_names,
        },
    )?;
    for (name, why) in &analysis.promote {
        let entry = entries.get_mut(name).unwrap();
        entry.access = "exclusive".to_owned();
        entry.why = Some(why.clone());
        if classes.manual.iter().any(|m| m == name) {
            eprintln!("note: {name} is wrapped by hand: check that its wrapper is exclusive");
        }
    }
    for (code, why) in &analysis.restrict {
        eprintln!(
            "warning: {} cannot be shared with handlers: {why}",
            code_names[code]
        );
    }

    let mut table = String::from(
        "# Generated by `lvgl-rust-gen audit` from the call graph of LVGL and\n\
         # audit.toml. Do not edit: change audit.toml (or the sources) and rerun\n\
         # `mise run audit`.\n",
    );
    let mut counts = [0usize; 4];
    for (name, e) in &entries {
        match e.access.as_str() {
            "shared" => counts[0] += 1,
            "exclusive" => counts[1] += 1,
            "raw" => counts[2] += 1,
            _ => counts[3] += 1,
        }
        writeln!(table, "\n[fn.{name}]\naccess = \"{}\"", e.access).unwrap();
        if let Some(why) = &e.why {
            writeln!(table, "why = {:?}", why).unwrap();
        }
        if e.consumes {
            writeln!(table, "consumes = true").unwrap();
        }
    }
    if verbose {
        for (cb, path) in graph.deleting_slot_functions("_lv_obj_class_t.3") {
            eprintln!("class event_cb {cb} reaches a sink: {}", path.join(" -> "));
        }
        for (handler, codes) in &graph.handlers {
            if let Some(path) = graph.handler_path(handler) {
                eprintln!(
                    "handler {handler} {codes:?} reaches a sink: {}",
                    path.join(" -> ")
                );
            }
        }
        for line in &analysis.report {
            eprintln!("{line}");
        }
    } else if let Some(last) = analysis.report.last() {
        eprintln!("{last}");
    }
    if !analysis.uncleared.is_empty() {
        return Err(format!(
            "shared functions free memory a global still names (make them raw or crate-owned): {:?}",
            analysis.uncleared
        ));
    }
    let deleting: Vec<String> = graph
        .deleting_codes()
        .into_iter()
        .map(|c| c.map_or("ALL".to_owned(), |c| c.to_string()))
        .collect();
    eprintln!(
        "{} shared, {} exclusive ({} from held regions), {} raw, {} other; handlers delete on event codes {deleting:?}",
        counts[0],
        counts[1],
        analysis.promote.len(),
        counts[2],
        counts[3]
    );

    let held = events::HeldFacts {
        restrict: analysis.restrict.clone(),
        send_hazard: analysis.send_hazard.clone(),
        blocked: analysis.blocked.clone(),
    };
    let entries = events::decide(&graph, &module, &config, &codes, &held)?;
    let tiers = |tier: &str| entries.iter().filter(|e| e.handler == tier).count();
    eprintln!(
        "{} event codes: handlers {} shared, {} restricted, {} none; {} deleting; sendable {} shared, {} exclusive",
        entries.len(),
        tiers("shared"),
        tiers("restricted"),
        tiers("none"),
        entries.iter().filter(|e| e.deleting).count(),
        entries.iter().filter(|e| e.sendable == "shared").count(),
        entries.iter().filter(|e| e.sendable == "exclusive").count(),
    );
    let events_table = events::render(&entries);

    let mut stale = Vec::new();
    for (name, contents, marker) in [
        ("access.toml", &table, "\n[fn."),
        ("events.toml", &events_table, "\n[code."),
    ] {
        let path = tables.join(name);
        if check {
            let current = fs::read_to_string(&path).unwrap_or_default();
            if &current != contents {
                let differing = diff_entries(&current, contents, marker);
                stale.push(format!(
                    "{} is out of date with the audit ({} entries differ, e.g. {:?}); run `mise run audit`",
                    path.display(),
                    differing.len(),
                    differing.iter().take(5).collect::<Vec<_>>()
                ));
            } else {
                eprintln!("{} is up to date", path.display());
            }
        } else {
            fs::write(&path, contents).map_err(|e| e.to_string())?;
            eprintln!("wrote {}", path.display());
        }
    }
    if !stale.is_empty() {
        return Err(stale.join("\n"));
    }
    Ok(())
}

fn bump(check: bool, bindings: Option<PathBuf>) -> Result<(), String> {
    use lvgl_rust_gen::{bump, parse::Bindings, tables::Tables};
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let lvgl = workspace.join("lvgl-rust-sys/lvgl");
    let (major, minor) = lvgl_version(&lvgl)?;
    let tables_dir = lvgl_rust_gen::tables_root().join(format!("lvgl-{major}.{minor}"));
    let bindings = match bindings {
        Some(path) => path,
        None => build_bindings(workspace)?,
    };
    let source =
        fs::read_to_string(&bindings).map_err(|e| format!("{}: {e}", bindings.display()))?;
    let bindings = Bindings::parse(&source).map_err(|e| e.to_string())?;
    let mut problems = Vec::new();

    let universe = bump::universe(&bindings, &format!("{major}.{minor}"));
    let universe_path = tables_dir.join("universe.txt");
    let current = fs::read_to_string(&universe_path).unwrap_or_default();
    if current != universe {
        if check {
            problems.push(format!(
                "{} is out of date: run `mise run bump`",
                universe_path.display()
            ));
        } else {
            fs::write(&universe_path, &universe).map_err(|e| e.to_string())?;
            eprintln!("wrote {}", universe_path.display());
        }
    }

    let sources: Vec<String> = ir::c_sources(&lvgl.join("src"))
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect();
    let found = bump::class_bases(sources.iter().map(String::as_str));
    let tables = Tables::load(&tables_dir).map_err(|e| e.to_string())?;
    // Classes the bindings declare (private ones have no view type).
    let declared = |class: &str| bindings.statics.contains(&format!("lv_{class}_class"));
    for line in bump::hierarchy_diff(&found, &tables.classes.hierarchy, declared) {
        problems.push(format!("classes.toml [hierarchy]: {line}"));
    }
    problems.extend(bump::stale_entries(&tables, &bindings));

    if problems.is_empty() {
        eprintln!("tables are up to date; review `mise run audit` for access.toml");
        return Ok(());
    }
    let report = problems.join("\n");
    if check {
        Err(report)
    } else {
        eprintln!("{report}");
        Ok(())
    }
}

/// Builds `lvgl-rust-sys` with `lvgl-rust/conf` and returns its bindings.
fn build_bindings(workspace: &Path) -> Result<PathBuf, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args([
            "build",
            "-q",
            "-p",
            "lvgl-rust-sys",
            "--message-format=json",
        ])
        .env("DEP_LV_CONFIG_PATH", workspace.join("lvgl-rust/conf"))
        .env("CARGO_TARGET_DIR", workspace.join("target/bump"))
        .current_dir(workspace)
        .output()
        .map_err(|e| format!("cargo: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "building lvgl-rust-sys failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| l.contains("\"build-script-executed\"") && l.contains("lvgl-rust-sys"))
        .find_map(|l| {
            let start = l.find("\"out_dir\":\"")? + "\"out_dir\":\"".len();
            let end = start + l[start..].find('"')?;
            Some(PathBuf::from(&l[start..end]).join("bindings.rs"))
        })
        .ok_or_else(|| "no build script output for lvgl-rust-sys".to_owned())
}

fn lvgl_version(lvgl: &Path) -> Result<(u32, u32), String> {
    let header = fs::read_to_string(lvgl.join("include/lvgl/lv_version.h"))
        .or_else(|_| fs::read_to_string(lvgl.join("lv_version.h")))
        .map_err(|e| format!("lv_version.h: {e}"))?;
    let value = |name: &str| {
        header.lines().find_map(|l| {
            let rest = l
                .trim()
                .strip_prefix("#define")?
                .trim()
                .strip_prefix(name)?;
            rest.trim().parse().ok()
        })
    };
    Ok((
        value("LVGL_VERSION_MAJOR").ok_or("no LVGL_VERSION_MAJOR")?,
        value("LVGL_VERSION_MINOR").ok_or("no LVGL_VERSION_MINOR")?,
    ))
}

fn diff_entries(old: &str, new: &str, marker: &str) -> Vec<String> {
    let entries = |s: &str| -> BTreeSet<String> {
        s.split(marker)
            .skip(1)
            .map(|e| e.trim().to_owned())
            .collect()
    };
    let (old, new) = (entries(old), entries(new));
    old.symmetric_difference(&new)
        .map(|e| e.split(']').next().unwrap_or("").to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
