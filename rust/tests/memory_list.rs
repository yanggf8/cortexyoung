use cort::memory_store::{self, Ledger};
use cort::memory_tiers;
use cort::remote::{SqliteTransport, Transport};
use serde_json::{json, Value};
use std::process::Command;
use std::sync::Arc;

const NS: i128 = 1_000_000_000;
const DAY: i128 = 86_400 * NS;
/// 2026-11-14T00:00:00Z, a Saturday.
const EPOCH_NS: i128 = 1_794_614_400 * NS;
const NOW_NS: i128 = EPOCH_NS + DAY;

struct Fixture {
    _temp: tempfile::TempDir,
    ledger: Ledger,
    inner: Arc<SqliteTransport>,
}

struct Shared(Arc<SqliteTransport>);

impl Transport for Shared {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, cort::remote::RemoteError> {
        self.0.post(url, body, headers)
    }
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        for args in [
            vec!["init"],
            vec!["remote", "add", "origin", "https://example.test/ledger.git"],
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
        let ledger = Ledger::with_transport(
            "https://example.test",
            "test",
            Box::new(Shared(inner.clone())),
        )
        .unwrap();
        let f = Self {
            _temp: temp,
            ledger,
            inner,
        };
        // A read provisions the tables, exactly as `recall` and `status` do.
        f.ledger.read(&f.project()).unwrap();
        f
    }
    fn root(&self) -> &std::path::Path {
        self._temp.path()
    }
    fn project(&self) -> String {
        memory_store::project(self.root()).unwrap()
    }
    fn sql(&self, sql: &str) {
        let body =
            json!({"requests":[{"type":"execute","stmt":{"sql":sql,"args":[]}},{"type":"close"}]})
                .to_string();
        let response = self
            .inner
            .post("https://example.test", body.as_bytes(), &[])
            .unwrap();
        let value: Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(value["results"][0]["type"], "ok", "{value}");
    }
    fn insert(
        &self,
        project: &str,
        id: &str,
        key: &str,
        claim: &str,
        at_ns: i128,
        sup: Option<&str>,
    ) {
        self.insert_raw(project, id, key, claim, &at_ns.to_string(), sup);
    }
    fn insert_raw(
        &self,
        project: &str,
        id: &str,
        key: &str,
        claim: &str,
        recorded_at: &str,
        sup: Option<&str>,
    ) {
        let mut entry = json!({"id":id,"key":key,"claim":claim,"source":format!(".cort-unsourced/{id}"),"source_line":1,
            "source_sha256":"","unsourced":true,"recorded_at":recorded_at});
        if let Some(s) = sup {
            entry["supersedes"] = json!(s);
        }
        let body = entry.to_string().replace('\'', "''");
        self.sql(&format!(
            "INSERT INTO project_memory VALUES ('{project}', '{id}', '{body}')"
        ));
    }
    fn report(&self) -> memory_tiers::Report {
        self.report_at(NOW_NS)
    }
    fn report_at(&self, now_ns: i128) -> memory_tiers::Report {
        let recall = memory_store::recall_with(self.root(), "", &self.ledger).unwrap();
        memory_tiers::build(&self.project(), "2026-11-14", EPOCH_NS, now_ns, &recall)
    }
}

fn seed(f: &Fixture) {
    let p = f.project();
    // 2026-10-01: before epoch - 7d, so long; forgetting due 2026-11-30 (a Monday).
    f.insert(
        &p,
        "aaaaaaaa11111111",
        "alpha",
        "the User chose A",
        1_790_812_800 * NS,
        None,
    );
    f.insert(
        &p,
        "bbbbbbbb22222222",
        "beta",
        "old beta",
        EPOCH_NS - 20 * DAY,
        None,
    );
    f.insert(
        &p,
        "cccccccc33333333",
        "beta",
        "new beta",
        EPOCH_NS - DAY,
        Some("bbbbbbbb22222222"),
    );
    f.insert(
        &p,
        "dddddddd44444444",
        "gamma",
        "plain",
        EPOCH_NS - 3 * DAY,
        None,
    );
}

