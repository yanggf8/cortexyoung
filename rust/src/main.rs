//! CLI dispatch. JS `bin/cort.js` with clap; `--help`/`-h`/`help` are side-effect free.

use clap::Parser;
use cort::ast_grep::{assert_ast_grep_version, resolve_ast_grep_bin};
use cort::context::{context_command, ContextOptions, DEFAULT_BUDGET};
use cort::db::{
    db_path_for, delete_project, ensure_schema, list_projects, open_db, with_busy_retry, Db,
    SqliteErrorCode, WithBusyRetryError,
};
use cort::errors::CortError;
use cort::impact::{impact_command, DEFAULT_DEPTH};
use cort::incremental::incremental_index;
use cort::indexer::{canonicalize_root, full_index, status_of, CanonicalRoot, IndexError};
use cort::r#struct::{struct_command, StructOptions};
use cort::readings::{parse_content_mode, read_fragment, recall_readings};
use cort::render::{parse_format, render, render_error, Format};
use cort::staleness::compute_stale;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const KNOWN_COMMANDS: [&str; 9] = [
    "index", "status", "projects", "delete", "struct", "context", "impact", "read", "recall",
];

fn usage_value() -> Value {
    json!({
        "usage": "cort <command> [options]",
        "commands": {
            "index": "cort index [root] [--incremental]",
            "status": "cort status [root]",
            "projects": "cort projects",
            "delete": "cort delete [root]",
            "struct": "cort struct -p '<pattern>' --lang <lang> [-g <glob>] [--budget <n>] [-f json|lean]",
            "context": "cort context <symbol|query> [--budget <n>] [--include-ambiguous] [--content full] [-f json|lean]",
            "impact": "cort impact --symbol <name> [--depth <n>] [-f json|lean]",
            "read": "cort read <file> [--start <line>] [--end <line>] [-f json|lean]",
            "recall": "cort recall <query> [--limit <n>] [--content full] [-f json|lean]",
        },
        "env": {
            "CORT_CACHE_DIR": "where indexes live (default ~/.cache/cortex-ng)",
        },
        "note": "Commands read the project at the cwd unless they take a root argument.",
    })
}

fn wants_help(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "help" || a == "--help" || a == "-h")
}

fn map_index(err: IndexError) -> CortError {
    match err {
        IndexError::Cort(c) => c,
        IndexError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            CortError::new("file_not_found", json!({ "message": io.to_string() }))
        }
        IndexError::Io(io) => CortError::new("file_not_found", json!({ "message": io.to_string() })),
        IndexError::Sqlite(e) => CortError::new("storage_busy", json!({ "message": e.to_string() })),
    }
}

struct IdxWrap(IndexError);

impl std::fmt::Display for IdxWrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl SqliteErrorCode for IdxWrap {
    fn sqlite_code(&self) -> Option<&str> {
        match &self.0 {
            IndexError::Sqlite(e) => e.sqlite_code(),
            _ => None,
        }
    }
}

struct CortWrap(CortError);

impl std::fmt::Display for CortWrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl SqliteErrorCode for CortWrap {
    fn sqlite_code(&self) -> Option<&str> {
        None
    }
}

fn unwrap_busy<T, E: SqliteErrorCode + std::fmt::Display>(
    r: Result<T, WithBusyRetryError<E>>,
    map_other: impl FnOnce(E) -> CortError,
) -> Result<T, CortError> {
    match r {
        Ok(v) => Ok(v),
        Err(WithBusyRetryError::Cort(c)) => Err(c),
        Err(WithBusyRetryError::Other(e)) => Err(map_other(e)),
    }
}

fn open_project(root: &Path) -> Result<(CanonicalRoot, Db), CortError> {
    let canon = canonicalize_root(root).map_err(map_index)?;
    let db = open_db(db_path_for(&canon.path_str)).map_err(|e| {
        CortError::new("storage_busy", json!({ "message": e.to_string() }))
    })?;
    ensure_schema(&db)?;
    Ok((canon, db))
}

fn resolve_fmt(raw: Option<&str>) -> Result<Format, CortError> {
    parse_format(raw).ok_or_else(|| {
        CortError::new("unknown_format", json!({ "hint": "--format json|lean" }))
    })
}

