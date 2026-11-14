//! Would recall's matching give today's answer if SQL narrowed the rows first?
//!
//! Read-only. Replays every real `cort memory recall "<query>"` found in Claude Code transcripts
//! (the walk and the query extraction are `recall_hits`'s) twice: once through today's
//! `recall_selection` over every active row as stored (the baseline), and once per SQL matching
//! form over the raw `project_memory` rows, where the server drops replaced entries and narrows by
//! text and the client then applies the exact decision to what came back. The arms are correct
//! only if the SQL is a superset of what the client would match, which is what this measures.

use crate::recall_hits::queries;
use cort::memory::{recall_selection, MemoryEntry};
use cort::remote::{Cell, RemoteDb};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// The lowercase haystack recall matches on: `key + " " + claim`.
const HAYSTACK: &str = "lower(json_extract(body,'$.key') || ' ' || json_extract(body,'$.claim'))";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    Like,
    Instr,
}

impl Arm {
    pub const ALL: [Arm; 2] = [Arm::Like, Arm::Instr];
    pub fn name(self) -> &'static str {
        match self {
            Arm::Like => "like",
            Arm::Instr => "instr",
        }
    }
    fn clause(self) -> String {
        match self {
            Arm::Like => format!("{HAYSTACK} LIKE '%' || ? || '%' ESCAPE '\\'"),
            Arm::Instr => format!("instr({HAYSTACK}, ?) > 0"),
        }
    }
    fn bind(self, term: &str) -> String {
        match self {
            Arm::Like => like_escape(term),
            Arm::Instr => term.to_string(),
        }
    }
}

