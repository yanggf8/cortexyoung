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
// The integrity screen, the whole-ledger count statements and subset decoding (Task 4a).
// ---------------------------------------------------------------------------------------------

use cort::memory_search::{conflict_keys, integrity_screen, ledger_count, lifecycle_rows};
use cort::memory_store::{screen_check, Ledger, LedgerError};
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
    fn screen_rows(&self) -> Vec<(String, bool)> {
        let stmt = integrity_screen(PROJECT);
        self.db
            .execute(&stmt.sql, stmt.args)
            .unwrap()
            .rows
            .iter()
            .map(|row| match (&row[0], &row[1]) {
                (Cell::Text(b), Cell::Int(s)) => (b.clone(), *s == 1),
                other => panic!("{other:?}"),
            })
            .collect()
    }
    fn screened(&self) -> Result<(), String> {
        screen_check(self.screen_rows())
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
fn subset_decoding_accepts_an_absent_predecessor() {
    let env = Env::new();
    env.entry("b", "k", "second", json!({"supersedes":"a"}));
    // Only `b` is given, not suspect: the subset rule accepts the absent `a`.
    let body = env.screen_rows_for_test_b();
    assert_eq!(screen_check(vec![(body, false)]), Ok(()));
    // The whole read of the same ledger keeps today's message, and so does a suspect `b`.
    let whole = env.whole().unwrap_err();
    assert_eq!(
        whole,
        "memory b supersedes missing id a; re-add the alias with cort memory alias add"
    );
    assert_eq!(env.screened().unwrap_err(), whole);
}

impl Env {
    fn screen_rows_for_test_b(&self) -> String {
        let r = self
            .db
            .execute("SELECT body FROM project_memory WHERE id = 'b'", vec![])
            .unwrap();
        match &r.rows[0][0] {
            Cell::Text(b) => b.clone(),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn subset_decoding_still_checks_a_present_predecessor() {
    let env = Env::new();
    env.entry("a", "k1", "first", json!({}));
    env.entry("b", "k2", "second", json!({"supersedes":"a"}));
    let rows: Vec<(String, bool)> = env.screen_rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        screen_check(rows.into_iter().map(|(b, _)| (b, false)).collect()).unwrap_err(),
        "memory b has invalid supersession"
    );
}

#[test]
fn screen_check_messages_equal_whole_read() {
    type Seed = Box<dyn Fn(&Env)>;
    let cases: Vec<(&str, Seed)> = vec![
        ("invalid json", Box::new(|e| e.insert("x", "{not json"))),
        (
            "missing key",
            Box::new(|e| {
                e.insert(
                    "x",
                    r#"{"id":"x","claim":"c","source":"s","source_line":1,"source_sha256":"","recorded_at":"1"}"#,
                )
            }),
        ),
        (
            "blank key",
            Box::new(|e| e.entry("x", "  ", "c", json!({}))),
        ),
        (
            "supersedes missing id",
            Box::new(|e| e.entry("x", "k", "c", json!({"supersedes":"gone"}))),
        ),
        (
            "supersedes own id",
            Box::new(|e| e.entry("x", "k", "c", json!({"supersedes":"x"}))),
        ),
        (
            "supersedes a different key",
            Box::new(|e| {
                e.entry("a", "k1", "c", json!({}));
                e.entry("b", "k2", "c", json!({"supersedes":"a"}));
            }),
        ),
        (
            "one id under two aliases",
            Box::new(|e| {
                e.alias(PROJECT, "p0");
                e.entry("x", "k", "first claim", json!({}));
                e.entry_in("p0", "x", "k", "second claim", json!({}));
            }),
        ),
    ];
    for (name, seed) in cases {
        let env = Env::new();
        seed(&env);
        let whole = env.whole().expect_err(name);
        let screened = env.screened().expect_err(name);
        assert_eq!(screened, whole, "{name}");
    }
    // The alias hint really is part of the compared text.
    let env = Env::new();
    env.entry("x", "k", "c", json!({"supersedes":"gone"}));
    assert!(env
        .screened()
        .unwrap_err()
        .ends_with("; re-add the alias with cort memory alias add"));
}

#[test]
fn two_errors_report_the_earlier_phase() {
    let env = Env::new();
    env.insert("a-bad", "{not json");
    env.entry("b", "k1", "c", json!({}));
    env.entry("c", "k2", "c", json!({"supersedes":"b"}));
    let whole = env.whole().unwrap_err();
    assert!(whole.starts_with("invalid remote memory row"), "{whole}");
    assert_eq!(env.screened().unwrap_err(), whole);
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
    let rows = env.screen_rows();
    assert!(rows.iter().all(|(_, suspect)| !suspect), "{rows:?}");
    assert_eq!(env.screened(), Ok(()));
    assert_eq!(env.whole(), Ok(()));
}

#[test]
fn screen_suspect_is_not_a_verdict() {
    let env = Env::new();
    env.entry("   ", "k", "claim", json!({}));
    let rows = env.screen_rows();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].1, "an id of spaces is suspect");
    assert_eq!(env.screened(), Ok(()));
    assert_eq!(env.whole(), Ok(()));
}

#[test]
fn screen_returns_a_named_predecessor_as_not_suspect() {
    let env = Env::new();
    env.entry("a", "k1", "c", json!({}));
    env.entry("b", "k2", "c", json!({"supersedes":"a"}));
    env.entry("z", "other", "unrelated", json!({}));
    let rows = env.screen_rows();
    let flags: Vec<bool> = rows.iter().map(|(_, s)| *s).collect();
    // a (named, not suspect), b (suspect); z is not returned.
    assert_eq!(flags, vec![false, true]);
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
    assert!(rows.iter().any(|(b, s)| b == "{not json" && *s));
    assert!(rows
        .iter()
        .filter(|(b, _)| b != "{not json")
        .all(|(_, s)| !s));
}
