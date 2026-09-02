//! CLI dispatch with clap; `--help`/`-h`/`help` are side-effect free.

use clap::Parser;
use cort::ast_grep::{assert_ast_grep_version, resolve_ast_grep_bin};
use cort::context::{context_command, ContextOptions, DEFAULT_BUDGET};
use cort::coverage;
use cort::db::{
    db_path_for, delete_project, ensure_schema, list_projects, open_db, with_busy_retry, Db,
    SqliteErrorCode, WithBusyRetryError,
};
use cort::errors::CortError;
use cort::impact::{impact_command, DEFAULT_DEPTH};
use cort::incremental::incremental_index;
use cort::indexer::{canonicalize_root, full_index, status_of, CanonicalRoot, IndexError};
use cort::r#struct::{struct_command, StructOptions};
use cort::readings::{parse_content_mode, read_fragment, recall_readings, ContentMode};
use cort::render::{parse_format, render, render_error, Format};
use cort::settings;
use cort::staleness::compute_stale;
use cort::usage::{self, CommandRecord};
use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// `hook-suggest` is here so its rows are recorded under their own name rather than `_unknown`:
// whether the harness hook fires, and whether a fire is followed by an `impact` call, is the one
// measurement that has a numerator. It is not a verb anyone types.
const KNOWN_COMMANDS: [&str; 12] = [
    "index",
    "status",
    "projects",
    "delete",
    "struct",
    "context",
    "impact",
    "read",
    "recall",
    "usage",
    "hook-suggest",
    "hook-install",
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
            "impact": "cort impact --symbol <name> [--depth <n>] [--coverage] [-f json|lean]",
            "read": "cort read <file> [--start <line>] [--end <line>] [-f json|lean]",
            "recall": "cort recall <query> [--limit <n>] [--content full] [-f json|lean]",
            "usage": "cort usage [days] [-f json|lean]",
            "hook-suggest": "cort hook-suggest  (PreToolUse hook: reads the harness payload on stdin, prints a suggestion or nothing; not a verb to type)",
            "hook-install": "cort hook-install [--settings <path>] [--command <cmd>] [--remove|--status]  (installer-invoked: wires hook-suggest into settings.json; not a verb to type)",
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
        IndexError::Io(io) => {
            CortError::new("file_not_found", json!({ "message": io.to_string() }))
        }
        IndexError::Sqlite(e) => {
            CortError::new("storage_busy", json!({ "message": e.to_string() }))
        }
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

fn open_project_tracked(
    root: &Path,
    usage: &mut UsageEvent,
) -> Result<(CanonicalRoot, Db), CortError> {
    match canonicalize_root(root) {
        Ok(canon) => {
            usage.project_id = Some(canon.project_id.clone());
            let db = open_db(db_path_for(&canon.path_str))
                .map_err(|e| CortError::new("storage_busy", json!({ "message": e.to_string() })))?;
            ensure_schema(&db)?;
            Ok((canon, db))
        }
        Err(e) => {
            usage.project_id = Some("_unknown".into());
            Err(map_index(e))
        }
    }
}

#[derive(Debug, Clone)]
struct UsageEvent {
    command: String,
    project_id: Option<String>,
    args_summary: String,
    read_source: Option<String>,
    requested_content_mode: Option<String>,
    effective_content_mode: Option<String>,
    receipt_hit: Option<bool>,
    index_stale: Option<bool>,
    saved_bytes: i64,
}

struct Emit {
    render_command: Option<&'static str>,
    format: Format,
    payload: Value,
}

fn usage_from_args(args: &[String]) -> UsageEvent {
    let raw = args.first().map(String::as_str).unwrap_or("");
    let command = if KNOWN_COMMANDS.contains(&raw) {
        raw
    } else {
        "unknown"
    };
    UsageEvent {
        command: command.to_string(),
        project_id: None,
        args_summary: usage::args_summary(None, None, None, None),
        read_source: None,
        requested_content_mode: None,
        effective_content_mode: None,
        receipt_hit: None,
        index_stale: None,
        saved_bytes: 0,
    }
}

fn fill_stale(usage: &mut UsageEvent, payload: &Value) {
    usage.index_stale = payload.get("index_is_stale").and_then(Value::as_bool);
}

fn stored_omitted_len(db: &Db, project_id: &str, file: &str, start: i64, end: i64) -> i64 {
    let row = db.query_row(
        "SELECT content, start_line FROM reading_notes
          WHERE project_id = ?1 AND file_path = ?2 AND start_line <= ?3 AND end_line >= ?4
          ORDER BY start_line DESC LIMIT 1",
        params![project_id, file, start, end],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    );
    match row {
        Ok((content, note_start)) => usage::omitted_body_len(&content, note_start, start, end),
        Err(_) => 0,
    }
}

fn finish_record(ev: &UsageEvent, status: &str, error_code: Option<&str>, bytes_out: usize) {
    let rec = CommandRecord {
        now_ms: usage::now_ms(),
        project_id: ev.project_id.clone(),
        command: ev.command.clone(),
        args_summary: ev.args_summary.clone(),
        status: status.to_string(),
        error_code: error_code.map(str::to_string),
        read_source: ev.read_source.clone(),
        requested_content_mode: ev.requested_content_mode.clone(),
        effective_content_mode: ev.effective_content_mode.clone(),
        receipt_hit: ev.receipt_hit,
        index_stale: ev.index_stale,
        bytes_out: bytes_out as i64,
        saved_bytes: ev.saved_bytes,
    };
    usage::record_command(&rec);
}

fn render_emit(emit: &Emit) -> String {
    if emit.render_command == Some("usage") && emit.format == Format::Lean {
        usage::render_usage_lean(&emit.payload)
    } else {
        render(emit.render_command, emit.format, &emit.payload)
    }
}

fn resolve_fmt(raw: Option<&str>) -> Result<Format, CortError> {
    parse_format(raw)
        .ok_or_else(|| CortError::new("unknown_format", json!({ "hint": "--format json|lean" })))
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

fn content_mode_name(mode: ContentMode) -> &'static str {
    match mode {
        ContentMode::Auto => "auto",
        ContentMode::Receipt => "receipt",
        ContentMode::Full => "full",
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct IndexArgs {
    root: Option<PathBuf>,
    #[arg(long)]
    incremental: bool,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct RootArgs {
    root: Option<PathBuf>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct HookInstallArgs {
    /// The settings.json to edit. Defaults to $CLAUDE_SKILL_HOME (or ~/.claude)/settings.json --
    /// the same override the installer uses to place the skill.
    #[arg(long = "settings")]
    settings: Option<String>,
    /// The command line to configure. Defaults to this binary's own path plus `hook-suggest`.
    #[arg(long = "command")]
    command: Option<String>,
    /// Take the entry back out instead of putting one in.
    #[arg(long = "remove")]
    remove: bool,
    /// Report what is wired without writing anything -- what `install.sh --check` needs.
    #[arg(long = "status")]
    status: bool,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct FormatOnlyArgs {
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
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
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
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
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct ImpactArgs {
    #[arg(long)]
    symbol: Option<String>,
    #[arg(long)]
    depth: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// Report what the enumeration may have missed, not just what it found: `dependents=0` and "no
    /// caller was ever extracted" look identical without this.
    #[arg(long)]
    coverage: bool,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
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
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct RecallArgs {
    query: Option<String>,
    #[arg(long)]
    limit: Option<String>,
    #[arg(long = "content", num_args = 0..=1, default_missing_value = "")]
    content: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct UsageArgs {
    #[arg(allow_hyphen_values = true)]
    days: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

fn pin_bin() -> Result<String, CortError> {
    let bin = resolve_ast_grep_bin()?;
    assert_ast_grep_version(&bin)?;
    Ok(bin)
}

fn dispatch(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let command = args.first().map(String::as_str);
    match command {
        Some("index") => cmd_index(&args[1..], usage),
        Some("status") => cmd_status(&args[1..], usage),
        Some("projects") => cmd_projects(&args[1..], usage),
        Some("delete") => cmd_delete(&args[1..], usage),
        Some("struct") => cmd_struct(&args[1..], usage),
        Some("context") => cmd_context(&args[1..], usage),
        Some("impact") => cmd_impact(&args[1..], usage),
        Some("read") => cmd_read(&args[1..], usage),
        Some("recall") => cmd_recall(&args[1..], usage),
        Some("usage") => cmd_usage(&args[1..], usage),
        Some("hook-suggest") => cmd_hook_suggest(usage),
        Some("hook-install") => cmd_hook_install(&args[1..], usage),
        other => Err(CortError::new(
            "unknown_command",
            json!({
                "command": other,
                "known": KNOWN_COMMANDS,
            }),
        )),
    }
}

/// A PreToolUse hook, not a verb anyone types. It reads the harness's hook payload on stdin and,
/// when the shell command about to run is a caller-set search, hands back one line of context
/// naming the query that answers it.
///
/// Three rules, each measured rather than assumed:
///
/// * It never blocks and never denies. The agent is right about `rg` most of the time -- 409
///   searches in the sampled sessions, ~6 of them caller-set work -- so anything stronger than a
///   suggestion would be wrong far more often than right.
/// * Silence is the default. No fire, no output, exit 0. A hook that prints on every search is
///   noise, and noise is what gets ignored.
/// * It stays silent when this project has no index. Suggesting a query that can only answer
///   `no_seed_resolved` would spend the agent's turn to tell it nothing, and would make the
///   suggestion itself untrustworthy the first time it happened.
fn cmd_hook_suggest(usage: &mut UsageEvent) -> Result<Emit, CortError> {
    usage.args_summary = "hook".into();
    let mut payload = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut payload);
    let quiet = || {
        Ok(Emit {
            payload: json!({}),
            format: Format::Lean,
            render_command: Some("hook-suggest"),
        })
    };
    let Ok(v) = serde_json::from_str::<Value>(&payload) else {
        return quiet();
    };
    let Some(command) = v
        .get("tool_input")
        .and_then(|i| i.get("command"))
        .and_then(Value::as_str)
    else {
        return quiet();
    };
    let Some(hit) = cort::hook::suggests_impact(command) else {
        return quiet();
    };
    // The gate is "this project has an index", which is what `cort status` means by
    // `indexed: true` -- a row in `projects`. It used to be "a db file exists", and those are not
    // the same claim: opening a project creates the schema, so a db with 0 chunks satisfied the
    // file test and the hook then told the agent `cort has an index for this project` on a tree
    // where `impact` can only answer `no_seed_resolved / stale=true`. That is the exact failure
    // the doc comment above forbids, and it was live on this machine on 2026-09-02.
    if !project_is_indexed() {
        return quiet();
    }
    let context = format!(
        "cort has an index for this project. `cort impact --symbol '{}' --depth 1 --coverage -f lean` \
answers who calls it in one call, and `--coverage` lists what the enumeration could not see -- which \
a grep cannot tell you. Use it before concluding nothing else uses this; keep the grep for anything \
literal.",
        hit.symbol
    );
    Ok(Emit {
        payload: json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": context,
            },
            "suppressOutput": true,
        }),
        format: Format::Json,
        render_command: Some("hook-suggest"),
    })
}

/// Does the cwd's project have an index with a `projects` row -- the same question `cort status`
/// answers as `indexed`. Every failure is a `false`: a hook that cannot read the index has nothing
/// to suggest, and must never turn a broken cache into a broken tool call.
fn project_is_indexed() -> bool {
    let Ok(canon) = canonicalize_root(cwd()) else {
        return false;
    };
    let path = db_path_for(&canon.path_str);
    if !path.exists() {
        return false;
    }
    let Ok(db) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    status_of(&db, &canon.path)
        .map(|s| s.indexed)
        .unwrap_or(false)
}

/// Wiring the PreToolUse hook into a Claude Code `settings.json`. `install.sh` calls this so the
/// hook ships with the skill instead of being a separate thing to remember per agent home; it is
/// not a verb anyone types either. `--remove` is what `install.sh --uninstall` calls.
fn cmd_hook_install(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = HookInstallArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let path = match a.settings {
        Some(p) => PathBuf::from(p),
        None => settings::default_settings_path().ok_or_else(|| {
            CortError::new(
                "file_not_found",
                json!({ "message": "no HOME or CLAUDE_SKILL_HOME to resolve settings.json from" }),
            )
        })?,
    };
    let map_err = |e: settings::SettingsError| match e {
        settings::SettingsError::Unparsable(m) => {
            CortError::new("bad_settings", json!({ "message": m }))
        }
        settings::SettingsError::Io(io) => {
            CortError::new("file_not_found", json!({ "message": io.to_string() }))
        }
    };
    if a.status {
        let wired = settings::installed_command(&path);
        return Ok(Emit {
            render_command: None,
            format: Format::Json,
            payload: json!({
                "settings": path.to_string_lossy(),
                "wired": wired.is_some(),
                "command": wired,
            }),
        });
    }
    if a.remove {
        let out = settings::remove_hook(&path).map_err(map_err)?;
        return Ok(Emit {
            render_command: None,
            format: Format::Json,
            payload: json!({
                "settings": path.to_string_lossy(),
                "change": out.change.as_str(),
                "backup": out.backup.map(|b| b.to_string_lossy().into_owned()),
            }),
        });
    }
    // Absolute by default: a hook runs with the harness's environment, not the login shell's, so
    // a bare `cort` is a PATH bet the installer has no reason to take.
    let command = match a.command {
        Some(c) => c,
        None => {
            let exe = std::env::current_exe().map_err(|e| {
                CortError::new("file_not_found", json!({ "message": e.to_string() }))
            })?;
            format!("{} hook-suggest", exe.to_string_lossy())
        }
    };
    let out = settings::install_hook(&path, &command).map_err(map_err)?;
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: json!({
            "settings": path.to_string_lossy(),
            "command": command,
            "change": out.change.as_str(),
            "backup": out.backup.map(|b| b.to_string_lossy().into_owned()),
        }),
    })
}

fn cmd_index(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = IndexArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let bin = pin_bin()?;
    let root = a.root.unwrap_or_else(cwd);
    let (canon, mut db) = open_project_tracked(&root, usage)?;
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
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload,
    })
}

fn cmd_status(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = RootArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let root = a.root.unwrap_or_else(cwd);
    let canon = match canonicalize_root(&root) {
        Ok(c) => c,
        Err(e) => {
            usage.project_id = Some("_unknown".into());
            return Err(map_index(e));
        }
    };
    usage.project_id = Some(canon.project_id.clone());
    let db_path = db_path_for(&canon.path_str);
    if !db_path.exists() {
        return Ok(Emit {
            render_command: None,
            format: Format::Json,
            payload: json!({
                "project_id": canon.project_id,
                "path": canon.path_str,
                "indexed": false,
            }),
        });
    }
    let db = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| CortError::new("storage_busy", json!({ "message": e.to_string() })))?;
    let st = status_of(&db, &canon.path).map_err(map_index)?;
    if !st.indexed {
        return Ok(Emit {
            render_command: None,
            format: Format::Json,
            payload: json!({
                "project_id": st.project_id,
                "path": st.path,
                "indexed": false,
            }),
        });
    }
    let bin = pin_bin()?;
    let stale = compute_stale(&db, &bin, &canon.path, &st.project_id).map_err(map_index)?;
    usage.index_stale = Some(stale.index_is_stale);
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: json!({
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
    })
}

fn cmd_projects(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
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
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: Value::Array(rows),
    })
}

