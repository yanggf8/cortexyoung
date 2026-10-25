//! Scan Claude Code transcripts into the measurement store, and print one compaction's detail
//! locally. Message text and paths only ever reach stdout through [`compaction_details`]; the
//! store receives counts and nothing else.

use crate::data_dir::{data_dir, write_atomic};
use crate::measure_store::{CompactionRow, MeasureStore};
use crate::memory_store::project_from_git_origin;
use crate::telemetry::{upload_events, upload_health_events, TelemetryEvent, UploadInputs};
use crate::transcript::{compactions, parse_transcript_bytes, Compaction, Step, WindowDetails};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-file `(size, mtime_ms)` seen at the last successful scan, keyed by file path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watermarks {
    files: BTreeMap<String, (u64, i64)>,
}

impl Watermarks {
    pub fn default_path() -> PathBuf {
        data_dir().join("measure-transcripts.json")
    }

    /// A missing or unreadable file is an empty set: the worst case is a full rescan.
    pub fn load(path: &Path) -> Self {
        let files = fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Watermarks { files }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string(&self.files).map_err(|e| e.to_string())?;
        write_atomic(path, &text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub files_seen: usize,
    pub files_scanned: usize,
    pub compactions: usize,
    pub skipped_lines: usize,
    pub unknown_type_lines: usize,
    /// What the event upload did (`None` when events are not part of this run). Local only.
    pub events: Option<Value>,
    /// Health upload for this run (`None` for the existing manual record command).
    pub health: Option<Value>,
}

impl ScanSummary {
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "files_seen": self.files_seen,
            "files_scanned": self.files_scanned,
            "compactions": self.compactions,
            "skipped_lines": self.skipped_lines,
            "unknown_type_lines": self.unknown_type_lines,
        });
        if let Some(events) = &self.events {
            v["events"] = events.clone();
        }
        if let Some(health) = &self.health {
            v["health"] = health.clone();
        }
        v
    }
}

pub fn claude_projects_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(dir).join("projects");
    }
    std::env::var_os("HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude/projects")
}

struct TranscriptFile {
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
}

