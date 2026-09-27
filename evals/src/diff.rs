//! `run-diff`: compare two `adopt-mine` reports and name what makes the subtraction honest.
//!
//! The instrument has changed shape on known dates, so a naive delta between an old window and a
//! new one silently mixes calibres. This module prints the deltas AND the breaks: every known
//! instant where the measuring itself changed that falls between the two windows. It also
//! refuses to diff reports from different machines -- a number that cannot say which machine
//! produced it is not comparable to anything (`usage.db` stamps `MACHINE_ID` for exactly this
//! reason).

use serde_json::{json, Map, Value};

/// Instants where the measuring instrument itself changed. A number from before and a number
/// from after are not the same number, whatever the subtraction says.
const KNOWN_BREAKS: &[(i64, &str)] = &[
    // 2026-09-27 00:00 +08: the parser admitted `cd X && grep` / `; echo; grep` populations
    // (eab18be9, searches denominator roughly doubled), the hook was muted into a pure recorder
    // (428e3398, injections are 0 by design after it), and agent-portal-web/api received
    // indexes (their outcome vocabulary shifted off `no_index*`).
    (
        1_790_438_400_000,
        "2026-09-27: parser widened (eab18be9), hook muted into a pure recorder (428e3398), \
         agent-portal-web/api indexed -- searches denominators and outcome vocabulary changed; \
         injections are 0 by design after this instant",
    ),
];

/// The scalar fields a diff is allowed to subtract, with the human-facing note each carries.
/// Only mechanism-stable counters belong here; anything whose meaning depends on the window's
/// calibre must ride the breaks list instead.
const SCALARS: &[(&str, &str)] = &[
    (
        "searches",
        "only comparable within one calibre; read the breaks",
    ),
    ("injections", "0 by design after the 2026-09-27 muting"),
    ("adopted_same_symbol", ""),
    ("adopted_other_symbol", ""),
    ("not_adopted", ""),
    ("shape_would_fire", "offline matcher upper bound"),
    ("sessions_in_window", ""),
    ("sidechain_files_read", ""),
    ("excluded_sessions", ""),
    ("files_unreadable", ""),
    ("lines_unparsed", ""),
    ("records_without_timestamp", ""),
];

pub fn diff(old: &Value, new: &Value) -> Result<Value, String> {
    let old_machine = old
        .get("machine")
        .and_then(|m| m.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    let new_machine = new
        .get("machine")
        .and_then(|m| m.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    // A diff of two machines' numbers is a diff of two computers: the 2026-09-03 reconciliation
    // of 417 fires against 2 happened because nobody asked this question first.
    if old_machine != new_machine {
        return Err(format!(
            "the two reports come from different machines ({old_machine} vs {new_machine}); \
             rows carry no machine of their own, so nothing can be separated after the fact"
        ));
    }
    let old_since = window_edge(old, "since_ms")?;
    let new_since = window_edge(new, "since_ms")?;

    let mut scalars = Map::new();
    for (field, note) in SCALARS {
        let (o, n) = (as_i64(old, field), as_i64(new, field));
        scalars.insert(
            (*field).to_string(),
            json!({
                "old": o,
                "new": n,
                "delta": n.map(|n| n - o.unwrap_or(0)),
                "note": note,
            }),
        );
    }

    let empty_demand = Value::Object(Map::new());
    let old_demand = old.get("demand_context").unwrap_or(&empty_demand);
    let new_demand = new.get("demand_context").unwrap_or(&empty_demand);
    let mut demand = Map::new();
    for key in demand_keys(old).chain(demand_keys(new)) {
        let (o, n) = (as_i64(old_demand, &key), as_i64(new_demand, &key));
        demand.insert(
            key,
            json!({"old": o, "new": n, "delta": n.map(|n| n - o.unwrap_or(0))}),
        );
    }

    let mut by_project = Map::new();
    let empty = Map::new();
    let old_projects = old
        .get("by_project")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let new_projects = new
        .get("by_project")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (name, _) in old_projects.iter().chain(new_projects.iter()) {
        if by_project.contains_key(name) {
            continue;
        }
        let get = |side: &Map<String, Value>, f: &str| {
            side.get(name)
                .and_then(|p| p.get(f))
                .and_then(Value::as_i64)
        };
        by_project.insert(
            name.clone(),
            json!({
                "searches": {"old": get(old_projects, "searches"), "new": get(new_projects, "searches")},
                "injections": {"old": get(old_projects, "injections"), "new": get(new_projects, "injections")},
            }),
        );
    }

    // A break matters three ways: inside the old window (that report mixes calibres on its own),
    // inside the new one (ditto), or between the two windows (the subtraction crosses it). An
    // open-ended window -- `until_ms: null` -- extends to now, so every later break falls inside
    // it; that is the honest reading of a window that was never closed.
    let edge = |report: &Value, field: &str| -> Option<i64> {
        report
            .get("window")
            .and_then(|w| w.get(field))
            .and_then(Value::as_i64)
    };
    let old_until = edge(old, "until_ms");
    let new_until = edge(new, "until_ms");
    let breaks: Vec<Value> = KNOWN_BREAKS
        .iter()
        .filter_map(|(at, why)| {
            let where_at = if *at > old_since && old_until.map_or(true, |u| *at <= u) {
                Some("inside_old_window")
            } else if *at > new_since && new_until.map_or(true, |u| *at <= u) {
                Some("inside_new_window")
            } else if old_until.map_or(true, |u| *at > u) && *at < new_since {
                Some("between_windows")
            } else {
                None
            };
            where_at.map(|w| json!({"at_ms": at, "where": w, "why": why}))
        })
        .collect();

    Ok(json!({
        "old_window": old.get("window").cloned().unwrap_or(Value::Null),
        "new_window": new.get("window").cloned().unwrap_or(Value::Null),
        "machine": old_machine,
        "scalars": scalars,
        "demand_context": demand,
        "by_project": by_project,
        "calibre_breaks": breaks,
        "reading": "deltas are subtraction, not causation; every entry in calibre_breaks is a \
                    change to the measuring instrument between these windows -- numbers on \
                    either side of one are different numbers whatever the delta says. injections \
                    are 0 by design after the 2026-09-27 muting: read demand_context, not \
                    adoption.",
    }))
}

fn window_edge(report: &Value, field: &str) -> Result<i64, String> {
    report
        .get("window")
        .and_then(|w| w.get(field))
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            "both reports must carry window.since_ms (re-run adopt-mine on this tree)".to_string()
        })
}

fn as_i64(container: &Value, field: &str) -> Option<i64> {
    container.get(field).and_then(Value::as_i64)
}

fn demand_keys(report: &Value) -> impl Iterator<Item = String> {
    report
        .get("demand_context")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
}
