//! `events.toml`: what the audit concludes about every event code of
//! `lv_event_code_t`. Generated next to `access.toml`; the generator turns
//! it into the handler mask and the sendable code types of `lvgl-rust`.
//!
//! Per code:
//! - `deleting`: a handler registered for it (or for every code), or the
//!   class event callbacks run by sending it, reach a deletion sink;
//! - `param`: the getters of the event parameter that code running for it
//!   calls (`lv_event_get_param`, or getters that return or dereference
//!   the parameter without a NULL check); safe code sends with a NULL
//!   parameter, so such codes are not sendable;
//! - `sendable`: `"no"` if `param` is not empty (or by hand), else
//!   `"exclusive"` if `deleting` or if what the send frees is held across
//!   a send of a shared code, else `"shared"`;
//! - `handler`: `"shared"` (handlers get `&Lvgl`), `"restricted"` (only
//!   queries; behaves as `"none"` until the restricted tier exists) or
//!   `"none"`. Hand decisions (`[event_policy]` in `audit.toml`) come
//!   first; `blockable_codes` are restricted; the other codes are shared
//!   unless the held-region analysis finds a hold it cannot resolve by
//!   making the freeing function exclusive.

use super::graph::{AuditConfig, Graph};
use super::ir::Module;
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeEntry {
    pub name: String,
    pub value: i64,
    pub handler: String,
    pub deleting: bool,
    pub param: Vec<String>,
    pub sendable: String,
    pub why: Vec<String>,
}

/// What the held-region analysis says about a code.
#[derive(Clone, Debug, Default)]
pub struct HeldFacts {
    /// Codes whose handlers must not get shared access: the witness.
    pub restrict: BTreeMap<i64, String>,
    /// Codes whose send frees memory held across a shared code: the witness.
    pub send_hazard: BTreeMap<i64, String>,
    /// Blockable codes with hazards (the witness), for the record.
    pub blocked: BTreeMap<i64, String>,
}

/// The real event codes: above `LV_EVENT_ALL` and below `LV_EVENT_LAST`
/// (custom codes, flags and sentinels are out of scope).
pub fn real_codes(codes: &[(String, i64)]) -> Vec<(String, i64)> {
    let last = codes
        .iter()
        .find(|(n, _)| n == "LV_EVENT_LAST")
        .map_or(i64::MAX, |(_, v)| *v);
    codes
        .iter()
        .filter(|(_, v)| *v > 0 && *v < last)
        .cloned()
        .collect()
}

pub fn decide(
    graph: &Graph,
    module: &Module,
    config: &AuditConfig,
    codes: &[(String, i64)],
    held: &HeldFacts,
) -> Result<Vec<CodeEntry>, String> {
    let sender = config
        .object_sender
        .as_deref()
        .ok_or("audit.toml: no object_sender")?;
    for name in config.event_policy.keys().chain(&config.blockable_codes) {
        if !codes.iter().any(|(n, _)| n == name) {
            return Err(format!("audit.toml: {name} is not an event code"));
        }
    }
    let deleting_codes = graph.deleting_codes();
    let mut entries = Vec::new();
    for (name, value) in real_codes(codes) {
        let mut why = Vec::new();
        // Handlers of the code (`deleting_codes`), or the class event
        // callbacks the send runs (which `send_deletes` also covers).
        let send_path = graph.send_deletes(sender, value);
        let deleting = deleting_codes.contains(&None)
            || deleting_codes.contains(&Some(value))
            || send_path.is_some();
        if let Some(path) = send_path {
            why.push(format!("deletes: {}", path.join(" -> ")));
        }
        let param = match graph.param_reads(
            module,
            sender,
            value,
            &config.param_getters,
            &config.param_checked_getters,
        ) {
            Some((getters, path)) => {
                why.push(format!("reads the parameter: {}", path.join(" -> ")));
                getters.into_iter().collect()
            }
            None => Vec::new(),
        };
        let policy = config.event_policy.get(&name);
        let handler = if let Some(h) = policy.and_then(|p| p.handler.clone()) {
            why.push(format!("handler: {}", policy.unwrap().why));
            h
        } else if config.blockable_codes.contains(&name) {
            match held.blocked.get(&value) {
                Some(w) => why.push(format!("handler restricted (blockable): {w}")),
                None => why.push(
                    "handler restricted (blockable query or draw code; no hold found)".into(),
                ),
            }
            "restricted".into()
        } else if let Some(w) = held.restrict.get(&value) {
            why.push(format!("handler restricted: {w}"));
            "restricted".into()
        } else {
            "shared".into()
        };
        if !matches!(handler.as_str(), "shared" | "restricted" | "none") {
            return Err(format!("audit.toml: {name}: unknown handler {handler:?}"));
        }
        let sendable = if let Some(s) = policy.and_then(|p| p.sendable.clone()) {
            if s != "no" {
                return Err(format!("audit.toml: {name}: sendable can only be \"no\""));
            }
            if policy.unwrap().handler.is_none() {
                why.push(format!("not sendable: {}", policy.unwrap().why));
            }
            s
        } else if !param.is_empty() {
            "no".into()
        } else if deleting {
            "exclusive".into()
        } else if let Some(w) = held.send_hazard.get(&value) {
            why.push(format!("send frees held memory: {w}"));
            "exclusive".into()
        } else {
            "shared".into()
        };
        entries.push(CodeEntry {
            name,
            value,
            handler,
            deleting,
            param,
            sendable,
            why,
        });
    }
    Ok(entries)
}

