//! Structural search + containment join.

use crate::ast_grep::{exec_ast_grep, ExecOpts};
use crate::budget::apply_budget;
use crate::chunker::{parse_scan_stream, ScanStream};
use crate::db::Db;
use crate::errors::CortError;
use crate::graph::{containment_join as graph_containment_join, get_neighbors, ContainingChunk};
use crate::indexer::IndexError;
use crate::scan;
use crate::staleness::compute_stale;
use rusqlite::params;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;

pub const MAX_MALFORMED_RATIO: f64 = 0.10;
pub const MAX_NEIGHBORS: i64 = 3;
pub const UNBOUNDED_SCAN_FILE_LIMIT: i64 = 2000;

const ERROR_NODE_MARKER: &str = "Pattern contains an ERROR node";

fn map_index(err: IndexError) -> CortError {
    match err {
        IndexError::Cort(c) => c,
        IndexError::Sqlite(e) => crate::db::classify_sqlite(&e),
        other => CortError::new("storage_busy", json!({ "message": other.to_string() })),
    }
}

fn map_sql(err: rusqlite::Error) -> CortError {
    crate::db::classify_sqlite(&err)
}

fn exec_args(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

pub fn preflight_pattern(
    bin: &str,
    pattern: &str,
    lang: &str,
    paths: &[String],
) -> Result<(), CortError> {
    // The crate backend validates the pattern itself -- parse failure and ERROR-node detection
    // happen against the pattern text, so `paths` never mattered here; the CLI leg keeps them for
    // its run. See scan.rs for why each in-process check mirrors the CLI's own behavior.
    if scan::scan_backend()? == scan::Backend::Crate {
        return scan::preflight_pattern_in_process(pattern, lang);
    }
    let mut args = vec![
        "run".into(),
        "--debug-query=ast".into(),
        "--lang".into(),
        lang.to_string(),
        "-p".into(),
        pattern.to_string(),
    ];
    args.extend(paths.iter().cloned());
    let r = exec_ast_grep(bin, &exec_args(&args), ExecOpts::default())?;
    if r.code == 2 || r.stderr.contains(ERROR_NODE_MARKER) {
        return Err(CortError::new(
            "parse_failed",
            json!({
                "pattern": pattern,
                "lang": lang,
                "detail": r.stderr.trim(),
            }),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct PatternMatch {
    pub file: String,
    pub text: String,
    pub start_line: i64,
    pub end_line: i64,
    #[allow(dead_code)]
    pub replacement: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct RunPatternResult {
    pub matches: Vec<PatternMatch>,
    pub malformed: usize,
    pub total: usize,
}

pub fn run_pattern(
    bin: &str,
    pattern: &str,
    lang: &str,
    root: &str,
    globs: &[String],
    rewrite: Option<&str>,
    skip_preflight: bool,
) -> Result<RunPatternResult, CortError> {
    if !skip_preflight {
        preflight_pattern(bin, pattern, lang, &[root.to_string()])?;
    }
    let parsed = match scan::scan_backend()? {
        scan::Backend::Crate => {
            if rewrite.is_some() {
                return Err(CortError::new(
                    "rewrite_requires_cli_backend",
                    json!({ "hint": "CORT_SCAN_BACKEND=cli runs `ast-grep run --rewrite`" }),
                ));
            }
            let key = scan::pattern_lang(lang).ok_or_else(|| {
                CortError::new(
                    "unknown_lang",
                    json!({ "lang": lang, "supported": ["rust", "ts", "tsx", "js", "jsx", "py"] }),
                )
            })?;
            let root_path = Path::new(root);
            let files = scan::search_files(root_path, globs, key)?;
            ScanStream {
                total: 0,
                malformed: 0,
                records: scan::run_pattern_in_process(pattern, lang, key, &files)?,
            }
        }
        scan::Backend::Cli => {
            let mut args = vec![
                "run".into(),
                "--json=stream".into(),
                "--strictness".into(),
                "ast".into(),
                "--lang".into(),
                lang.to_string(),
                "-p".into(),
                pattern.to_string(),
            ];
            if let Some(rw) = rewrite {
                args.push("--rewrite".into());
                args.push(rw.to_string());
            }
            // Globs go to the CLI's own `--globs` flag, matched relative to the search root --
            // passing them positionally sent the glob string to the OS as a literal path and
            // answered `ast_grep_run_failed: No such file or directory` for every `-g` query
            // (measured 2026-09-01: the README's own `-g` hint never worked).
            for g in globs {
                // Same root-relative normalization the crate path does: a glob formed from an
                // absolute path must match the CLI's walk-relative glob space.
                let raw = g.as_str();
                let negated = raw.starts_with('!');
                let body = raw.strip_prefix('!').unwrap_or(raw);
                let rel = std::path::Path::new(body)
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| body.to_string());
                args.push("--globs".into());
                args.push(if negated { format!("!{rel}") } else { rel });
            }
            args.push(root.to_string());
            // The glob base is the subprocess's cwd: pin it to the search root so `-g 'src/**'`
            // means the same thing regardless of where the user invoked cort from, and so the CLI
            // and crate backends match against the same relative space.
            let r = exec_ast_grep(
                bin,
                &exec_args(&args),
                ExecOpts {
                    timeout_ms: None,
                    cwd: Some(root.to_string().into()),
                },
            )?;
            if r.code != 0 && r.stdout.is_empty() && !r.stderr.trim().is_empty() {
                return Err(CortError::new(
                    "ast_grep_run_failed",
                    json!({ "code": r.code, "detail": r.stderr.trim() }),
                ));
            }
            parse_scan_stream(&r.stdout)
        }
    };
    if parsed.total > 0 && (parsed.malformed as f64) / (parsed.total as f64) > MAX_MALFORMED_RATIO {
        return Err(CortError::new(
            "run_aborted_malformed",
            json!({
                "malformed": parsed.malformed,
                "total": parsed.total,
                "ratio": MAX_MALFORMED_RATIO,
            }),
        ));
    }
    let matches = parsed
        .records
        .iter()
        .filter_map(|rec| {
            let file = rec.get("file").and_then(Value::as_str)?.to_string();
            let text = rec
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let start_line = rec
                .get("range")
                .and_then(|r| r.get("start"))
                .and_then(|p| p.get("line"))
                .and_then(Value::as_i64)
                .or_else(|| {
                    rec.get("range")
                        .and_then(|r| r.get("start"))
                        .and_then(|p| p.get("line"))
                        .and_then(Value::as_u64)
                        .and_then(|u| i64::try_from(u).ok())
                })?;
            let end_line = rec
                .get("range")
                .and_then(|r| r.get("end"))
                .and_then(|p| p.get("line"))
                .and_then(Value::as_i64)
                .or_else(|| {
                    rec.get("range")
                        .and_then(|r| r.get("end"))
                        .and_then(|p| p.get("line"))
                        .and_then(Value::as_u64)
                        .and_then(|u| i64::try_from(u).ok())
                })?;
            Some(PatternMatch {
                file,
                text,
                start_line: start_line + 1,
                end_line: end_line + 1,
                replacement: rec.get("replacement").cloned(),
            })
        })
        .collect();
    Ok(RunPatternResult {
        matches,
        malformed: parsed.malformed,
        total: parsed.total,
    })
}

pub fn containment_join(
    db: &Db,
    project_id: &str,
    file_path: &str,
    start_line: i64,
    end_line: i64,
) -> Result<Option<ContainingChunk>, CortError> {
    graph_containment_join(db, project_id, file_path, start_line, end_line).map_err(map_sql)
}

#[derive(Clone, Serialize)]
struct NeighborOut {
    chunk_id: String,
    symbol_name: Option<String>,
    file_path: String,
    start_line: i64,
    end_line: i64,
    rel_type: String,
    confidence: String,
    confidence_score: f64,
    direction: String,
}

#[derive(Clone, Serialize)]
struct StructMatchOut {
    file_path: String,
    start_line: i64,
    end_line: i64,
    text: String,
    chunk_id: Option<String>,
    symbol_name: Option<String>,
    chunk_type: Option<String>,
    neighbors: Vec<NeighborOut>,
}

fn rel_path(file: &str, root: &str) -> String {
    if file.starts_with(root) && file.len() > root.len() {
        file[root.len() + 1..].replace('\\', "/")
    } else {
        file.to_string()
    }
}

/// Same parity-vs-clippy trade as `ContextOptions`: query-shaping knobs bundled.
#[derive(Debug, Clone)]
pub struct StructOptions {
    pub globs: Vec<String>,
    pub budget: usize,
    pub file_limit: Option<i64>,
}

pub fn struct_command(
    db: &Db,
    bin: &str,
    root: impl AsRef<Path>,
    project_id: &str,
    pattern: &str,
    lang: &str,
    opts: StructOptions,
) -> Result<Value, CortError> {
    let StructOptions {
        globs,
        budget,
        file_limit,
    } = opts;
    let globs = &globs;
    let root = root.as_ref();
    let root_str = root.to_str().unwrap_or("");
    let file_limit = file_limit.unwrap_or(UNBOUNDED_SCAN_FILE_LIMIT);
    if globs.is_empty() {
        let indexed_files: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM file_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .map_err(map_sql)?;
        if indexed_files > file_limit {
            return Err(CortError::new(
                "scan_too_broad",
                json!({
                    "indexed_files": indexed_files,
                    "limit": file_limit,
                    "hint": "narrow the scan with -g '<glob>', e.g. cort struct -p '<pattern>' --lang ts -g 'src/**/*.ts'",
                }),
            ));
        }
    }
    let run = run_pattern(bin, pattern, lang, root_str, globs, None, false)?;
    let mut enriched = Vec::new();
    for m in run.matches {
        let file_path = rel_path(&m.file, root_str);
        let chunk = containment_join(db, project_id, &file_path, m.start_line, m.end_line)?;
        let neighbors = if let Some(ch) = &chunk {
            get_neighbors(db, &ch.chunk_id, MAX_NEIGHBORS)
                .map_err(map_sql)?
                .into_iter()
                .filter(|n| n.confidence == "EXTRACTED" || n.confidence == "INFERRED")
                .take(MAX_NEIGHBORS as usize)
                .map(|n| NeighborOut {
                    chunk_id: n.chunk_id,
                    symbol_name: n.symbol_name,
                    file_path: n.file_path,
                    start_line: n.start_line,
                    end_line: n.end_line,
                    rel_type: n.rel_type,
                    confidence: n.confidence,
                    confidence_score: n.confidence_score,
                    direction: n.direction,
                })
                .collect()
        } else {
            Vec::new()
        };
        enriched.push(StructMatchOut {
            file_path,
            start_line: m.start_line,
            end_line: m.end_line,
            text: m.text,
            chunk_id: chunk.as_ref().map(|c| c.chunk_id.clone()),
            symbol_name: chunk.as_ref().and_then(|c| c.symbol_name.clone()),
            chunk_type: chunk.as_ref().and_then(|c| c.chunk_type.clone()),
            neighbors,
        });
    }
    let match_count = enriched.len();
    let budgeted = apply_budget(enriched, budget, |m| {
        serde_json::to_string(m).unwrap_or_default()
    });
    let stale = compute_stale(db, bin, root, project_id).map_err(map_index)?;
    Ok(json!({
        "pattern": pattern,
        "lang": lang,
        "matches": budgeted.kept,
        "match_count": match_count,
        "malformed_lines": run.malformed,
        "truncated": budgeted.truncated,
        "index_is_stale": stale.index_is_stale,
    }))
}
