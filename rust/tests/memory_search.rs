use cort::memory_search::{query_terms, sql_exempt};

#[test]
fn exempt_terms() {
    // Cased non-ASCII letters, and both ends of the combining-mark range.
    for t in [
        "café",
        "ｄｅｐｌｏｙ",
        "i\u{307}stanbul",
        "Ωmega",
        "e\u{300}",
        "a\u{36f}",
    ] {
        assert!(sql_exempt(t), "{t}");
    }
    // CJK, ASCII with SQL-looking characters, uncased non-ASCII, and just outside the range.
    for t in ["部署", "deploy-v2", "50%_done", "ª", "١٢", "a\u{2ff}"] {
        assert!(!sql_exempt(t), "{t}");
    }
}

#[test]
fn query_terms_lowercases_and_flags_exempt() {
    let q = query_terms("The Deploy 部署 deploy");
    assert_eq!(q.terms, vec!["the", "deploy", "部署", "deploy"]);
    assert_eq!(q.partial, vec!["deploy", "部署"]);
    assert!(!q.exempt);
    let q = query_terms("Café x");
    assert_eq!(q.terms, vec!["café", "x"]);
    assert!(q.exempt);
}

/// SQLite's `lower()` folds ASCII only, so a stored non-ASCII character whose Rust lowercase
/// differs would make the SQL haystack disagree with the client's. Every such character either
/// lowercases to something holding an ASCII letter -- exactly the two the SQL haystack folds
/// itself -- or to characters that make any term holding them `sql_exempt`, so no SQL condition
/// can meet them. Checked over every scalar value, not assumed.
#[test]
fn rust_lowercase_reaches_ascii_only_from_kelvin_and_dotted_i() {
    let mut reaches_ascii = Vec::new();
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        if c.is_ascii() {
            continue;
        }
        let low: String = c.to_lowercase().collect();
        if low == c.to_string() {
            continue;
        }
        if low.chars().any(|d| d.is_ascii()) {
            reaches_ascii.push((c, low));
            continue;
        }
        for d in low.chars() {
            assert!(sql_exempt(&d.to_string()), "{c:?} lowercases to {low:?}");
        }
    }
    assert_eq!(
        reaches_ascii,
        vec![
            ('\u{130}', "i\u{307}".to_string()),
            ('\u{212a}', "k".to_string())
        ]
    );
}
// ---------------------------------------------------------------------------------------------
// Stage statements, run against an in-memory SQLite behind the Hrana client.
// ---------------------------------------------------------------------------------------------

use cort::memory::{recall_selection_in, Haystack, MemoryEntry, MemoryLedger};
use cort::memory_lifecycle::{decode_rows, fold, State};
use cort::memory_search::{archived_excluded, stage1, stage2, Stage2, StageQuery};
use cort::remote::{Cell, RemoteDb, SqliteTransport, Stmt};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const PROJECT: &str = "p1";

struct Fx {
    db: RemoteDb,
}