#[test]
fn list_json_summary_counts() {
    let f = Fixture::new();
    seed(&f);
    let v = f.report().to_json();
    assert_eq!(v["project_id"], f.project());
    assert_eq!(v["epoch"], "2026-11-14");
    assert_eq!(v["epoch_utc"], "2026-11-14T00:00:00Z");
    let s = &v["summary"];
    assert_eq!(s["active"], 3, "{v}");
    assert_eq!(s["long"], 1);
    assert_eq!(s["short"], 2);
    // NOW is epoch + 1d, so nothing recorded around the epoch has expired yet.
    assert_eq!(s["short_term_expired"], 0);
    assert_eq!(s["mentions_user"], 1);
    assert_eq!(s["forgetting_due_by_week"], json!({"2026-11-30": 1}));
    assert_eq!(s["by_evidence"], json!({"unsourced": 3}));
    let entries = v["entries"].as_array().unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        ["aaaaaaaa11111111", "dddddddd44444444", "cccccccc33333333"]
    );
    assert_eq!(entries[0]["tier"], "long");
    assert_eq!(entries[0]["forgetting_due_at_utc"], "2026-11-30T00:00:00Z");
    assert_eq!(entries[0]["recorded_at_utc"], "2026-10-01T00:00:00Z");
    assert_eq!(entries[0]["age_days"], 45);
    assert!(entries[0].get("expires_at_utc").is_none());
    assert_eq!(entries[1]["tier"], "short");
    assert_eq!(entries[1]["expires_at_utc"], "2026-11-21T00:00:00Z");
    assert!(entries[1].get("forgetting_due_at_utc").is_none());
    assert_eq!(entries[0]["mentions_user"], true);
}

#[test]
fn list_lean_has_no_empty_field_and_escapes_tabs() {
    let f = Fixture::new();
    let p = f.project();
    let long_claim = format!("tab\there\nnew\rline {}", "x".repeat(300));
    f.insert(
        &p,
        "eeeeeeee55555555",
        "k\tey",
        &long_claim,
        EPOCH_NS - 2 * DAY,
        None,
    );
    // Control characters pass the ledger's nonblank check but print as blanks.
    f.insert(
        &p,
        "ffffffff66666666",
        "blank",
        "\u{1}\u{2}",
        EPOCH_NS - DAY,
        None,
    );
    let lean = f.report().lean();
    let mut lines = lean.lines();
    assert_eq!(
        lines.next().unwrap(),
        "active=2 short=2 long=0 short_term_expired=0 mentions_user=0 epoch=2026-11-14 epoch_utc=2026-11-14T00:00:00Z"
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 2, "{lean}");
    for row in &rows {
        let fields: Vec<&str> = row.split('\t').collect();
        assert_eq!(fields.len(), 8, "{row:?}");
        assert!(fields.iter().all(|x| !x.trim().is_empty()), "{row:?}");
    }
    let first: Vec<&str> = rows[0].split('\t').collect();
    assert_eq!(first[0], "eeeeeeee");
    assert_eq!(first[1], "2026-11-12");
    assert_eq!(first[2], "3");
    assert_eq!(first[3], "short");
    assert_eq!(first[4], "unsourced");
    assert_eq!(first[5], "n");
    assert_eq!(first[6], "k ey");
    assert!(
        first[7].starts_with("tab here new line xxx"),
        "{}",
        first[7]
    );
    assert_eq!(first[7].chars().count(), 161);
    assert!(first[7].ends_with('…'));
    let second: Vec<&str> = rows[1].split('\t').collect();
    assert_eq!(second[7], "-");
}

