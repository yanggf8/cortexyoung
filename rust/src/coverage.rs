//! Recall side: **what did this enumeration miss?**
//!
//! `impact` answers "who calls this" and `verify-impact` checks that each reported dependent really
//! mentions the seed. Both are precision instruments: neither can see a caller that never became an
//! edge, which is the failure that matters for the verbs the product exists for ("safe to remove",
//! "rename it", "nothing else uses this"). Measured example, on this repo: `tally.add(&project, &text)`
//! appears twice in `evals/src/demand.rs`, while `cort impact --symbol Tally::add` reports
//! `dependents=0` with `stale=false` — a confident empty answer, because the Rust pack extracts only
//! bare and `Type::method` call shapes and not receiver calls.
//!
//! So this module answers the other question by comparing the graph against two things it cannot lie
//! about: what the extractor recorded (`raw_edges`) and what is on disk. Three layers, weakest first:
//!
//! * `mentions_without_edge` — a line on disk names the seed and produced no edge at all. Covers
//!   extractor blind spots (receiver and module-path call forms, unindexed call styles, and the
//!   const-binding shape that cost round 2 its labels).
//! * `extracted_but_unresolved` — the extractor saw the call, but resolution dropped it, which is how
//!   `relationship_rows_for_symbol_map`'s `targets.is_empty() { continue }` looks from the outside:
//!   silently.
//! * `blind_files` — files with no chunks (parse degraded) or not indexed at all, where an edge is
//!   impossible by construction.
//!
//! What this is **not**: proof. A mention can be a comment, a string, or a same-named symbol in
//! another module, so the counts are candidates to look at, deliberately over-inclusive. The field is
//! named `enumeration_may_be_incomplete` rather than anything containing "complete", because `false`
//! means "no gap signal", never "verified none".
//!
//! **Read the rows, not the boolean.** The boolean has exactly two ways to be true -- a named gap row,
//! or a file this screen never read (`unindexed`, `scan_skipped`) -- and one way to be false: nothing
//! of either kind. Files that exist in the index but produced no chunks (`unparsed`: barrels,
//! type-only modules, a `pub mod`-only `lib.rs`) used to flip it too, which made two files in this
//! repo silently declare every seed in it incomplete. A boolean that is always true is not a warning
//! light, it is noise -- and the agents that stop believing it are the same agents that then ignore
//! the one it was protecting. Those files are still listed with their paths and still reach `why` as
//! the advisory `unparsed_files`, because their *text* was scanned: a caller in them appears as a row.

use crate::db::Db;
use crate::errors::CortError;
use crate::indexer::{walk_files, SOURCE_EXT};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Cap on printed gap rows. A screen that dumps 900 hits is not a screen; the total is always
/// reported, so truncation cannot hide the size of a hole.
pub const MAX_GAP_ROWS: usize = 20;

/// A raw edge's `start_line` is the line the matched node started on, which for a multi-line call is
/// not the line the callee's name sits on. Two lines of slack, and nothing more.
pub const LINE_TOLERANCE: i64 = 2;

/// Named so a reader can tell which screen produced a claim, and so both shapes of the report
/// (with seeds and without) agree on what generated them.
/// v2 changed what the boolean *means*, so the version has to change with it: a report is read long
/// after the build that wrote it, and `enumeration_may_be_incomplete: true` from a v1 screen is a
/// different claim from `true` from a v2 one.
pub const COVERAGE_METHOD: &str = "coverage-v2 (three-layer recall screen: mentions, dropped \
                                   resolutions, blind files -- the boolean means 'a named gap, or a \
                                   file this screen never read'; unparsed files are advisory)";

/// How many files a blind-layer field reports, whether it carries a count or the path list. The
/// flag has to be right whichever shape the field has: reading an array as "no count" and defaulting
/// to zero is exactly how a blind file once produced a clean bill of health.
fn blind_count(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Array(list)) => list.len(),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as usize,
        _ => 0,
    }
}