impl Fx {
    fn new() -> Fx {
        let db = RemoteDb::new(
            "https://example.test",
            "test",
            Box::new(SqliteTransport::new().unwrap()),
        )
        .unwrap();
        for sql in [
            "CREATE TABLE project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))",
            "CREATE TABLE project_memory_lifecycle (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))",
            "CREATE TABLE project_aliases (old_project TEXT NOT NULL, new_project TEXT NOT NULL, created_at TEXT NOT NULL, machine TEXT NOT NULL, PRIMARY KEY (old_project, new_project))",
        ] {
            db.execute(sql, vec![]).unwrap();
        }
        Fx { db }
    }
    fn raw(&self, table: &str, id: &str, body: &str) {
        self.raw_in(PROJECT, table, id, body);
    }
    fn raw_in(&self, project: &str, table: &str, id: &str, body: &str) {
        self.db
            .execute(
                &format!("INSERT INTO {table} VALUES (?, ?, ?)"),
                vec![
                    Cell::Text(project.into()),
                    Cell::Text(id.into()),
                    Cell::Text(body.into()),
                ],
            )
            .unwrap();
    }
    fn entry_with(&self, id: &str, key: &str, claim: &str, extra: Value) {
        let mut body = json!({"id":id,"key":key,"claim":claim,"source":"s","source_line":1,
            "source_sha256":"","unsourced":true,"recorded_at":"100"});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        self.raw("project_memory", id, &body.to_string());
    }
    fn entry(&self, id: &str, key: &str, claim: &str) {
        self.entry_with(id, key, claim, json!({}));
    }
    fn life_in(&self, project: &str, id: &str, kind: &str, entry: Option<&str>, at: u64) {
        let body = json!({"id":id,"kind":kind,"entry":entry,"recorded_at":at.to_string()});
        self.raw_in(project, "project_memory_lifecycle", id, &body.to_string());
    }
    fn entry_in(&self, project: &str, id: &str, key: &str, claim: &str, extra: Value) {
        let mut body = json!({"id":id,"key":key,"claim":claim,"source":"s","source_line":1,
            "source_sha256":"","unsourced":true,"recorded_at":"100"});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        self.raw_in(project, "project_memory", id, &body.to_string());
    }
    fn life(&self, id: &str, kind: &str, entry: Option<&str>, at: u64) {
        let body = json!({"id":id,"kind":kind,"entry":entry,"recorded_at":at.to_string()});
        self.raw("project_memory_lifecycle", id, &body.to_string());
    }
    fn run(&self, stmt: Stmt) -> Vec<(String, String)> {
        let r = self.db.execute(&stmt.sql, stmt.args).unwrap();
        r.rows
            .iter()
            .map(|row| match (&row[0], &row[1]) {
                (Cell::Text(p), Cell::Text(b)) => (p.clone(), b.clone()),
                other => panic!("{other:?}"),
            })
            .collect()
    }
    fn ids(&self, stmt: Stmt) -> Vec<String> {
        self.run(stmt)
            .into_iter()
            .map(|(_, b)| {
                serde_json::from_str::<Value>(&b).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }
    fn bodies(&self, table: &str) -> Vec<String> {
        let r = self
            .db
            .execute(&format!("SELECT body FROM {table} ORDER BY id"), vec![])
            .unwrap();
        r.rows
            .iter()
            .map(|row| match &row[0] {
                Cell::Text(b) => b.clone(),
                other => panic!("{other:?}"),
            })
            .collect()
    }
    /// Active entries as the client sees them.
    fn client_active(&self) -> Vec<MemoryEntry> {
        let all: Vec<MemoryEntry> = self
            .bodies("project_memory")
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect();
        let replaced: BTreeSet<String> = all
            .iter()
            .flat_map(|e| e.supersedes.iter().chain(e.resolves.iter()).cloned())
            .collect();
        all.into_iter()
            .filter(|e| !replaced.contains(&e.id))
            .collect()
    }
}

fn s1(fx: &Fx, query: &str, h: Haystack, archived: bool) -> Vec<String> {
    let t = query_terms(query);
    fx.ids(stage1(
        PROJECT,
        &StageQuery {
            terms: &t,
            haystack: h,
            archived_in_sql: archived,
            stage2: Stage2::Gated,
        },
    ))
}

fn s2(fx: &Fx, query: &str, h: Haystack, gate: Stage2) -> Option<Vec<String>> {
    let t = query_terms(query);
    stage2(
        PROJECT,
        &StageQuery {
            terms: &t,
            haystack: h,
            archived_in_sql: true,
            stage2: gate,
        },
    )
    .map(|s| fx.ids(s))
}

fn fixture() -> Fx {
    let fx = Fx::new();
    let rows = [
        ("e01", "deploy-plan", "nuc runs the deploy plan"),
        ("e02", "deploy-v2", "Deploy PLAN v2 supersedes nothing"),
        ("e03", "部署", "部署 在 nuc 上執行"),
        ("e04", "nuc-host", "bd is the nuc11i5 machine"),
        ("e05", "path.rule", "no /home paths in fixtures"),
        ("e06", "cfg:timeout", "timeout is 3800 ms"),
        ("e07", "50%_done", "literal percent and underscore"),
        ("e08", "it's", "apostrophe a\\b backslash"),
        ("e09", "memory_tiers", "short term and long term entries"),
        ("e10", "Mixed-Case", "ClaudeCat lives in its own repo"),
        ("e11", "alpha", "alpha only"),
        ("e12", "alpha-beta", "alpha beta together"),
        ("e13", "alpha-beta-gamma", "alpha beta gamma all"),
        ("e14", "unrelated", "nothing to see"),
        ("e15", "ploy-word", "ploy starts here and deploy ends"),
    ];
    for (id, k, c) in rows {
        fx.entry(id, k, c);
    }
    fx
}

const QUERIES: [&str; 6] = [
    "deploy",
    "部署 nuc",
    "deploy-v2",
    "DEPLOY plan",
    "nope",
    "ploy nuc",
];

#[test]
fn stage1_is_a_superset_of_the_client_full_match() {
    let fx = fixture();
    let active = fx.client_active();
    for h in [Haystack::KeyClaim, Haystack::Key] {
        for q in QUERIES {
            let (ids, partial) = recall_selection_in(q, &active, h);
            if partial {
                continue;
            }
            let sql: BTreeSet<String> = s1(&fx, q, h, true).into_iter().collect();
            for id in ids {
                assert!(sql.contains(&id), "{q} {h:?}: {id} missing from stage 1");
            }
        }
    }
}

#[test]
fn stage2_is_a_superset_of_the_client_partial_rule() {
    let fx = fixture();
    let active = fx.client_active();
    let mut saw_partial = false;
    for h in [Haystack::KeyClaim, Haystack::Key] {
        for q in [
            "nuc zzz",
            "alpha zzz",
            "部署 zzz",
            "deploy beta",
            "ploy nuc",
            "deploy plan nuc",
        ] {
            let (_, partial) = recall_selection_in(q, &active, h);
            if !partial {
                continue;
            }
            saw_partial = true;
            // The partial selection before the client's cap of 5: no entry holds every term (the
            // match is partial), so an entry recalled on its own also goes through the partial
            // rule, and it is selected exactly when it scores at least half.
            let uncapped: Vec<String> = active
                .iter()
                .filter(|e| {
                    !recall_selection_in(q, std::slice::from_ref(*e), h)
                        .0
                        .is_empty()
                })
                .map(|e| e.id.clone())
                .collect();
            let sql: BTreeSet<String> = s2(&fx, q, h, Stage2::Ungated)
                .unwrap()
                .into_iter()
                .collect();
            for id in uncapped {
                assert!(sql.contains(&id), "{q} {h:?}: {id} missing from stage 2");
            }
        }
    }
    assert!(saw_partial);
    // SQL is substring-based, so it is wider than the client's word-start rule.
    let wide = s2(&fx, "ploy nuc", Haystack::KeyClaim, Stage2::Ungated).unwrap();
    assert!(wide.contains(&"e01".to_string()), "{wide:?}");
}

#[test]
fn stage2_not_built_for_one_term_or_only_noise() {
    let fx = fixture();
    assert!(s2(&fx, "deploy", Haystack::KeyClaim, Stage2::Gated).is_none());
    assert!(s2(&fx, "the a", Haystack::KeyClaim, Stage2::Gated).is_none());
    assert!(s2(&fx, "", Haystack::KeyClaim, Stage2::Gated).is_none());
}

#[test]
fn stage2_gate() {
    let fx = fixture();
    // Stage 1 matches e12 and e13: gated stage 2 stays silent, ungated speaks.
    assert!(!s1(&fx, "alpha beta", Haystack::KeyClaim, true).is_empty());
    assert_eq!(
        s2(&fx, "alpha beta", Haystack::KeyClaim, Stage2::Gated).unwrap(),
        Vec::<String>::new()
    );
    let open = s2(&fx, "alpha beta", Haystack::KeyClaim, Stage2::Ungated).unwrap();
    assert!(
        open.contains(&"e11".to_string()) && open.contains(&"e12".to_string()),
        "{open:?}"
    );
    // Stage 1 matches nothing: gated stage 2 returns its rows.
    assert!(s1(&fx, "the alpha zzz", Haystack::KeyClaim, true).is_empty());
    let gated = s2(&fx, "the alpha zzz", Haystack::KeyClaim, Stage2::Gated).unwrap();
    assert!(gated.contains(&"e11".to_string()), "{gated:?}");
}

#[test]
fn stage1_returns_exactly_the_matching_rows() {
    let fx = fixture();
    let active = fx.client_active();
    let mut queries = QUERIES.to_vec();
    queries.extend([
        "",
        "alpha beta",
        "50%_done",
        "it's",
        "a\\b",
        "CLAUDECAT repo",
    ]);
    for h in [Haystack::KeyClaim, Haystack::Key] {
        for q in &queries {
            let terms = query_terms(q);
            let mut want: Vec<String> = active
                .iter()
                .filter(|e| {
                    let hay = h.text(e);
                    terms.terms.iter().all(|t| hay.contains(t.as_str()))
                })
                .map(|e| e.id.clone())
                .collect();
            want.sort();
            assert_eq!(s1(&fx, q, h, true), want, "{q:?} {h:?}");
        }
    }
    // Empty query: every active row.
    assert_eq!(s1(&fx, "", Haystack::KeyClaim, true).len(), 15);
}

#[test]
fn stage2_threshold() {
    let fx = Fx::new();
    fx.entry("t0", "k0", "nothing here");
    fx.entry("t1", "k1", "alpha only");
    fx.entry("t2", "k2", "alpha and beta");
    fx.entry("t3", "k3", "alpha beta gamma");
    let got = s2(&fx, "alpha beta gamma", Haystack::KeyClaim, Stage2::Ungated).unwrap();
    assert_eq!(got, vec!["t2", "t3"]);
    // Two terms: threshold 1.
    let got = s2(&fx, "alpha gamma", Haystack::KeyClaim, Stage2::Ungated).unwrap();
    assert_eq!(got, vec!["t1", "t2", "t3"]);
}

#[test]
fn ascii_upper_case_in_claim_matches() {
    let fx = Fx::new();
    fx.entry("u1", "k", "Deploy PLAN");
    assert_eq!(s1(&fx, "deploy", Haystack::KeyClaim, true), vec!["u1"]);
}

#[test]
fn exempt_term_keeps_rows() {
    let fx = fixture();
    let got = s1(&fx, "café 部署", Haystack::KeyClaim, true);
    assert_eq!(got, vec!["e03"]);
    // All terms exempt: no condition at all.
    assert_eq!(s1(&fx, "café", Haystack::KeyClaim, true).len(), 15);
}

#[test]
fn active_rule_counts_supersedes_and_resolves() {
    let fx = Fx::new();
    fx.entry("a1", "k", "old claim one");
    fx.entry_with("a2", "k", "new claim two", json!({"supersedes":"a1"}));
    fx.entry("b1", "j", "claim b1");
    fx.entry("b2", "j", "claim b2");
    fx.entry("b3", "j", "claim b3 other");
    fx.entry_with(
        "b4",
        "j",
        "claim b4 resolved",
        json!({"resolves":["b1","b2"]}),
    );
    let mut got = s1(&fx, "claim", Haystack::KeyClaim, true);
    got.sort();
    assert_eq!(got, vec!["a2", "b3", "b4"]);
}

#[test]
fn json_invalid_row_does_not_fail_the_statement() {
    let fx = fixture();
    fx.raw("project_memory", "bad", "{not json");
    fx.raw("project_memory_lifecycle", "badl", "{not json");
    fx.life("l1", "on", None, 1);
    fx.life("l2", "archive", Some("e14"), 2);
    assert_eq!(s1(&fx, "deploy plan", Haystack::KeyClaim, true).len(), 2);
    assert!(s2(&fx, "alpha zzz", Haystack::Key, Stage2::Gated).is_some());
    assert_eq!(fx.ids(archived_excluded(PROJECT)), vec!["e14"]);
}

#[test]
fn stage1_applies_archived_condition_only_when_asked() {
    let fx = fixture();
    fx.life("l1", "on", None, 1);
    fx.life("l2", "archive", Some("e01"), 2);
    assert_eq!(
        s1(&fx, "deploy-plan", Haystack::Key, true),
        Vec::<String>::new()
    );
    assert_eq!(s1(&fx, "deploy-plan", Haystack::Key, false), vec!["e01"]);
}

/// What SQL excludes, checked against the fold: every id SQL returns is hidden by the fold, and
/// every entry the fold marks Archived is returned.
fn assert_agrees_with_fold(fx: &Fx, name: &str) -> Vec<String> {
    let sql = fx.ids(archived_excluded(PROJECT));
    let ledger = MemoryLedger {
        schema: 1,
        entries: fx.client_active(),
    };
    let (events, unreadable) = decode_rows(&fx.bodies("project_memory_lifecycle"));
    let view = fold(&ledger, events, unreadable, 10_000, None);
    for id in &sql {
        assert!(
            view.state_of(id).is_some_and(|s| s.is_hidden()),
            "{name}: SQL excluded {id} but the fold shows {:?}",
            view.state_of(id)
        );
    }
    for e in &ledger.entries {
        if view.state_of(&e.id) == Some(State::Archived) {
            assert!(
                sql.contains(&e.id),
                "{name}: fold archived {} but SQL did not",
                e.id
            );
        }
    }
    sql
}

type Ev = (&'static str, &'static str, Option<&'static str>, u64);

fn three_entries() -> Fx {
    let fx = Fx::new();
    for (id, k) in [("e1", "one"), ("e2", "two"), ("e3", "three")] {
        fx.entry(id, k, "claim text");
    }
    fx
}

#[test]
fn archived_condition_agrees_with_the_fold() {
    let cases: Vec<(&str, Vec<Ev>)> = vec![
        (
            "archive",
            vec![("l1", "on", None, 1), ("l2", "archive", Some("e1"), 2)],
        ),
        (
            "restore after archive",
            vec![
                ("l1", "on", None, 1),
                ("l2", "archive", Some("e1"), 2),
                ("l3", "restore", Some("e1"), 3),
            ],
        ),
        (
            "archive after restore",
            vec![
                ("l1", "on", None, 1),
                ("l2", "archive", Some("e1"), 2),
                ("l3", "restore", Some("e1"), 3),
                ("l4", "archive", Some("e1"), 4),
            ],
        ),
        (
            "drop",
            vec![("l1", "on", None, 1), ("l2", "drop", Some("e1"), 2)],
        ),
        (
            "drop then archive (fold: dropped, SQL: returned)",
            vec![
                ("l1", "on", None, 1),
                ("l2", "drop", Some("e1"), 2),
                ("l3", "archive", Some("e1"), 3),
            ],
        ),
        (
            "archive before an off/on cycle",
            vec![
                ("l1", "on", None, 1),
                ("l2", "archive", Some("e1"), 2),
                ("l3", "off", None, 3),
                ("l4", "on", None, 4),
            ],
        ),
        (
            "project switched off",
            vec![
                ("l1", "on", None, 1),
                ("l2", "archive", Some("e1"), 2),
                ("l3", "off", None, 3),
            ],
        ),
        ("never enabled", vec![("l2", "archive", Some("e1"), 2)]),
        (
            "two entries, one archived",
            vec![
                ("l1", "on", None, 1),
                ("l2", "archive", Some("e2"), 2),
                ("l3", "promote", Some("e3"), 3),
            ],
        ),
    ];
    let mut any_excluded = false;
    for (name, events) in cases {
        let fx = three_entries();
        for (id, kind, entry, at) in events {
            fx.life(id, kind, entry, at);
        }
        any_excluded |= !assert_agrees_with_fold(&fx, name).is_empty();
    }
    assert!(any_excluded);
    // The drop-then-archive case really is the SQL-wider one.
    let fx = three_entries();
    fx.life("l1", "on", None, 1);
    fx.life("l2", "drop", Some("e1"), 2);
    fx.life("l3", "archive", Some("e1"), 3);
    assert_eq!(
        assert_agrees_with_fold(&fx, "drop then archive"),
        vec!["e1"]
    );
}

#[test]
fn archived_ordering_is_numeric_then_lifecycle_id() {
    let fx = three_entries();
    fx.life("l1", "on", None, 1);
    // e1: archive @99 (id l9) then restore @100 (id l1). Text order would put "99" after "100".
    fx.life("l9", "archive", Some("e1"), 99);
    fx.life("l1b", "restore", Some("e1"), 100);
    // e2: same ns, the later lifecycle id wins: restore "lb" beats archive "la".
    fx.life("la", "archive", Some("e2"), 100);
    fx.life("lb", "restore", Some("e2"), 100);
    // e3: same ns, archive "lz" beats restore "ly".
    fx.life("ly", "restore", Some("e3"), 100);
    fx.life("lz", "archive", Some("e3"), 100);
    assert_eq!(assert_agrees_with_fold(&fx, "ordering"), vec!["e3"]);
}

#[test]
fn archived_ignores_non_canonical_lifecycle_rows() {
    let fx = three_entries();
    fx.life("l1", "on", None, 1);
    fx.life("l2", "archive", Some("e1"), 100);
    assert_eq!(assert_agrees_with_fold(&fx, "baseline"), vec!["e1"]);
    // Later restores the SQL must not read (each would un-archive e1 if it did).
    let bad = [
        (
            "n1",
            json!({"id":"n1","kind":"restore","entry":"e1","recorded_at":"12x"}),
        ),
        (
            "n2",
            json!({"id":"n2","kind":"frobnicate","entry":"e1","recorded_at":"500"}),
        ),
        (
            "n3",
            json!({"id":"n3","kind":"restore","recorded_at":"500"}),
        ),
        (
            "n4",
            json!({"id":5,"kind":"restore","entry":"e1","recorded_at":"500"}),
        ),
        (
            "n5",
            json!({"id":"n5","kind":"restore","entry":"e1","recorded_at":500}),
        ),
        (
            "n6",
            json!({"id":"n6","kind":"restore","entry":"","recorded_at":"500"}),
        ),
    ];
    for (id, body) in bad {
        fx.raw("project_memory_lifecycle", id, &body.to_string());
        assert_eq!(
            assert_agrees_with_fold(&fx, id),
            vec!["e1"],
            "{id} changed the excluded set"
        );
    }
    // "+500" is accepted by the client's parser but not by SQL's digits-only rule: SQL keeps
    // e1 excluded (the client's fold, which un-archives it, is what the integrity check against
    // `archived_excluded` catches).
    fx.raw(
        "project_memory_lifecycle",
        "n7",
        &json!({"id":"n7","kind":"restore","entry":"e1","recorded_at":"+500"}).to_string(),
    );
    assert_eq!(fx.ids(archived_excluded(PROJECT)), vec!["e1"]);
}

#[test]
fn aliases_widen_the_component_and_other_projects_stay_out() {
    let fx = Fx::new();
    fx.db
        .execute(
            "INSERT INTO project_aliases VALUES ('p1', 'p2', 't', 'm')",
            vec![],
        )
        .unwrap();
    // Main project: one entry that an aliased-project row supersedes, one archived through the
    // aliased project's lifecycle, two that only the unrelated project touches.
    fx.entry_in("p1", "a1", "k1", "alias target", json!({}));
    fx.entry_in("p1", "x1", "k2", "alias archived", json!({}));
    fx.entry_in("p1", "y1", "k3", "alias archive elsewhere", json!({}));
    fx.entry_in("p1", "y2", "k4", "alias superseded elsewhere", json!({}));
    fx.entry_in("p2", "a2", "k1", "alias new", json!({"supersedes":"a1"}));
    fx.life_in("p2", "l1", "on", None, 1);
    fx.life_in("p2", "l2", "archive", Some("x1"), 2);
    // Unrelated project: its own entry, a switch and an archive of y1, a supersession of y2.
    fx.entry_in("p3", "z1", "k5", "alias unrelated", json!({}));
    fx.entry_in("p3", "z2", "k4", "alias z2", json!({"supersedes":"y2"}));
    fx.life_in("p3", "l3", "on", None, 1);
    fx.life_in("p3", "l4", "archive", Some("y1"), 2);
    let mut got = s1(&fx, "alias", Haystack::KeyClaim, true);
    got.sort();
    assert_eq!(got, vec!["a2", "y1", "y2"]);
    assert_eq!(fx.ids(archived_excluded(PROJECT)), vec!["x1"]);
}

#[test]
fn stage2_exempt_terms_count_toward_the_threshold() {
    let fx = fixture();
    // n = 2, threshold 1, the exempt term always counts: every active row.
    let all = s2(&fx, "café zzz", Haystack::KeyClaim, Stage2::Ungated).unwrap();
    assert_eq!(all.len(), 15);
    // n = 3, threshold 2, one point from café: rows that also hold alpha.
    let got = s2(&fx, "café alpha zzz", Haystack::KeyClaim, Stage2::Ungated).unwrap();
    assert_eq!(got, vec!["e11", "e12", "e13"]);
}

#[test]
fn stage2_with_an_exempt_term_is_never_gated() {
    let fx = fixture();
    // Stage 1 (café gives it no condition) matches the alpha rows, so a gated stage 2 would be
    // empty; with an exempt term it must open anyway.
    assert!(!s1(&fx, "café alpha", Haystack::KeyClaim, true).is_empty());
    let got = s2(&fx, "café alpha", Haystack::KeyClaim, Stage2::Gated).unwrap();
    assert_eq!(got.len(), 15);
}

#[test]
fn gated_stage2_opens_when_the_only_stage1_hit_is_archived() {
    let fx = fixture();
    fx.entry("q1", "quokka-wombat", "quokka wombat together");
    fx.entry("q2", "quokka-solo", "quokka alone");
    fx.life("l1", "on", None, 1);
    fx.life("l2", "archive", Some("q1"), 2);
    // Without the archived condition stage 1 finds q1 and the gate would stay shut.
    assert_eq!(
        s1(&fx, "quokka wombat", Haystack::KeyClaim, false),
        vec!["q1"]
    );
    assert!(s1(&fx, "quokka wombat", Haystack::KeyClaim, true).is_empty());
    let got = s2(&fx, "quokka wombat", Haystack::KeyClaim, Stage2::Gated).unwrap();
    assert_eq!(got, vec!["q2"]);
}

// ---------------------------------------------------------------------------------------------
// The integrity screen and the whole-ledger count statements (Task 4a). Subset decoding is
// tested through the search below (`predecessor_not_returned_is_not_an_error`).
// ---------------------------------------------------------------------------------------------

use cort::memory_search::{conflict_keys, integrity_screen, ledger_count, lifecycle_rows};
use cort::memory_store::{Ledger, LedgerError};
use cort::remote::Transport;
use std::sync::Arc;

struct SharedTransport(Arc<SqliteTransport>);

impl Transport for SharedTransport {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, cort::remote::RemoteError> {
        self.0.post(url, body, headers)
    }
}

/// One in-memory database seen both through a `Ledger` (today's whole read) and a `RemoteDb`
/// (the screen's statements).
struct Env {
    ledger: Ledger,
    db: RemoteDb,
}

impl Env {
    fn new() -> Env {
        let inner = Arc::new(SqliteTransport::new().unwrap());
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(SharedTransport(inner.clone())),
        )
        .unwrap();
        // A read provisions the tables, exactly as `recall` does.
        ledger.read(PROJECT).unwrap();
        let db = RemoteDb::new(
            "https://example.test",
            "test",
            Box::new(SharedTransport(inner)),
        )
        .unwrap();
        Env { ledger, db }
    }
    fn insert_in(&self, project: &str, id: &str, body: &str) {
        self.db
            .execute(
                "INSERT INTO project_memory VALUES (?, ?, ?)",
                vec![
                    Cell::Text(project.into()),
                    Cell::Text(id.into()),
                    Cell::Text(body.into()),
                ],
            )
            .unwrap();
    }
    fn insert(&self, id: &str, body: &str) {
        self.insert_in(PROJECT, id, body);
    }
    fn entry(&self, id: &str, key: &str, claim: &str, extra: Value) {
        self.entry_in(PROJECT, id, key, claim, extra);
    }
    fn entry_in(&self, project: &str, id: &str, key: &str, claim: &str, extra: Value) {
        let mut body = json!({"id":id,"key":key,"claim":claim,"source":"s","source_line":1,
            "source_sha256":"","unsourced":true,"recorded_at":"100"});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        self.insert_in(project, id, &body.to_string());
    }
    fn alias(&self, old: &str, new: &str) {
        self.db
            .execute(
                "INSERT INTO project_aliases VALUES (?, ?, '1', 'm')",
                vec![Cell::Text(old.into()), Cell::Text(new.into())],
            )
            .unwrap();
    }
    /// The screen's rows, each `(body, 1)`, as their bodies.
    fn screen_rows(&self) -> Vec<String> {
        let stmt = integrity_screen(PROJECT);
        self.db
            .execute(&stmt.sql, stmt.args)
            .unwrap()
            .rows
            .iter()
            .map(|row| match row.as_slice() {
                [Cell::Text(b), Cell::Int(1)] => b.clone(),
                other => panic!("{other:?}"),
            })
            .collect()
    }
    /// The body of the first row stored under `id`.
    fn body_of(&self, id: &str) -> String {
        let r = self
            .db
            .execute(
                "SELECT body FROM project_memory WHERE id = ? ORDER BY project",
                vec![Cell::Text(id.into())],
            )
            .unwrap();
        match &r.rows[0][0] {
            Cell::Text(b) => b.clone(),
            other => panic!("{other:?}"),
        }
    }
    /// What today's whole read says about the same database.
    fn whole(&self) -> Result<(), String> {
        match self.ledger.read(PROJECT) {
            Ok(_) => Ok(()),
            Err(LedgerError::Message(m)) => Err(m),
            Err(other) => panic!("{other:?}"),
        }
    }
    fn count(&self, stmt: Stmt) -> Vec<Cell> {
        self.db
            .execute(&stmt.sql, stmt.args)
            .unwrap()
            .rows
            .into_iter()
            .map(|mut r| r.remove(0))
            .collect()
    }
}

#[test]
fn screen_returns_a_row_whose_predecessor_is_absent() {
    let env = Env::new();
    env.entry("b", "k", "second", json!({"supersedes":"a"}));
    assert_eq!(env.screen_rows(), vec![env.body_of("b")]);
    // The whole read recall then falls back to gives today's message.
    assert_eq!(
        env.whole().unwrap_err(),
        "memory b supersedes missing id a; re-add the alias with cort memory alias add"
    );
}

#[test]
fn screen_returns_a_row_for_every_fault_the_whole_read_fails_on() {
    type Seed = Box<dyn Fn(&Env)>;
    // Each fault, and the row id the screen must return for it.
    let cases: Vec<(&str, &str, Seed)> = vec![
        (
            "invalid json",
            "x",
            Box::new(|e| e.insert("x", "{not json")),
        ),
        (
            "missing key",
            "x",
            Box::new(|e| {
                e.insert(
                    "x",
                    r#"{"id":"x","claim":"c","source":"s","source_line":1,"source_sha256":"","recorded_at":"1"}"#,
                )
            }),
        ),
        (
            "blank key",
            "x",
            Box::new(|e| e.entry("x", "  ", "c", json!({}))),
        ),
        (
            "supersedes missing id",
            "x",
            Box::new(|e| e.entry("x", "k", "c", json!({"supersedes":"gone"}))),
        ),
        (
            "supersedes own id",
            "x",
            Box::new(|e| e.entry("x", "k", "c", json!({"supersedes":"x"}))),
        ),
        (
            "supersedes a different key",
            "b",
            Box::new(|e| {
                e.entry("a", "k1", "c", json!({}));
                e.entry("b", "k2", "c", json!({"supersedes":"a"}));
            }),
        ),
        (
            "one id under two aliases",
            "x",
            Box::new(|e| {
                e.alias(PROJECT, "p0");
                e.entry("x", "k", "first claim", json!({}));
                e.entry_in("p0", "x", "k", "second claim", json!({}));
            }),
        ),
        // `resolves` reaches the screen through `json_each`, a separate branch from
        // `supersedes`; the bad element sits first in one case and second in the others.
        (
            "resolves a missing id",
            "x",
            Box::new(|e| {
                e.entry("a", "k", "c", json!({}));
                e.entry("x", "k", "c", json!({"resolves":["gone","a"]}));
            }),
        ),
        (
            "resolves a different key",
            "x",
            Box::new(|e| {
                e.entry("a", "k", "c", json!({}));
                e.entry("b", "k2", "c", json!({}));
                e.entry("x", "k", "c", json!({"resolves":["a","b"]}));
            }),
        ),
        (
            "resolves its own id",
            "x",
            Box::new(|e| {
                e.entry("a", "k", "c", json!({}));
                e.entry("x", "k", "c", json!({"resolves":["a","x"]}));
            }),
        ),
    ];
    for (name, id, seed) in cases {
        let env = Env::new();
        seed(&env);
        env.whole().expect_err(name);
        let rows = env.screen_rows();
        assert!(rows.contains(&env.body_of(id)), "{name}: {rows:?}");
    }
}

#[test]
fn screen_returns_every_suspect_row() {
    let env = Env::new();
    env.insert("a-bad", "{not json");
    env.entry("b", "k1", "c", json!({}));
    env.entry("c", "k2", "c", json!({"supersedes":"b"}));
    let whole = env.whole().unwrap_err();
    assert!(whole.starts_with("invalid remote memory row"), "{whole}");
    // An invalid body has no `$.id`, and a null id sorts first; `b` is named, not suspect.
    assert_eq!(
        env.screen_rows(),
        vec!["{not json".to_string(), env.body_of("c")]
    );
}

#[test]
fn screen_is_silent_on_a_valid_ledger() {
    let env = Env::new();
    env.alias(PROJECT, "p0");
    env.entry("a", "k", "one", json!({}));
    env.entry("b", "k", "two", json!({"supersedes":"a"}));
    env.entry("c", "k", "three", json!({"supersedes":"b"}));
    env.entry("d", "j", "d", json!({}));
    env.entry("e", "j", "e", json!({}));
    env.entry("f", "j", "f", json!({"resolves":["d","e"]}));
    // An alias pair: the same id, the same bytes, under both projects.
    env.entry("g", "m", "g", json!({}));
    env.entry_in("p0", "g", "m", "g", json!({}));
    assert_eq!(env.screen_rows(), Vec::<String>::new());
    assert_eq!(env.whole(), Ok(()));
}

#[test]
fn screen_suspect_is_not_a_verdict() {
    let env = Env::new();
    env.entry("   ", "k", "claim", json!({}));
    // An id of spaces is suspect, and today's whole read accepts it.
    assert_eq!(env.screen_rows(), vec![env.body_of("   ")]);
    assert_eq!(env.whole(), Ok(()));
}

#[test]
fn screen_does_not_return_the_rows_a_suspect_names() {
    let env = Env::new();
    env.entry("a", "k1", "c", json!({}));
    env.entry("b", "k2", "c", json!({"supersedes":"a"}));
    env.entry("z", "other", "unrelated", json!({}));
    // b only: the named `a` is not fetched (the arm that did so drew a 502 from TursoDB, 2026-11-16),
    // and z is not returned.
    assert_eq!(env.screen_rows(), vec![env.body_of("b")]);
}

#[test]
fn conflict_keys_counts_hidden_entries() {
    let env = Env::new();
    env.entry("a", "dup", "one", json!({}));
    env.entry("b", "dup", "two", json!({}));
    env.entry("c", "solo", "alone", json!({}));
    env.entry("d", "chain", "old", json!({}));
    env.entry("e", "chain", "new", json!({"supersedes":"d"}));
    // `a` is archived: still a conflict, because the statement ignores the lifecycle.
    env.db
        .execute(
            "INSERT INTO project_memory_lifecycle VALUES ('p1','l1','{\"id\":\"l1\",\"kind\":\"on\",\"recorded_at\":\"1\"}'), ('p1','l2','{\"id\":\"l2\",\"kind\":\"archive\",\"entry\":\"a\",\"recorded_at\":\"2\"}')",
            vec![],
        )
        .unwrap();
    assert_eq!(
        env.count(conflict_keys(PROJECT)),
        vec![Cell::Text("dup".into())]
    );
    assert_eq!(env.count(lifecycle_rows(PROJECT)).len(), 2);
}

#[test]
fn ledger_count_counts_distinct_ids_across_aliases() {
    let env = Env::new();
    assert_eq!(env.count(ledger_count(PROJECT)), vec![Cell::Int(0)]);
    env.alias(PROJECT, "p0");
    env.entry("a", "k", "one", json!({}));
    env.entry_in("p0", "a", "k", "one", json!({}));
    env.entry("b", "k2", "two", json!({}));
    env.insert("bad", "{not json");
    assert_eq!(env.count(ledger_count(PROJECT)), vec![Cell::Int(2)]);
}

#[test]
fn screen_survives_malformed_json_beside_a_supersedes_row() {
    let env = Env::new();
    env.insert("bad", "{not json");
    env.entry("a", "k", "one", json!({}));
    env.entry("b", "k", "two", json!({"supersedes":"a"}));
    let rows = env.screen_rows();
    // The malformed row alone: the valid chain beside it is not suspect.
    assert_eq!(rows, vec!["{not json".to_string()]);
}

// ---------------------------------------------------------------------------------------------
// The search itself (Task 4b): `recall_all_with` against today's whole read, `recall_whole_with`.
// ---------------------------------------------------------------------------------------------

mod search {
    use super::SharedTransport;
    use cort::memory::{recall_selection_in, Haystack, MemoryEntry};
    use cort::memory_lifecycle::fold;
    use cort::memory_store::{self, Ledger, LedgerError, SearchRequest};
    use cort::remote::{Cell, RemoteDb, SqliteTransport, Transport};
    use serde_json::{json, Value};
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const NS: i128 = 1_000_000_000;
    const DAY: i128 = 86_400 * NS;

    /// Counts every request the ledger sends: one per round trip.
    struct Counting {
        inner: Arc<SqliteTransport>,
        posts: Arc<AtomicUsize>,
    }

    impl Transport for Counting {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, cort::remote::RemoteError> {
            self.posts.fetch_add(1, Ordering::SeqCst);
            self.inner.post(url, body, headers)
        }
    }

    struct Fixture {
        temp: tempfile::TempDir,
        ledger: Ledger,
        /// The same database, uncounted, for seeding.
        db: RemoteDb,
        posts: Arc<AtomicUsize>,
        /// The in-memory server itself, for a ledger over a transport of a test's own.
        inner: Arc<SqliteTransport>,
    }

    fn now_ns() -> i128 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        i128::try_from(nanos).unwrap()
    }

    /// A git repository with a committed source and an origin, and a ledger over an in-memory
    /// database that has no tables yet (the first read creates them, as `recall` does).
    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("decision.md"), "one\ntwo\nthree\n").unwrap();
        for args in [
            vec!["init"],
            vec!["add", "decision.md"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "source",
            ],
            vec!["remote", "add", "origin", "https://example.test/search.git"],
        ] {
            assert!(Command::new("git")
                .current_dir(temp.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        }
        let inner = Arc::new(SqliteTransport::new().unwrap());
        let posts = Arc::new(AtomicUsize::new(0));
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(Counting {
                inner: inner.clone(),
                posts: posts.clone(),
            }),
        )
        .unwrap();
        let db = RemoteDb::new(
            "https://example.test",
            "test",
            Box::new(SharedTransport(inner.clone())),
        )
        .unwrap();
        Fixture {
            temp,
            ledger,
            db,
            posts,
            inner,
        }
    }