impl TranscriptFile {
    /// Millisecond form, kept only for the watermark format; ordering uses `mtime`.
    fn mtime_ms(&self) -> i64 {
        self.mtime
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// `projects_dir/*/*.jsonl` only; `<session>/subagents/` is a directory and never matches.
fn transcript_files(projects_dir: &Path) -> Result<Vec<TranscriptFile>, String> {
    collect_files(projects_dir, true).map(|(files, _)| files)
}

/// Strict (`scan`): any traversal error is returned. Lenient (`show`): unreadable entries are
/// skipped and counted, so one bad sibling cannot hide a readable transcript.
fn collect_files(
    projects_dir: &Path,
    strict: bool,
) -> Result<(Vec<TranscriptFile>, usize), String> {
    let mut out = Vec::new();
    let mut unreadable = 0usize;
    // A missing projects dir is a confirmed absence; any other failure is not.
    let projects = match fs::read_dir(projects_dir) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((out, 0)),
        Err(e) => return Err(format!("cannot read {}: {e}", projects_dir.display())),
    };
    let io = |p: &Path, e: std::io::Error| format!("cannot read {}: {e}", p.display());
    macro_rules! tolerate {
        ($res:expr, $path:expr) => {
            match $res {
                Ok(v) => v,
                Err(e) if strict => return Err(io($path, e)),
                Err(_) => {
                    unreadable += 1;
                    continue;
                }
            }
        };
    }
    for project in projects {
        let project = tolerate!(project, projects_dir);
        let ftype = tolerate!(project.file_type(), &project.path());
        if !ftype.is_dir() {
            continue;
        }
        let entries = tolerate!(fs::read_dir(project.path()), &project.path());
        for entry in entries {
            let entry = tolerate!(entry, &project.path());
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let meta = tolerate!(entry.metadata(), &path);
            if !meta.is_file() {
                continue;
            }
            let mtime = tolerate!(meta.modified(), &path);
            out.push(TranscriptFile {
                path,
                size: meta.len(),
                mtime,
            });
        }
    }
    out.sort_by(|a, b| (a.mtime, &a.path).cmp(&(b.mtime, &b.path)));
    Ok((out, unreadable))
}

pub fn scan_transcripts(
    store: &mut dyn MeasureStore,
    projects_dir: &Path,
    marks: &mut Watermarks,
    machine_id: &str,
) -> Result<ScanSummary, String> {
    if matches!(fs::metadata(projects_dir), Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(ScanSummary::default());
    }
    let files = transcript_files(projects_dir)?;
    let mut summary = ScanSummary {
        files_seen: files.len(),
        ..ScanSummary::default()
    };
    let mut projects: HashMap<String, Option<String>> = HashMap::new();
    let mut next: BTreeMap<String, (u64, i64)> = BTreeMap::new();
    for file in &files {
        let key = file.path.to_string_lossy().into_owned();
        let stamp = (file.size, file.mtime_ms());
        if marks.files.get(&key) == Some(&stamp) {
            next.insert(key, stamp);
            continue;
        }
        // An unreadable file keeps its old watermark, so the next scan tries it again.
        let Ok(text) = fs::read(&file.path) else {
            if let Some(old) = marks.files.get(&key) {
                next.insert(key, *old);
            }
            continue;
        };
        let parsed = parse_transcript_bytes(&text);
        summary.files_scanned += 1;
        summary.skipped_lines += parsed.skipped_lines;
        summary.unknown_type_lines += parsed.unknown_type_lines;
        for c in compactions(&parsed.steps) {
            let owner = store.compaction_session(machine_id, &c.boundary.uuid)?;
            if matches!(&owner, Some(s) if *s != c.boundary.session_id) {
                continue;
            }
            let project_id = c.boundary.cwd.as_ref().and_then(|cwd| {
                projects
                    .entry(cwd.clone())
                    .or_insert_with(|| project_from_git_origin(Path::new(cwd)).ok())
                    .clone()
            });
            store.upsert_compaction(&CompactionRow {
                machine_id: machine_id.to_string(),
                boundary_uuid: c.boundary.uuid.clone(),
                session_id: c.boundary.session_id.clone(),
                time_ms: c.boundary.time_ms,
                project_id,
                trigger: c.boundary.trigger.clone(),
                pre_tokens: c.boundary.pre_tokens,
                post_tokens: c.boundary.post_tokens,
                pre: c.pre.counts(),
                post: c.post.counts(),
                post_complete: c.post_complete,
            })?;
            summary.compactions += 1;
        }
        next.insert(key, stamp);
    }
    // Files that disappeared drop out of the watermarks; store rows are never deleted.
    marks.files = next;
    Ok(summary)
}

fn details_json(d: &WindowDetails) -> Value {
    json!({
        "tool_calls": d.tool_calls,
        "rereads": d.rereads,
        "repeated_commands": d.repeated_commands,
        "transcript_lookups": d.transcript_lookups,
        "human_messages": d.human_messages,
        "corrections": d.corrections,
    })
}

fn compaction_json(file: &Path, c: &Compaction) -> Value {
    json!({
        "file": file.to_string_lossy(),
        "uuid": c.boundary.uuid,
        "session_id": c.boundary.session_id,
        "time_ms": c.boundary.time_ms,
        "cwd": c.boundary.cwd,
        "trigger": c.boundary.trigger,
        "pre_tokens": c.boundary.pre_tokens,
        "post_tokens": c.boundary.post_tokens,
        "post_complete": c.post_complete,
        "pre": details_json(&c.pre),
        "post": details_json(&c.post),
    })
}

/// Local detail for one transcript (`id` = file stem, every boundary in it) or one boundary
/// (`id` = boundary uuid). Reads local files only and never opens the store.
pub fn compaction_details(projects_dir: &Path, id: &str) -> Result<Value, String> {
    let mut found = Vec::new();
    let (files, mut unreadable) = collect_files(projects_dir, false)?;
    for file in files {
        let by_stem = file.path.file_stem().and_then(|s| s.to_str()) == Some(id);
        let Ok(text) = fs::read(&file.path) else {
            unreadable += 1;
            continue;
        };
        if !by_stem && !String::from_utf8_lossy(&text).contains(id) {
            continue;
        }
        let parsed = parse_transcript_bytes(&text);
        let has_boundary = parsed.steps.iter().any(|s| matches!(s, Step::Boundary(_)));
        if !has_boundary {
            continue;
        }
        for c in compactions(&parsed.steps) {
            if by_stem || c.boundary.uuid == id {
                found.push(compaction_json(&file.path, &c));
            }
        }
    }
    if found.is_empty() {
        return Err(format!(
            "no compaction found for {id} ({unreadable} unreadable entries skipped)"
        ));
    }
    Ok(json!({"id": id, "compactions": found}))
}

// ---------------------------------------------------------------------------------------------
// record / report
// ---------------------------------------------------------------------------------------------

pub(crate) const DAY_MS: i64 = 86_400_000;

pub struct RecordInputs {
    pub projects_dir: PathBuf,
    pub marks_path: PathBuf,
    pub machine_id: String,
    pub host: String,
    pub now_ms: i64,
    /// Event upload for this run; `None` leaves events out (compaction only).
    pub events: Option<UploadInputs>,
    /// Local usage database to upload health observations from.
    pub health: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    Pushed,
    SavedLocallyPushFailed(String),
    /// The push landed but the pull did not; the scan summary must still reach the caller.
    PushedPullFailed(String),
}

/// Local writes first (host mapping, compaction rows), then the watermarks, then the network.
/// A failed local write returns before the watermarks move, so the next run rescans; a failed
/// push never loses data, because the rows are already in the local replica.
pub fn record(store: &mut dyn MeasureStore, input: &RecordInputs) -> Result<RecordOutcome, String> {
    match record_with_scan(store, input)?.1 {
        RecordOutcome::PushedPullFailed(e) => Err(format!("pushed, but pull failed: {e}")),
        other => Ok(other),
    }
}

/// [`record`], also handing back what the scan saw. The summary is local only: it is printed to
/// the caller (cron appends it to measure.log) and never stored or uploaded.
pub fn record_with_scan(
    store: &mut dyn MeasureStore,
    input: &RecordInputs,
) -> Result<(ScanSummary, RecordOutcome), String> {
    store.upsert_machine_host(&input.machine_id, &input.host, input.now_ms)?;
    let mut marks = Watermarks::load(&input.marks_path);
    let mut scan = scan_transcripts(store, &input.projects_dir, &mut marks, &input.machine_id)?;
    marks.save(&input.marks_path)?;
    // Events are independent of compactions: a refusal or failure here is reported beside the scan
    // and never fails the record (see `upload_events`).
    if let Some(events) = &input.events {
        scan.events = Some(upload_events(store, events).to_json());
    }
    if let Some(usage_db) = &input.health {
        scan.health = Some(upload_health_events(store, usage_db, &input.machine_id).to_json());
    }
    if let Err(e) = store.push() {
        return Ok((scan, RecordOutcome::SavedLocallyPushFailed(e)));
    }
    if let Err(e) = store.pull() {
        return Ok((scan, RecordOutcome::PushedPullFailed(e)));
    }
    Ok((scan, RecordOutcome::Pushed))
}

/// Push then pull. `Ok` means the local replica is as current as the remote was a moment ago.
pub fn sync(store: &mut dyn MeasureStore) -> Result<(), String> {
    store.push().map_err(|e| format!("push failed: {e}"))?;
    store.pull().map_err(|e| format!("pull failed: {e}"))
}

/// Events per page when copying between stores.
const COPY_PAGE: usize = 2000;
/// How many conflicting keys a [`CopyReport`] names.
const CONFLICT_EXAMPLES: usize = 5;

/// What [`copy_events`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyReport {
    /// `(source, machine_id, rows in from, rows in to)` for each group `from` holds.
    pub groups: Vec<(String, String, usize, usize)>,
    /// Source rows whose key already existed in `to` with different column values (kept as is).
    pub conflicts: usize,
    /// The first few conflicts: `(source, event_id, names of the differing columns)`.
    pub conflict_examples: Vec<(String, String, Vec<&'static str>)>,
}

fn differing_columns(a: &TelemetryEvent, b: &TelemetryEvent) -> Vec<&'static str> {
    let mut out = Vec::new();
    if a.machine_id != b.machine_id {
        out.push("machine_id");
    }
    if a.machine_source != b.machine_source {
        out.push("machine_source");
    }
    if a.project_id != b.project_id {
        out.push("project_id");
    }
    if a.time_ms != b.time_ms {
        out.push("time_ms");
    }
    if a.payload != b.payload {
        out.push("payload");
    }
    out
}