/// Escape `%`, `_` and `\` so a term matches literally under `LIKE ... ESCAPE '\'`.
pub fn like_escape(term: &str) -> String {
    let mut out = String::with_capacity(term.len());
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `cjk` (any non-ASCII char), else `identifier` (a term with `- _ . / :` or letters mixed with
/// digits), else `english`.
pub fn group_of(query: &str) -> &'static str {
    if !query.is_ascii() {
        return "cjk";
    }
    let identifier = query.split_whitespace().any(|t| {
        t.contains(['-', '_', '.', '/', ':'])
            || (t.chars().any(|c| c.is_ascii_alphabetic()) && t.chars().any(|c| c.is_ascii_digit()))
    });
    if identifier {
        "identifier"
    } else {
        "english"
    }
}

fn marks(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// SQL predicate and bindings for "row is active among `projects`": not named by any `supersedes`
/// or `resolves` element of a row of those projects. Appends bindings to `args`.
fn active_filter(projects: &[String], args: &mut Vec<Cell>) -> String {
    let m = marks(projects.len());
    let push = |args: &mut Vec<Cell>| args.extend(projects.iter().cloned().map(Cell::Text));
    push(args);
    push(args);
    push(args);
    format!(
        "project IN ({m}) AND id NOT IN (\
         SELECT json_extract(body,'$.supersedes') FROM project_memory \
         WHERE project IN ({m}) AND json_extract(body,'$.supersedes') IS NOT NULL \
         UNION SELECT j.value FROM project_memory p, json_each(p.body,'$.resolves') j \
         WHERE p.project IN ({m}) AND j.value IS NOT NULL)"
    )
}

fn fetch(
    db: &RemoteDb,
    projects: &[String],
    arm: Option<(Arm, &[String], &str)>,
) -> Result<Vec<MemoryEntry>, String> {
    let mut args = Vec::new();
    let mut sql = format!("SELECT body FROM project_memory WHERE {}", {
        active_filter(projects, &mut args)
    });
    if let Some((arm, terms, joiner)) = arm {
        if !terms.is_empty() {
            let clause = arm.clause();
            let parts = vec![clause; terms.len()].join(&format!(" {joiner} "));
            sql.push_str(&format!(" AND ({parts})"));
            args.extend(terms.iter().map(|t| Cell::Text(arm.bind(t))));
        }
    }
    sql.push_str(" ORDER BY id");
    let result = db.execute(&sql, args).map_err(|e| e.to_string())?;
    result
        .rows
        .iter()
        .map(|row| match row.first() {
            Some(Cell::Text(body)) => {
                serde_json::from_str::<MemoryEntry>(body).map_err(|e| format!("body: {e}"))
            }
            _ => Err("body cell is not text".to_string()),
        })
        .collect()
}

/// Ids of the active entries by SQL alone, no term filter.
pub fn active_ids(db: &RemoteDb, projects: &[String]) -> Result<BTreeSet<String>, String> {
    Ok(fetch(db, projects, None)?
        .into_iter()
        .map(|e| e.id)
        .collect())
}

/// One arm's replay of `query`: the client's decision over the rows SQL returned, and how many
/// rows crossed the wire (both stages).
pub fn replay_arm(
    db: &RemoteDb,
    projects: &[String],
    query: &str,
    arm: Arm,
) -> Result<(Vec<String>, bool, usize), String> {
    let terms: Vec<String> = query.split_whitespace().map(|s| s.to_lowercase()).collect();
    let mut rows = fetch(db, projects, Some((arm, &terms, "AND")))?;
    let mut transferred = rows.len();
    if rows.is_empty() && terms.len() >= 2 {
        rows = fetch(db, projects, Some((arm, &terms, "OR")))?;
        transferred += rows.len();
    }
    let (ids, partial) = recall_selection(query, &rows);
    Ok((ids, partial, transferred))
}

#[derive(Default)]
struct Group {
    queries: u64,
    equal: BTreeMap<&'static str, u64>,
    rows: BTreeMap<&'static str, Vec<u64>>,
    ms: BTreeMap<&'static str, Vec<u64>>,
}

fn sum_max(v: &[u64]) -> Value {
    json!({"sum": v.iter().sum::<u64>(), "max": v.iter().max().copied().unwrap_or(0)})
}

fn p50_max(v: &[u64]) -> Value {
    let mut s = v.to_vec();
    s.sort_unstable();
    json!({"p50": s.get(s.len() / 2).copied().unwrap_or(0), "max": s.last().copied().unwrap_or(0)})
}

/// Replay `queries` (deduplicated by the caller or not: each occurrence counts) and build the report.
pub fn report(
    db: &RemoteDb,
    projects: &[String],
    ledger: &[MemoryEntry],
    queries: &[String],
) -> Result<Value, String> {
    // The baseline is today's decision over every active row as stored (nanosecond `recorded_at`, the
    // bodies `cort memory recall` decodes), so it differs from an arm only by the narrowing. The
    // `--ledger` snapshot only checks that the SQL active filter agrees with cort's own rule.
    let active_rows = fetch(db, projects, None)?;
    let sql_active: BTreeSet<String> = active_rows.iter().map(|e| e.id.clone()).collect();
    let ledger_active: BTreeSet<String> = ledger.iter().map(|e| e.id.clone()).collect();
    let only = |a: &BTreeSet<String>, b: &BTreeSet<String>| -> Vec<String> {
        a.difference(b).cloned().collect()
    };
    let active_check = json!({
        "sql_active": sql_active.len(),
        "ledger_active": ledger_active.len(),
        "only_in_sql": only(&sql_active, &ledger_active),
        "only_in_ledger": only(&ledger_active, &sql_active),
    });
    let mut groups: BTreeMap<&'static str, Group> = BTreeMap::new();
    let mut mismatches = Vec::new();
    for query in queries {
        let group = group_of(query);
        let (baseline, _) = recall_selection(query, &active_rows);
        let g = groups.entry(group).or_default();
        g.queries += 1;
        for arm in Arm::ALL {
            let started = Instant::now();
            let (got, partial, rows) = replay_arm(db, projects, query, arm)?;
            let ms = started.elapsed().as_millis() as u64;
            g.rows.entry(arm.name()).or_default().push(rows as u64);
            g.ms.entry(arm.name()).or_default().push(ms);
            if got == baseline {
                *g.equal.entry(arm.name()).or_default() += 1;
            } else {
                mismatches.push(json!({"query": query, "group": group, "arm": arm.name(),
                    "baseline": baseline, "got": got, "partial": partial}));
            }
        }
    }
    let mut out = Map::new();
    for (name, g) in groups {
        let mut o = Map::new();
        o.insert("queries".into(), json!(g.queries));
        for arm in Arm::ALL {
            let a = arm.name();
            o.insert(
                format!("{a}_equal"),
                json!(g.equal.get(a).copied().unwrap_or(0)),
            );
            let empty = Vec::new();
            o.insert(
                format!("rows_{a}"),
                sum_max(g.rows.get(a).unwrap_or(&empty)),
            );
            o.insert(format!("ms_{a}"), p50_max(g.ms.get(a).unwrap_or(&empty)));
        }
        out.insert(name.to_string(), Value::Object(o));
    }
    Ok(json!({
        "report": "recall-sql",
        "projects": projects,
        "queries": queries.len(),
        "active_check": active_check,
        "groups": Value::Object(out),
        "mismatches": mismatches,
    }))
}

/// The queries `recall-hits` would replay, in transcript order, one per call.
pub fn transcript_queries(
    dir: &std::path::Path,
    project: &str,
    days: Option<u64>,
    now_ms: i64,
) -> Result<Vec<String>, String> {
    queries(dir, Some(project), days, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cort::remote::SqliteTransport;

    fn db() -> RemoteDb {
        let db = RemoteDb::new(
            "turso://unit.test",
            "test",
            Box::new(SqliteTransport::new().unwrap()),
        )
        .unwrap();
        db.execute(
            "CREATE TABLE project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))",
            vec![],
        )
        .unwrap();
        db
    }

    fn put(
        db: &RemoteDb,
        project: &str,
        id: &str,
        key: &str,
        claim: &str,
        sup: Option<&str>,
        res: &[&str],
    ) -> MemoryEntry {
        let mut body = json!({"id": id, "key": key, "claim": claim, "source": "", "source_line": 0,
            "source_sha256": "", "unsourced": true, "recorded_at": format!("2026-01-01T00:00:00Z{id}")});
        if let Some(s) = sup {
            body["supersedes"] = json!(s);
        }
        if !res.is_empty() {
            body["resolves"] = json!(res);
        }
        db.execute(
            "INSERT INTO project_memory (project, id, body) VALUES (?, ?, ?)",
            vec![
                Cell::Text(project.into()),
                Cell::Text(id.into()),
                Cell::Text(body.to_string()),
            ],
        )
        .unwrap();
        serde_json::from_value(body).unwrap()
    }

    fn projects() -> Vec<String> {
        vec!["p".into(), "alias".into()]
    }

    #[test]
    fn sql_active_filter_drops_superseded_and_resolved() {
        let db = db();
        put(&db, "p", "a1", "k one", "old", None, &[]);
        put(&db, "alias", "a2", "k one", "new", Some("a1"), &[]);
        put(&db, "p", "b1", "q x", "left", None, &[]);
        put(&db, "p", "b2", "q y", "right", None, &[]);
        put(&db, "p", "b3", "q z", "merged", None, &["b1", "b2"]);
        put(&db, "other", "c1", "k one", "foreign", None, &[]);
        let ids: Vec<_> = active_ids(&db, &projects()).unwrap().into_iter().collect();
        assert_eq!(ids, vec!["a2", "b3"]);
    }

    #[test]
    fn like_arm_matches_percent_and_underscore_literally() {
        let db = db();
        let lit = put(&db, "p", "e1", "snake_case", "100% done", None, &[]);
        let other = put(&db, "p", "e2", "snakeXcase", "1000 done", None, &[]);
        let ledger = [lit, other];
        for q in ["snake_case", "100%"] {
            let (base, _) = recall_selection(q, &ledger);
            assert_eq!(base, vec!["e1"], "{q}");
            for arm in Arm::ALL {
                let (got, _, rows) = replay_arm(&db, &projects(), q, arm).unwrap();
                assert_eq!(got, base, "{q} {arm:?}");
                assert_eq!(rows, 1, "{q} {arm:?}");
            }
        }
        assert_eq!(like_escape(r"a_b%c\d"), r"a\_b\%c\\d");
    }

    #[test]
    fn partial_path_equals_baseline() {
        let db = db();
        let ledger = [
            put(&db, "p", "f1", "deploy plan", "ship friday", None, &[]),
            put(&db, "p", "f2", "review rota", "muse reviews", None, &[]),
            put(&db, "p", "f3", "unrelated", "nothing", None, &[]),
        ];
        let q = "deploy rota";
        let (base, partial) = recall_selection(q, &ledger);
        assert!(partial && !base.is_empty());
        for arm in Arm::ALL {
            let (got, p, rows) = replay_arm(&db, &projects(), q, arm).unwrap();
            assert_eq!((got, p), (base.clone(), true), "{arm:?}");
            assert_eq!(rows, 2, "{arm:?}");
        }
    }

    #[test]
    fn cjk_query_equals_baseline() {
        let db = db();
        let ledger = [
            put(&db, "p", "g1", "記憶", "遠端資料庫決定", None, &[]),
            put(&db, "p", "g2", "other", "english only", None, &[]),
        ];
        for q in ["遠端", "記憶 決定", "記憶 沒有"] {
            let (base, _) = recall_selection(q, &ledger);
            assert!(!base.is_empty(), "{q}");
            for arm in Arm::ALL {
                assert_eq!(replay_arm(&db, &projects(), q, arm).unwrap().0, base, "{q}");
            }
        }
    }

    #[test]
    fn report_flags_active_set_disagreement_and_counts_groups() {
        let db = db();
        let kept = put(&db, "p", "h1", "alpha", "beta", None, &[]);
        let r = report(&db, &projects(), &[kept], &["alpha".into(), "記".into()]).unwrap();
        assert_eq!(r["active_check"]["sql_active"], 1);
        assert_eq!(r["groups"]["english"]["like_equal"], 1);
        assert_eq!(r["groups"]["cjk"]["queries"], 1);
        assert!(r["mismatches"].as_array().unwrap().is_empty());
    }

    #[test]
    fn grouping() {
        assert_eq!(group_of("記憶 plan"), "cjk");
        assert_eq!(group_of("memory ledger"), "english");
        assert_eq!(group_of("recall_selection"), "identifier");
        assert_eq!(group_of("a/b"), "identifier");
        assert_eq!(group_of("v4 schema"), "identifier");
        assert_eq!(group_of("cort-evals"), "identifier");
        assert_eq!(group_of("src/x.rs"), "identifier");
    }
}