fn cmd_delete(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = RootArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let root = a.root.unwrap_or_else(cwd);
    let canon = match canonicalize_root(&root) {
        Ok(c) => c,
        Err(e) => {
            usage.project_id = Some("_unknown".into());
            return Err(map_index(e));
        }
    };
    usage.project_id = Some(canon.project_id.clone());
    let r = delete_project(&canon.path_str);
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: json!({ "deleted": r.deleted, "db_path": r.db_path }),
    })
}

fn cmd_struct(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
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
    let (canon, db) = open_project_tracked(&cwd(), usage)?;
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
        StructOptions {
            globs,
            budget,
            file_limit: None,
        },
    )?;
    fill_stale(usage, &out);
    Ok(Emit {
        render_command: Some("struct"),
        format,
        payload: out,
    })
}

fn cmd_context(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = ContextArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let query = a.query.ok_or_else(|| {
        CortError::new(
            "missing_query",
            json!({ "hint": "cort context <symbol|query>" }),
        )
    })?;
    let format = resolve_fmt(a.format.as_deref())?;
    let bin = pin_bin()?;
    let (canon, db) = open_project_tracked(&cwd(), usage)?;
    let budget = parse_usize_flag(a.budget.as_deref(), DEFAULT_BUDGET);
    let full_content = a.content.as_deref() == Some("full");
    let out = context_command(
        &db,
        &bin,
        &canon.path,
        &canon.project_id,
        &query,
        ContextOptions {
            budget,
            include_ambiguous: a.include_ambiguous,
            full_content,
        },
    )?;
    fill_stale(usage, &out);
    if out.get("resolution").and_then(Value::as_str) == Some("exact_symbol") {
        usage.args_summary = usage::args_summary(Some(&query), None, None, None);
    }
    Ok(Emit {
        render_command: Some("context"),
        format,
        payload: out,
    })
}