    impl Fixture {
        fn root(&self) -> &Path {
            self.temp.path()
        }
        fn project(&self) -> String {
            memory_store::project(self.root()).unwrap()
        }
        fn tables(&self) {
            for sql in [
                "CREATE TABLE IF NOT EXISTS project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))",
                "CREATE TABLE IF NOT EXISTS project_memory_lifecycle (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))",
                "CREATE TABLE IF NOT EXISTS project_aliases (old_project TEXT NOT NULL, new_project TEXT NOT NULL, created_at TEXT NOT NULL, machine TEXT NOT NULL, PRIMARY KEY (old_project, new_project))",
            ] {
                self.db.execute(sql, vec![]).unwrap();
            }
        }
        fn insert_in(&self, table: &str, project: &str, id: &str, body: &str) {
            self.tables();
            self.db
                .execute(
                    &format!("INSERT INTO {table} VALUES (?, ?, ?)"),
                    vec![
                        Cell::Text(project.into()),
                        Cell::Text(id.into()),
                        Cell::Text(body.into()),
                    ],
                )
                .unwrap();
        }
        fn raw(&self, id: &str, body: &str) {
            self.insert_in("project_memory", &self.project(), id, body);
        }
        fn body(id: &str, key: &str, claim: &str, at: i128, extra: Value) -> Value {
            let mut body = json!({"id":id,"key":key,"claim":claim,
                "source":format!(".cort-unsourced/{id}"),"source_line":1,"source_sha256":"",
                "unsourced":true,"recorded_at":at.to_string()});
            for (k, v) in extra.as_object().unwrap() {
                body[k] = v.clone();
            }
            body
        }
        fn entry_with(&self, id: &str, key: &str, claim: &str, at: i128, extra: Value) {
            self.raw(id, &Self::body(id, key, claim, at, extra).to_string());
        }
        fn entry(&self, id: &str, key: &str, claim: &str, at: i128) {
            self.entry_with(id, key, claim, at, json!({}));
        }
        fn entry_in(&self, project: &str, id: &str, key: &str, claim: &str, at: i128) {
            let body = Self::body(id, key, claim, at, json!({})).to_string();
            self.insert_in("project_memory", project, id, &body);
        }
        fn alias(&self, old: &str) {
            self.tables();
            self.db
                .execute(
                    "INSERT INTO project_aliases VALUES (?, ?, '1', 'm')",
                    vec![Cell::Text(old.into()), Cell::Text(self.project())],
                )
                .unwrap();
        }
        fn life(&self, id: &str, kind: &str, entry: Option<&str>, at: i128) {
            let body = json!({"id":id,"kind":kind,"entry":entry,"recorded_at":at.to_string()});
            self.life_body(id, &body.to_string());
        }
        fn life_body(&self, id: &str, body: &str) {
            self.insert_in("project_memory_lifecycle", &self.project(), id, body);
        }
        fn new_recall(&self, query: &str, include_hidden: bool) -> Result<Value, LedgerError> {
            memory_store::recall_all_with(self.root(), query, &self.ledger, include_hidden)
                .map(|r| serde_json::to_value(r).unwrap())
        }
        fn old_recall(&self, query: &str, include_hidden: bool) -> Result<Value, LedgerError> {
            memory_store::recall_whole_with(self.root(), query, &self.ledger, include_hidden)
                .map(|r| serde_json::to_value(r).unwrap())
        }
        /// Round trips `recall_all_with` spends on one call.
        fn trips(&self, query: &str, include_hidden: bool) -> usize {
            let before = self.posts.load(Ordering::SeqCst);
            let _ = self.new_recall(query, include_hidden);
            self.posts.load(Ordering::SeqCst) - before
        }
        /// `search_with` over several queries in one request.
        fn search_many(
            &self,
            queries: &[&str],
            include_hidden: bool,
            now: i128,
        ) -> memory_store::Searched {
            let queries: Vec<String> = queries.iter().map(|q| q.to_string()).collect();
            memory_store::search_with(
                &self.ledger,
                &self.project(),
                &SearchRequest {
                    queries: &queries,
                    haystack: Haystack::KeyClaim,
                    include_hidden,
                },
                now,
            )
            .map(answered)
            .unwrap()
        }
        fn search_trips(&self, query: &str, include_hidden: bool) -> usize {
            let queries = [query.to_string()];
            memory_store::search_with(
                &self.ledger,
                &self.project(),
                &SearchRequest {
                    queries: &queries,
                    haystack: Haystack::KeyClaim,
                    include_hidden,
                },
                now_ns(),
            )
            .map(answered)
            .unwrap()
            .trips
        }
    }