/// Copy every event and alias of `from` into `to`, unchanged, `INSERT OR IGNORE`, and push `to`
/// after each page so a failure leaves a resumable state. `from` is only pulled and read: it is
/// never pushed to or written. A row whose key `to` already holds is never overwritten; if its
/// columns differ it is reported as a conflict.
pub fn copy_events(
    from: &mut dyn MeasureStore,
    to: &mut dyn MeasureStore,
) -> Result<CopyReport, String> {
    from.pull()
        .map_err(|e| format!("source pull failed: {e}"))?;
    to.pull().map_err(|e| format!("pull failed: {e}"))?;
    let mut conflicts = 0usize;
    let mut examples = Vec::new();
    let mut after: Option<(String, String)> = None;
    loop {
        let page = from.events_page(after.clone(), COPY_PAGE)?;
        let Some(last) = page.last() else { break };
        let last = (last.source.clone(), last.event_id.clone());
        after = Some(last.clone());
        let inserted = to.insert_events(&page)?;
        if inserted < page.len() {
            let first = (page[0].source.clone(), page[0].event_id.clone());
            let stored: HashMap<(String, String), TelemetryEvent> = to
                .events_in_key_range(&first, &last)?
                .into_iter()
                .map(|e| ((e.source.clone(), e.event_id.clone()), e))
                .collect();
            let mut missing: Vec<&TelemetryEvent> = Vec::new();
            for e in &page {
                let Some(have) = stored.get(&(e.source.clone(), e.event_id.clone())) else {
                    missing.push(e);
                    continue;
                };
                let cols = differing_columns(e, have);
                if !cols.is_empty() {
                    conflicts += 1;
                    if examples.len() < CONFLICT_EXAMPLES {
                        examples.push((e.source.clone(), e.event_id.clone(), cols));
                    }
                }
            }
            if let Some(first_missing) = missing.first() {
                return Err(format!(
                    "{} event(s) were neither inserted nor already present in the destination \
                     (first: source {:?}, event_id {:?}); the destination refused them",
                    missing.len(),
                    first_missing.source,
                    first_missing.event_id
                ));
            }
        }
        to.push().map_err(|e| format!("push failed: {e}"))?;
    }
    let aliases = from.aliases()?;
    to.upsert_aliases(&aliases)?;
    to.push().map_err(|e| format!("push failed: {e}"))?;
    let dest: HashMap<(String, String), usize> = to
        .event_counts()?
        .into_iter()
        .map(|(s, m, n)| ((s, m), n))
        .collect();
    let groups = from
        .event_counts()?
        .into_iter()
        .map(|(s, m, n)| {
            let d = dest.get(&(s.clone(), m.clone())).copied().unwrap_or(0);
            (s, m, n, d)
        })
        .collect();
    Ok(CopyReport {
        groups,
        conflicts,
        conflict_examples: examples,
    })
}