fn cmd_impact(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = ImpactArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let symbol = a.symbol.filter(|s| !s.is_empty()).ok_or_else(|| {
        CortError::new(
            "missing_symbol",
            json!({ "hint": "cort impact --symbol <name>" }),
        )
    })?;
    let format = resolve_fmt(a.format.as_deref())?;
    let bin = pin_bin()?;
    let (canon, db) = open_project_tracked(&cwd(), usage)?;
    let depth = parse_i64_flag(a.depth.as_deref(), DEFAULT_DEPTH);
    let mut out = impact_command(&db, &bin, &canon.path, &canon.project_id, &symbol, depth)?;
    // Recall is a separate question from cost, so it is opt-in: the default payload stays the small
    // answer the eval priced, and `--coverage` pays for a walk of the indexed files.
    if a.coverage {
        coverage::attach(&db, &canon.project_id, Path::new(&canon.path), &mut out)?;
    }
    fill_stale(usage, &out);
    usage.args_summary = usage::args_summary(Some(&symbol), None, None, None);
    Ok(Emit {
        render_command: Some("impact"),
        format,
        payload: out,
    })
}

fn cmd_read(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = ReadArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let format = resolve_fmt(a.format.as_deref())?;
    let (canon, db) = open_project_tracked(&cwd(), usage)?;
    let file = a.file.unwrap_or_default();
    let start = parse_line_flag(a.start.as_deref(), "start")?;
    let end = parse_line_flag(a.end.as_deref(), "end")?;
    let mode = match a.content.as_deref() {
        None => parse_content_mode(None)?,
        Some(s) => parse_content_mode(Some(s))?,
    };
    let payload = unwrap_busy(
        with_busy_retry(|| {
            read_fragment(&db, &canon.path, &canon.project_id, &file, start, end, mode)
                .map_err(CortWrap)
        }),
        |w| w.0,
    )?;
    let value = serde_json::to_value(&payload).unwrap_or(Value::Null);
    let source = value.get("source").and_then(Value::as_str);
    let effective = value.get("content_mode").and_then(Value::as_str);
    usage.read_source = source.map(str::to_string);
    usage.requested_content_mode = Some(content_mode_name(mode).to_string());
    usage.effective_content_mode = effective.map(str::to_string);
    usage.receipt_hit = match mode {
        ContentMode::Auto => Some(source == Some("store") && effective == Some("receipt")),
        ContentMode::Full | ContentMode::Receipt => None,
    };
    let start_line = value.get("start_line").and_then(Value::as_i64);
    let end_line = value.get("end_line").and_then(Value::as_i64);
    let rel = value.get("file_path").and_then(Value::as_str);
    usage.args_summary = usage::args_summary(None, rel, start_line, end_line);
    if usage.receipt_hit == Some(true) {
        let omitted = stored_omitted_len(
            &db,
            &canon.project_id,
            rel.unwrap_or(""),
            start_line.unwrap_or(1),
            end_line.unwrap_or(1),
        );
        usage.saved_bytes = usage::saved_bytes_for(source, effective, omitted);
    }
    Ok(Emit {
        render_command: Some("read"),
        format,
        payload: value,
    })
}