/// Paths that are machine-generated copies of the source tree. A bundle cannot be a caller of a
/// source symbol, and `cct`'s index currently contains seven `.wrangler/tmp/deploy-*/index.js`
/// copies because `IGNORE_DIRS` does not list `.wrangler` -- reported separately rather than
/// silently excluded, since the underlying scope defect is a decision for the maintainer, not for
/// a screen to hide.
const GENERATED_MARKERS: &[&str] = &[
    ".wrangler/",
    "dist/",
    "build/",
    ".next/",
    ".svelte-kit/",
    "node_modules/",
    "vendor/",
    "/target/",
];

fn looks_generated(path: &str, first_line_chars: usize) -> bool {
    GENERATED_MARKERS.iter().any(|m| path.contains(m)) || first_line_chars > 400
}

/// Lines that *declare* the name somewhere else. With name-based resolution a second `d` in another
/// file is a real hazard, so this is kept and labelled rather than filtered out -- but it is not a
/// missed caller, and it used to be reported as one.
fn definition_lines(
    db: &Db,
    project_id: &str,
    name: &str,
) -> Result<HashSet<(String, i64)>, CortError> {
    let err =
        |e: rusqlite::Error| CortError::new("storage_busy", json!({ "message": e.to_string() }));
    let mut stmt = db
        .prepare(
            "SELECT file_path, start_line FROM chunks
          WHERE project_id = ?1 AND symbol_name IS NOT NULL
            AND (symbol_name = ?2 OR symbol_name LIKE '%:' || ?3 OR symbol_name LIKE '%.' || ?4)",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(params![project_id, name, name, name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(err)?
        .flatten()
        .collect();
    Ok(rows)
}

fn cause_rank(cause: &str) -> u8 {
    match cause {
        "receiver" => 0,
        "call" => 1,
        "mention" => 2,
        "import" => 3, // names the symbol, proves the file is already reachable
        "definition" => 4,
        "quoted" => 5,
        "comment" => 6, // named the thing, proved nothing
        _ => 7,         // artifact: a generated copy of the tree
    }
}

/// Files bigger than this are skipped for mention scanning rather than read into memory.
const MAX_SCAN_BYTES: u64 = 2_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedRef {
    pub symbol: String,
    pub chunk_id: String,
    pub file_path: String,
    pub start_line: i64,
}

/// Build the coverage section from an `impact` payload's own seeds, so the CLI can attach it without
/// `impact_command` growing a parameter (and without the two functions disagreeing about which
/// symbols were actually seeded). `symbol_name` is read back from the database rather than parsed out
/// of the request string, because a batched `--symbol a,b,c` seed list and the resolved chunk list are
/// not the same thing when a name is qualified or absent.
pub fn attach(
    db: &Db,
    project_id: &str,
    root: &Path,
    payload: &mut Value,
) -> Result<(), CortError> {
    let mut seeds = Vec::new();
    if let Some(list) = payload.get("seeds").and_then(Value::as_array) {
        for seed in list {
            let chunk_id = match seed.get("chunk_id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => continue,
            };
            let symbol: String = db
                .query_row(
                    "SELECT COALESCE(symbol_name, file_path) FROM chunks WHERE chunk_id = ?1",
                    params![chunk_id],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| chunk_id.clone());
            seeds.push(SeedRef {
                symbol,
                file_path: seed
                    .get("file_path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                start_line: seed.get("start_line").and_then(Value::as_i64).unwrap_or(0),
                chunk_id,
            });
        }
    }
    if seeds.is_empty() {
        // K3's review: this used to exit 1 with `nothing_indexed`, turning "the index cannot answer
        // this" into a tool failure -- and anything that treats a clean exit as a passing check
        // would read an unindexed symbol as a green light. Report it as the worst recall state
        // instead: no seed resolved, so nothing was looked at in the first place.
        let blind = blind_files(db, project_id, root, &index_set_of(db, project_id)?)?;
        if let Some(map) = payload.as_object_mut() {
            map.insert(
                "coverage".to_string(),
                json!({
                    "method": COVERAGE_METHOD,
                    "no_seed_resolved": true,
                    "seeds": [],
                    "blind_files": blind,
                    "enumeration_may_be_incomplete": true,
                    "why": ["no_seed_resolved"],
                    "reading": "none of the names asked for exist in the index, so this enumeration \
                                covered nothing at all. Not a clean answer: the symbol may be new, \
                                renamed, misspelled, or in a file the indexer does not read.",
                }),
            );
        }
        return Ok(());
    }
    let report = coverage_for(db, project_id, root, &seeds)?;
    if let Some(map) = payload.as_object_mut() {
        map.insert("coverage".to_string(), report);
    }
    Ok(())
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '\u{00b7}'
}

/// `Type::method`, `crate::m::f` and `x.method` all name the method `method`. The one split both the
/// receiver gate (`graph`) and this screen use; re-exported so the two cannot drift apart.
pub use crate::chunker::bare_name;

/// Every whole-word occurrence of `name`, as 1-based (line, column). The column is what makes a
/// cause guess possible, and the cause is what makes the list usable: a screen that reports a git
/// argument (`["add", "-A"]`) the same way it reports a real receiver call gets ignored.
pub fn mentions(text: &str, name: &str) -> Vec<(usize, usize)> {
    let needle: Vec<char> = name.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < needle.len() {
            continue;
        }
        for start in 0..=(chars.len() - needle.len()) {
            if chars[start..start + needle.len()] != needle[..] {
                continue;
            }
            let before_ok = start == 0 || !is_word_char(chars[start - 1]);
            let after = start + needle.len();
            let after_ok = after == chars.len() || !is_word_char(chars[after]);
            if before_ok && after_ok {
                out.push((index + 1, start + 1));
            }
        }
    }
    out
}

/// The keywords that put a *name being declared* immediately after them. `impl`/`type` included:
/// naming a type in those positions is not a call on it either.
pub const DECLARATION_KEYWORDS: &[&str] = &[
    "fn", "struct", "enum", "trait", "union", "type", "const", "static", "mod", "impl",
];

/// Why a mention produced no edge. A hint for triage, never a verdict: `receiver` is the pack's known
/// blind spot, `quoted` is usually a string or a shell argument, and everything else is unexplained.
pub fn cause_of(line_text: &str, column: usize, name_len: usize) -> &'static str {
    let chars: Vec<char> = line_text.chars().collect();
    let at = column.saturating_sub(1);
    let before: String = chars[..at.min(chars.len())].iter().collect();
    // Order matters, and it is ordered by "how much this line can possibly be a call": prose can
    // contain quotes and identifiers, so it is settled before the quote arithmetic is consulted.
    let opener = line_text.trim_start().chars().next().unwrap_or(' ');
    if before.contains("//") || before.contains("/*") || opener == '*' || opener == '#' {
        return "comment";
    }
    let head = line_text.trim_start();
    if head.starts_with("import ")
        || head.starts_with("import{")
        || head.starts_with("use ")
        || head.starts_with("from ")
        || head.starts_with("require(")
    {
        return "import";
    }
    // Both quote styles: the TS/JS/Python that this pack indexes overwhelmingly well uses `'`, and
    // K3's review found `'./alpha'` being reported as a bare mention. A Rust lifetime before the
    // name can then be mislabelled `quoted`, which demotes a row rather than promoting one -- the
    // safe direction for a screen whose only job is to keep real holes near the top.
    if before.matches('"').count() % 2 == 1 || before.matches('\'').count() % 2 == 1 {
        return "quoted";
    }
    // A declaration, not a call. The chunk-based definition test matches the declaration line to a
    // chunk that starts on it, and a trait method *signature* (`fn add(&self, x: i32) -> i32;`) is a
    // `function_signature_item` in this grammar -- never a chunk -- so the line fell through to
    // `call`, which is the second-most-severe cause. Over-reporting, but a wrong label on a row whose
    // whole purpose is telling a reader what kind of thing they are looking at.
    if before
        .split_whitespace()
        .next_back()
        .is_some_and(|token| DECLARATION_KEYWORDS.contains(&token))
    {
        return "definition";
    }
    if matches!(before.chars().next_back(), Some('.') | Some(':')) {
        return "receiver";
    }
    let after = chars
        .iter()
        .skip(at + name_len)
        .find(|c| !c.is_whitespace())
        .copied();
    if after == Some('(') {
        return "call";
    }
    "mention"
}