pub fn last_sync_path() -> PathBuf {
    data_dir().join("measure-last-sync")
}

pub fn write_last_sync(ms: i64) -> Result<(), String> {
    let path = last_sync_path();
    write_atomic(&path, &ms.to_string())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn read_last_sync() -> Option<i64> {
    fs::read_to_string(last_sync_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// First non-empty of the kernel hostname, `/etc/hostname`, `$HOSTNAME`; else `unknown`.
pub fn host_name() -> String {
    let files = ["/proc/sys/kernel/hostname", "/etc/hostname"];
    for f in files {
        if let Ok(s) = fs::read_to_string(f) {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
    }
    match std::env::var("HOSTNAME") {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => "unknown".to_string(),
    }
}

// Howard Hinnant's civil-date algorithms, so no date crate is needed.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-Www` for the UTC ISO week containing `ms`.
pub fn iso_week(ms: i64) -> String {
    let days = ms.div_euclid(DAY_MS);
    let weekday = (days + 3).rem_euclid(7); // 0 = Monday; 1970-01-01 was a Thursday
    let thursday = days - weekday + 3;
    let (year, _, _) = civil_from_days(thursday);
    let week = (thursday - days_from_civil(year, 1, 1)) / 7 + 1;
    format!("{year}-W{week:02}")
}

/// `YYYY-MM-DD HH:MM UTC`.
pub fn format_utc(ms: i64) -> String {
    let days = ms.div_euclid(DAY_MS);
    let rem = ms.rem_euclid(DAY_MS) / 60_000;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 60, rem % 60)
}

pub struct ReportInputs {
    pub days: i64,
    pub compaction_days: i64,
    pub now_ms: i64,
    /// `Some(last sync text)` when this report is read from the local replica after a failed sync.
    pub offline: Option<String>,
}

const METRICS: [&str; 4] = [
    "rereads",
    "repeated_commands",
    "transcript_lookups",
    "corrections",
];

fn deltas(r: &CompactionRow) -> [i64; 4] {
    [
        r.post.rereads - r.pre.rereads,
        r.post.repeated_commands - r.pre.repeated_commands,
        r.post.transcript_lookups - r.pre.transcript_lookups,
        r.post.corrections - r.pre.corrections,
    ]
}

fn median(sorted: &[i64]) -> Option<f64> {
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2] as f64),
        _ => Some((sorted[n / 2 - 1] + sorted[n / 2]) as f64 / 2.0),
    }
}

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

