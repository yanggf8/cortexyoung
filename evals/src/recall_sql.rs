//! Does the product's cloud search give the whole-ledger recall's answer on real queries?
//!
//! Replays every real `cort memory recall "<query>"` found in Claude Code transcripts (the walk
//! and the query extraction are `recall_hits`'s) twice through the product itself: once with
//! `select_whole_with`, today's selection over every row of the ledger (the baseline), and once
//! with `select_search_with`, the search `cort memory recall` ships, which returns only the rows
//! its query can match (a query past the term cap goes through the whole read, as recall answers
//! it). Only a call's positional query is replayed, never its `--also` values; a `--keys` call is
//! skipped and counted. The eval owns no SQL: it grades the statements that ship, not a copy of them.
//! It compares the selection (ids in order and the partial flag), not the rendered recall output.
//! Hidden entries are left out on both sides, as a plain `recall` leaves them out. Both reads
//! create any of the five memory tables that are missing (`CREATE TABLE IF NOT EXISTS`) and write
//! nothing else.

use crate::recall_hits::{queries, Found};
use cort::memory_store::{select_search_with, select_whole_with, Ledger};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Instant;

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

#[derive(Default)]
struct Group {
    queries: u64,
    equal: u64,
    baseline_hits: u64,
    partial: u64,
    rows: Vec<u64>,
    trips: Vec<u64>,
    ms: Vec<u64>,
    baseline_ms: Vec<u64>,
    server_rows_read: Vec<Option<u64>>,
    baseline_server_rows_read: Vec<Option<u64>>,
}

fn sum_max(v: &[u64]) -> Value {
    json!({"sum": v.iter().sum::<u64>(), "max": v.iter().max().copied().unwrap_or(0)})
}

/// `sum_max`, or `null` when any value is unknown: a sum missing a term would understate.
fn known_sum_max(v: &[Option<u64>]) -> Value {
    v.iter()
        .copied()
        .collect::<Option<Vec<u64>>>()
        .map_or(Value::Null, |known| sum_max(&known))
}