/// (file, bare target) -> lines the extractor recorded a call on.
fn extracted_calls(
    db: &Db,
    project_id: &str,
) -> Result<HashMap<(String, String), Vec<i64>>, CortError> {
    let mut stmt = db
        .prepare(
            "SELECT file_path, raw_target, start_line FROM raw_edges
          WHERE project_id = ?1 AND rel_type = 'calls'",
        )
        .map_err(|e| CortError::new("storage_busy", json!({ "message": e.to_string() })))?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| CortError::new("storage_busy", json!({ "message": e.to_string() })))?;
    let mut map: HashMap<(String, String), Vec<i64>> = HashMap::new();
    for row in rows.flatten() {
        map.entry((row.0, bare_name(&row.1).to_string()))
            .or_default()
            .push(row.2);
    }
    Ok(map)
}

/// The indexed file set as a lookup, so `blind_files` can tell "on disk, never indexed" from
/// "indexed but produced no chunks". An empty set here would call every source file in the project
/// unindexed -- which is exactly what the first cut of the no-seed path did.
fn index_set_of(db: &Db, project_id: &str) -> Result<HashSet<String>, CortError> {
    Ok(indexed_files(db, project_id)?.into_iter().collect())
}

fn indexed_files(db: &Db, project_id: &str) -> Result<Vec<String>, CortError> {
    let mut stmt = db
        .prepare("SELECT file_path FROM file_state WHERE project_id = ?1 ORDER BY file_path")
        .map_err(|e| CortError::new("storage_busy", json!({ "message": e.to_string() })))?;
    let rows = stmt
        .query_map(params![project_id], |r| r.get::<_, String>(0))
        .map_err(|e| CortError::new("storage_busy", json!({ "message": e.to_string() })))?;
    Ok(rows.flatten().collect())
}