fn parse_usize_flag(raw: Option<&str>, default: usize) -> usize {
    raw.and_then(|s| s.parse::<f64>().ok())
        .map(|n| n as usize)
        .unwrap_or(default)
}

fn parse_i64_flag(raw: Option<&str>, default: i64) -> i64 {
    raw.and_then(|s| s.parse::<f64>().ok())
        .map(|n| n as i64)
        .unwrap_or(default)
}

fn parse_line_flag(raw: Option<&str>, name: &str) -> Result<Option<i64>, CortError> {
    match raw {
        None => Ok(None),
        Some(s) => s.parse::<i64>().map(Some).map_err(|_| {
            let mut map = serde_json::Map::new();
            map.insert(name.to_string(), json!(s));
            CortError::new("invalid_line_range", Value::Object(map))
        }),
    }
}

fn clap_fail(err: clap::Error) -> CortError {
    CortError::new(
        "unknown_command",
        json!({
            "command": Value::Null,
            "known": KNOWN_COMMANDS,
            "message": err.to_string(),
        }),
    )
}

fn emit(command: Option<&str>, format: Format, payload: &Value) {
    print!("{}", render(command, format, payload));
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct IndexArgs {
    root: Option<PathBuf>,
    #[arg(long)]
    incremental: bool,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct RootArgs {
    root: Option<PathBuf>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct FormatOnlyArgs {
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct StructArgs {
    #[arg(short = 'p', long = "pattern")]
    pattern: Option<String>,
    #[arg(long)]
    lang: Option<String>,
    #[arg(short = 'g')]
    g: Option<String>,
    #[arg(long)]
    budget: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct ContextArgs {
    query: Option<String>,
    #[arg(long)]
    budget: Option<String>,
    #[arg(long = "include-ambiguous")]
    include_ambiguous: bool,
    #[arg(long = "content", num_args = 0..=1, default_missing_value = "")]
    content: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct ImpactArgs {
    #[arg(long)]
    symbol: Option<String>,
    #[arg(long)]
    depth: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct ReadArgs {
    file: Option<String>,
    #[arg(long)]
    start: Option<String>,
    #[arg(long)]
    end: Option<String>,
    #[arg(long = "content", num_args = 0..=1, default_missing_value = "")]
    content: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(no_binary_name = true, disable_help_flag = true, disable_version_flag = true)]
struct RecallArgs {
    query: Option<String>,
    #[arg(long)]
    limit: Option<String>,
    #[arg(long = "content", num_args = 0..=1, default_missing_value = "")]
    content: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

fn pin_bin() -> Result<String, CortError> {
    let bin = resolve_ast_grep_bin()?;
    assert_ast_grep_version(&bin)?;
    Ok(bin)
}

fn dispatch(args: &[String]) -> Result<(), CortError> {
    let command = args.first().map(String::as_str);
    match command {
        Some("index") => cmd_index(&args[1..]),
        Some("status") => cmd_status(&args[1..]),
        Some("projects") => cmd_projects(&args[1..]),
        Some("delete") => cmd_delete(&args[1..]),
        Some("struct") => cmd_struct(&args[1..]),
        Some("context") => cmd_context(&args[1..]),
        Some("impact") => cmd_impact(&args[1..]),
        Some("read") => cmd_read(&args[1..]),
        Some("recall") => cmd_recall(&args[1..]),
        other => Err(CortError::new(
            "unknown_command",
            json!({
                "command": other,
                "known": KNOWN_COMMANDS,
            }),
        )),
    }
}

fn cmd_index(args: &[String]) -> Result<(), CortError> {
    let a = IndexArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let bin = pin_bin()?;
    let root = a.root.unwrap_or_else(cwd);
    let (canon, mut db) = open_project(&root)?;
    let stats = unwrap_busy(
        with_busy_retry(|| {
            if a.incremental {
                incremental_index(&mut db, &bin, &canon.path).map_err(IdxWrap)
            } else {
                full_index(&mut db, &bin, &canon.path)
                    .map(|s| cort::incremental::IncrementalIndexResult {
                        mode: "full".into(),
                        files: s.files,
                        chunks: s.chunks,
                        unparsed: s.unparsed,
                        files_examined: 0,
                        files_reindexed: 0,
                        files_skipped: 0,
                        files_removed: 0,
                        relationships: s.relationships,
                        elapsed_ms: s.elapsed_ms,
                    })
                    .map_err(IdxWrap)
            }
        }),
        |w| map_index(w.0),
    )?;
    let payload = if a.incremental && stats.mode == "incremental" {
        json!({
            "mode": "incremental",
            "files_examined": stats.files_examined,
            "files_reindexed": stats.files_reindexed,
            "files_skipped": stats.files_skipped,
            "files_removed": stats.files_removed,
            "relationships": stats.relationships,
            "elapsed_ms": stats.elapsed_ms,
        })
    } else if a.incremental {
        json!({
            "mode": "full",
            "files": stats.files,
            "chunks": stats.chunks,
            "unparsed": stats.unparsed,
            "relationships": stats.relationships,
            "elapsed_ms": stats.elapsed_ms,
        })
    } else {
        json!({
            "files": stats.files,
            "chunks": stats.chunks,
            "unparsed": stats.unparsed,
            "relationships": stats.relationships,
            "elapsed_ms": stats.elapsed_ms,
        })
    };
    emit(None, Format::Json, &payload);
    Ok(())
}

fn cmd_status(args: &[String]) -> Result<(), CortError> {
    let a = RootArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let root = a.root.unwrap_or_else(cwd);
    let (canon, db) = open_project(&root)?;
    let st = status_of(&db, &canon.path).map_err(map_index)?;
    if !st.indexed {
        emit(
            None,
            Format::Json,
            &json!({
                "project_id": st.project_id,
                "path": st.path,
                "indexed": false,
            }),
        );
        return Ok(());
    }
    let bin = pin_bin()?;
    let stale = compute_stale(&db, &bin, &canon.path, &st.project_id).map_err(map_index)?;
    emit(
        None,
        Format::Json,
        &json!({
            "project_id": st.project_id,
            "path": st.path,
            "indexed": true,
            "files": st.files,
            "chunks": st.chunks,
            "readings": st.readings,
            "relationships": st.relationships,
            "extractor_version": st.extractor_version,
            "git_head": st.git_head,
            "last_indexed_at": st.last_indexed_at,
            "index_is_stale": stale.index_is_stale,
            "deleted_files": stale.deleted_files,
            "changed_files": stale.changed_files,
        }),
    );
    Ok(())
}

fn cmd_projects(args: &[String]) -> Result<(), CortError> {
    let _a = FormatOnlyArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let rows: Vec<Value> = list_projects()
        .into_iter()
        .map(|r| {
            json!({
                "project_id": r.project_id,
                "name": r.name,
                "path": r.path,
                "git_head": r.git_head,
                "last_indexed_at": r.last_indexed_at,
                "db_path": r.db_path,
            })
        })
        .collect();
    emit(None, Format::Json, &Value::Array(rows));
    Ok(())
}

fn cmd_delete(args: &[String]) -> Result<(), CortError> {
    let a = RootArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let root = a.root.unwrap_or_else(cwd);
    let canon = canonicalize_root(&root).map_err(map_index)?;
    let r = delete_project(&canon.path_str);
    emit(
        None,
        Format::Json,
        &json!({ "deleted": r.deleted, "db_path": r.db_path }),
    );
    Ok(())
}

fn cmd_struct(args: &[String]) -> Result<(), CortError> {
    let a = StructArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let pattern = a.pattern.filter(|s| !s.is_empty()).ok_or_else(|| {
        CortError::new(
            "missing_pattern",
            json!({ "hint": "cort struct -p '<pattern>' --lang ts" }),
        )
    })?;
    let lang = a.lang.filter(|s| !s.is_empty()).ok_or_else(|| {
        CortError::new(
            "missing_lang",
            json!({ "hint": "pre-flight pattern validation requires --lang" }),
        )
    })?;
    let format = resolve_fmt(a.format.as_deref())?;
    let bin = pin_bin()?;
    let (canon, db) = open_project(&cwd())?;
    let globs = match a.g {
        Some(g) => vec![g],
        None => Vec::new(),
    };
    let budget = parse_usize_flag(a.budget.as_deref(), DEFAULT_BUDGET);
    let out = struct_command(
        &db,
        &bin,
        &canon.path,
        &canon.project_id,
        &pattern,
        &lang,
        StructOptions { globs, budget, file_limit: None },
    )?;
    emit(Some("struct"), format, &out);
    Ok(())
}

fn cmd_context(args: &[String]) -> Result<(), CortError> {
    let a = ContextArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let query = a.query.ok_or_else(|| {
        CortError::new(
            "missing_query",
            json!({ "hint": "cort context <symbol|query>" }),
        )
    })?;
    let format = resolve_fmt(a.format.as_deref())?;
    let bin = pin_bin()?;
    let (canon, db) = open_project(&cwd())?;
    let budget = parse_usize_flag(a.budget.as_deref(), DEFAULT_BUDGET);
    let full_content = a.content.as_deref() == Some("full");
    let out = context_command(
        &db,
        &bin,
        &canon.path,
        &canon.project_id,
        &query,
        ContextOptions { budget, include_ambiguous: a.include_ambiguous, full_content },
    )?;
    emit(Some("context"), format, &out);
    Ok(())
}

fn cmd_impact(args: &[String]) -> Result<(), CortError> {
    let a = ImpactArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let symbol = a.symbol.filter(|s| !s.is_empty()).ok_or_else(|| {
        CortError::new(
            "missing_symbol",
            json!({ "hint": "cort impact --symbol <name>" }),
        )
    })?;
    let format = resolve_fmt(a.format.as_deref())?;
    let bin = pin_bin()?;
    let (canon, db) = open_project(&cwd())?;
    let depth = parse_i64_flag(a.depth.as_deref(), DEFAULT_DEPTH);
    let out = impact_command(&db, &bin, &canon.path, &canon.project_id, &symbol, depth)?;
    emit(Some("impact"), format, &out);
    Ok(())
}

fn cmd_read(args: &[String]) -> Result<(), CortError> {
    let a = ReadArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let format = resolve_fmt(a.format.as_deref())?;
    let (canon, db) = open_project(&cwd())?;
    let file = a.file.unwrap_or_default();
    let start = parse_line_flag(a.start.as_deref(), "start")?;
    let end = parse_line_flag(a.end.as_deref(), "end")?;
    let mode = match a.content.as_deref() {
        None => parse_content_mode(None)?,
        Some(s) => parse_content_mode(Some(s))?,
    };
    let payload = unwrap_busy(
        with_busy_retry(|| {
            read_fragment(
                &db,
                &canon.path,
                &canon.project_id,
                &file,
                start,
                end,
                mode,
            )
            .map_err(CortWrap)
        }),
        |w| w.0,
    )?;
    let value = serde_json::to_value(&payload).unwrap_or(Value::Null);
    emit(Some("read"), format, &value);
    Ok(())
}

fn cmd_recall(args: &[String]) -> Result<(), CortError> {
    let a = RecallArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let query = a.query.ok_or_else(|| {
        CortError::new(
            "missing_query",
            json!({ "hint": "cort recall <query>" }),
        )
    })?;
    let format = resolve_fmt(a.format.as_deref())?;
    let (canon, db) = open_project(&cwd())?;
    let limit = a
        .limit
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok());
    let full_content = a.content.as_deref() == Some("full");
    let payload = unwrap_busy(
        with_busy_retry(|| {
            recall_readings(
                &db,
                &canon.path,
                &canon.project_id,
                &query,
                limit,
                full_content,
            )
            .map_err(CortWrap)
        }),
        |w| w.0,
    )?;
    let value = serde_json::to_value(&payload).unwrap_or(Value::Null);
    emit(Some("recall"), format, &value);
    Ok(())
}

fn peek_format(args: &[String]) -> Format {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-f" || args[i] == "--format" {
            if let Some(v) = args.get(i + 1) {
                if let Some(fmt) = parse_format(Some(v)) {
                    return fmt;
                }
            }
        } else if let Some(rest) = args[i].strip_prefix("-f=") {
            if let Some(fmt) = parse_format(Some(rest)) {
                return fmt;
            }
        } else if let Some(rest) = args[i].strip_prefix("--format=") {
            if let Some(fmt) = parse_format(Some(rest)) {
                return fmt;
            }
        }
        i += 1;
    }
    Format::Json
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if wants_help(&args) {
        emit(None, Format::Json, &usage_value());
        return;
    }
    match dispatch(&args) {
        Ok(()) => {}
        Err(err) => {
            let format = peek_format(&args);
            print!("{}", render_error(format, &err));
            std::process::exit(1);
        }
    }
}