#[test]
fn list_reads_alias_component() {
    let f = Fixture::new();
    let p = f.project();
    f.sql(&format!(
        "INSERT INTO project_aliases VALUES ('old-project-id', '{p}', 'now', 'test')"
    ));
    f.insert(
        "old-project-id",
        "11111111aaaaaaaa",
        "legacy",
        "kept under the old id",
        EPOCH_NS - DAY,
        None,
    );
    f.insert(
        "unrelated-project",
        "22222222bbbbbbbb",
        "other",
        "not ours",
        EPOCH_NS - DAY,
        None,
    );
    f.insert(
        &p,
        "33333333cccccccc",
        "current",
        "recorded here",
        EPOCH_NS - 2 * DAY,
        None,
    );
    let v = f.report().to_json();
    assert_eq!(v["summary"]["active"], 2, "{v}");
    let keys: Vec<&str> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["current", "legacy"]);
}

#[test]
fn unparsable_recorded_at_is_unknown_not_1970() {
    let f = Fixture::new();
    let p = f.project();
    f.insert_raw(&p, "99999999aaaaaaaa", "bad", "no date", "x", None);
    // Parses as an integer but lies far outside the printable UTC range.
    f.insert_raw(
        &p,
        "88888888bbbbbbbb",
        "huge",
        "overflow",
        &i128::MAX.to_string(),
        None,
    );
    f.insert(&p, "77777777cccccccc", "ok", "fine", EPOCH_NS - DAY, None);
    let report = f.report();
    let v = report.to_json();
    let s = &v["summary"];
    assert_eq!(s["active"], 3, "{v}");
    assert_eq!(s["short"], 1);
    assert_eq!(s["long"], 0);
    assert_eq!(s["short_term_expired"], 0);
    assert_eq!(s["unparsed_recorded_at"], 2);
    assert_eq!(s["forgetting_due_by_week"], json!({}));
    let entries = v["entries"].as_array().unwrap();
    // Unknown rows sort last, so the listing stays oldest first.
    assert_eq!(entries[0]["key"], "ok", "{v}");
    assert!(entries[1..].iter().all(|e| e["tier"] == "unknown"), "{v}");
    let bad = entries.iter().find(|e| e["key"] == "bad").unwrap();
    assert_eq!(bad["tier"], "unknown");
    for field in [
        "recorded_at_utc",
        "age_days",
        "expires_at_utc",
        "forgetting_due_at_utc",
    ] {
        assert!(bad.get(field).is_none(), "{field}: {bad}");
    }
    let lean = report.lean();
    let header = lean.lines().next().unwrap();
    assert!(header.ends_with(" unparsed_recorded_at=2"), "{header}");
    let row = lean.lines().find(|l| l.contains("\tbad\t")).unwrap();
    let fields: Vec<&str> = row.split('\t').collect();
    assert_eq!(&fields[1..4], ["-", "-", "unknown"], "{row:?}");
}

#[test]
fn expired_short_term_is_counted_in_json_and_lean() {
    let f = Fixture::new();
    let p = f.project();
    // Expires epoch + 7d: gone by epoch + 8d.
    f.insert(&p, "a1a1a1a1a1a1a1a1", "old", "x", EPOCH_NS - 3 * DAY, None);
    // Expires epoch + 10d: still live at epoch + 8d.
    f.insert(&p, "b2b2b2b2b2b2b2b2", "new", "y", EPOCH_NS + 3 * DAY, None);
    let report = f.report_at(EPOCH_NS + 8 * DAY);
    assert_eq!(report.to_json()["summary"]["short_term_expired"], 1);
    let lean = report.lean();
    assert!(
        lean.lines()
            .next()
            .unwrap()
            .contains(" short_term_expired=1 "),
        "{lean}"
    );
}

#[test]
fn invalid_epoch_is_refused_before_any_network_use() {
    let temp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_cort"))
        .args(["memory", "list", "--epoch", "2026-02-31"])
        .current_dir(temp.path())
        .env("CORT_CACHE_DIR", temp.path().join("cache"))
        .env("CORT_MEMORY_DIR", temp.path().join("data"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("memory_list_epoch_invalid"), "{text}");
}