fn host_section(host: &str, machine_ids: &[String], rows: &[&CompactionRow]) -> Value {
    let compared: Vec<[i64; 4]> = rows
        .iter()
        .filter(|r| r.pre.tool_calls > 0)
        .map(|r| deltas(r))
        .collect();
    let mut metrics = serde_json::Map::new();
    for (i, name) in METRICS.iter().enumerate() {
        let mut col: Vec<i64> = compared.iter().map(|d| d[i]).collect();
        col.sort_unstable();
        metrics.insert(
            (*name).to_string(),
            json!({"median": median(&col), "sum": col.iter().sum::<i64>()}),
        );
    }
    let mut weeks: BTreeMap<String, (i64, i64, [i64; 4])> = BTreeMap::new();
    for r in rows {
        let e = weeks.entry(iso_week(r.time_ms)).or_default();
        e.0 += 1;
        if r.pre.tool_calls > 0 {
            e.1 += 1;
            for (acc, d) in e.2.iter_mut().zip(deltas(r)) {
                *acc += d;
            }
        }
    }
    let weeks: Vec<Value> = weeks
        .into_iter()
        .map(|(week, (n, cmp, sums))| {
            json!({"week": week, "compactions": n, "compared": cmp,
                "rereads": sums[0], "repeated_commands": sums[1],
                "transcript_lookups": sums[2], "corrections": sums[3]})
        })
        .collect();
    json!({
        "host": host,
        "machine_ids": machine_ids,
        "compactions": rows.len(),
        "compared": compared.len(),
        "no_baseline": rows.len() - compared.len(),
        "metrics": metrics,
        "weeks": weeks,
    })
}