    /// The search answers exactly as today's whole read; returns the answer.
    fn assert_same(f: &Fixture, query: &str, include_hidden: bool) -> Value {
        let new = f.new_recall(query, include_hidden);
        let old = f.old_recall(query, include_hidden);
        assert_eq!(new, old, "query {query:?}, include_hidden {include_hidden}");
        new.unwrap()
    }

    /// The search's own reading; a screen row is a test failure here.
    fn answered(outcome: memory_store::SearchOutcome) -> memory_store::Searched {
        match outcome {
            memory_store::SearchOutcome::Searched(searched) => *searched,
            memory_store::SearchOutcome::Suspect { trips, .. } => {
                panic!("the integrity screen returned a row (trip {trips})")
            }
        }
    }

    /// One query's selection over a (possibly multi-query) search's merged rows, as recall makes
    /// it: hidden entries left out unless `include_hidden`. The rows are all active.
    fn select_in(
        searched: &memory_store::Searched,
        query: &str,
        include_hidden: bool,
    ) -> (Vec<String>, bool) {
        let visible: Vec<MemoryEntry> = searched
            .ledger
            .entries
            .iter()
            .filter(|e| {
                include_hidden || !searched.view.state_of(&e.id).is_some_and(|s| s.is_hidden())
            })
            .cloned()
            .collect();
        recall_selection_in(query, &visible, Haystack::KeyClaim)
    }