/// L2: the extractor saw a reference to `name` from this file -- a call, or an import the pack
/// extracted -- and no `relationships` row covers it. The two rel_types suppress differently, and
/// the difference is the finding:
///
/// * A **call** is suppressed when the chunk containing that line already has an edge to the seed:
///   that exact call site is in the graph.
/// * An **import** is suppressed at *file* level. A top-level `use` belongs to no function chunk --
///   `source_symbol` is the empty string -- so an import edge can never resolve in today's graph
///   and is dropped before resolution even runs (measured on this repo: 336 import raw edges, 0
///   import relationships). What the graph can still carry is the same dependency arriving through
///   a resolved call from any chunk in the file; when that exists the import drop is a duplicate,
///   and when it does not, the file's dependency on the seed is wholly absent from the graph and
///   this row is its only pack-attested trace. That distinction is what the mention layer cannot
///   make: L1 sees the text of any `use` line, and cannot tell "the pack never extracted this"
///   from "the pack extracted it and resolution discarded it".
fn extracted_but_unresolved(
    db: &Db,
    project_id: &str,
    seed: &SeedRef,
    name: &str,
) -> Result<Vec<Value>, CortError> {
    let err =
        |e: rusqlite::Error| CortError::new("storage_busy", json!({ "message": e.to_string() }));
    let mut stmt = db
        .prepare(
            "SELECT file_path, source_symbol, raw_target, start_line FROM raw_edges
          WHERE project_id = ?1 AND rel_type = 'calls'
            AND (raw_target = ?2 OR raw_target LIKE '%:' || ?3 OR raw_target LIKE '%.' || ?4)
          ORDER BY file_path, start_line",
        )
        .map_err(err)?;
    let candidates = stmt
        .query_map(params![project_id, name, name, name], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(err)?
        .flatten()
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    for (file, source_symbol, raw_target, line) in candidates {
        let mut check = db
            .prepare(
                "SELECT 1 FROM relationships r
               JOIN chunks sc ON sc.chunk_id = r.source_chunk_id
              WHERE r.target_chunk_id = ?1 AND sc.file_path = ?2
                AND sc.start_line <= ?3 AND sc.end_line >= ?3
              LIMIT 1",
            )
            .map_err(err)?;
        let resolved: bool = check
            .query_map(params![seed.chunk_id, file, line], |r| r.get::<_, i64>(0))
            .map_err(err)?
            .next()
            .is_some();
        if !resolved {
            out.push(json!({
                "file_path": file,
                "line": line,
                "from_symbol": source_symbol,
                "raw_target": raw_target,
            }));
        }
    }

    // Imports. SQL `LIKE` cannot open `use crate::foo::{a, b}`, so the leaf match runs in Rust via
    // the same expander the call-narrowing map uses -- one matcher, not two that can drift. A
    // relative module specifier (`./utils`, the JS/TS shape) expands to nothing and honestly
    // matches no symbol name.
    let mut stmt = db
        .prepare(
            "SELECT file_path, source_symbol, raw_target, start_line FROM raw_edges
          WHERE project_id = ?1 AND rel_type = 'imports'
          ORDER BY file_path, start_line",
        )
        .map_err(err)?;
    let imports = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(err)?
        .flatten()
        .collect::<Vec<_>>();
    let mut seen: std::collections::HashSet<(String, i64)> = std::collections::HashSet::new();
    for (file, source_symbol, raw_target, line) in imports {
        let names_it = crate::graph::expand_use_path(&raw_target)
            .iter()
            .any(|segs| segs.last().map(String::as_str) == Some(name));
        if !names_it || !seen.insert((file.clone(), line)) {
            continue;
        }
        // File-level suppression: does any chunk of this file already reach the seed?
        let mut check = db
            .prepare(
                "SELECT 1 FROM relationships r
               JOIN chunks sc ON sc.chunk_id = r.source_chunk_id
              WHERE r.target_chunk_id = ?1 AND sc.file_path = ?2
              LIMIT 1",
            )
            .map_err(err)?;
        let reached: bool = check
            .query_map(params![seed.chunk_id, file], |r| r.get::<_, i64>(0))
            .map_err(err)?
            .next()
            .is_some();
        if !reached {
            out.push(json!({
                "file_path": file,
                "line": line,
                "from_symbol": source_symbol,
                "raw_target": raw_target,
                "via": "import",
            }));
        }
    }
    out.sort_by(|a, b| {
        (
            a["file_path"].as_str().unwrap_or(""),
            a["line"].as_i64().unwrap_or(0),
        )
            .cmp(&(
                b["file_path"].as_str().unwrap_or(""),
                b["line"].as_i64().unwrap_or(0),
            ))
    });
    Ok(out)
}

fn snippet(line: &str) -> String {
    let trimmed = line.trim();
    trimmed.chars().take(120).collect()
}

/// L3: files the graph cannot possibly see through.
fn blind_files(
    db: &Db,
    project_id: &str,
    root: &Path,
    indexed: &HashSet<String>,
) -> Result<Value, CortError> {
    let err =
        |e: rusqlite::Error| CortError::new("storage_busy", json!({ "message": e.to_string() }));
    // Paths, not just a count: "1 file is blind" is not actionable, "rust/src/legacy.rs is blind" is.
    let mut unparsed_stmt = db
        .prepare(
            "SELECT DISTINCT file_path FROM chunks
              WHERE project_id = ?1 AND chunk_source = 'unparsed' ORDER BY file_path",
        )
        .map_err(err)?;
    let unparsed: Vec<String> = unparsed_stmt
        .query_map(params![project_id], |r| r.get::<_, String>(0))
        .map_err(err)?
        .flatten()
        .collect();
    let mut unindexed: Vec<String> = walk_files(root)
        .into_iter()
        .filter(|rel| {
            let path = root.join(rel);
            let is_source = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                SOURCE_EXT
                    .iter()
                    .any(|ext| ext.trim_start_matches('.') == e)
            });
            is_source && !indexed.contains(rel)
        })
        .collect();
    unindexed.sort();
    let hidden: Vec<String> = unindexed.iter().take(MAX_GAP_ROWS).cloned().collect();
    Ok(json!({
        "unparsed": unparsed.len(),
        "unparsed_example": unparsed.iter().take(MAX_GAP_ROWS).cloned().collect::<Vec<_>>(),
        "unindexed": unindexed.len(),
        "unindexed_example": hidden,
        "unindexed_truncated": unindexed.len() > MAX_GAP_ROWS,
    }))
}

