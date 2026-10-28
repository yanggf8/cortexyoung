use cort::measure::*;
use cort::measure_progress::Progress;
use cort::measure_store::{MeasureStore, SqliteStore};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const M: &str = "m1";

fn line(v: Value) -> String {
    format!("{v}\n")
}
fn boundary(uuid: &str, session: &str, cwd: &str) -> String {
    line(
        json!({"type":"system","subtype":"compact_boundary","uuid":uuid,"sessionId":session,
        "timestamp":"2026-10-28T00:37:47.345Z","cwd":cwd,
        "compactMetadata":{"trigger":"auto","preTokens":358791,"postTokens":19780}}),
    )
}
fn tool(name: &str, input: Value) -> String {
    line(
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":name,"input":input}]}}),
    )
}
fn bash(c: &str) -> String {
    tool("Bash", json!({"command":c}))
}
fn read(p: &str) -> String {
    tool("Read", json!({"file_path":p}))
}
fn human(t: &str) -> String {
    line(json!({"type":"user","origin":{"kind":"human"},"message":{"content":t}}))
}
fn filler(n: usize) -> String {
    (0..n).map(|i| bash(&format!("filler {i}"))).collect()
}

struct Env {
    _tmp: tempfile::TempDir,
    projects: PathBuf,
    store: SqliteStore,
    marks: Watermarks,
    progress: Progress,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let projects = tmp.path().join("projects");
    fs::create_dir_all(&projects).unwrap();
    let store = SqliteStore::open(&tmp.path().join("store.db")).unwrap();
    let mut progress = Progress::default();
    progress.compactions_complete = true;
    Env {
        _tmp: tmp,
        projects,
        store,
        marks: Watermarks::default(),
        progress,
    }
}

fn write(path: &Path, content: &str, age_secs: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
    let t = SystemTime::now() - Duration::from_secs(age_secs);
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn scan(e: &mut Env) -> ScanSummary {
    scan_transcripts(
        &mut e.store,
        &e.projects,
        &mut e.marks,
        M,
        &mut e.progress,
        &e._tmp.path().join("measure-progress.json"),
    )
    .unwrap()
}

#[test]
fn compaction_boundary_skips_old_and_uploads_start() {
    let mut e = env();
    let cwd = notgit(&e);
    let old =
        boundary("old", "s1", &cwd).replace("2026-10-28T00:37:47.345Z", "2026-10-27T15:59:59.999Z");
    let start = boundary("start", "s1", &cwd)
        .replace("2026-10-28T00:37:47.345Z", "2026-10-27T16:00:00.000Z");
    write(&e.projects.join("p/s1.jsonl"), &(old + &start), 0);
    let summary = scan(&mut e);
    assert_eq!(summary.compactions, 1);
    let rows = e.store.compactions_since(0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].boundary_uuid, "start");
    assert_eq!(rows[0].time_ms, cort::telemetry::MEASURE_START_MS);
}

fn notgit(e: &Env) -> String {
    e._tmp.path().join("nogit").to_string_lossy().into_owned()
}

#[test]
fn subagent_files_are_skipped() {
    let mut e = env();
    let c = notgit(&e);
    write(
        &e.projects.join("p/s1.jsonl"),
        &format!("{}{}", boundary("u1", "s1", &c), filler(3)),
        10,
    );
    write(
        &e.projects.join("p/s1/subagents/a.jsonl"),
        &format!("{}{}", boundary("u2", "s1", &c), filler(3)),
        10,
    );
    let s = scan(&mut e);
    assert_eq!(s.files_seen, 1);
    assert_eq!(s.compactions, 1);
    assert!(!e
        .store
        .compaction_owners(M)
        .unwrap()
        .iter()
        .any(|(b, _)| b == "u2"));
    assert!(e
        .store
        .compaction_owners(M)
        .unwrap()
        .iter()
        .any(|(b, _)| b == "u1"));
}