/// Reads the store as it stands; the caller decides whether to sync first and says so in
/// `offline`. Only the compaction section exists so far.
pub fn report(store: &mut dyn MeasureStore, input: &ReportInputs) -> Result<Value, String> {
    let since = input.now_ms - input.compaction_days * DAY_MS;
    let rows = store.compactions_since(since)?;
    let hosts = store.machine_hosts()?;

    // machine_id -> hosts, most recently seen first; the first one names the machine.
    let mut by_machine: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
    for h in &hosts {
        by_machine
            .entry(h.machine_id.clone())
            .or_default()
            .push((h.last_seen_ms, h.host.clone()));
    }
    let mut warnings = Vec::new();
    let mut host_of: BTreeMap<String, String> = BTreeMap::new();
    for (machine, list) in by_machine.iter_mut() {
        list.sort_by(|a, b| b.cmp(a));
        host_of.insert(machine.clone(), list[0].1.clone());
        if list.len() >= 2 {
            let names: Vec<&str> = list.iter().map(|(_, h)| h.as_str()).collect();
            warnings.push(format!(
                "machine {machine} has {} hosts: {} (counted under {})",
                list.len(),
                names.join(", "),
                list[0].1
            ));
        }
    }
    // Every recorded (machine, host) pair counts here, not just each machine's latest host.
    let mut machines_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for h in &hosts {
        let ids = machines_of.entry(h.host.clone()).or_default();
        if !ids.contains(&h.machine_id) {
            ids.push(h.machine_id.clone());
        }
    }
    for ids in machines_of.values_mut() {
        ids.sort();
    }
    for (host, ids) in &machines_of {
        if ids.len() >= 2 {
            warnings.push(format!(
                "host {host} has {} machine ids: {} (counted together)",
                ids.len(),
                ids.join(", ")
            ));
        }
    }

    // Every recorded host gets a section, compactions or not: a quiet machine is not an absent one.
    let mut grouped: BTreeMap<String, (Vec<String>, Vec<&CompactionRow>)> = BTreeMap::new();
    for (machine, host) in &host_of {
        grouped
            .entry(host.clone())
            .or_default()
            .0
            .push(machine.clone());
    }
    for r in &rows {
        let host = host_of
            .get(&r.machine_id)
            .cloned()
            .unwrap_or_else(|| format!("machine {}", short(&r.machine_id)));
        let entry = grouped.entry(host).or_default();
        if !entry.0.contains(&r.machine_id) {
            entry.0.push(r.machine_id.clone());
        }
        entry.1.push(r);
    }
    let sections: Vec<Value> = grouped
        .iter()
        .map(|(host, (ids, rs))| {
            let mut ids = ids.clone();
            ids.sort();
            host_section(host, &ids, rs)
        })
        .collect();
    Ok(json!({
        "offline": input.offline,
        "days": input.days,
        "compactions": {
            "days": input.compaction_days,
            "total": rows.len(),
            "hosts": sections,
        },
        "warnings": warnings,
    }))
}

struct Paint(bool);

impl Paint {
    fn bold(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn yellow(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[33m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

pub fn render_report(v: &Value, color: bool) -> String {
    let p = Paint(color);
    let mut out = String::new();
    if let Some(when) = v["offline"].as_str() {
        out.push_str(&p.yellow(&format!("離線：讀本機副本，最後同步於 {when}")));
        out.push('\n');
    }
    let c = &v["compactions"];
    let days = c["days"].as_i64().unwrap_or(0);
    out.push_str(&p.bold(&format!("壓縮（最近 {days} 天）")));
    out.push('\n');
    let hosts = c["hosts"].as_array().cloned().unwrap_or_default();
    if hosts.is_empty() {
        out.push_str("這段期間沒有壓縮\n");
    }
    for h in &hosts {
        let ids: Vec<&str> = h["machine_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        out.push('\n');
        out.push_str(&p.bold(&format!(
            "{}  ({})",
            h["host"].as_str().unwrap_or("?"),
            ids.join(", ")
        )));
        out.push('\n');
        if h["compactions"].as_i64().unwrap_or(0) == 0 {
            out.push_str("  這段期間沒有壓縮\n");
            continue;
        }
        out.push_str(&format!(
            "  壓縮 {} 次；可比較 {}，no baseline {}（前窗沒有工具呼叫，只列後窗、不拿來比）\n",
            h["compactions"], h["compared"], h["no_baseline"]
        ));
        out.push_str("  後窗減前窗            中位數    總和\n");
        for name in METRICS {
            let m = &h["metrics"][name];
            let med = m["median"]
                .as_f64()
                .map_or("-".to_string(), |x| format!("{x:+.1}"));
            out.push_str(&format!(
                "  {name:<20} {med:>8} {:>7}\n",
                format!("{:+}", m["sum"].as_i64().unwrap_or(0))
            ));
        }
        out.push_str("  每週（UTC ISO 週，總和）\n");
        for w in h["weeks"].as_array().cloned().unwrap_or_default() {
            let n = |k: &str| w[k].as_i64().unwrap_or(0);
            out.push_str(&format!(
                "  {}  壓縮 {:>3}  rereads {:+}  repeated {:+}  lookups {:+}  corrections {:+}\n",
                w["week"].as_str().unwrap_or("?"),
                n("compactions"),
                n("rereads"),
                n("repeated_commands"),
                n("transcript_lookups"),
                n("corrections"),
            ));
        }
    }
    for w in v["warnings"].as_array().cloned().unwrap_or_default() {
        out.push_str(&p.yellow(&format!("警告：{}", w.as_str().unwrap_or(""))));
        out.push('\n');
    }
    out
}