fn p50_max(v: &[u64]) -> Value {
    let mut s = v.to_vec();
    s.sort_unstable();
    json!({"p50": s.get(s.len() / 2).copied().unwrap_or(0), "max": s.last().copied().unwrap_or(0)})
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Replay `queries` against memory project `project` (its alias component is the product's to
/// resolve) and build the report. Each occurrence counts; both sides fold the lifecycle at
/// `now_ns`, so a state boundary cannot fall between them. A query either side fails on stops the
/// report, naming its place, its side and how many were equal before it: an integrity error fails
/// every query alike, and a figure computed past an outage would not describe the ledger. No
/// queries, or a baseline that is empty for every query (a wrong project id reads an empty
/// ledger), is an error rather than a vacuous pass. `skipped_keys` is reported as given: the `--keys`
/// calls the transcript walk left out. A statement the server refused in the replay is named
/// with its trip, step, kind, query and the server's message.
pub fn report(
    ledger: &Ledger,
    project: &str,
    queries: &[String],
    skipped_keys: u64,
    now_ns: i128,
) -> Result<Value, String> {
    if queries.is_empty() {
        return Err("no recall queries to replay".into());
    }
    let n = queries.len();
    let mut groups: BTreeMap<&'static str, Group> = BTreeMap::new();
    let mut mismatches = Vec::new();
    let mut equal = 0u64;
    for (i, query) in queries.iter().enumerate() {
        let failed = |side: &str, e: cort::memory_store::LedgerError| {
            let why = match e {
                cort::memory_store::LedgerError::RemoteStatement(f) => {
                    format!("remote sql: {}", f.to_json())
                }
                other => format!("{other:?}"),
            };
            format!(
                "query {} of {n} {query:?}: {side} failed after {equal} equal: {why}",
                i + 1
            )
        };
        let group = group_of(query);
        let started = Instant::now();
        let base = select_whole_with(ledger, project, query, false, now_ns)
            .map_err(|e| failed("baseline", e))?;
        let baseline_ms = elapsed_ms(started);
        let started = Instant::now();
        let got = select_search_with(ledger, project, query, false, now_ns)
            .map_err(|e| failed("replay", e))?;
        let ms = elapsed_ms(started);
        let g = groups.entry(group).or_default();
        g.queries += 1;
        g.baseline_hits += u64::from(!base.ids.is_empty());
        g.partial += u64::from(base.partial);
        g.rows.push(got.rows as u64);
        g.trips.push(got.trips as u64);
        g.ms.push(ms);
        g.baseline_ms.push(baseline_ms);
        g.server_rows_read.push(got.server_rows_read);
        g.baseline_server_rows_read.push(base.server_rows_read);
        // The partial flag is part of the answer: it decides `match: "partial"` in the output.
        if (&got.ids, got.partial) == (&base.ids, base.partial) {
            g.equal += 1;
            equal += 1;
        } else {
            mismatches.push(json!({"query": query, "group": group,
                "baseline": base.ids, "baseline_partial": base.partial,
                "got": got.ids, "partial": got.partial}));
        }
    }
    if groups.values().all(|g| g.baseline_hits == 0) {
        return Err(
            "every baseline answer is empty: check --memory-project and the aliases".into(),
        );
    }
    let mut out = Map::new();
    for (name, g) in groups {
        out.insert(
            name.to_string(),
            json!({
                "queries": g.queries,
                "equal": g.equal,
                "baseline_hits": g.baseline_hits,
                "partial": g.partial,
                "rows": sum_max(&g.rows),
                "trips": sum_max(&g.trips),
                "ms": p50_max(&g.ms),
                "baseline_ms": p50_max(&g.baseline_ms),
                "server_rows_read": known_sum_max(&g.server_rows_read),
                "baseline_server_rows_read": known_sum_max(&g.baseline_server_rows_read),
            }),
        );
    }
    Ok(json!({
        "report": "recall-sql",
        "memory_project": project,
        "now_ns": i64::try_from(now_ns).map_err(|e| format!("now_ns {now_ns}: {e}"))?,
        "queries": n,
        "skipped_keys": skipped_keys,
        "equal": equal,
        "groups": Value::Object(out),
        "mismatches": mismatches,
    }))
}

/// The queries `recall-hits` would replay, in transcript order, one per call, and how many `--keys`
/// calls were left out. No query is an error: a report over nothing would read as a pass.
pub fn transcript_queries(
    dir: &std::path::Path,
    project: &str,
    days: Option<u64>,
    now_ms: i64,
) -> Result<Found, String> {
    let found = queries(dir, Some(project), days, now_ms)?;
    if found.queries.is_empty() {
        let span = days.map_or("all days".to_string(), |d| format!("{d} days"));
        return Err(format!(
            "no recall queries found in {}/{project} over {span}",
            dir.display()
        ));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cort::remote::{Cell, RemoteDb, RemoteError, SqliteTransport, Transport};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const NS: i128 = 1_000_000_000;
    const DAY: i128 = 86_400 * NS;
    const PROJECT: &str = "p1";
    const ALIAS: &str = "old-p1";
    const URL: &str = "https://unit.test";

    /// One in-memory database behind two clients: the product's `Ledger` and a seeding `RemoteDb`.
    struct Shared(Arc<SqliteTransport>);

    impl Transport for Shared {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, RemoteError> {
            self.0.post(url, body, headers)
        }
    }

    /// What `Tamper` does once its `after`-th request has been answered.
    enum Act {
        /// Run this SQL on the server: the ledger changes between two requests.
        Insert(String),
        /// Answer every later request with a body that is not Hrana: the read fails.
        Break,
        /// Fail this step of the next batch with this SQLite code: the server refuses a statement.
        FailStep(usize, &'static str),
    }

    /// Passes requests through, then acts once `after` of them have been answered.
    struct Tamper {
        inner: Arc<SqliteTransport>,
        posts: AtomicUsize,
        after: usize,
        act: Act,
    }

    impl Transport for Tamper {
        fn post(
            &self,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> Result<Vec<u8>, RemoteError> {
            let seen = self.posts.fetch_add(1, Ordering::SeqCst);
            if seen >= self.after {
                if let Act::Break = self.act {
                    return Ok(b"{}".to_vec());
                }
            }
            if seen == self.after {
                if let Act::FailStep(step, code) = self.act {
                    self.inner.fail_next_step(step, code);
                }
            }
            let answer = self.inner.post(url, body, headers)?;
            if seen + 1 == self.after {
                if let Act::Insert(sql) = &self.act {
                    let request = json!({"requests":[
                        {"type":"execute","stmt":{"sql":sql,"args":[]}},{"type":"close"}]});
                    self.inner
                        .post(url, request.to_string().as_bytes(), headers)
                        .unwrap();
                }
            }
            Ok(answer)
        }
    }

    struct Fixture {
        inner: Arc<SqliteTransport>,
        ledger: Ledger,
        db: RemoteDb,
        now: i128,
    }

    impl Fixture {
        fn new() -> Fixture {
            let inner = Arc::new(SqliteTransport::new().unwrap());
            let ledger =
                Ledger::with_transport(URL, "test", Box::new(Shared(inner.clone()))).unwrap();
            let db = RemoteDb::new(URL, "test", Box::new(Shared(inner.clone()))).unwrap();
            for sql in [
                "CREATE TABLE project_memory (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))",
                "CREATE TABLE project_memory_lifecycle (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))",
                "CREATE TABLE project_aliases (old_project TEXT NOT NULL, new_project TEXT NOT NULL, created_at TEXT NOT NULL, machine TEXT NOT NULL, PRIMARY KEY (old_project, new_project))",
            ] {
                db.execute(sql, vec![]).unwrap();
            }
            let now = i128::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            )
            .unwrap();
            Fixture {
                inner,
                ledger,
                db,
                now,
            }
        }
        /// A ledger over the same database whose transport acts after `after` requests.
        fn tampered(&self, after: usize, act: Act) -> Ledger {
            let tamper = Tamper {
                inner: self.inner.clone(),
                posts: AtomicUsize::new(0),
                after,
                act,
            };
            Ledger::with_transport(URL, "test", Box::new(tamper)).unwrap()
        }
        fn body(&self, id: &str, key: &str, claim: &str, extra: Value) -> Value {
            let mut body = json!({"id":id,"key":key,"claim":claim,
                "source":format!(".cort-unsourced/{id}"),"source_line":1,"source_sha256":"",
                "unsourced":true,"recorded_at":(self.now - 40 * DAY).to_string()});
            for (k, v) in extra.as_object().unwrap() {
                body[k] = v.clone();
            }
            body
        }
        fn insert(&self, table: &str, project: &str, id: &str, body: Value) {
            self.db
                .execute(
                    &format!("INSERT INTO {table} VALUES (?, ?, ?)"),
                    vec![
                        Cell::Text(project.into()),
                        Cell::Text(id.into()),
                        Cell::Text(body.to_string()),
                    ],
                )
                .unwrap();
        }
        fn entry_in(&self, project: &str, id: &str, key: &str, claim: &str, extra: Value) {
            let body = self.body(id, key, claim, extra);
            self.insert("project_memory", project, id, body);
        }
        fn entry(&self, id: &str, key: &str, claim: &str) {
            self.entry_in(PROJECT, id, key, claim, json!({}));
        }
        fn life(&self, id: &str, kind: &str, entry: Option<&str>, days_ago: i128) {
            let body = json!({"id":id,"kind":kind,"entry":entry,
                "recorded_at":(self.now - days_ago * DAY).to_string()});
            self.insert("project_memory_lifecycle", PROJECT, id, body);
        }
        fn baseline(&self, query: &str) -> (Vec<String>, bool) {
            self.baseline_with(query, false)
        }
        fn baseline_with(&self, query: &str, include_hidden: bool) -> (Vec<String>, bool) {
            let s =
                select_whole_with(&self.ledger, PROJECT, query, include_hidden, self.now).unwrap();
            (s.ids, s.partial)
        }
        fn report(&self, queries: &[&str]) -> Result<Value, String> {
            let queries: Vec<String> = queries.iter().map(|q| q.to_string()).collect();
            report(&self.ledger, PROJECT, &queries, 0, self.now)
        }
    }

    /// Rows in `fixture()`: 15 memory rows (12 under the project, 3 under the alias) and 3
    /// lifecycle rows.
    const MEMORY_ROWS: u64 = 15;
    const LIFECYCLE_ROWS: u64 = 3;

    /// An alias pair with a supersession across it, a resolution, a conflict pair, and lifecycle
    /// on with a dropped and an archived entry, each the only full match of a query.
    fn fixture() -> Fixture {
        let f = Fixture::new();
        f.db.execute(
            "INSERT INTO project_aliases VALUES (?, ?, '1', 'm')",
            vec![Cell::Text(ALIAS.into()), Cell::Text(PROJECT.into())],
        )
        .unwrap();
        f.entry("a1000001", "deploy-notes", "nuc runs the deploy notes");
        f.entry_in(ALIAS, "o1000001", "部署", "部署 在 nuc 上執行", json!({}));
        f.entry_in(
            ALIAS,
            "s1000001",
            "rollout",
            "rollout waits on nuc",
            json!({}),
        );
        f.entry_in(
            PROJECT,
            "s1000002",
            "rollout",
            "rollout moved to thinkpad",
            json!({"supersedes":"s1000001"}),
        );
        f.entry("r1000001", "recall-timeout", "timeout is 3800 ms");
        f.entry("r1000002", "recall-timeout", "timeout is 4300 ms");
        f.entry_in(
            PROJECT,
            "r1000003",
            "recall-timeout",
            "timeout stays 3800 ms after measuring",
            json!({"resolves":["r1000001","r1000002"]}),
        );
        f.entry("c1000001", "split-key", "alpha split current side");
        f.entry_in(
            ALIAS,
            "c1000002",
            "split-key",
            "beta split alias side",
            json!({}),
        );
        f.entry("m1000001", "cfg:timeout", "the cfg value");
        f.entry("m1000002", "50%_done", "literal percent and underscore");
        f.entry("m1000003", "café", "accented key in the ledger");
        // Recorded after the switch-on, so short-term: `drop` applies only to those.
        let short = json!({"recorded_at": (f.now - 20 * DAY).to_string()});
        f.entry_in(
            PROJECT,
            "d1000001",
            "gone",
            "deploy plan dropped",
            short.clone(),
        );
        f.entry_in(PROJECT, "d1000002", "half", "plan alpha", short);
        f.entry("x1000001", "shelf", "shelved gamma archived");
        f.life("l1", "on", None, 30);
        f.life("l2", "drop", Some("d1000001"), 10);
        f.life("l3", "archive", Some("x1000001"), 10);
        f
    }

    const QUERIES: [&str; 14] = [
        "deploy",
        "deploy plan",
        "部署",
        "部署 nuc",
        "café 部署",
        "timeout 3800",
        "cfg:timeout",
        "50%_done",
        "alpha beta",
        "rollout",
        "shelved gamma",
        "zzz qqq",
        "split",
        "",
    ];

    #[test]
    fn replay_equals_baseline_on_fixture() {
        let f = fixture();
        // The fixture is not vacuous: the alias row is read, the hidden full matches leave their
        // queries partial, and the superseded and resolved rows stay out.
        assert_eq!(f.baseline("部署").0, ["o1000001"]);
        let (mut ids, partial) = f.baseline("deploy plan");
        ids.sort();
        assert_eq!(
            (ids, partial),
            (vec!["a1000001".into(), "d1000002".into()], true)
        );
        assert_eq!(
            f.baseline_with("deploy plan", true),
            (vec!["d1000001".into()], false)
        );
        assert!(f.baseline("shelved gamma").0.is_empty());
        assert_eq!(f.baseline_with("shelved gamma", true).0, ["x1000001"]);
        assert_eq!(f.baseline("rollout").0, ["s1000002"]);
        assert_eq!(f.baseline("timeout 3800").0, ["r1000003"]);
        assert!(f.baseline("café 部署").1);

        let r = f.report(&QUERIES).unwrap();
        assert_eq!(r["report"], "recall-sql");
        assert_eq!(r["memory_project"], PROJECT);
        assert_eq!(r["queries"], QUERIES.len());
        assert_eq!(r["equal"], QUERIES.len());
        assert_eq!(r["now_ns"], json!(i64::try_from(f.now).unwrap()));
        assert_eq!(r["mismatches"], json!([]), "{r:#}");
        let groups = r["groups"].as_object().unwrap();
        assert_eq!(
            groups.keys().map(String::as_str).collect::<Vec<_>>(),
            ["cjk", "english", "identifier"]
        );
        let (mut total, mut hits, mut partials) = (0, 0, 0);
        for (name, g) in groups {
            assert_eq!(g["equal"], g["queries"], "{name}: {g}");
            for field in [
                "rows",
                "trips",
                "server_rows_read",
                "baseline_server_rows_read",
            ] {
                assert!(
                    g[field]["sum"].as_u64().unwrap() >= g[field]["max"].as_u64().unwrap(),
                    "{name} {field}: {g}"
                );
            }
            for field in ["ms", "baseline_ms"] {
                assert!(g[field]["max"].as_u64().unwrap() >= g[field]["p50"].as_u64().unwrap());
            }
            // Every whole read returns every memory and lifecycle row, and the in-memory
            // server's counter is returned rows.
            assert_eq!(
                g["baseline_server_rows_read"]["max"],
                MEMORY_ROWS + LIFECYCLE_ROWS
            );
            total += g["queries"].as_u64().unwrap();
            hits += g["baseline_hits"].as_u64().unwrap();
            partials += g["partial"].as_u64().unwrap();
        }
        assert_eq!(total, QUERIES.len() as u64);
        // The group counts agree with the baseline asked directly.
        let direct: Vec<(Vec<String>, bool)> = QUERIES.iter().map(|q| f.baseline(q)).collect();
        assert_eq!(
            hits,
            direct.iter().filter(|(ids, _)| !ids.is_empty()).count() as u64
        );
        assert_eq!(partials, direct.iter().filter(|(_, p)| *p).count() as u64);
        assert!(hits >= 10 && partials >= 3, "{hits} {partials}");
        // "deploy plan" fully matches only a dropped entry, which SQL returns and the client hides:
        // the product resends without the stage-2 gate.
        assert_eq!(groups["english"]["trips"]["max"], 2);

        // One query alone, every figure exact: one stage-1 row (no stage 2 for one term), one
        // trip, and the whole read's 15 + 3 rows.
        let one = f.report(&["cfg:timeout"]).unwrap();
        let g = &one["groups"]["identifier"];
        assert_eq!(g["rows"], json!({"sum": 1, "max": 1}));
        assert_eq!(g["trips"], json!({"sum": 1, "max": 1}));
        assert_eq!(
            (g["baseline_hits"].clone(), g["partial"].clone()),
            (json!(1), json!(0))
        );
        let whole = MEMORY_ROWS + LIFECYCLE_ROWS;
        assert_eq!(
            g["baseline_server_rows_read"],
            json!({"sum": whole, "max": whole})
        );
        assert!(g["server_rows_read"]["sum"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn a_ledger_changed_between_the_two_reads_is_one_mismatch() {
        let f = fixture();
        // Request 1 is "zebra"'s baseline (one pipeline); the row lands before its replay, so
        // only "zebra" sees the two sides disagree. Every later query reads it on both sides.
        let row = f.body("z1000001", "zebra", "zebra crossing", json!({}));
        let sql = format!(
            "INSERT INTO project_memory VALUES ('{PROJECT}', 'z1000001', '{}')",
            row.to_string().replace('\'', "''")
        );
        let ledger = f.tampered(1, Act::Insert(sql));
        let queries: Vec<String> = ["zebra", "deploy", "rollout", "alpha beta"]
            .iter()
            .map(|q| q.to_string())
            .collect();
        let r = report(&ledger, PROJECT, &queries, 0, f.now).unwrap();
        let mismatches = r["mismatches"].as_array().unwrap();
        assert_eq!(mismatches.len(), 1, "{r:#}");
        assert_eq!(
            mismatches[0],
            json!({"query": "zebra", "group": "english", "baseline": [],
                "baseline_partial": false, "got": ["z1000001"], "partial": false})
        );
        let g = &r["groups"]["english"];
        assert_eq!(g["queries"], 4);
        assert_eq!(g["equal"], 3);
        assert_eq!(r["equal"], 3);
    }

    #[test]
    fn a_failing_query_names_its_place_side_and_progress() {
        let f = fixture();
        let queries: Vec<String> = ["deploy", "rollout", "split"]
            .iter()
            .map(|q| q.to_string())
            .collect();
        // "deploy" takes two requests (baseline, one-trip replay), so the third request is
        // "rollout"'s baseline.
        let ledger = f.tampered(2, Act::Break);
        let err = report(&ledger, PROJECT, &queries, 0, f.now).unwrap_err();
        assert!(
            err.starts_with("query 2 of 3 \"rollout\": baseline failed after 1 equal: "),
            "{err}"
        );
        let ledger = f.tampered(3, Act::Break);
        let err = report(&ledger, PROJECT, &queries, 0, f.now).unwrap_err();
        assert!(
            err.starts_with("query 2 of 3 \"rollout\": replay failed after 1 equal: "),
            "{err}"
        );
    }

    #[test]
    fn a_refused_search_statement_is_named_in_the_error() {
        let f = fixture();
        let queries = vec!["deploy".to_string()];
        // Request 0 is the baseline; request 1 the replay's trip, whose step 11 is the first
        // query's stage 1 (five CREATEs, count, conflicts, screen, lifecycle, excluded).
        let ledger = f.tampered(1, Act::FailStep(11, "SQLITE_ERROR"));
        let err = report(&ledger, PROJECT, &queries, 0, f.now).unwrap_err();
        assert_eq!(
            err,
            "query 1 of 1 \"deploy\": replay failed after 0 equal: remote sql: \
             {\"message\":\"SQLITE_ERROR\",\"query\":0,\"statement\":10,\
             \"statement_kind\":\"stage1\",\"trip\":1}"
        );
    }

    #[test]
    fn skipped_keys_calls_are_reported() {
        let f = fixture();
        let queries = vec!["deploy".to_string()];
        let r = report(&f.ledger, PROJECT, &queries, 3, f.now).unwrap();
        assert_eq!(r["skipped_keys"], 3);
        assert_eq!(r["queries"], 1);
    }

    #[test]
    fn nothing_to_grade_is_an_error() {
        let f = fixture();
        assert_eq!(f.report(&[]).unwrap_err(), "no recall queries to replay");
        assert_eq!(
            f.report(&["zzz", "qqq yyy"]).unwrap_err(),
            "every baseline answer is empty: check --memory-project and the aliases"
        );
        // The wrong memory project reads an empty ledger: the same refusal.
        let queries = vec!["deploy".to_string()];
        assert_eq!(
            report(&f.ledger, "not-a-project", &queries, 0, f.now).unwrap_err(),
            "every baseline answer is empty: check --memory-project and the aliases"
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("proj")).unwrap();
        let err = transcript_queries(dir.path(), "proj", Some(7), 0).unwrap_err();
        assert_eq!(
            err,
            format!(
                "no recall queries found in {}/proj over 7 days",
                dir.path().display()
            )
        );
        let err = transcript_queries(dir.path(), "proj", None, 0).unwrap_err();
        assert!(err.ends_with("/proj over all days"), "{err}");
    }

    #[test]
    fn an_unknown_server_counter_makes_the_figure_null() {
        assert_eq!(
            known_sum_max(&[Some(2), Some(3)]),
            json!({"sum": 5, "max": 3})
        );
        assert_eq!(known_sum_max(&[Some(2), None]), Value::Null);
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