pub fn render(entries: &[CodeEntry]) -> String {
    let mut out = String::from(
        "# Generated by `lvgl-rust-gen audit` from the call graph of LVGL and\n\
         # audit.toml. Do not edit: change audit.toml (or the sources) and rerun\n\
         # `mise run audit`.\n",
    );
    for e in entries {
        writeln!(out, "\n[code.{}]", e.name).unwrap();
        writeln!(out, "value = {}", e.value).unwrap();
        writeln!(out, "handler = {:?}", e.handler).unwrap();
        writeln!(out, "deleting = {}", e.deleting).unwrap();
        writeln!(out, "param = {:?}", e.param).unwrap();
        writeln!(out, "sendable = {:?}", e.sendable).unwrap();
        if !e.why.is_empty() {
            writeln!(out, "why = {:?}", e.why.join("; ")).unwrap();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_deleting_param_and_sendable_per_code() {
        let mut m = Module::default();
        m.add_ir(
            "w",
            r#"
@lv_w_class = dso_local constant { ptr, ptr, ptr, ptr } { ptr null, ptr null, ptr null, ptr @w_event }, align 8
define internal void @w_event(ptr noundef %0, ptr noundef %1) {
  %3 = call i32 @lv_event_get_code(ptr noundef %1)
  switch i32 %3, label %9 [
    i32 21, label %4
    i32 10, label %6
    i32 39, label %7
  ]
4:
  %5 = call ptr @lv_event_get_param(ptr noundef %1)
  br label %9
6:
  call void @obj_delete_core(ptr %0)
  br label %9
7:
  %8 = call i32 @lv_event_get_key(ptr noundef %1)
  call void @helper(ptr noundef %1)
  br label %9
9:
  ret void
}
define internal void @helper(ptr noundef %0) {
  %2 = call i32 @lv_event_get_code(ptr noundef %0)
  ret void
}
define dso_local ptr @lv_event_get_param(ptr noundef %0) {
  ret ptr null
}
define dso_local i32 @lv_event_get_key(ptr noundef %0) {
  %2 = call ptr @lv_event_get_param(ptr noundef %0)
  ret i32 0
}
define internal void @obj_delete_core(ptr noundef %0) {
  ret void
}
define dso_local void @lv_obj_event_base(ptr noundef %0, ptr noundef %1) {
  %3 = getelementptr inbounds nuw %struct._lv_obj_class_t, ptr %0, i32 0, i32 3
  %4 = load ptr, ptr %3, align 8
  call void %4(ptr noundef %0, ptr noundef %1)
  ret void
}
define dso_local i32 @lv_obj_send_event(ptr noundef %0, i32 noundef %1, ptr noundef %2) {
  call void @lv_obj_event_base(ptr noundef %0, ptr noundef %0)
  ret i32 0
}
"#,
            &[("lv_*_class".into(), "_lv_obj_class_t".into())],
        );
        let config: AuditConfig = toml::from_str(
            r#"
sinks = ["w::obj_delete_core"]
deleting_sinks = ["w::obj_delete_core"]
object_sender = "lv_obj_send_event"
param_getters = ["lv_event_get_param"]
param_checked_getters = ["lv_event_get_key"]
blockable_codes = ["LV_EVENT_COVER_CHECK"]
[send]
lv_obj_send_event = 1
[event_policy.LV_EVENT_CREATE]
handler = "none"
sendable = "no"
why = "built"
"#,
        )
        .unwrap();
        let graph = Graph::build(&m, &config).unwrap();
        let codes: Vec<(String, i64)> = [
            ("LV_EVENT_ALL", 0),
            ("LV_EVENT_CLICKED", 10),
            ("LV_EVENT_KEY", 21),
            ("LV_EVENT_COVER_CHECK", 30),
            ("LV_EVENT_VALUE_CHANGED", 39),
            ("LV_EVENT_CREATE", 47),
            ("LV_EVENT_LAST", 80),
            ("LV_EVENT_PREPROCESS", 0x8000),
        ]
        .into_iter()
        .map(|(n, v)| (n.to_owned(), v))
        .collect();
        let entries = decide(&graph, &m, &config, &codes, &HeldFacts::default()).unwrap();
        let get = |n: &str| entries.iter().find(|e| e.name == n).unwrap();
        assert_eq!(entries.len(), 5, "only real codes");
        assert!(get("LV_EVENT_CLICKED").deleting);
        assert_eq!(get("LV_EVENT_CLICKED").sendable, "exclusive");
        assert_eq!(get("LV_EVENT_KEY").param, ["lv_event_get_param"]);
        assert_eq!(get("LV_EVENT_KEY").sendable, "no");
        // A checked getter is not a read, and is not followed.
        assert!(get("LV_EVENT_VALUE_CHANGED").param.is_empty());
        assert_eq!(get("LV_EVENT_VALUE_CHANGED").sendable, "shared");
        assert_eq!(get("LV_EVENT_VALUE_CHANGED").handler, "shared");
        assert_eq!(get("LV_EVENT_COVER_CHECK").handler, "restricted");
        assert_eq!(get("LV_EVENT_CREATE").handler, "none");
        assert_eq!(get("LV_EVENT_CREATE").sendable, "no");
        let rendered = render(&entries);
        assert!(rendered.contains("[code.LV_EVENT_KEY]\nvalue = 21\nhandler = \"shared\""));
    }
}