    fn hit<'a>(recall: &'a Value, id: &str) -> Option<&'a Value> {
        recall["hits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["id"] == id)
    }

    fn ids(recall: &Value) -> Vec<String> {
        recall["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// Supersession chains, a two-element `resolves`, an alias pair, lifecycle on with archive,
    /// drop and restore, a conflict, sourced and unsourced rows, and SQL-looking text.
    fn rich() -> Fixture {
        let f = fixture();
        let now = now_ns();
        let old = now - 40 * DAY;
        f.alias("old-project");
        // A chain of three, a resolution of two.
        f.entry("a1000001", "deploy-plan", "nuc runs the deploy plan", old);
        f.entry_with(
            "a1000002",
            "deploy-plan",
            "deploy plan moved to thinkpad",
            old + DAY,
            json!({"supersedes":"a1000001"}),
        );
        f.entry_with(
            "a1000003",
            "deploy-plan",
            "Deploy PLAN runs on thinkpad and nuc",
            old + 2 * DAY,
            json!({"supersedes":"a1000002"}),
        );
        f.entry("r1000001", "recall-timeout", "timeout is 3800 ms", old);
        f.entry("r1000002", "recall-timeout", "timeout is 4300 ms", old);
        f.entry_with(
            "r1000003",
            "recall-timeout",
            "timeout stays 3800 ms after measuring",
            old + DAY,
            json!({"resolves":["r1000001","r1000002"]}),
        );
        // Rows under the old alias, one byte-equal copy under both projects.
        f.entry_in("old-project", "o1000001", "部署", "部署 在 nuc 上執行", old);
        f.entry_in(
            "old-project",
            "o1000002",
            "shared-row",
            "shared alias row",
            old,
        );
        f.entry("o1000002", "shared-row", "shared alias row", old);
        // Lifecycle: archived, dropped, archived then restored, and a conflict pair.
        f.entry("x1000001", "shelf", "alpha shelved deploy note", old);
        f.entry("x1000002", "bin", "deploy dropped entry", old);
        f.entry("x1000003", "back", "alpha restored deploy note", old);
        f.entry("c1000001", "dup-key", "duplicate one alpha", old);
        f.entry("c1000002", "dup-key", "duplicate two beta", old);
        f.entry(
            "m1000001",
            "50%_done",
            "literal percent and underscore",
            old,
        );
        f.entry("m1000002", "it's", "apostrophe a\\b backslash", old);
        f.entry("m1000003", "cfg:timeout", "the cfg value", old);
        f.entry("m1000004", "path.rule", "no /home paths in fixtures", old);
        f.entry(
            "m1000005",
            "Mixed-Case",
            "ClaudeCat lives in its own repo",
            old,
        );
        f.entry(
            "m1000006",
            "alpha-beta-gamma",
            "alpha beta gamma all three",
            old,
        );
        f.entry(
            "m1000007",
            "fresh",
            "a fresh deploy note recorded today",
            now - DAY,
        );
        // Supersession across the aliases: the predecessor exists only under the old alias.
        f.entry_in(
            "old-project",
            "s1000001",
            "rollout",
            "deploy rollout waits on nuc",
            old,
        );
        f.entry_with(
            "s1000002",
            "rollout",
            "deploy rollout moved to thinkpad",
            old + DAY,
            json!({"supersedes":"s1000001"}),
        );
        // A conflict pair split across the aliases.
        f.entry("k1000001", "split-key", "alpha split current side", old);
        f.entry_in(
            "old-project",
            "k1000002",
            "split-key",
            "alpha split alias side",
            old,
        );
        // Due for review, each far from its boundary (the epoch is `now - 30 days`; both paths
        // read the clock separately). Recorded 100 days ago: long-term, forgetting due 40 days
        // ago. Recorded 20 days ago: short-term, expired 13 days ago.
        f.entry(
            "v1000001",
            "stale-deploy",
            "deploy stale note never promoted",
            now - 100 * DAY,
        );
        f.entry(
            "v1000002",
            "expired-timeout",
            "timeout expired short note",
            now - 20 * DAY,
        );
        // A lifecycle row nothing can read.
        f.life_body("l0000007", "{not json");
        f.life("l0000001", "on", None, now - 30 * DAY);
        f.life("l0000002", "archive", Some("x1000001"), now - 10 * DAY);
        f.life("l0000003", "drop", Some("x1000002"), now - 10 * DAY);
        f.life("l0000004", "archive", Some("x1000003"), now - 10 * DAY);
        f.life("l0000005", "restore", Some("x1000003"), now - 9 * DAY);
        f.life("l0000006", "promote", Some("a1000003"), now - 5 * DAY);
        // Sourced rows, through the product's own write.
        memory_store::record_with(
            f.root(),
            "choice",
            "deploy option B is current",
            Path::new("decision.md"),
            1,
            None,
            &f.ledger,
        )
        .unwrap();
        memory_store::record_with(
            f.root(),
            "second-choice",
            "option A kept for the nuc",
            Path::new("decision.md"),
            2,
            None,
            &f.ledger,
        )
        .unwrap();
        f
    }

    const RICH_QUERIES: [&str; 32] = [
        "deploy",
        "deploy plan",
        "DEPLOY Plan",
        "部署",
        "部署 nuc",
        "nuc 部署 zzz",
        "timeout",
        "timeout 3800",
        "cfg:timeout",
        "path.rule",
        "50%_done",
        "it's",
        "a\\b",
        "alpha",
        "alpha beta",
        "alpha beta gamma",
        "shelved",
        "dropped",
        "restored deploy",
        "dup-key",
        "choice option",
        "claudecat",
        "mixed-case",
        "zzz",
        "zzz qqq",
        "the a",
        "deploy thinkpad nuc",
        "ploy nuc",
        "café 部署",
        "option b current",
        "moved thinkpad zzz",
        "shared alias",
    ];

    #[test]
    fn queries_equal_oracle() {
        let f = rich();
        let mut hits = 0;
        let mut due = 0;
        for query in RICH_QUERIES {
            for include_hidden in [false, true] {
                let answer = assert_same(&f, query, include_hidden);
                hits += usize::from(!ids(&answer).is_empty());
                if !include_hidden {
                    due += answer["hits"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|h| h["lifecycle"].as_str().is_some_and(|s| !s.is_empty()))
                        .count();
                }
                assert_eq!(answer["lifecycle_unreadable"], 1, "{query:?}");
            }
        }
        // The fixture really exercises hits, not just empty answers.
        assert!(hits > 30, "{hits}");
        // ... and visible hits that carry a lifecycle state (only due ones can, unhidden).
        assert!(due > 0, "{due}");
        let deploy = assert_same(&f, "deploy", false);
        assert_eq!(
            hit(&deploy, "v1000001").unwrap()["lifecycle"],
            "forgetting_candidate"
        );
        let timeout = assert_same(&f, "timeout", false);
        assert_eq!(
            hit(&timeout, "v1000002").unwrap()["lifecycle"],
            "short_term_expired"
        );
        // The cross-alias predecessor is superseded, and its successor is no conflict.
        assert!(hit(&deploy, "s1000001").is_none());
        assert_ne!(
            hit(&deploy, "s1000002").unwrap()["evidence_status"],
            "conflict"
        );
        // The split pair is a conflict on both sides.
        let alpha = assert_same(&f, "alpha", false);
        for id in ["k1000001", "k1000002"] {
            assert_eq!(
                hit(&alpha, id).unwrap()["evidence_status"],
                "conflict",
                "{id}"
            );
        }
    }

    /// A dropped full match for "deploy plan" (stage 1 finds it, the client hides it) beside a
    /// visible full match for "beta gamma" and for "deploy notes".
    fn two_query_fixture() -> Fixture {
        let f = fixture();
        let now = now_ns();
        f.entry("d1000001", "gone", "deploy plan dropped", now - 20 * DAY);
        f.entry("d1000002", "half", "deploy notes", now - 20 * DAY);
        f.entry("d1000003", "other-half", "plan alpha", now - 20 * DAY);
        f.entry("e1000001", "pair", "beta gamma visible", now - 20 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life("l2", "drop", Some("d1000001"), now - 10 * DAY);
        f
    }

    #[test]
    fn two_queries_resend_once_and_answer_as_alone() {
        let f = two_query_fixture();
        // Alone, only the second query needs the resend.
        assert_eq!(f.search_trips("beta gamma", false), 1);
        assert_eq!(f.search_trips("deploy plan", false), 2);
        let project = f.project();
        for queries in [["beta gamma", "deploy plan"], ["deploy plan", "beta gamma"]] {
            // With hidden entries kept the dropped row is a visible full match: no resend.
            for (include_hidden, trips) in [(false, 2), (true, 1)] {
                let now = now_ns();
                let searched = f.search_many(&queries, include_hidden, now);
                assert_eq!(searched.trips, trips, "{queries:?} {include_hidden}");
                for query in queries {
                    let whole = memory_store::select_whole_with(
                        &f.ledger,
                        &project,
                        query,
                        include_hidden,
                        now,
                    )
                    .unwrap();
                    assert_eq!(
                        select_in(&searched, query, include_hidden),
                        (whole.ids, whole.partial),
                        "{queries:?} {query:?} {include_hidden}"
                    );
                }
            }
        }
        // The answers the comparison covered are the interesting ones.
        let now = now_ns();
        let searched = f.search_many(&["beta gamma", "deploy plan"], false, now);
        assert_eq!(
            select_in(&searched, "deploy plan", false),
            (vec!["d1000002".to_string(), "d1000003".to_string()], true)
        );
        assert_eq!(
            select_in(&searched, "beta gamma", false),
            (vec!["e1000001".to_string()], false)
        );
    }

    #[test]
    fn two_queries_without_a_resend_cost_one_trip() {
        let f = two_query_fixture();
        let project = f.project();
        let queries = ["beta gamma", "deploy notes"];
        for include_hidden in [false, true] {
            let now = now_ns();
            let searched = f.search_many(&queries, include_hidden, now);
            assert_eq!(searched.trips, 1, "{include_hidden}");
            for query in queries {
                let whole = memory_store::select_whole_with(
                    &f.ledger,
                    &project,
                    query,
                    include_hidden,
                    now,
                )
                .unwrap();
                assert!(!whole.ids.is_empty(), "{query:?}");
                assert_eq!(
                    select_in(&searched, query, include_hidden),
                    (whole.ids, whole.partial),
                    "{query:?} {include_hidden}"
                );
            }
        }
    }

    #[test]
    fn select_search_equals_select_whole() {
        let f = rich();
        let project = f.project();
        for query in RICH_QUERIES {
            for include_hidden in [false, true] {
                let now = now_ns();
                let whole = memory_store::select_whole_with(
                    &f.ledger,
                    &project,
                    query,
                    include_hidden,
                    now,
                )
                .unwrap();
                let memory_store::Selection {
                    ids,
                    partial,
                    trips,
                    ..
                } = memory_store::select_search_with(
                    &f.ledger,
                    &project,
                    query,
                    include_hidden,
                    now,
                )
                .unwrap();
                assert_eq!(
                    (ids, partial),
                    (whole.ids, whole.partial),
                    "{query:?} {include_hidden}"
                );
                assert!((1..=2).contains(&trips));
            }
        }
    }

    #[test]
    fn empty_query_equals_oracle() {
        let f = rich();
        for include_hidden in [false, true] {
            let answer = assert_same(&f, "", include_hidden);
            assert!(ids(&answer).len() > 10);
        }
    }

    #[test]
    fn thirty_term_query_equals_oracle() {
        let f = rich();
        let words: Vec<&str> =
            "the deploy plan for the nuc and thinkpad was moved after we measured \
             the timeout of 3800 ms on the shared alias row with claudecat in its own repo \
             thinkpad"
                .split_whitespace()
                .collect();
        assert_eq!(words.len(), 30);
        let query = words.join(" ");
        for include_hidden in [false, true] {
            assert_same(&f, &query, include_hidden);
        }
        // A thirty-term query every term of which one entry holds: a full match.
        let f = fixture();
        let claim: Vec<String> = (0..30).map(|i| format!("w{i:02}x")).collect();
        f.entry("t1000001", "long", &claim.join(" "), now_ns() - DAY);
        f.entry("t1000002", "short", "w00x w01x", now_ns() - DAY);
        for include_hidden in [false, true] {
            let answer = assert_same(&f, &claim.join(" "), include_hidden);
            assert_eq!(ids(&answer), ["t1000001"]);
        }
    }

    #[test]
    fn literal_special_characters_match_as_today() {
        let f = fixture();
        let at = now_ns() - DAY;
        f.entry("s1000001", "pct", "50%_done literally", at);
        f.entry("s1000002", "pct-decoy", "50xxdone and 50done", at);
        f.entry("s1000003", "quote", "it's quoted", at);
        f.entry("s1000004", "quote-decoy", "its not quoted", at);
        f.entry("s1000005", "slash", "a\\b with a backslash", at);
        f.entry("s1000006", "slash-decoy", "ab without one", at);
        for (query, want) in [
            ("50%_done", "s1000001"),
            ("it's", "s1000003"),
            ("a\\b", "s1000005"),
        ] {
            for include_hidden in [false, true] {
                let answer = assert_same(&f, query, include_hidden);
                assert_eq!(ids(&answer), [want], "{query}");
            }
        }
    }

    #[test]
    fn fresh_database_is_absent_not_an_error() {
        let f = fixture();
        let answer = f.new_recall("deploy", false).unwrap();
        assert_eq!(answer["ledger_status"], "absent");
        assert_eq!(answer["hits"], json!([]));
        // The read created the tables, as today's read does.
        let tables =
            f.db.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
                vec![],
            )
            .unwrap();
        let names: Vec<Cell> = tables.rows.into_iter().map(|mut r| r.remove(0)).collect();
        for t in [
            "project_aliases",
            "project_memory",
            "project_memory_lifecycle",
            "project_questions",
            "project_reminders",
        ] {
            assert!(names.contains(&Cell::Text(t.into())), "{t}: {names:?}");
        }
        assert_same(&f, "deploy", false);
        assert_same(&f, "", true);
    }

    #[test]
    fn cafe_query_falls_through_to_partial_as_today() {
        let f = fixture();
        let at = now_ns() - DAY;
        f.entry("k1000001", "部署", "部署 在 nuc 上執行", at);
        f.entry("k1000002", "menu", "Café menu", at);
        f.entry("k1000003", "other", "nothing here", at);
        let answer = assert_same(&f, "café 部署", false);
        assert_eq!(answer["match"], "partial");
        assert_eq!(ids(&answer), ["k1000001", "k1000002"]);
        assert_eq!(f.trips("café 部署", false), 1);
        assert_eq!(f.search_trips("café 部署", false), 1);
    }

    #[test]
    fn full_match_only_on_a_dropped_entry() {
        let f = fixture();
        let now = now_ns();
        f.entry("d1000001", "gone", "deploy plan dropped", now - 20 * DAY);
        f.entry("d1000002", "half", "deploy notes", now - 20 * DAY);
        f.entry("d1000003", "other-half", "plan alpha", now - 20 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life("l2", "drop", Some("d1000001"), now - 10 * DAY);
        let answer = assert_same(&f, "deploy plan", false);
        assert_eq!(answer["match"], "partial");
        assert_eq!(ids(&answer), ["d1000002", "d1000003"]);
        assert_eq!(f.trips("deploy plan", false), 2);
        assert_eq!(f.search_trips("deploy plan", false), 2);
        // With hidden entries kept the dropped one is the full match, in one trip.
        let all = assert_same(&f, "deploy plan", true);
        assert_eq!(ids(&all), ["d1000001"]);
        assert_eq!(f.trips("deploy plan", true), 1);
    }

    #[test]
    fn no_match_costs_one_trip() {
        let f = rich();
        assert_same(&f, "zzz qqq", false);
        assert_eq!(f.trips("zzz qqq", false), 1);
        assert_eq!(f.trips("zzz qqq", true), 1);
        assert_eq!(f.trips("deploy", false), 1);
    }

    #[test]
    fn excluded_entry_the_fold_shows_is_resent() {
        let now = now_ns();
        let cases: [(&str, &str); 3] = [
            (
                "plus-prefixed restore",
                &json!({"id":"l3","kind":"restore","entry":"e1000001",
                    "recorded_at":format!("+{}", now - 5 * DAY)})
                .to_string(),
            ),
            (
                "archive with a numeric note",
                &json!({"id":"l3","kind":"archive","entry":"e1000002","note":5,
                    "recorded_at":(now - 5 * DAY).to_string()})
                .to_string(),
            ),
            (
                "duplicated kind key",
                &format!(
                    r#"{{"id":"l3","kind":"archive","entry":"e1000002","recorded_at":"{}","kind":"restore"}}"#,
                    now - 5 * DAY
                ),
            ),
        ];
        for (name, body) in cases {
            let f = fixture();
            f.entry("e1000001", "first", "alpha first", now - 20 * DAY);
            f.entry("e1000002", "second", "alpha second", now - 20 * DAY);
            f.life("l1", "on", None, now - 30 * DAY);
            // e1000001 is archived by a canonical row; the odd row decides what the fold shows.
            f.life("l2", "archive", Some("e1000001"), now - 10 * DAY);
            f.life_body("l3", body);
            let answer = assert_same(&f, "alpha", false);
            assert!(!ids(&answer).is_empty(), "{name}");
            assert_eq!(f.trips("alpha", false), 2, "{name}");
            assert_eq!(f.search_trips("alpha", false), 2, "{name}");
        }
    }

    #[test]
    fn same_id_under_two_aliases() {
        let now = now_ns();
        for stamped in [false, true] {
            let f = fixture();
            f.alias("old-project");
            let body = Fixture::body("i1000001", "twin", "deploy twin row", now - DAY, json!({}));
            let mut other = body.clone();
            if stamped {
                other["recorded_machine"] = json!("m-1");
                other["recorded_machine_source"] = json!("test");
            }
            f.insert_in(
                "project_memory",
                &f.project(),
                "i1000001",
                &body.to_string(),
            );
            f.insert_in(
                "project_memory",
                "old-project",
                "i1000001",
                &other.to_string(),
            );
            let answer = assert_same(&f, "deploy", false);
            assert_eq!(ids(&answer), ["i1000001"], "stamped {stamped}");
            assert_ne!(answer["hits"][0]["evidence_status"], "conflict");
        }
    }

    #[test]
    fn conflict_across_a_hidden_partner() {
        let f = fixture();
        let now = now_ns();
        f.entry("h1000001", "dup", "visible alpha", now - 20 * DAY);
        f.entry("h1000002", "dup", "hidden beta", now - 20 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life("l2", "archive", Some("h1000002"), now - 10 * DAY);
        let answer = assert_same(&f, "alpha", false);
        assert_eq!(ids(&answer), ["h1000001"]);
        assert_eq!(answer["hits"][0]["evidence_status"], "conflict");
    }

    #[test]
    fn subset_fold_equals_whole_fold() {
        let f = fixture();
        let now = now_ns();
        f.entry("f1000001", "match-me", "deploy target", now - 40 * DAY);
        f.entry("f1000002", "match-me-too", "deploy other", now - 2 * DAY);
        f.entry("f1000003", "elsewhere", "unrelated one", now - 40 * DAY);
        f.entry("f1000004", "elsewhere-too", "unrelated two", now - 40 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life("l2", "promote", Some("f1000001"), now - 20 * DAY);
        f.life("l3", "archive", Some("f1000003"), now - 10 * DAY);
        f.life("l4", "drop", Some("f1000004"), now - 10 * DAY);
        f.life("l5", "archive", Some("gone0001"), now - 10 * DAY);
        for include_hidden in [false, true] {
            assert_same(&f, "deploy", include_hidden);
        }
        let at = now_ns();
        let queries = ["deploy".to_string()];
        let searched = memory_store::search_with(
            &f.ledger,
            &f.project(),
            &SearchRequest {
                queries: &queries,
                haystack: Haystack::KeyClaim,
                include_hidden: false,
            },
            at,
        )
        .map(answered)
        .unwrap();
        let (whole, rows) = f.ledger.read_with_lifecycle(&f.project()).unwrap();
        let view = fold(&whole, rows.events, rows.unreadable, at, None);
        assert_eq!(searched.ledger.entries.len(), 2);
        for entry in &searched.ledger.entries {
            let (a, b) = (
                searched.view.entries.get(&entry.id).unwrap(),
                view.entries.get(&entry.id).unwrap(),
            );
            assert_eq!(a.state, b.state, "{}", entry.id);
            assert_eq!(a.due_at_ns, b.due_at_ns, "{}", entry.id);
        }
    }

    #[test]
    fn predecessor_not_returned_is_not_an_error() {
        let f = fixture();
        let at = now_ns() - DAY;
        f.entry("p1000001", "plan", "old wording zzz", at);
        f.entry_with(
            "p1000002",
            "plan",
            "deploy current wording",
            at,
            json!({"supersedes":"p1000001"}),
        );
        let answer = assert_same(&f, "deploy", false);
        assert_eq!(ids(&answer), ["p1000002"]);
    }

    /// Which `rows_read` counters a `Recording` transport deletes from the server's answers.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Strip {
        None,
        /// The first in each response: on a pipeline, the first statement's.
        First,
        All,
    }

    /// Passes every request to the in-memory server and adds up every `rows_read` its responses
    /// carry (any depth: a pipeline's per-statement results and a batch's step results alike).
    struct Recording {
        inner: Arc<SqliteTransport>,
        reported: Arc<std::sync::atomic::AtomicU64>,
        strip: Strip,
    }

    fn rows_read_in(value: &mut Value, strip: &mut Strip) -> u64 {
        match value {
            Value::Object(map) => {
                let mut sum = 0;
                if *strip != Strip::None && map.contains_key("rows_read") {
                    map.remove("rows_read");
                    if *strip == Strip::First {
                        *strip = Strip::None;
                    }
                } else if let Some(n) = map.get("rows_read").and_then(Value::as_u64) {
                    sum += n;
                }
                for v in map.values_mut() {
                    sum += rows_read_in(v, strip);
                }
                sum
            }
            Value::Array(items) => items.iter_mut().map(|v| rows_read_in(v, strip)).sum(),
            _ => 0,
        }
    }

    impl Transport for Recording {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, cort::remote::RemoteError> {
            let bytes = self.inner.post(url, body, headers)?;
            let mut value: Value = serde_json::from_slice(&bytes).unwrap();
            let mut strip = self.strip;
            let n = rows_read_in(&mut value, &mut strip);
            self.reported.fetch_add(n, Ordering::SeqCst);
            Ok(serde_json::to_vec(&value).unwrap())
        }
    }

    impl Fixture {
        fn recording(&self, strip: Strip) -> (Ledger, Arc<std::sync::atomic::AtomicU64>) {
            let reported = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let ledger = Ledger::with_transport(
                "https://example.test",
                "test",
                Box::new(Recording {
                    inner: self.inner.clone(),
                    reported: reported.clone(),
                    strip,
                }),
            )
            .unwrap();
            (ledger, reported)
        }
    }

    #[test]
    fn server_rows_read_is_what_the_server_reported() {
        let f = fixture();
        let now = now_ns();
        f.entry("d1000001", "gone", "deploy plan dropped", now - 20 * DAY);
        f.entry("d1000002", "half", "deploy notes", now - 20 * DAY);
        f.entry("d1000003", "other-half", "plan alpha", now - 20 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life("l2", "drop", Some("d1000001"), now - 10 * DAY);
        let project = f.project();
        // Every statement of both trips (the resend) and of the whole read, summed; the
        // in-memory server reports returned rows, and `BEGIN`/`COMMIT` return none.
        let (ledger, reported) = f.recording(Strip::None);
        let search =
            memory_store::select_search_with(&ledger, &project, "deploy plan", false, now).unwrap();
        assert_eq!(search.trips, 2);
        let n = reported.swap(0, Ordering::SeqCst);
        assert!(n > 0);
        assert_eq!(search.server_rows_read, Some(n));
        let whole =
            memory_store::select_whole_with(&ledger, &project, "deploy plan", false, now).unwrap();
        let n = reported.swap(0, Ordering::SeqCst);
        // Three memory rows and two lifecycle rows came back.
        assert_eq!(n, 5);
        assert_eq!(whole.server_rows_read, Some(n));
        assert_eq!((whole.rows, whole.trips), (3, 1));
        assert_eq!(
            (&search.ids, search.partial),
            (&whole.ids, whole.partial),
            "the counter changes no answer"
        );
        // A server that reports no counter is unknown, not zero; so is one that leaves out a
        // single statement's, since the sum would understate the read.
        let (ledger, _) = f.recording(Strip::All);
        let search =
            memory_store::select_search_with(&ledger, &project, "deploy plan", false, now).unwrap();
        assert_eq!(search.server_rows_read, None);
        assert_eq!(search.ids, whole.ids);
        let all =
            memory_store::select_whole_with(&ledger, &project, "deploy plan", false, now).unwrap();
        assert_eq!(all.server_rows_read, None);
        let (ledger, _) = f.recording(Strip::First);
        let one =
            memory_store::select_whole_with(&ledger, &project, "deploy plan", false, now).unwrap();
        assert_eq!(one.server_rows_read, None);
        assert_eq!(one.ids, whole.ids);
    }

    #[test]
    fn reads_only_matching_rows() {
        let f = fixture();
        let at = now_ns() - DAY;
        for i in 0..60 {
            let claim = match i {
                0..=2 => format!("deploy note {i}"),
                3..=5 => format!("alpha beta pair {i}"),
                6 => "beta gamma pair".to_string(),
                7..=9 => format!("alpha alone {i}"),
                10..=12 => format!("gamma alone {i}"),
                _ => format!("filler row {i}"),
            };
            f.entry(&format!("n{i:07}"), &format!("key-{i}"), &claim, at);
        }
        let project = f.project();
        let now = now_ns();
        let memory_store::Selection {
            ids,
            partial,
            rows,
            trips,
            ..
        } = memory_store::select_search_with(&f.ledger, &project, "deploy", false, now).unwrap();
        assert_eq!((ids.len(), partial, trips), (3, false, 1));
        assert!(rows <= 3, "{rows}");
        let memory_store::Selection {
            ids,
            partial,
            rows,
            trips,
            ..
        } = memory_store::select_search_with(&f.ledger, &project, "alpha beta gamma", false, now)
            .unwrap();
        assert!(partial);
        assert_eq!(trips, 1);
        // Rows holding two or more of the three terms: n0000003..5 and n0000006.
        assert!(rows <= 4, "{rows}");
        assert_eq!(ids.len(), 4);
        assert_eq!(
            (ids, partial),
            memory_store::select_whole_with(&f.ledger, &project, "alpha beta gamma", false, now)
                .map(|s| (s.ids, s.partial))
                .unwrap()
        );
    }

    #[test]
    fn two_errors_report_the_earlier_phase() {
        let f = fixture();
        let at = now_ns() - DAY;
        f.raw("a-bad", "{not json");
        f.entry("b1000001", "k1", "c", at);
        f.entry_with("c1000001", "k2", "c", at, json!({"supersedes":"b1000001"}));
        let new = f.new_recall("c", false).unwrap_err();
        let old = f.old_recall("c", false).unwrap_err();
        assert_eq!(new, old);
        assert!(
            matches!(&new, LedgerError::Message(m) if m.starts_with("invalid remote memory row")),
            "{new:?}"
        );
    }

    #[test]
    fn screen_messages_equal_today() {
        type Seed = Box<dyn Fn(&Fixture)>;
        let at = now_ns() - DAY;
        let cases: Vec<(&str, Seed)> = vec![
            ("invalid json", Box::new(|f| f.raw("x", "{not json"))),
            (
                "missing key",
                Box::new(|f| {
                    f.raw(
                        "x",
                        r#"{"id":"x","claim":"c","source":"s","source_line":1,"source_sha256":"","recorded_at":"1"}"#,
                    )
                }),
            ),
            ("blank key", Box::new(move |f| f.entry("x", "  ", "c", at))),
            (
                "supersedes a missing id",
                Box::new(move |f| f.entry_with("x", "k", "c", at, json!({"supersedes":"gone"}))),
            ),
            (
                "supersedes its own id",
                Box::new(move |f| f.entry_with("x", "k", "c", at, json!({"supersedes":"x"}))),
            ),
            (
                "supersedes a different key",
                Box::new(move |f| {
                    f.entry("a", "k1", "c", at);
                    f.entry_with("b", "k2", "c", at, json!({"supersedes":"a"}));
                }),
            ),
            (
                "one id under two aliases with different claims",
                Box::new(move |f| {
                    f.alias("old-project");
                    f.entry("x", "k", "first claim", at);
                    f.entry_in("old-project", "x", "k", "second claim", at);
                }),
            ),
            (
                "resolves a missing id",
                Box::new(move |f| {
                    f.entry("a", "k", "c", at);
                    f.entry_with("x", "k", "c", at, json!({"resolves":["gone","a"]}));
                }),
            ),
            (
                "resolves a different key",
                Box::new(move |f| {
                    f.entry("a", "k", "c", at);
                    f.entry("b", "k2", "c", at);
                    f.entry_with("x", "k", "c", at, json!({"resolves":["a","b"]}));
                }),
            ),
            (
                "resolves its own id",
                Box::new(move |f| {
                    f.entry("a", "k", "c", at);
                    f.entry_with("x", "k", "c", at, json!({"resolves":["a","x"]}));
                }),
            ),
        ];
        for (name, seed) in cases {
            let f = fixture();
            seed(&f);
            // Plus a healthy row, so the error does not depend on the query matching it.
            f.entry("ok000001", "fine", "unrelated row", at);
            let project = f.project();
            for query in ["c", "unrelated", "zzz", ""] {
                let new = f.new_recall(query, false).expect_err(name);
                let old = f.old_recall(query, false).expect_err(name);
                assert_eq!(new, old, "{name} {query:?}");
                // `--also`, `--keys` and the eval's replay fall back to the same whole read.
                let also = strings(&[query, "zzz"]);
                let many = memory_store::recall_many_with(f.root(), &also, &f.ledger, false);
                assert_eq!(many.expect_err(name), old, "{name} {query:?} --also");
                let keys =
                    memory_store::recall_keys_with(f.root(), &strings(&[query]), &f.ledger, false);
                assert_eq!(keys.expect_err(name), old, "{name} {query:?} --keys");
                let now = now_ns();
                let replay =
                    memory_store::select_search_with(&f.ledger, &project, query, false, now);
                let base = memory_store::select_whole_with(&f.ledger, &project, query, false, now);
                assert_eq!(replay.expect_err(name), base.expect_err(name), "{name}");
            }
        }
        // The alias hint is part of the compared text.
        let f = fixture();
        f.entry_with("x", "k", "c", at, json!({"supersedes":"gone"}));
        assert!(matches!(f.new_recall("c", false),
            Err(LedgerError::Message(m)) if m.ends_with("; re-add the alias with cort memory alias add")));
        // The `resolves` cases fail on the element they name, not on something incidental.
        let f = fixture();
        f.entry("a", "k", "c", at);
        f.entry_with("x", "k", "c", at, json!({"resolves":["gone","a"]}));
        assert!(matches!(f.new_recall("c", false),
            Err(LedgerError::Message(m)) if m.starts_with("memory x supersedes missing id gone")));
        for (named, want) in [
            ("b", "memory x has invalid supersession"),
            // Naming itself never sorts: the topological order reports it before `validate` can.
            ("x", "memory supersession has a missing id or cycle"),
        ] {
            let f = fixture();
            f.entry("a", "k", "c", at);
            f.entry("b", "k2", "c", at);
            f.entry_with("x", "k", "c", at, json!({"resolves":["a", named]}));
            assert_eq!(
                f.new_recall("c", false).unwrap_err(),
                LedgerError::Message(want.to_string()),
                "{named}"
            );
        }
    }

    /// A blank-id fixture: an id of spaces is suspect to the screen and accepted by today's whole
    /// read, beside an ordinary entry.
    fn blank_id_fixture() -> Fixture {
        let f = fixture();
        let at = now_ns() - DAY;
        f.entry("   ", "spaced", "deploy under a blank id", at);
        f.entry("s1000001", "deploy-plan", "deploy plan on nuc", at);
        f
    }

    /// A suspect that passes today's checks: recall makes one more request, the whole read, and
    /// answers exactly as it does.
    #[test]
    fn a_suspect_row_sends_recall_to_the_whole_read() {
        let f = blank_id_fixture();
        let answer = assert_same(&f, "deploy", false);
        let mut found = ids(&answer);
        found.sort();
        assert_eq!(found, ["   ", "s1000001"]);
        assert_eq!(
            f.trips("deploy", false),
            2,
            "the search, then the whole read"
        );
        let (ledger, sql) = f.logging();
        memory_store::recall_all_with(f.root(), "deploy", &ledger, false).unwrap();
        let sent: Vec<String> = sql.lock().unwrap().drain(..).collect();
        memory_store::recall_whole_with(f.root(), "deploy", &ledger, false).unwrap();
        let whole: Vec<String> = sql.lock().unwrap().drain(..).collect();
        assert_eq!(sent.len(), 1 + whole.len(), "{sent:?}");
        assert!(sent[0].contains("instr("), "the search first");
        assert_eq!(sent[1..], whole[..], "then exactly the whole read");
        // `--also` and `--keys` answer over the whole read too, in the same two requests.
        let queries = strings(&["deploy", "nuc"]);
        let before = f.posts.load(Ordering::SeqCst);
        let v = memory_store::recall_many_with(f.root(), &queries, &f.ledger, false).unwrap();
        assert_eq!(f.posts.load(Ordering::SeqCst) - before, 2);
        for (i, query) in queries.iter().enumerate() {
            let alone = f.old_recall(query, false).unwrap();
            assert_eq!(query_ids(&v["queries"][i]), ids(&alone), "{query}");
        }
        let before = f.posts.load(Ordering::SeqCst);
        let v = memory_store::recall_keys_with(f.root(), &strings(&["deploy"]), &f.ledger, false)
            .unwrap();
        assert_eq!(f.posts.load(Ordering::SeqCst) - before, 2);
        assert_eq!(query_ids(&v), ["s1000001"]);
        // The eval's replay selects as the whole read does and counts both reads.
        let project = f.project();
        let now = now_ns();
        let replay =
            memory_store::select_search_with(&f.ledger, &project, "deploy", false, now).unwrap();
        let base =
            memory_store::select_whole_with(&f.ledger, &project, "deploy", false, now).unwrap();
        assert_eq!((&replay.ids, replay.partial), (&base.ids, base.partial));
        assert_eq!(replay.trips, 2);
        assert_eq!(
            replay.rows,
            2 + base.rows,
            "two stage rows, then the whole read's"
        );
        // The search itself says why it stopped.
        let searched = memory_store::search_with(
            &f.ledger,
            &project,
            &SearchRequest {
                queries: &strings(&["deploy"]),
                haystack: Haystack::KeyClaim,
                include_hidden: false,
            },
            now,
        )
        .unwrap();
        assert!(matches!(
            searched,
            memory_store::SearchOutcome::Suspect { trips: 1, .. }
        ));
    }

    /// Passes requests through and, once the first is answered, inserts one row.
    struct InsertAfterFirst {
        inner: Arc<SqliteTransport>,
        posts: Arc<AtomicUsize>,
        sql: String,
    }

    impl Transport for InsertAfterFirst {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, cort::remote::RemoteError> {
            let answer = self.inner.post(url, body, headers)?;
            if self.posts.fetch_add(1, Ordering::SeqCst) == 0 {
                let request = json!({"requests":[
                    {"type":"execute","stmt":{"sql":self.sql,"args":[]}},{"type":"close"}]});
                self.inner
                    .post(url, request.to_string().as_bytes(), headers)
                    .unwrap();
            }
            Ok(answer)
        }
    }

    /// A suspect row that lands between the two search trips sends recall to the whole read too.
    #[test]
    fn a_suspect_row_in_the_second_trip_sends_recall_to_the_whole_read() {
        let f = two_query_fixture();
        assert_eq!(f.search_trips("deploy plan", false), 2);
        let body = Fixture::body(
            "   ",
            "late",
            "deploy plan landed late",
            now_ns() - DAY,
            json!({}),
        );
        let sql = format!(
            "INSERT INTO project_memory VALUES ('{}', '   ', '{}')",
            f.project(),
            body.to_string().replace('\'', "''")
        );
        let posts = Arc::new(AtomicUsize::new(0));
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(InsertAfterFirst {
                inner: f.inner.clone(),
                posts: posts.clone(),
                sql,
            }),
        )
        .unwrap();
        let new = memory_store::recall_all_with(f.root(), "deploy plan", &ledger, false).unwrap();
        assert_eq!(
            posts.load(Ordering::SeqCst),
            3,
            "two search trips, then the whole read"
        );
        let old = f.old_recall("deploy plan", false).unwrap();
        assert_eq!(serde_json::to_value(new).unwrap(), old);
        assert!(ids(&old).contains(&"   ".to_string()), "{old}");
    }

    #[test]
    fn lifecycle_unreadable_is_reported_as_today() {
        let f = fixture();
        let now = now_ns();
        f.entry("u1000001", "live", "alpha live", now - DAY);
        f.life("l1", "on", None, now - 2 * DAY);
        f.life_body("l2", "{not json");
        let answer = assert_same(&f, "alpha", false);
        assert_eq!(answer["lifecycle_unreadable"], 1);
        assert!(answer["lifecycle_hint"].is_string());
    }

    fn strings(queries: &[&str]) -> Vec<String> {
        queries.iter().map(|q| q.to_string()).collect()
    }

    fn query_ids(query: &Value) -> Vec<String> {
        query["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["id"].as_str().unwrap().to_string())
            .collect()
    }

    const OTHER: &str = "\"<the same words in the other language>\"";

    fn request<'a>(
        query: &'a str,
        also: &'a [String],
        keys: bool,
        include_hidden: bool,
    ) -> memory_store::RecallRequest<'a> {
        memory_store::RecallRequest {
            query,
            also,
            keys,
            include_hidden,
        }
    }

    fn report(f: &Fixture, req: &memory_store::RecallRequest) -> Value {
        memory_store::recall_report_with(f.root(), req, &f.ledger).unwrap()
    }

    /// The command's branch selection: one query without `--also` or `--keys` is today's
    /// `recall_all_with` answer, byte for byte as the handler serializes it. (The binary only
    /// speaks HTTPS, so no process test can serve it a database; the handler adds its own three
    /// fields to this value unchanged.)
    #[test]
    fn single_query_cli_output_is_unchanged() {
        let f = rich();
        for query in ["deploy plan", "部署", "alpha zzz"] {
            for include_hidden in [false, true] {
                let new = report(&f, &request(query, &[], false, include_hidden));
                let old = f.old_recall(query, include_hidden).unwrap();
                assert_eq!(
                    serde_json::to_string_pretty(&new).unwrap(),
                    serde_json::to_string_pretty(&old).unwrap(),
                    "{query:?} {include_hidden}"
                );
            }
        }
        // The partial form is among the compared answers.
        assert_eq!(
            f.old_recall("alpha zzz", false).unwrap()["match"],
            "partial"
        );
    }

    /// `部署` and `deploy` share one entry; each query alone also finds one the other does not.
    fn also_fixture() -> Fixture {
        let f = fixture();
        let at = now_ns() - 20 * DAY;
        f.entry("b1000001", "deploy-plan", "部署 runs on the nuc", at);
        f.entry("b1000002", "other", "部署 only in chinese", at);
        f.entry("b1000003", "deploy-x", "english only", at);
        f
    }

    #[test]
    fn also_shape() {
        let f = also_fixture();
        let queries = strings(&["部署", "deploy"]);
        let before = f.posts.load(Ordering::SeqCst);
        let v = memory_store::recall_many_with(f.root(), &queries, &f.ledger, false).unwrap();
        assert_eq!(f.posts.load(Ordering::SeqCst) - before, 1, "one search");
        assert_eq!(report(&f, &request("部署", &queries[1..], false, false)), v);
        assert_eq!(v["ledger_status"], "present");
        assert!(v.get("match").is_none(), "{v}");
        assert!(v.get("next").is_none(), "{v}");
        let qs = v["queries"].as_array().unwrap();
        assert_eq!(qs.len(), 2);
        for (i, query) in queries.iter().enumerate() {
            let alone = f.old_recall(query, false).unwrap();
            assert_eq!(qs[i]["query"], query.as_str());
            assert_eq!(qs[i]["match_kind"], "all");
            assert_eq!(query_ids(&qs[i]), ids(&alone), "{query}");
            for h in qs[i]["hits"].as_array().unwrap() {
                assert_eq!(h.as_object().unwrap().len(), 1, "{h}");
            }
        }
        // Query order, each query's own order, the first occurrence kept.
        assert_eq!(ids(&v), ["b1000001", "b1000002", "b1000003"]);
        let both = hit(&v, "b1000001").unwrap();
        assert_eq!(both["matched_queries"], json!([0, 1]));
        assert!(both.get("matched_terms").is_none());
        assert_eq!(hit(&v, "b1000002").unwrap()["matched_queries"], json!([0]));
        assert_eq!(hit(&v, "b1000003").unwrap()["matched_queries"], json!([1]));
        // A top-level hit is today's hit for its entry, plus `matched_queries`.
        let alone = f.old_recall("部署", false).unwrap();
        let mut expected = hit(&alone, "b1000001").unwrap().clone();
        expected["matched_queries"] = json!([0, 1]);
        assert_eq!(both, &expected);
    }

    #[test]
    fn also_partial_hits_carry_their_own_matched_terms() {
        let f = also_fixture();
        let queries = strings(&["部署", "deploy zzz"]);
        let v = memory_store::recall_many_with(f.root(), &queries, &f.ledger, false).unwrap();
        let qs = v["queries"].as_array().unwrap();
        assert_eq!(qs[0]["match_kind"], "all");
        assert_eq!(qs[1]["match_kind"], "partial");
        let alone = f.old_recall("deploy zzz", false).unwrap();
        assert_eq!(alone["match"], "partial");
        let want: Vec<Value> = alone["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| json!({"id": h["id"], "matched_terms": h["matched_terms"]}))
            .collect();
        assert!(!want.is_empty());
        assert_eq!(qs[1]["hits"], Value::Array(want));
        for h in v["hits"].as_array().unwrap() {
            assert!(h.get("matched_terms").is_none(), "{h}");
        }
    }

    /// The conflict set is the database's, over every row: a partner the query does not match is
    /// still counted (fails if `recall_each` counts over the returned rows alone).
    #[test]
    fn also_conflict_counts_a_partner_the_query_did_not_match() {
        let f = fixture();
        let at = now_ns() - 20 * DAY;
        f.entry("c2000001", "dup-key", "alpha one", at);
        f.entry("c2000002", "dup-key", "beta two", at);
        let v =
            memory_store::recall_many_with(f.root(), &strings(&["alpha", "zzz"]), &f.ledger, false)
                .unwrap();
        assert_eq!(ids(&v), ["c2000001"]);
        assert_eq!(hit(&v, "c2000001").unwrap()["evidence_status"], "conflict");
        assert_eq!(
            hit(&f.old_recall("alpha", false).unwrap(), "c2000001").unwrap()["evidence_status"],
            "conflict"
        );
    }

    #[test]
    fn keys_rows() {
        let f = fixture();
        let at = now_ns() - 20 * DAY;
        f.entry("k2000001", "deploy-plan", "nuc runs it", at);
        f.entry("k2000002", "other", "deploy plan in the claim only", at);
        f.entry("k2000003", "deploy-dup", "first", at);
        f.entry("k2000004", "deploy-dup", "second", at + 1);
        let v = memory_store::recall_keys_with(f.root(), &strings(&["deploy"]), &f.ledger, false)
            .unwrap();
        assert_eq!(report(&f, &request("deploy", &[], true, false)), v);
        let next = format!("cort memory recall \"deploy\" --also {OTHER}");
        assert_eq!(v["next"], next.as_str());
        assert_eq!(v["ledger_status"], "present");
        // The key alone is searched: the claim-only match is today's, not layer 1's.
        let whole = f.old_recall("deploy", false).unwrap();
        assert!(hit(&whole, "k2000002").is_some());
        assert_eq!(ids(&v), ["k2000003", "k2000004", "k2000001"]);
        assert_eq!(
            v["queries"],
            json!([{"query":"deploy","match_kind":"all",
                "hits":[{"id":"k2000003"},{"id":"k2000004"},{"id":"k2000001"}]}])
        );
        let utc = hit(&whole, "k2000001").unwrap()["recorded_at_utc"].clone();
        assert!(utc.is_string());
        // One query: no `matched_queries`.
        assert_eq!(
            v["hits"],
            json!([
                {"key":"deploy-dup","id":"k2000003","recorded_at_utc":utc,"conflict":true},
                {"key":"deploy-dup","id":"k2000004","recorded_at_utc":utc,"conflict":true},
                {"key":"deploy-plan","id":"k2000001","recorded_at_utc":utc},
            ])
        );
        let utc = utc.as_str().unwrap();
        let lean = memory_store::render_keys_lean(&v);
        assert_eq!(
            lean,
            format!(
                "deploy-dup\tk2000003\t{utc}\t-\tconflict\n\
                 deploy-dup\tk2000004\t{utc}\t-\tconflict\n\
                 deploy-plan\tk2000001\t{utc}\t-\t-\n\
                 {next}\n"
            )
        );
        assert_five_fields(&lean);
        assert_eq!(memory_store::lifecycle_unreadable_line(&v), None);
    }

    /// Hit lines are exactly five TAB-separated non-empty fields; every other line has no TAB.
    fn assert_five_fields(lean: &str) {
        for line in lean.lines().filter(|l| l.contains('\t')) {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 5, "{line:?}");
            assert!(fields.iter().all(|f| !f.trim().is_empty()), "{line:?}");
        }
    }

    #[test]
    fn keys_lean_status_lines() {
        let f = fixture();
        let next = format!("cort memory recall \"deploy zzz\" --also {OTHER}");
        // No entry at all.
        let v =
            memory_store::recall_keys_with(f.root(), &strings(&["deploy zzz"]), &f.ledger, false)
                .unwrap();
        assert_eq!(v["ledger_status"], "absent");
        assert_eq!(
            memory_store::render_keys_lean(&v),
            format!("ledger absent\n{next}\n")
        );
        // Entries, none matching.
        let at = now_ns() - 20 * DAY;
        f.entry("k4000001", "other-key", "deploy in the claim", at);
        let v =
            memory_store::recall_keys_with(f.root(), &strings(&["deploy zzz"]), &f.ledger, false)
                .unwrap();
        assert_eq!(
            memory_store::render_keys_lean(&v),
            format!("no hits\n{next}\n")
        );
        // A partial query names itself after the hit lines; a full one adds nothing.
        f.entry("k4000002", "deploy-plan", "x", at);
        let queries = strings(&["deploy zzz", "plan"]);
        let v = memory_store::recall_keys_with(f.root(), &queries, &f.ledger, false).unwrap();
        assert_eq!(v["queries"][0]["match_kind"], "partial");
        assert_eq!(v["queries"][1]["match_kind"], "all");
        let utc = hit(&v, "k4000002").unwrap()["recorded_at_utc"]
            .as_str()
            .unwrap()
            .to_string();
        let lean = memory_store::render_keys_lean(&v);
        assert_eq!(
            lean,
            format!(
                "deploy-plan\tk4000002\t{utc}\t-\t-\n\
                 partial \"deploy zzz\": no key held every term; these rows are candidates\n\
                 cort memory recall \"deploy zzz\" --also \"plan\"\n"
            )
        );
        assert_five_fields(&lean);
        // Two queries: every hit says which found it.
        assert_eq!(
            hit(&v, "k4000002").unwrap()["matched_queries"],
            json!([0, 1])
        );
    }

    /// `next` is a shell command: each query double-quoted with `\`, `"`, `$` and backtick
    /// escaped, `--all` kept.
    #[test]
    fn keys_next_names_the_queries_as_a_shell_command() {
        let f = fixture();
        let odd = "say \"hi\" $HOME `id` a\\b";
        let quoted = "\"say \\\"hi\\\" \\$HOME \\`id\\` a\\\\b\"";
        let v =
            memory_store::recall_keys_with(f.root(), &strings(&[odd]), &f.ledger, false).unwrap();
        assert_eq!(
            v["next"],
            format!("cort memory recall {quoted} --also {OTHER}").as_str()
        );
        let v = memory_store::recall_keys_with(
            f.root(),
            &strings(&["deploy", odd, "部署"]),
            &f.ledger,
            true,
        )
        .unwrap();
        let next = format!("cort memory recall \"deploy\" --also {quoted} --also \"部署\" --all");
        assert_eq!(v["next"], next.as_str());
        assert!(memory_store::render_keys_lean(&v).ends_with(&format!("{next}\n")));
    }

    #[test]
    fn keys_lifecycle_unreadable_goes_to_its_own_line() {
        let f = fixture();
        let now = now_ns();
        f.entry("k5000001", "deploy-live", "kept", now - 20 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life_body("l2", "{not json");
        let v = memory_store::recall_keys_with(f.root(), &strings(&["deploy"]), &f.ledger, false)
            .unwrap();
        assert_eq!(v["lifecycle_unreadable"], 1);
        // The hint without its own opening clause, which would repeat the count's.
        let hint = v["lifecycle_hint"].as_str().unwrap();
        let rest = hint
            .strip_prefix("some lifecycle rows could not be read; ")
            .unwrap();
        assert_eq!(
            memory_store::lifecycle_unreadable_line(&v),
            Some(format!(
                "cort memory: 1 lifecycle rows could not be read; {rest}"
            ))
        );
        assert!(rest.starts_with("cort memory review --all"), "{rest}");
        // Stdout's lean text does not carry it.
        let lean = memory_store::render_keys_lean(&v);
        assert!(!lean.contains("lifecycle"), "{lean}");
    }

    #[test]
    fn keys_with_also_and_all() {
        let f = fixture();
        let now = now_ns();
        f.entry("k3000001", "deploy-live", "kept", now - 20 * DAY);
        f.entry("k3000002", "deploy-old", "shelved", now - 20 * DAY);
        f.entry("k3000003", "部署-key", "a key in chinese", now - 20 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life("l2", "archive", Some("k3000002"), now - 10 * DAY);
        let queries = strings(&["deploy", "部署"]);
        let all = memory_store::recall_keys_with(f.root(), &queries, &f.ledger, true).unwrap();
        assert_eq!(
            report(&f, &request("deploy", &queries[1..], true, true)),
            all
        );
        assert_eq!(
            all["next"],
            "cort memory recall \"deploy\" --also \"部署\" --all"
        );
        assert_eq!(all["queries"].as_array().unwrap().len(), 2);
        assert_eq!(ids(&all), ["k3000001", "k3000002", "k3000003"]);
        assert_eq!(hit(&all, "k3000002").unwrap()["state"], "archived");
        for id in ["k3000001", "k3000003"] {
            let state = hit(&all, id).unwrap()["state"].as_str().unwrap();
            assert!(!state.is_empty() && state != "archived", "{id} {state}");
        }
        for (id, matched) in [("k3000001", [0]), ("k3000002", [0]), ("k3000003", [1])] {
            assert_eq!(hit(&all, id).unwrap()["matched_queries"], json!(matched));
        }
        let lean = memory_store::render_keys_lean(&all);
        assert!(
            lean.contains("deploy-old\tk3000002\t") && lean.contains("\tarchived\t-\n"),
            "{lean}"
        );
        assert_five_fields(&lean);
        let shown = memory_store::recall_keys_with(f.root(), &queries, &f.ledger, false).unwrap();
        assert_eq!(ids(&shown), ["k3000001", "k3000003"]);
        assert_eq!(
            shown["next"],
            "cort memory recall \"deploy\" --also \"部署\""
        );
    }

    /// The conflict set is the database's: a partner the archived condition kept out of the
    /// returned rows still makes the visible hit a conflict (fails if `recall_each` counts over
    /// the returned rows alone).
    #[test]
    fn keys_conflict_counts_an_archived_partner() {
        let f = fixture();
        let now = now_ns();
        f.entry("k6000001", "deploy-dup", "kept", now - 20 * DAY);
        f.entry("k6000002", "deploy-dup", "shelved", now - 20 * DAY);
        f.life("l1", "on", None, now - 30 * DAY);
        f.life("l2", "archive", Some("k6000002"), now - 10 * DAY);
        let v = memory_store::recall_keys_with(f.root(), &strings(&["deploy"]), &f.ledger, false)
            .unwrap();
        assert_eq!(ids(&v), ["k6000001"]);
        assert_eq!(hit(&v, "k6000001").unwrap()["conflict"], true);
    }

    /// Rust lowercases U+212A KELVIN SIGN to `k` and U+0130 to `i` + U+0307; SQLite's `lower()`
    /// leaves both, so the SQL haystack folds them itself or loses rows the whole read finds.
    #[test]
    fn kelvin_sign_and_dotted_i_match_as_today() {
        let f = fixture();
        let at = now_ns() - 20 * DAY;
        f.entry("t1000001", "temperature", "measured in \u{212a}elvin", at);
        f.entry("t1000002", "city", "\u{130}stanbul office", at);
        f.entry("t1000003", "spooler", "the PR\u{130}NT queue", at);
        f.entry("t1000004", "\u{212a}elvin-scale", "a key with the sign", at);
        f.entry("t1000005", "plain", "nothing special", at);
        for query in [
            "kelvin",
            "elvin",
            "stanbul",
            "i\u{307}stanbul",
            "pri",
            "kelvin scale",
            "pri queue",
            "kelvin zzz",
        ] {
            for include_hidden in [false, true] {
                let answer = assert_same(&f, query, include_hidden);
                assert!(!ids(&answer).is_empty(), "{query:?}");
            }
        }
        // Layer 1 folds the key the same way.
        let keys =
            memory_store::recall_keys_with(f.root(), &["kelvin".to_string()], &f.ledger, false)
                .unwrap();
        assert_eq!(ids(&keys), ["t1000004"]);
    }

    /// Answers the batch step `step` (BEGIN is step 0, statement i is step i + 1) with a Hrana
    /// error of `code` and `message`, as a server refusing that statement would.
    struct StepFailing {
        inner: Arc<SqliteTransport>,
        step: usize,
        code: &'static str,
        message: String,
    }

    impl Transport for StepFailing {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, cort::remote::RemoteError> {
            let request: Value = serde_json::from_slice(body).unwrap();
            let bytes = self.inner.post(url, body, headers)?;
            if request["requests"][0]["type"] != "batch" {
                return Ok(bytes);
            }
            let mut value: Value = serde_json::from_slice(&bytes).unwrap();
            let result = &mut value["results"][0]["response"]["result"];
            result["step_results"][self.step] = Value::Null;
            result["step_errors"][self.step] = json!({"code": self.code, "message": self.message});
            Ok(serde_json::to_vec(&value).unwrap())
        }
    }

    /// Records the SQL of every statement the ledger sends.
    struct Logging {
        inner: Arc<SqliteTransport>,
        sql: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Transport for Logging {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, cort::remote::RemoteError> {
            let text = String::from_utf8_lossy(body).to_string();
            self.sql.lock().unwrap().push(text);
            self.inner.post(url, body, headers)
        }
    }

    impl Fixture {
        fn failing_at(&self, step: usize, code: &'static str, message: &str) -> Ledger {
            Ledger::with_transport(
                "https://example.test",
                "test",
                Box::new(StepFailing {
                    inner: self.inner.clone(),
                    step,
                    code,
                    message: message.to_string(),
                }),
            )
            .unwrap()
        }
        fn logging(&self) -> (Ledger, Arc<std::sync::Mutex<Vec<String>>>) {
            let sql = Arc::new(std::sync::Mutex::new(Vec::new()));
            let ledger = Ledger::with_transport(
                "https://example.test",
                "test",
                Box::new(Logging {
                    inner: self.inner.clone(),
                    sql: sql.clone(),
                }),
            )
            .unwrap();
            (ledger, sql)
        }
    }

    /// The trip's statements: five CREATEs (steps 1-5), count 6, conflicts 7, screen 8,
    /// lifecycle 9, excluded 10 (unless `--all`), then per query stage 1 and stage 2.
    #[test]
    fn a_refused_statement_is_named_with_the_server_message() {
        let f = fixture();
        f.entry("s3000001", "deploy-plan", "nuc runs it", now_ns() - DAY);
        let msg = "no such function: row_number";
        for (step, include_hidden, query, kind, q) in [
            (9, false, "deploy", "lifecycle", None),
            (10, false, "deploy", "excluded", None),
            (11, false, "deploy", "stage1", Some(0)),
            (12, false, "deploy plan", "stage2", Some(0)),
            (10, true, "deploy", "stage1", Some(0)),
            (8, false, "deploy", "screen", None),
        ] {
            let ledger = f.failing_at(step, "SQLITE_ERROR", msg);
            let err = memory_store::recall_all_with(f.root(), query, &ledger, include_hidden)
                .unwrap_err();
            assert_eq!(
                err,
                LedgerError::RemoteStatement(memory_store::StatementFailure {
                    trip: 1,
                    statement: Some(step - 1),
                    statement_kind: kind,
                    query: q,
                    message: msg.to_string(),
                }),
                "step {step}"
            );
        }
        // BEGIN has no statement of its own.
        let err = memory_store::recall_all_with(
            f.root(),
            "deploy",
            &f.failing_at(0, "SQLITE_ERROR", msg),
            false,
        )
        .unwrap_err();
        assert!(
            matches!(&err, LedgerError::RemoteStatement(s)
                if s.statement.is_none() && s.statement_kind == "begin"),
            "{err:?}"
        );
        // The message reaches one JSON line: control characters flattened, at most 300 bytes,
        // cut on a character boundary.
        let long = format!("near \"x\":\nsyntax\terror {}", "é".repeat(200));
        let err = memory_store::recall_all_with(
            f.root(),
            "deploy",
            &f.failing_at(11, "SQLITE_ERROR", &long),
            false,
        )
        .unwrap_err();
        let LedgerError::RemoteStatement(s) = &err else {
            panic!("{err:?}")
        };
        assert!(s.message.starts_with("near \"x\": syntax error é"), "{s:?}");
        assert_eq!(s.message.len(), 299, "{s:?}");
        // A refusal that is not a SQL error keeps today's shape.
        let err = memory_store::recall_all_with(
            f.root(),
            "deploy",
            &f.failing_at(11, "BLOCKED", "blocked"),
            false,
        )
        .unwrap_err();
        assert_eq!(err, LedgerError::Remote(cort::remote::ErrorKind::Blocked));
    }

    fn words(n: usize) -> String {
        (0..n)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A query past the term cap answers through the whole read, as before the search existed:
    /// SQLite's expression depth would refuse its chain of `instr` terms.
    #[test]
    fn a_query_over_the_term_cap_reads_the_whole_ledger() {
        let f = rich();
        let (ledger, sql) = f.logging();
        let long = format!("deploy plan {}", words(99));
        assert_eq!(long.split_whitespace().count(), 101);
        let new = memory_store::recall_all_with(f.root(), &long, &ledger, false).unwrap();
        let searched: Vec<String> = sql.lock().unwrap().drain(..).collect();
        let old = memory_store::recall_whole_with(f.root(), &long, &ledger, false).unwrap();
        let whole: Vec<String> = sql.lock().unwrap().drain(..).collect();
        assert_eq!(
            serde_json::to_value(new).unwrap(),
            serde_json::to_value(old).unwrap()
        );
        assert_eq!(searched, whole, "the same requests as the whole read");
        assert!(searched.iter().all(|s| !s.contains("instr(")));
        // The eval's replay selects through the same read recall ships.
        let now = now_ns();
        let replay =
            memory_store::select_search_with(&ledger, &f.project(), &long, false, now).unwrap();
        let replayed: Vec<String> = sql.lock().unwrap().drain(..).collect();
        let base =
            memory_store::select_whole_with(&ledger, &f.project(), &long, false, now).unwrap();
        sql.lock().unwrap().clear();
        assert_eq!(replay, base);
        assert_eq!(replayed, whole);
        // At the cap, the search.
        let at_cap = format!("deploy plan {}", words(98));
        assert_eq!(at_cap.split_whitespace().count(), 100);
        assert_same(&f, &at_cap, false);
        memory_store::recall_all_with(f.root(), &at_cap, &ledger, false).unwrap();
        assert!(sql.lock().unwrap().iter().any(|s| s.contains("instr(")));
        assert_eq!(memory_search_cap(), 100);
    }

    fn memory_search_cap() -> usize {
        cort::memory_search::MAX_QUERY_TERMS
    }

    #[test]
    fn also_and_keys_over_the_term_cap_are_refused_before_any_read() {
        let f = rich();
        let long = words(101);
        let also = [long.clone()];
        for (query, also, keys, refused) in [
            ("deploy", &also[..], false, 1),
            (long.as_str(), &[][..], true, 0),
            ("deploy", &also[..], true, 1),
        ] {
            let before = f.posts.load(Ordering::SeqCst);
            let err = memory_store::recall_report_with(
                f.root(),
                &memory_store::RecallRequest {
                    query,
                    also,
                    keys,
                    include_hidden: false,
                },
                &f.ledger,
            )
            .unwrap_err();
            assert_eq!(f.posts.load(Ordering::SeqCst), before, "no request");
            match err {
                LedgerError::Refused {
                    code,
                    message,
                    detail,
                } => {
                    assert_eq!(code, "memory_recall_too_many_terms");
                    assert_eq!(
                        message,
                        format!(
                            "query {refused} has 101 terms; --also and --keys take at most 100 per query"
                        )
                    );
                    assert_eq!(detail, json!({"query": refused, "terms": 101, "max": 100}));
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