pub fn coverage_for(
    db: &Db,
    project_id: &str,
    root: &Path,
    seeds: &[SeedRef],
) -> Result<Value, CortError> {
    let indexed = indexed_files(db, project_id)?;
    let index_set: HashSet<String> = indexed.iter().cloned().collect();
    let extracted = extracted_calls(db, project_id)?;
    // Read each indexed source file once, not once per seed: a batched `--symbol a,b,c` should cost
    // one walk of the tree.
    let mut skipped: Vec<String> = Vec::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    for file in &indexed {
        let path = root.join(file);
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > MAX_SCAN_BYTES {
                continue;
            }
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                sources.insert(file.clone(), text);
            }
            // K3's worst false negative, and the twin of the bug 78fad50 fixed one layer up: an
            // indexed, parsed, perfectly healthy file can be skipped here (over the size cap, or an
            // unreadable path) and its callers were then never looked at -- while the answer still
            // read `incomplete=false`. A file the mention scan could not read is a blind file by
            // definition, so it has to reach the verdict instead of disappearing.
            Err(_) => skipped.push(file.clone()),
        }
    }
    let size_skipped: Vec<String> = indexed
        .iter()
        .filter(|f| {
            let path = root.join(f);
            std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SCAN_BYTES)
        })
        .cloned()
        .collect();
    for f in size_skipped {
        if !skipped.contains(&f) {
            skipped.push(f);
        }
    }
    skipped.sort();
    skipped.dedup();

    let mut blind = blind_files(db, project_id, root, &index_set)?;
    if let Some(map) = blind.as_object_mut() {
        map.insert("scan_skipped".to_string(), json!(skipped.len()));
        map.insert(
            "scan_skipped_files".to_string(),
            json!(skipped
                .iter()
                .take(MAX_GAP_ROWS)
                .cloned()
                .collect::<Vec<_>>()),
        );
    }
    // Two different kinds of blind, and the first cut of this screen conflated them.
    //
    // * `unindexed` / `scan_skipped`: **this screen never read the file**. A caller can be sitting in
    //   it with nothing else to say so, which is the false-safe reading agy found on 2026-08-31.
    //   These have to flip the boolean, in every seed, because the hole is a property of the tree.
    // * `unparsed`: the file has no chunks -- usually a barrel, a types-only module or a
    //   `pub mod`-only `lib.rs`. The *text* of the file **was** read by the mention layer above, so a
    //   caller in it already appears as a row, and its edgelessness is not a missing signal but a
    //   missing graph. Counting it as a gap anyway made 2 files in this repo (4 in cct) flip *every*
    //   seed to `true`, which is a boolean with no discriminating power: agents stop reading the rows
    //   and start reading the always-true flag as noise. So it is advisory now, and still listed --
    //   in `blind_files` with its paths, and in `why` as `unparsed_files`.
    let unread_gap =
        blind_count(blind.get("unindexed")) + blind_count(blind.get("scan_skipped")) > 0;
    let unparsed_advisory = blind_count(blind.get("unparsed")) > 0;
    let mut per_seed = Vec::new();
    for seed in seeds {
        let name = bare_name(&seed.symbol).to_string();
        let mut gap_rows_sorted: Vec<(u8, u8, String, usize, &'static str, u32, String)> =
            Vec::new();
        let mut orphan_files: Vec<String> = Vec::new();
        let mut artifact_files = 0usize;
        let defs = definition_lines(db, project_id, &name)?;
        let mut mention_count = 0usize;
        let mut covered_count = 0usize;
        let mut by_cause: HashMap<&'static str, usize> = HashMap::new();
        for file in &indexed {
            let Some(text) = sources.get(file) else {
                continue;
            };
            let edge_lines = extracted
                .get(&(file.clone(), name.clone()))
                .cloned()
                .unwrap_or_default();
            let mut file_rows = Vec::new();
            let mut file_covered = false;
            let first_line_chars = text.lines().next().map_or(0, |l| l.chars().count());
            let generated = looks_generated(file, first_line_chars);
            for (line, column) in mentions(text, &name) {
                if file == &seed.file_path && (line as i64) == seed.start_line {
                    continue; // the definition itself is not a caller
                }
                mention_count += 1;
                let source_line = text.lines().nth(line - 1).unwrap_or("");
                let cause = if generated {
                    "artifact"
                } else if defs.contains(&(file.clone(), line as i64)) {
                    "definition"
                } else {
                    cause_of(source_line, column, name.chars().count())
                };
                // A quoted mention can never become an edge -- the extractor does not read strings --
                // so it is exempt from the line tolerance. Counting it as covered because a real call
                // happens to sit two lines away is exactly the swallowing this screen exists to avoid.
                let covered = cause != "quoted"
                    && edge_lines
                        .iter()
                        .any(|edge| (edge - line as i64).abs() <= LINE_TOLERANCE);
                if covered {
                    covered_count += 1;
                    file_covered = true;
                    continue;
                }
                *by_cause.entry(cause).or_insert(0) += 1;
                file_rows.push((line, cause, snippet(source_line)));
            }
            // The strongest signal here is file-level: a file that mentions the seed and has *no*
            // covered mention anywhere is a caller the enumeration never saw. A file whose calls are
            // already indexed contributes only import/prose noise, however many lines it has.
            if !file_rows.is_empty() {
                if generated {
                    // Counted only when it actually produced a gap row: "a bundle that also has a
                    // covered call" is not a hole in anything.
                    artifact_files += 1;
                }
                // A file whose only mention is its own declaration of a same-named symbol is not a
                // caller it missed -- name resolution just has two candidates. Counting those made the
                // orphan list cry wolf on every duplicated name.
                let meaningful = file_rows.iter().any(|(_, cause, _)| *cause != "definition");
                if !file_covered && !generated && meaningful {
                    orphan_files.push(file.clone());
                }
                // Two mentions on one line are one row with a count: K3's review saw the same
                // `export { alpha as beta } from './alpha';` printed twice, and a duplicated row
                // makes a one-line file look like a two-line hole.
                let mut per_line: std::collections::BTreeMap<(usize, &'static str), (u32, String)> =
                    std::collections::BTreeMap::new();
                for (line, cause, text_snippet) in file_rows {
                    per_line
                        .entry((line, cause))
                        .and_modify(|(n, _)| *n += 1)
                        .or_insert((1, text_snippet));
                }
                for ((line, cause), (occurrences, text_snippet)) in per_line {
                    // Cause first, file-level coverage second. The reverse order -- which this had --
                    // demotes the most severe row in a partially-covered file below comment noise from
                    // an unrelated file, and with `MAX_GAP_ROWS` truncating the list that pushed a real
                    // `receiver` hole off the printed page while `gap_count` still said it existed. The
                    // independent review (K3, 2026-09-01) found it; the boolean was never the thing at
                    // risk here, the *rows* were, and "read the rows" is only true if the rows are ordered
                    // by what they are.
                    gap_rows_sorted.push((
                        cause_rank(cause),
                        u8::from(file_covered),
                        file.clone(),
                        line,
                        cause,
                        occurrences,
                        text_snippet,
                    ));
                }
            }
        }
        gap_rows_sorted.sort();
        let gaps: Vec<Value> = gap_rows_sorted
            .iter()
            .take(MAX_GAP_ROWS)
            .map(|(_, _, file, line, cause, occurrences, text)| {
                json!({
                    "file_path": file,
                    "line": line,
                    "cause": cause,
                    "occurrences": occurrences,
                    "text": text,
                })
            })
            .collect();
        let unresolved = extracted_but_unresolved(db, project_id, seed, &name)?;
        let mut causes: Vec<(&str, usize)> = by_cause.into_iter().collect();
        causes.sort_by(|a, b| a.0.cmp(b.0));
        // Rows, not mentions: with duplicates folded, the mention layer's contribution to `gap_count`
        // counts distinct (file, line, cause) findings.
        let gap_rows = gap_rows_sorted.len() + unresolved.len();
        let mut reasons: Vec<&str> = Vec::new();
        if !gap_rows_sorted.is_empty() {
            reasons.push("mentions_without_edge");
        }
        if !unresolved.is_empty() {
            reasons.push("extracted_but_unresolved");
        }
        if unread_gap {
            reasons.push("blind_files");
        }
        if !skipped.is_empty() {
            reasons.push("scan_skipped");
        }
        // Advisory, and deliberately last: it says "these files contribute no edges to the graph",
        // not "a caller may be hiding here unread". It does not affect the boolean below.
        if unparsed_advisory {
            reasons.push("unparsed_files");
        }
        per_seed.push(json!({
            "symbol": seed.symbol,
            "chunk_id": seed.chunk_id,
            "mentions_on_disk": mention_count,
            "mentions_covered_by_edge": covered_count,
            "mentions_truncated": gap_rows_sorted.len() > MAX_GAP_ROWS,
            // The number the boolean reads: both layers. Until 2026-09-01 this was the mention
            // layer's count alone, so a seed whose entire signal was a dropped resolution published
            // `gap_count: 0` beside `enumeration_may_be_incomplete: true` -- a count that says
            // "nothing" under a flag that says "something" is the false-safe shape this screen
            // exists to prevent. The mention layer's own count stays published beside it, because
            // the truncation math (`mentions_without_edge` is capped) needs the uncapped L1 figure
            // and must not be asked to subtract its way to it.
            "mention_gap_count": gap_rows_sorted.len(),
            "gap_count": gap_rows,
            "files_with_no_edge_at_all": orphan_files,
            "generated_files_with_gaps": artifact_files,
            "gap_cause_totals": Value::Object(
                causes
                    .iter()
                    .map(|(k, v)| (k.to_string(), json!(v)))
                    .collect(),
            ),
            "mentions_without_edge": gaps,
            "extracted_but_unresolved": unresolved,
            "enumeration_may_be_incomplete": gap_rows > 0 || unread_gap,
            "why": reasons,
        }));
    }

    Ok(json!({
        "method": COVERAGE_METHOD,
        "seeds": per_seed,
        "blind_files": blind,
        "reading": "Read the rows, not the boolean. `enumeration_may_be_incomplete: true` means only \
                    that something specific was found -- a named gap row, or a file this screen never \
                    read -- and `false` means only that every file it did read produced no gap signal \
                    for that seed; neither is a proof of anything. `why` names which case fired, and \
                    `unparsed_files` in `why` is advisory (a chunk-less file is still text-scanned, so \
                    its callers appear as rows) and does not flip the boolean. \
                    mentions_without_edge and extracted_but_unresolved are candidates, not proof: a \
                    mention may be a comment, a string, or a same-named symbol elsewhere. They exist \
                    because dependents=0 is otherwise indistinguishable from an enumeration that never \
                    saw the caller. Silent either way, and outside all three layers: a caller in a file \
                    the indexer does not read at all (.sh, .txt, config, or anything under dist/, \
                    build/, target/, node_modules/), a file over 2 MB that the mention scan skips (that \
                    one does flip the boolean, as scan_skipped), a re-export chain reported at its \
                    barrel line, a mention within 2 lines of an extracted call (counted covered), and a \
                    call that only exists after macro expansion -- nothing on disk names the callee, so \
                    no layer of this screen can see it (`cargo check` is the tool that can), and `false` \
                    there is not clearance either.",
    }))
}