#[test]
fn unchanged_file_is_not_rescanned() {
    let mut e = env();
    let c = notgit(&e);
    write(
        &e.projects.join("p/s1.jsonl"),
        &boundary("u1", "s1", &c),
        10,
    );
    let a = scan(&mut e);
    assert_eq!((a.files_seen, a.files_scanned), (1, 1));
    let b = scan(&mut e);
    assert_eq!((b.files_seen, b.files_scanned, b.compactions), (1, 0, 0));
}

#[test]
fn grown_file_is_rescanned_and_upserted() {
    let mut e = env();
    let c = notgit(&e);
    let path = e.projects.join("p/s1.jsonl");
    write(
        &path,
        &format!("{}{}", boundary("u1", "s1", &c), filler(5)),
        10,
    );
    scan(&mut e);
    let rows = e.store.compactions_since(0).unwrap();
    assert!(!rows[0].post_complete);
    write(
        &path,
        &format!("{}{}", boundary("u1", "s1", &c), filler(60)),
        5,
    );
    let s = scan(&mut e);
    assert_eq!(s.files_scanned, 1);
    let rows = e.store.compactions_since(0).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].post_complete);
}

#[test]
fn shared_boundary_counted_once_first_writer_wins() {
    let mut e = env();
    let c = notgit(&e);
    write(
        &e.projects.join("p/old.jsonl"),
        &format!("{}{}", boundary("u1", "old", &c), filler(4)),
        100,
    );
    write(
        &e.projects.join("p/new.jsonl"),
        &format!("{}{}", boundary("u1", "new", &c), filler(9)),
        10,
    );
    scan(&mut e);
    let rows = e.store.compactions_since(0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, "old");
    assert_eq!(rows[0].post.tool_calls, 4);
}

#[test]
fn cwd_without_origin_gives_null_project() {
    let mut e = env();
    let dir = e._tmp.path().join("plain");
    fs::create_dir_all(&dir).unwrap();
    write(
        &e.projects.join("p/s1.jsonl"),
        &boundary("u1", "s1", &dir.to_string_lossy()),
        10,
    );
    scan(&mut e);
    assert_eq!(e.store.compactions_since(0).unwrap()[0].project_id, None);
}

#[test]
fn cwd_with_origin_gives_project_id() {
    let mut e = env();
    let dir = e._tmp.path().join("repo");
    fs::create_dir_all(&dir).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["remote", "add", "origin", "https://example.com/a/b.git"],
    ] {
        let ok = std::process::Command::new("git")
            .args(&args)
            .current_dir(&dir)
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }
    write(
        &e.projects.join("p/s1.jsonl"),
        &boundary("u1", "s1", &dir.to_string_lossy()),
        10,
    );
    scan(&mut e);
    assert!(e.store.compactions_since(0).unwrap()[0]
        .project_id
        .is_some());
}

#[test]
fn deleted_transcript_keeps_rows() {
    let mut e = env();
    let c = notgit(&e);
    let path = e.projects.join("p/s1.jsonl");
    write(&path, &boundary("u1", "s1", &c), 10);
    scan(&mut e);
    fs::remove_file(&path).unwrap();
    let s = scan(&mut e);
    assert_eq!(s.files_seen, 0);
    assert_eq!(e.store.compactions_since(0).unwrap().len(), 1);
    assert!(e.marks.is_empty());
}

#[test]
fn watermarks_round_trip() {
    let mut e = env();
    let c = notgit(&e);
    write(
        &e.projects.join("p/s1.jsonl"),
        &boundary("u1", "s1", &c),
        10,
    );
    scan(&mut e);
    let path = e._tmp.path().join("marks.json");
    e.marks.save(&path).unwrap();
    let loaded = Watermarks::load(&path);
    assert_eq!(loaded, e.marks);
    assert!(Watermarks::load(&e._tmp.path().join("missing.json")).is_empty());
}