fn cmd_recall(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = RecallArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let query = a
        .query
        .ok_or_else(|| CortError::new("missing_query", json!({ "hint": "cort recall <query>" })))?;
    let format = resolve_fmt(a.format.as_deref())?;
    let (canon, db) = open_project_tracked(&cwd(), usage)?;
    let limit = a.limit.as_deref().and_then(|s| s.parse::<i64>().ok());
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
    Ok(Emit {
        render_command: Some("recall"),
        format,
        payload: value,
    })
}

fn cmd_usage(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = UsageArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let days = usage::parse_usage_days(a.days.as_deref())?;
    let format = resolve_fmt(a.format.as_deref())?;
    let payload = match usage::usage_db_path() {
        Some(path) => usage::query_usage_at(&path, days, usage::now_ms())?,
        None => usage::empty_report(days),
    };
    Ok(Emit {
        render_command: Some("usage"),
        format,
        payload,
    })
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
        print!("{}", render(None, Format::Json, &usage_value()));
        return;
    }
    let mut usage_ev = usage_from_args(&args);
    match dispatch(&args, &mut usage_ev) {
        Ok(emitted) => {
            let rendered = render_emit(&emitted);
            print!("{rendered}");
            finish_record(&usage_ev, "ok", None, rendered.len());
        }
        Err(err) => {
            let format = peek_format(&args);
            let rendered = render_error(format, &err);
            print!("{rendered}");
            finish_record(&usage_ev, "error", Some(&err.code), rendered.len());
            std::process::exit(1);
        }
    }
}