#[test]
fn details_print_paths_and_corrections_locally() {
    let e = env();
    let c = notgit(&e);
    let body = format!(
        "{}{}{}{}{}",
        read("/w/a.rs"),
        boundary("u1", "s1", &c),
        read("/w/a.rs"),
        human("不對，重來"),
        filler(2)
    );
    write(&e.projects.join("p/s1.jsonl"), &body, 10);
    for id in ["s1", "u1"] {
        let v = compaction_details(&e.projects, id).unwrap();
        let text = v.to_string();
        assert!(text.contains("/w/a.rs"), "{id}: {text}");
        assert!(text.contains("不對，重來"), "{id}: {text}");
        assert!(text.contains("u1"));
    }
    assert!(compaction_details(&e.projects, "nope").is_err());
}

#[test]
fn file_written_before_and_read_after_counts_as_reread_in_store() {
    let mut e = env();
    let c = notgit(&e);
    let body = format!(
        "{}{}{}{}",
        tool("Write", json!({"file_path":"/w/a.rs","content":"x"})),
        boundary("u1", "s1", &c),
        read("/w/a.rs"),
        filler(2)
    );
    write(&e.projects.join("p/s1.jsonl"), &body, 10);
    scan(&mut e);
    let rows = e.store.compactions_since(0).unwrap();
    assert_eq!(rows[0].post.rereads, 1);
}

#[test]
fn files_within_one_millisecond_order_by_full_mtime() {
    let mut e = env();
    let c = notgit(&e);
    // Whole-second base, so +200us and +600us share one millisecond. The earlier file's path
    // sorts after the later file's path, so path order and mtime order disagree.
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let early = base + Duration::from_micros(200);
    let late = base + Duration::from_micros(600);
    for (name, session, t) in [("b.jsonl", "early", early), ("a.jsonl", "late", late)] {
        let path = e.projects.join("p").join(name);
        write(
            &path,
            &format!("{}{}", boundary("u1", session, &c), filler(2)),
            0,
        );
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(t)
            .unwrap();
        let got = fs::metadata(&path).unwrap().modified().unwrap();
        if got != t {
            println!("SKIP: filesystem truncates mtime precision");
            return;
        }
    }
    scan(&mut e);
    assert_eq!(e.store.compactions_since(0).unwrap()[0].session_id, "early");
}

#[test]
fn unreadable_subdirectory_is_an_error_and_keeps_watermarks() {
    use std::os::unix::fs::PermissionsExt;
    let mut e = env();
    let c = notgit(&e);
    write(
        &e.projects.join("p/s1.jsonl"),
        &boundary("u1", "s1", &c),
        10,
    );
    fs::create_dir_all(e.projects.join("locked")).unwrap();
    scan(&mut e);
    let before = e.marks.clone();
    let locked = e.projects.join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        println!("SKIP: permissions are not enforced (running as root?)");
        return;
    }
    let r = scan_transcripts(
        &mut e.store,
        &e.projects,
        &mut e.marks,
        M,
        &mut e.progress,
        &e._tmp.path().join("measure-progress.json"),
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(r.is_err());
    assert_eq!(e.marks, before);
}

#[test]
fn missing_projects_dir_is_empty_and_keeps_watermarks() {
    let mut e = env();
    let c = notgit(&e);
    write(
        &e.projects.join("p/s1.jsonl"),
        &boundary("u1", "s1", &c),
        10,
    );
    scan(&mut e);
    let before = e.marks.clone();
    let gone = e._tmp.path().join("nope");
    let s = scan_transcripts(
        &mut e.store,
        &gone,
        &mut e.marks,
        M,
        &mut e.progress,
        &e._tmp.path().join("measure-progress.json"),
    )
    .unwrap();
    assert_eq!(s.files_seen, 0);
    assert_eq!(e.marks, before);
}

#[test]
fn details_tolerate_an_unreadable_sibling_directory() {
    use std::os::unix::fs::PermissionsExt;
    let e = env();
    let c = notgit(&e);
    write(
        &e.projects.join("p/s1.jsonl"),
        &boundary("u1", "s1", &c),
        10,
    );
    let locked = e.projects.join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        println!("SKIP: permissions are not enforced (running as root?)");
        return;
    }
    let ok = compaction_details(&e.projects, "u1");
    let missing = compaction_details(&e.projects, "nope");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ok.is_ok());
    assert!(missing.unwrap_err().contains("1 unreadable"));
}
