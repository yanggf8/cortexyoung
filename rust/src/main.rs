//! CLI dispatch with clap; `--help`/`-h`/`help` are side-effect free.

use clap::{CommandFactory, Parser, Subcommand};
use cort::ast_grep::{assert_ast_grep_version, resolve_ast_grep_bin};
use cort::coverage;
use cort::db::{
    cache_dir, db_path_for, delete_project, ensure_schema, list_projects, open_db, project_id_for,
    project_root_for_path, with_busy_retry, Db, ProjectEntry, SqliteErrorCode, WithBusyRetryError,
};
use cort::errors::CortError;
use cort::gate::{self, GateConfig};
use cort::impact::{impact_command, DEFAULT_DEPTH};
use cort::incremental::{incremental_index, RebuildPolicy};
use cort::indexer::{
    canonicalize_root, full_index, git_head_of, status_of, CanonicalRoot, IndexError,
};
use cort::render::{parse_format, render, render_error, Format};
use cort::settings;
use cort::settings_kimi;
use cort::settings_toml;
use cort::staleness::compute_stale;
use cort::usage::{self, CommandRecord};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// `hook-suggest` is here so its rows are recorded under their own name rather than `_unknown`:
// whether the harness hook fires, and whether a fire is followed by an `impact` call, is the one
// measurement that has a numerator -- and since the thin trigger landed the row says only that a
// fire happened, so that pairing is now read from the rows rather than inferred from a verdict.
// It is not a verb anyone types.
const KNOWN_COMMANDS: [&str; 16] = [
    "index",
    "status",
    "projects",
    "delete",
    "impact",
    "usage",
    "memory",
    "measure",
    "hook-suggest",
    "hook-refresh",
    "hook-recall",
    "hook-gate",
    "hook-install",
    "internal-shim",
    "internal-ast-grep",
    "internal-manifest-keys",
];

fn usage_value() -> Value {
    json!({
        "usage": "cort <command> [options]",
        "commands": {
            "index": "cort index [root] [--incremental]",
            "status": "cort status [root]",
            "projects": "cort projects [--verdict]",
            "delete": "cort delete [root]",
            "impact": "cort impact --symbol <name> [--depth <n>] [--coverage] [-f json|lean]",
            "usage": "cort usage [days] [-f json|lean]",
            "memory": "cort memory <configure|import-config|record|resolve|recall|status|check|notes|alias> [options]",
            "measure": "cort measure <configure|show|record|collect|health|status|cron|report> [options]  (uploads start at 2026-10-28 00:00 Taiwan time; collect: scan and upload the batch; --background appends at most one result to measure.log (none when skipped_busy) without stdout; health: observe recently used indexes and write local index_health_log, reading indexes only, with no network; --background appends the result to measure.log without stdout; status: read local collection state, -f json|lean; cron: manual install or removal of the legacy entry, no longer installed automatically; hook kick is the only automatic collection trigger; command_log and index_health_log keep uploaded rows 90 days and rows not yet uploaded 180 days, pruned when a new command is recorded; rows still not uploaded after 180 days are deleted and counted in measure status (pruned_unuploaded_rows) and measure.log)",
            "hook-suggest": "cort hook-suggest  (PreToolUse hook: records one fire row, kicks collection, and relays a configured gate's deny; not a verb to type)",
            "hook-refresh": "cort hook-refresh  (PostToolUse hook: refreshes an existing index and may prompt on a private-memory save; not a verb to type)",
            "hook-recall": "cort hook-recall --harness <h>  (SessionStart hook: reads the shared memory database; not a verb to type)",
            "hook-gate": "cort hook-gate <set [--timeout-ms N] -- <argv...>|clear|status [-f json|lean]>",
            "hook-install": "cort hook-install [--all|--settings <path>] [--event pre|post|session] [--command <cmd>] [--remove|--status]  (installer-invoked hook wiring; not a verb to type)",
            "internal-shim": "cort internal-shim --cort-home <dir>  (installer-invoked: renders the $BIN_DIR/cort shim for a CORT_HOME; not a verb to type)",
            "internal-ast-grep": "cort internal-ast-grep  (installer-invoked: prints the pinned ast-grep release and its checksums as lean TSV; not a verb to type)",
            "internal-manifest-keys": "cort internal-manifest-keys  (installer-invoked: prints known and legacy manifest keys as lean TSV; not a verb to type)",
        },
        "env": {
            "CORT_CACHE_DIR": "where indexes live (default ~/.cache/cortex-ng)",
        },
        "note": "Commands read the project at the cwd unless they take a root argument.",
    })
}

/// The memory verbs take more options than the one line `usage_value` gives `memory`, so they get
/// their own usage. It is rendered from `MemoryArgs` rather than written out, so a verb or option
/// cannot be added to the parser without appearing here. clap's own `render_usage` folds every
/// optional argument into `[OPTIONS]`, which hides exactly the options a caller is looking for.
fn memory_usage_value() -> Value {
    fn arg_text(arg: &clap::Arg) -> String {
        let id = arg.get_id().as_str().to_uppercase();
        let choices: Vec<String> = arg
            .get_possible_values()
            .iter()
            .map(|v| v.get_name().to_string())
            .collect();
        let value = if choices.is_empty() {
            format!("<{id}>")
        } else {
            format!("<{}>", choices.join("|"))
        };
        let takes_value = !matches!(
            arg.get_action(),
            clap::ArgAction::SetTrue | clap::ArgAction::SetFalse | clap::ArgAction::Count
        );
        let text = match arg.get_long() {
            Some(long) if takes_value => format!("--{long} {value}"),
            Some(long) => format!("--{long}"),
            None => value,
        };
        if matches!(arg.get_action(), clap::ArgAction::Append) {
            format!("{text}...")
        } else {
            text
        }
    }
    fn command_usage(verb: &clap::Command, prefix: &str) -> String {
        let mut parts = vec![format!("{prefix} {}", verb.get_name())];
        // A required one-of group prints as `(--a <A> | --b <B>)`; its members are not optional.
        let mut grouped = Vec::new();
        for group in verb.get_groups() {
            if !group.is_required_set() {
                continue;
            }
            let members: Vec<&clap::Id> = group.get_args().collect();
            let texts: Vec<String> = verb
                .get_arguments()
                .filter(|arg| members.contains(&arg.get_id()))
                .map(arg_text)
                .collect();
            parts.push(format!("({})", texts.join(" | ")));
            grouped.extend(members);
        }
        for arg in verb.get_arguments() {
            if grouped.contains(&arg.get_id()) {
                continue;
            }
            let text = arg_text(arg);
            let required = arg.is_required_set() && arg.get_default_values().is_empty();
            parts.push(if required { text } else { format!("[{text}]") });
        }
        parts.join(" ")
    }
    let command = MemoryArgs::command();
    let mut commands = serde_json::Map::new();
    for verb in command.get_subcommands() {
        commands.insert(
            verb.get_name().to_string(),
            json!(command_usage(verb, "cort memory")),
        );
        let prefix = format!("cort memory {}", verb.get_name());
        for sub in verb.get_subcommands() {
            commands.insert(
                format!("{} {}", verb.get_name(), sub.get_name()),
                json!(command_usage(sub, &prefix)),
            );
        }
    }
    json!({
        "usage": "cort memory <command> [options]",
        "commands": commands,
        "note": "Commands read the project at --root (default: the cwd). resolve is an alias for record with repeated --supersedes.",
    })
}

/// The arguments cort itself may read as flags. Everything after a bare `--` belongs to another
/// program -- `cort hook-gate set -- /path/gate --help` configures a gate that takes `--help`, it
/// does not ask cort for its usage.
fn before_separator(args: &[String]) -> impl Iterator<Item = &String> {
    args.iter().take_while(|a| *a != "--")
}

/// `--help` and `-h` are flags and count wherever they appear before `--`. The bare word `help` is
/// a command word, so it counts only where one can stand: first, or directly after `memory`.
/// Anywhere else it is a value -- `cort memory recall help` searches the ledger for the topic "help".
fn wants_help(args: &[String]) -> bool {
    let at = |i: usize, word: &str| args.get(i).is_some_and(|a| a == word);
    before_separator(args).any(|a| a == "--help" || a == "-h")
        || at(0, "help")
        || (at(0, "memory") && at(1, "help"))
}

/// `cort memory record --help` answers with `record` alone, and `cort memory notes --help` with
/// every `notes` verb: an agent asking about one command should not have to read all of them. The
/// command words are the leading arguments before the first flag; when they name nothing, the
/// whole list is the answer.
fn narrow_usage(mut usage: Value, args: &[String]) -> Value {
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .take_while(|a| !a.starts_with('-') && *a != "help")
        .collect();
    if words.is_empty() {
        return usage;
    }
    let Some(commands) = usage.get("commands").and_then(Value::as_object) else {
        return usage;
    };
    // Longest leading run that names a command: `recall foo --help` is about `recall`.
    let narrowed = (1..=words.len()).rev().find_map(|n| {
        let wanted = words[..n].join(" ");
        let found: serde_json::Map<String, Value> = commands
            .iter()
            .filter(|(key, _)| {
                key.as_str() == wanted
                    || key
                        .strip_prefix(wanted.as_str())
                        .is_some_and(|rest| rest.starts_with(' '))
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        (!found.is_empty()).then_some(found)
    });
    if let Some(narrowed) = narrowed {
        usage["commands"] = Value::Object(narrowed);
    }
    usage
}

/// `--version` is answered by the binary, and that is the whole point of it.
///
/// It used to be intercepted by the installer's shim, which echoed a string baked in at install
/// time and returned *before* the `exec` line -- so it answered identically whether the payload
/// behind it was current, stale, or absent entirely. Everything downstream inherited that:
/// `install.sh --check` compared its own `CORT_VERSION` against a string its own run had written,
/// and the design spec (`docs/superpowers/specs/2026-09-06-cort-upgrade-design.md` §69, §267)
/// recorded the check as one that "can never be inconsistent". Answering from the binary makes the
/// same comparison name the payload that would actually run.
///
/// Plain text, not the JSON every other command emits, and byte-identical to what the shim
/// printed: `--check` greps `head -1` of it, and a JSON first line of `{` is exactly the failure
/// that hid a stale shim on a live machine.
fn wants_version(args: &[String]) -> bool {
    before_separator(args).any(|a| a == "--version" || a == "-V")
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
        IndexError::Sqlite(e) => cort::db::classify_sqlite(&e),
        IndexError::FullRebuildRequired { reasons } => {
            CortError::new("full_rebuild_required", json!({ "reasons": reasons }))
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
        // `classify_sqlite` folds the raw code into the detail payload precisely so it survives
        // classification; reading it back here is what lets `with_busy_retry` keep retrying a
        // BUSY that crossed a `CortError` boundary. Returning `None` unconditionally made the
        // busy-retry wrappers retry nothing -- the loop only ever saw a codeless error.
        self.0.detail.get("sqlite_code").and_then(Value::as_str)
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
            // `open_db` sets WAL before its own busy timeout, so the pragma itself can meet a
            // concurrent writer and reports BUSY with nothing yet waiting it out. Retried here,
            // raw, because `SqliteErrorCode` is already implemented for `rusqlite::Error`.
            let db = unwrap_busy(
                with_busy_retry(|| open_db(db_path_for(&canon.path_str))),
                |e| cort::db::classify_sqlite(&e),
            )?;
            // The first statement of `ensure_schema` is the first write transaction on the open
            // path, so it is exactly where a refresh hook's concurrent write is met. Foreground
            // commands wait the contention out instead of reporting the store as occupied; the
            // hook path stays as it is -- it opens unmigrated and quiet by contract. Re-running
            // `ensure_schema` after a BUSY is safe: the batch is idempotent and a migration that
            // lost the race rolled back, which is why it starts over rather than resumes.
            unwrap_busy(
                with_busy_retry(|| ensure_schema(&db).map_err(CortWrap)),
                |w| w.0,
            )?;
            Ok((canon, db))
        }
        Err(e) => {
            usage.project_id = Some("_unknown".into());
            Err(map_index(e))
        }
    }
}

/// Open a project without migrating it.
///
/// `open_project_tracked` calls `ensure_schema`, which migrates and sets `graph_pending`
/// (`db.rs:304-323`). That is right for a command a person ran and wrong for a hook: a migration is
/// a table rebuild, the hook has a five-second budget, and its contract is to be silent and exit 0
/// -- so it cannot report a migration it could not finish. It reads the stored version instead and
/// leaves the database as it found it.
fn open_project_unmigrated(
    root: &Path,
    usage: &mut UsageEvent,
) -> Result<(CanonicalRoot, Db), CortError> {
    match canonicalize_root(root) {
        Ok(canon) => {
            usage.project_id = Some(canon.project_id.clone());
            let db =
                open_db(db_path_for(&canon.path_str)).map_err(|e| cort::db::classify_sqlite(&e))?;
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
    let known = KNOWN_COMMANDS.contains(&raw);
    let command = if known { raw } else { "unknown" };
    let mut ev = UsageEvent {
        command: command.to_string(),
        project_id: None,
        args_summary: usage::args_summary(None, None, None, None),
        read_source: None,
        requested_content_mode: None,
        effective_content_mode: None,
        receipt_hit: None,
        index_stale: None,
        saved_bytes: 0,
    };
    if !known {
        // An unknown command's only diagnostic value IS the name the caller typed — without
        // it the row is `{"v":1}` and answers nothing about what was mistyped (found mining
        // one quarter's log: 13 such rows). Capped like every other free string in the log.
        let capped = usage::cap_str(raw);
        ev.args_summary = format!(
            "{{\"rejected\":{},\"v\":1}}",
            serde_json::to_string(capped.as_str()).unwrap_or_else(|_| "\"\"".into())
        );
    }
    ev
}

fn fill_stale(usage: &mut UsageEvent, payload: &Value) {
    usage.index_stale = payload.get("index_is_stale").and_then(Value::as_bool);
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
    } else if emit.render_command == Some("hook-install-all-lean")
        || emit.render_command == Some("internal-shim-lean")
    {
        // Emitted raw. Wrapping these lines in a JSON string would put the installer back where it
        // started -- reaching into a serialised object with a regex -- which is the whole thing
        // `--all --lean` exists to stop.
        emit.payload
            .get("lean")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else if matches!(
        emit.render_command,
        Some("hook-recall" | "hook-refresh-raw")
    ) {
        emit.payload.as_str().unwrap_or_default().to_string()
    } else if emit.render_command == Some("hook-gate") && emit.format == Format::Lean {
        let p = &emit.payload;
        let field = |key: &str| match p.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        };
        let mut lines = vec![
            format!("state\t{}", field("state")),
            format!("path\t{}", field("path")),
        ];
        for key in [
            "command",
            "timeout_ms",
            "reason",
            "executable_exists",
            "executable",
        ] {
            lines.push(format!("{key}\t{}", field(key)));
        }
        format!("{}\n", lines.join("\n"))
    } else {
        render(emit.render_command, emit.format, &emit.payload)
    }
}

fn resolve_fmt(raw: Option<&str>) -> Result<Format, CortError> {
    parse_format(raw)
        .ok_or_else(|| CortError::new("unknown_format", json!({ "hint": "--format json|lean" })))
}

fn parse_i64_flag(raw: Option<&str>, default: i64) -> i64 {
    raw.and_then(|s| s.parse::<f64>().ok())
        .map(|n| n as i64)
        .unwrap_or(default)
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
    /// Spawned by the self-heal path (`heal.rs`), never typed by a person: take the project's
    /// heal lock first so two stale queries cannot rebuild twice, and mark the usage row so
    /// the census can say a background heal happened. Exits quietly when the lock is busy.
    #[arg(long = "heal-background")]
    heal_background: bool,
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
struct HookSuggestArgs {
    /// Which harness wired this hook. Recorded on the usage row so the adoption mining can tell
    /// one harness's fires from another's -- they all call this one binary and write to one
    /// database, and a row that cannot say where it came from cannot be compared against any single
    /// harness's transcripts. Supplied by the installer, which is the only thing that knows; never
    /// sniffed from the environment, because a guess here silently corrupts the measurement.
    #[arg(long = "harness")]
    harness: Option<String>,
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
    /// Which of the two hooks to act on: `pre` (the thin trigger on the matched tool) or `post`
    /// (the index refresh). Defaults to `pre`, so every existing caller keeps its meaning.
    /// `--remove` ignores it and takes both out, because uninstalling is one act.
    #[arg(long = "event")]
    event: Option<String>,
    /// Which settings dialect the target file speaks: `json` (Claude Code, and Grok through it),
    /// `codex` (nested `[[hooks.PreToolUse]]` groups), `kimi` (a flat `[[hooks]]` array). Defaults
    /// to the target's extension -- `.toml` means Codex, anything else JSON -- which held while
    /// `.toml` named exactly one harness. It no longer does: Kimi's file is also called
    /// `config.toml`, so that harness must say so, and sniffing the path for `.kimi-code` would put
    /// the answer back in the hands of a string nobody controls (`KIMI_CODE_HOME` can point
    /// anywhere).
    #[arg(long = "format")]
    format: Option<String>,
    /// Report what is wired without writing anything -- what `install.sh --check` needs.
    #[arg(long = "status")]
    status: bool,
    /// Act on every harness and both events in one call, resolving each file and each command from
    /// the table in this binary rather than from the caller. Ignores `--settings`, `--format`,
    /// `--event` and `--command`: naming any of them would be the caller restating what `--all`
    /// exists to own.
    #[arg(long = "all")]
    all: bool,
    /// With `--all`: the cort the harness should run. Required, never defaulted to this binary --
    /// the installed layout puts a shim in front of the real executable and the wired command has
    /// to name the shim.
    #[arg(long = "command-prefix")]
    command_prefix: Option<String>,
    /// With `--all`: one tab-separated line per entry instead of JSON, so `install.sh` reads the
    /// result with `read` rather than with a second JSON parser written in `sed`.
    #[arg(long = "lean")]
    lean: bool,
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
struct ProjectsArgs {
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// One lean line for a caller that wants the compatibility answer and not the population.
    #[arg(long)]
    verdict: bool,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct InternalShimArgs {
    #[arg(long = "cort-home")]
    cort_home: String,
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
struct UsageArgs {
    #[arg(allow_hyphen_values = true)]
    days: Option<String>,
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct MemoryArgs {
    #[command(subcommand)]
    action: MemoryAction,
}

#[derive(Subcommand, Debug)]
enum MemoryAction {
    /// Link, list, or unlink the project id a renamed repository had before (its old origin).
    Alias {
        #[command(subcommand)]
        action: AliasAction,
    },
    /// Opt in, inspect, and upload Claude Code harness notes.
    Notes {
        #[command(subcommand)]
        action: NotesAction,
    },
    Configure {
        #[arg(long)]
        url: String,
        #[arg(long, required = true)]
        token_stdin: bool,
    },
    ImportConfig {
        #[arg(long)]
        from: PathBuf,
    },
    Record {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        claim: String,
        #[arg(long, requires = "line")]
        source: Option<PathBuf>,
        #[arg(long, requires = "source")]
        line: Option<usize>,
        #[arg(long, conflicts_with = "replace")]
        supersedes: Vec<String>,
        #[arg(long)]
        replace: bool,
    },
    /// Alias for record with repeated --supersedes.
    Resolve {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        claim: String,
        #[arg(long, requires = "line")]
        source: Option<PathBuf>,
        #[arg(long, requires = "source")]
        line: Option<usize>,
        #[arg(long, required = true)]
        supersedes: Vec<String>,
    },
    Recall {
        query: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long, default_value = "unknown", value_parser = ["unknown", "claude-code", "codex", "kimi", "grok", "other"])]
        harness: String,
        #[arg(long)]
        json: bool,
    },
    Status {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Check {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

#[derive(clap::Args, Debug)]
#[command(group(clap::ArgGroup::new("from").required(true).multiple(false).args(["from_remote", "from_project"])))]
struct AliasWriteArgs {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    from_remote: Option<String>,
    #[arg(long)]
    from_project: Option<String>,
}

impl AliasWriteArgs {
    fn from(&self) -> cort::memory_store::AliasFrom {
        match &self.from_remote {
            Some(remote) => cort::memory_store::AliasFrom::Remote(remote.clone()),
            None => cort::memory_store::AliasFrom::Project(
                self.from_project.clone().unwrap_or_default(),
            ),
        }
    }
}

#[derive(Subcommand, Debug)]
enum AliasAction {
    Add(AliasWriteArgs),
    List {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    Remove(AliasWriteArgs),
}

#[derive(Subcommand, Debug)]
enum NotesAction {
    OptIn {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    OptOut {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    Status,
    Upload {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        verbose: bool,
        #[arg(long, hide = true)]
        background: bool,
    },
    List {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    Show {
        note_id: String,
    },
}

fn pin_bin() -> Result<String, CortError> {
    let bin = resolve_ast_grep_bin()?;
    assert_ast_grep_version(&bin)?;
    Ok(bin)
}

fn gate_arg_error(message: &str) -> CortError {
    CortError::new("gate_invalid_args", json!({ "message": message }))
}

fn gate_status(format: Format) -> Emit {
    let path = gate::path();
    let payload = match gate::load() {
        GateConfig::Unconfigured => json!({
            "state": "unconfigured", "path": path, "command": null,
            "timeout_ms": null, "reason": null,
            "executable_exists": null, "executable": null,
        }),
        GateConfig::Invalid { reason } => json!({
            "state": "invalid", "path": path, "command": null,
            "timeout_ms": null, "reason": reason,
            "executable_exists": null, "executable": null,
        }),
        GateConfig::Configured {
            command,
            timeout_ms,
        } => {
            let executable = Path::new(&command[0]);
            let metadata = std::fs::metadata(executable).ok();
            let executable_exists = metadata.is_some();
            #[cfg(unix)]
            let can_execute = {
                use std::os::unix::fs::PermissionsExt;
                metadata
                    .as_ref()
                    .is_some_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            };
            #[cfg(not(unix))]
            let can_execute = metadata.as_ref().is_some_and(|m| m.is_file());
            json!({
                "state": "configured", "path": path, "command": command,
                "timeout_ms": timeout_ms, "reason": null,
                "executable_exists": executable_exists, "executable": can_execute,
            })
        }
    };
    Emit {
        render_command: Some("hook-gate"),
        format,
        payload,
    }
}

fn cmd_hook_gate(args: &[String]) -> Result<Emit, CortError> {
    match args.first().map(String::as_str) {
        Some("set") => {
            let separator = args
                .iter()
                .position(|s| s == "--")
                .ok_or_else(|| gate_arg_error("set requires -- before the command"))?;
            let options = &args[1..separator];
            let mut timeout_ms = None;
            let mut index = 0;
            while index < options.len() {
                if options[index] != "--timeout-ms" || timeout_ms.is_some() {
                    return Err(gate_arg_error("unknown or repeated set option"));
                }
                let value = options
                    .get(index + 1)
                    .ok_or_else(|| gate_arg_error("--timeout-ms requires a value"))?;
                timeout_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| gate_arg_error("--timeout-ms must be an unsigned integer"))?,
                );
                index += 2;
            }
            let command = args[separator + 1..].to_vec();
            let path = gate::save(command, timeout_ms)?;
            Ok(Emit {
                render_command: None,
                format: Format::Json,
                payload: json!({ "path": path, "state": "configured" }),
            })
        }
        Some("clear") if args.len() == 1 => {
            let removed = gate::clear().map_err(|e| {
                CortError::new(
                    "gate_storage_error",
                    json!({ "path": gate::path(), "message": e.to_string() }),
                )
            })?;
            Ok(Emit {
                render_command: None,
                format: Format::Json,
                payload: json!({ "path": gate::path(), "removed": removed }),
            })
        }
        Some("status") => {
            let format = match &args[1..] {
                [] => Format::Json,
                [flag, value] if flag == "-f" || flag == "--format" => resolve_fmt(Some(value))?,
                _ => return Err(gate_arg_error("status accepts only -f json|lean")),
            };
            Ok(gate_status(format))
        }
        _ => Err(gate_arg_error("expected set, clear, or status")),
    }
}

fn dispatch(args: &[String], usage: &mut UsageEvent, started: Instant) -> Result<Emit, CortError> {
    let command = args.first().map(String::as_str);
    match command {
        Some("index") => cmd_index(&args[1..], usage),
        Some("status") => cmd_status(&args[1..], usage),
        Some("projects") => cmd_projects(&args[1..], usage),
        Some("delete") => cmd_delete(&args[1..], usage),
        Some("impact") => cmd_impact(&args[1..], usage),
        Some("usage") => cmd_usage(&args[1..], usage),
        Some("memory") => cmd_memory(&args[1..], usage),
        Some("measure") => cmd_measure(&args[1..], usage),
        Some("hook-suggest") => cmd_hook_suggest(&args[1..], usage),
        Some("hook-refresh") => cmd_hook_refresh(&args[1..], usage),
        Some("hook-recall") => cmd_hook_recall(&args[1..], usage, started),
        Some("hook-gate") => cmd_hook_gate(&args[1..]),
        Some("hook-install") => cmd_hook_install(&args[1..], usage),
        Some("internal-shim") => cmd_internal_shim(&args[1..], usage),
        Some("internal-ast-grep") => cmd_internal_ast_grep(usage),
        Some("internal-manifest-keys") => cmd_internal_manifest_keys(usage),
        other => Err(CortError::new(
            "unknown_command",
            json!({
                "command": other,
                "known": KNOWN_COMMANDS,
            }),
        )),
    }
}

/// `{"hook":"<outcome>","v":1}` -- the same JSON-object shape every other command's
/// `args_summary` carries, so one parser reads the whole column.
/// `v: 2` adds `harness`. A v1 row predates the field and is not attributable to any harness --
/// `unspecified`, never defaulted to the one that happened to be wired first.
///
/// `declared` is recorded only when it disagrees with `harness`, because a disagreement is the
/// finding: one settings file can be read by more than one harness, and the flag then names the
/// installer's intent rather than the process that actually ran.
///
/// `v: 3` adds `model`, and the harness field alone is why it had to. A router -- this machine runs
/// one, `kimicode` -> `~/.claude-code-router/.../cc_claude` -- launches real Claude Code against a
/// different model's endpoint. Same binary, same `settings.json`, same `~/.claude/projects`, so
/// `harness_of` correctly answers `claude-code` and there is nothing left to tell the rows apart:
/// on 2026-09-03 the local Claude Code corpus held ~2,167 assistant messages from
/// `muse-spark-1.2-contributor`, `stealth/ox-alpha`, four `glm-5.*` variants, `deepseek-v4-flash`,
/// `k3` and `qwen3.5:4b` against ~5,072 Anthropic ones -- about 30% of a corpus quoted as evidence
/// about how *the agent* behaves. Harness is which program ran; model is who answered, and every
/// behavioural claim (search-without-cort rates, multi-hop error rates, uptake) is a claim about
/// the second one. The payload has carried it all along (`model`, beside `transcript_path` and
/// `session_id`; `docs/2026-09-02-hook-wiring-correction.md` §12) and nothing read it.
///
/// Omitted rather than guessed when the payload has none, and a v1/v2 row is a row from before the
/// field existed -- the version tells a reader which of the two silences it is looking at.
///
/// `v: 4` added the per-search keys stamped beside the outcome where the probe produced them:
/// `symbol` on `hit`/`hit_stale`/`no_index`/`no_index_hinted`/`no_evidence`, and `why` (an
/// identifier from the removed judging hook's `NoEvidenceWhy` vocabulary) on `no_evidence`.
/// A symbol name is a program identifier, not payload content -- the same class of fact as the
/// `decline` tag and the shape fingerprint.
///
/// `v: 5` added `demand` only when the hook emitted a suggestion: a redacted, 240-character
/// excerpt of the latest user-authored prompt, or a status explaining why none was captured.
/// Those two keys belong to a v≤5 row; the suggestion itself was removed in `v: 6`.
///
/// `v: 6` is the thin trigger. `hook-suggest` now writes only `no_payload` (stdin was not a
/// payload this hook could read) and `fired` (it was), and a `fired` row carries `tool`, the
/// harness's own tool name when that name is an identifier and `unrecognised` otherwise. The
/// `symbol`, `why`, `decline`, `shape`, `demand` and `kind` keys appear only in v≤5 rows, which
/// stay in `usage.db` for the 90-day retention (and forever on the remote). `hook-refresh` shares
/// this row shape, so its rows are v6 too and unchanged in every field they carry.
fn hook_row(outcome: &str, harness: &str, declared: Option<&str>, model: Option<&str>) -> Value {
    let mut v = json!({ "v": 6, "hook": outcome, "harness": harness });
    if let Some(d) = declared {
        v["harness_declared"] = json!(d);
    }
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        v["model"] = json!(m);
    }
    v
}

fn hook_args(outcome: &str, harness: &str, declared: Option<&str>, model: Option<&str>) -> String {
    hook_row(outcome, harness, declared, model).to_string()
}

/// 在已序列化的 hook row 上補一個穩定識別子。值永遠是識別子而非 payload 內容，兩個呼叫端共用
/// 同一份解析與退場行為，免得其中一個悄悄漂走。
fn hook_args_tag(hook_args_json: &str, key: &str, value: &str) -> String {
    let mut v: Value =
        serde_json::from_str(hook_args_json).unwrap_or_else(|_| json!({ "hook": "unknown" }));
    v[key] = json!(value);
    v.to_string()
}

/// The `tool` a fire row carries: the harness's own `tool_name` when it is shaped like an
/// identifier, `unrecognised` otherwise.
///
/// This is an identifier, not content, and it is the only payload value besides `model` and
/// `transcript_path` that the row reads. The check is hand-written rather than a regex because the
/// crate carries no regex and this is the whole rule: 1-64 bytes of `[A-Za-z0-9_.:-]`. Names that
/// are outside it -- a Kimi/MCP spelling with spaces, a number, a missing field, anything longer --
/// all collapse to one value, so the mining asks "how often was this harness's tool name outside
/// the vocabulary" rather than reading arbitrary payload text back out of a log.
fn hook_tool_label(v: &Value) -> &str {
    match v.get("tool_name").and_then(Value::as_str) {
        Some(name)
            if !name.is_empty()
                && name.len() <= 64
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')) =>
        {
            name
        }
        _ => "unrecognised",
    }
}

fn model_of_payload(v: &Value) -> Option<&str> {
    v.get("model").and_then(Value::as_str)
}

/// Which harness is running this hook, taken from what the harness said about itself.
///
/// The installer's `--harness` flag is an intent, and on 2026-09-02 that intent was measured wrong:
/// Grok reads `~/.claude/settings.json` for Claude Code compatibility, so the entry `install.sh`
/// wired there fires inside Grok too and carries `--harness claude-code` with it. Every one of those
/// rows would have been counted as a Claude Code injection with no Claude transcript to match.
///
/// `transcript_path` settles it without guessing: it is the harness naming its own session file,
/// not an environment variable that happens to be set. When it names a harness we know, that wins
/// over the flag and the flag is recorded alongside so the disagreement stays visible. When it
/// names nothing we recognise, the flag stands -- a declared value is still better than none.
///
/// Kimi (`kimi-code`) is deliberately absent from the list below, for a narrower reason than the
/// first version of this comment gave. Its `PreToolUse` payload does carry identity -- the runner
/// prepends `hook_event_name`, `session_id`, `cwd` and `client_type` to the tool fields
/// (`tool_name`, `tool_input`, `tool_call_id`), observed byte-for-byte on a live run, 2026-09-02 --
/// but no transcript path under either spelling, and neither spelling occurs anywhere in the
/// shipped `@moonshot-ai/kimi-code` bundle. A `/.kimi-code/` arm sat here until that was checked:
/// it could never match, and an arm that cannot fire made this list look like it covered a harness
/// that is not wired at all. The earlier claim that the payload carried the tool fields "and
/// nothing else" was read off `runPreToolUse` alone and missed what the runner adds; the conclusion
/// survived the correction, the reasoning did not. Re-add the arm when a Kimi payload actually
/// carries a path -- not on the assumption that it does.
///
/// Whether to wire Kimi at all was once a separate decision, not a leftover. Its `PreToolUse` keeps
/// only results whose `action` is `block` and discards every allow-shaped one before the model sees
/// it (`blockDecision`; every other hook event drops its result outright), so while `hook-suggest`
/// delivered a suggestion an entry there would have had to trade that rule for a deny
/// (`docs/2026-09-02-hook-wiring-correction.md` §14). The thin trigger has nothing to deliver: it
/// writes a row and returns `{}`, so the block-shaped-only rule costs it nothing, and Kimi is
/// wired like the other two. Its rows are the ones `harness_of` cannot settle from the payload --
/// no transcript path -- and they carry the declared `kimi-code`, which is the honest answer.
fn harness_of(payload: &Value, declared: &str) -> (String, Option<String>) {
    let path = payload
        .get("transcript_path")
        .or_else(|| payload.get("transcriptPath"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let observed = if path.contains("/.grok/") {
        Some("grok")
    } else if path.contains("/.codex/") {
        Some("codex")
    } else if path.contains("/.claude/") {
        Some("claude-code")
    } else {
        None
    };
    match observed {
        Some(o) if o != declared => (o.to_string(), Some(declared.to_string())),
        Some(o) => (o.to_string(), None),
        None => (declared.to_string(), None),
    }
}

/// A `PostToolUse` hook on the edit tools, not a verb anyone types. It brings this project's index
/// up to the tree it now describes, and may nudge on a private-memory save.
///
/// Why it exists. The `PreToolUse` hook could already tell that the index was behind, and all it
/// could do was say so -- and measured, saying so is worth almost nothing: 19 `cort index` runs
/// against 2,700+ hook fires in 90 days on this machine, and in the one live run where a model read
/// the warning it re-ran its grep instead. Worse, the staleness that hook reported compared git
/// heads, so the window in which a file is edited and not yet committed -- most of the time anyone
/// is working -- reads as fresh while the answers are already wrong. That window is where the
/// 2026-09-02 `impact` run put a caller at line 467 that was really at 482.
///
/// Three rules, and each one is a refusal:
///
/// * **It never creates an index.** No row in `projects` means this tree was never indexed on
///   purpose, and a hook that starts indexing whatever directory an agent edited in would be a
///   side effect nobody asked for. Bootstrapping stays an explicit `cort index`.
/// * **It gives up rather than wait.** Another refresh, or a real `cort index`, may hold the write
///   lock; a busy database is a reason to do nothing, not to block the agent's next tool call.
///   Nothing is lost -- the next edit refreshes it.
/// * **It always succeeds.** Exit 0; silent except for a capture line on a qualifying
///   private-memory save. Otherwise it prints `{}`. A
///   `PostToolUse` hook that reports failure would put an error in front of the user for a cache
///   they did not ask about, on a tool call that already succeeded.
///
/// The cost was measured before it was built: 23-37ms when nothing changed (the common case, since
/// most edits touch one file that is already current by the time a second tool call lands), ~206ms
/// after one edited file. `incremental_index` does its work in transactions, so the harness killing
/// this at its timeout rolls back rather than leaving a half-written graph.
fn cmd_hook_refresh(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    // Same `--harness` the installer already passes on this entry's command line (`install.sh`
    // builds both events' commands from one template), and the same row shape `hook-suggest`
    // writes (`hook_row`, now `v: 6`). It was being discarded: the argument list arrived as
    // `_args` and the summary was a bare `outcome=`, so once three harnesses were wired every
    // reindex became an anonymous row and "is Kimi's post-hook actually firing?" had no answer
    // in the data. That is the defect
    // `f3cb567f` fixed for the pre-event and never carried across to this one.
    let declared = HookSuggestArgs::try_parse_from(args.iter())
        .ok()
        .and_then(|a| a.harness)
        .unwrap_or_else(|| "unspecified".to_string());
    // The payload is read for two fields only. Which file changed is `incremental_index`'s
    // question, not ours -- but `transcript_path` is the harness naming its own session file and it
    // outranks the flag for the reason `harness_of` documents (Grok runs the entry installed as
    // `claude-code`), and `model` is the only thing that separates a router's session from the
    // harness it is wearing.
    let mut payload = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut payload);
    let parsed = serde_json::from_str::<Value>(&payload).ok();
    let (harness, declared_differs) = match parsed.as_ref() {
        Some(v) => harness_of(v, &declared),
        None => (declared, None),
    };
    let model = parsed
        .as_ref()
        .and_then(model_of_payload)
        .map(str::to_string);
    let edited = parsed
        .as_ref()
        .and_then(|v| v.get("tool_input"))
        .and_then(|i| i.get("file_path"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let stated_cwd = parsed
        .as_ref()
        .and_then(|v| v.get("cwd"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let edited_for_notes = edited.as_ref().map(|p| {
        if p.is_relative() {
            stated_cwd.clone().unwrap_or_else(cwd).join(p)
        } else {
            p.clone()
        }
    });
    let notes = cort::notes_kick::kick(edited_for_notes.as_deref());
    let (capture, context) = cort::notes_kick::capture(
        &cort::notes_kick::NotesKickPaths::current(),
        &harness,
        edited_for_notes.as_deref(),
    );
    // A nudge is written raw on one line (`hook-refresh-raw`), like hook-recall's answer; anything
    // else stays the `{}` this hook has always printed.
    let output = || match &context {
        Some(text) => Emit {
            payload: json!(format!(
                "{}\n",
                json!({"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":text}})
            )),
            format: Format::Lean,
            render_command: Some("hook-refresh-raw"),
        },
        None => Emit {
            payload: json!({}),
            format: Format::Lean,
            render_command: Some("hook-refresh"),
        },
    };
    // Every return from here on knows its harness, so every one kicks collection; `kick`
    // does its own upgrade check, which is why the stand-down below kicks too.
    let quiet = |outcome: &str, usage: &mut UsageEvent| {
        let row = hook_args_tag(
            &hook_args(
                outcome,
                &harness,
                declared_differs.as_deref(),
                model.as_deref(),
            ),
            "collect",
            cort::collect::kick(&harness),
        );
        let row = hook_args_tag(&row, "notes", notes);
        usage.args_summary = hook_args_tag(&row, "capture", capture);
        Ok(output())
    };

    // Upgrade stand-down (spec §3b). An upgrade holding admission exclusive is publishing a
    // new generation; refreshing an index against the old pack mid-flip would stamp it with a
    // dead extractor identity. The contract already says give up rather than wait, and an
    // upgrade in flight is exactly "rather than wait" — so: record the cause (unattributable
    // silences cannot be tuned, same rule as the decline tags) and go quiet. This branch sits
    // before any pack read or database open; only the payload was read, for attribution.
    let _guard = match cort::upgrade::try_protected_entry(&cache_dir()) {
        Ok(g) => g,
        Err(_) => return quiet("upgrade_stood_down", usage),
    };

    // Which project to repair comes from the edited file, not from the shell's working directory.
    // The comment above says which *file* changed is `incremental_index`'s question -- true, and
    // only once the right project is open. An agent that runs `cd rust && cargo test` used to take
    // every later edit's repair with it: measured at 79 `no_index` rows in four hours of editing
    // this repository, while the index drifted two lines behind and `status` still said fresh.
    //
    // `tool_input.file_path` is what Claude Code's Write and Edit send, absolute, captured rather
    // than assumed (`docs/2026-09-05-posttooluse-payloads.md`). `Bash` sends no path, and Codex,
    // Kimi, MultiEdit and NotebookEdit were never captured -- all of those fall through to the
    // payload's own `cwd`, which the harness asserts, and then to the process's, which it merely
    // inherited.
    // A path decides on its own. Falling back to cwd when an explicit path owns no index would
    // refresh a project that was never edited.
    //
    // A *relative* path still needs a base, and the base is the payload's `cwd` before the
    // process's, for the same reason the no-path arm below prefers it: one the harness asserts, one
    // it merely inherited. Claude Code sends absolute paths, so this arm is unexercised there --
    // but the two uncaptured harnesses are exactly where a relative path would arrive, and
    // resolving it against a directory the harness never claimed would repair the wrong project
    // with every test still green.
    let base = || stated_cwd.clone().unwrap_or_else(cwd);
    let resolved = match edited {
        Some(p) if p.is_relative() => project_root_for_path(&base().join(p)),
        Some(p) => project_root_for_path(&p),
        None => project_root_for_path(&base()),
    };
    let root = match resolved {
        Ok(Some(root)) => root,
        Ok(None) => return quiet("no_index", usage),
        // A candidate database exists and would not answer. Guessing an outer project instead
        // would repair the wrong index; the outcome for "could not do the work" already exists.
        Err(_) => return quiet("db_unavailable", usage),
    };
    // The same helpers `cmd_index` uses, deliberately: opening the database by anything other than
    // `db_path_for` silently addresses a different file, which is exactly how the first version of
    // this function reported success while refreshing nothing.
    let Ok(bin) = pin_bin() else {
        return quiet("no_ast_grep", usage);
    };
    let Ok((canon, mut db)) = open_project_unmigrated(&root, usage) else {
        return quiet("db_unavailable", usage);
    };
    match incremental_index(&mut db, &bin, &canon.path, RebuildPolicy::Forbid) {
        Ok(r) if r.files_reindexed > 0 || r.files_removed > 0 => {
            // The counts stay, but inside the object rather than beside it: this row used to be a
            // bare `outcome=refreshed reindexed=N removed=M`, which is not the JSON shape
            // `hook_args` documents for the whole column, so one parser could not read it.
            let mut row = hook_row(
                "refreshed",
                &harness,
                declared_differs.as_deref(),
                model.as_deref(),
            );
            row["reindexed"] = json!(r.files_reindexed);
            row["removed"] = json!(r.files_removed);
            let row = hook_args_tag(&row.to_string(), "collect", cort::collect::kick(&harness));
            let row = hook_args_tag(&row, "notes", notes);
            usage.args_summary = hook_args_tag(&row, "capture", capture);
            Ok(output())
        }
        Ok(_) => quiet("already_current", usage),
        Err(cort::indexer::IndexError::FullRebuildRequired { reasons }) => {
            let mut row = hook_row(
                "rebuild_required",
                &harness,
                declared_differs.as_deref(),
                model.as_deref(),
            );
            row["reasons"] = json!(reasons);
            let row = hook_args_tag(&row.to_string(), "collect", cort::collect::kick(&harness));
            let row = hook_args_tag(&row, "notes", notes);
            usage.args_summary = hook_args_tag(&row, "capture", capture);
            Ok(output())
        }
        Err(_) => quiet("busy_or_failed", usage),
    }
}

/// A `PreToolUse` hook, not a verb anyone types. It records one row per fire and kicks the
/// collection lanes. Its own judgment stays silent; a configured external gate's exit 75
/// is the only path that sends a deny to the agent.
///
/// This is what is left of the suggestion. The announcement was measured worthless -- 409 searches
/// in skill-bearing sessions followed by zero `cort` calls, then 0/9 external uptake and a window
/// in which no suggestion anyone acted on fired -- so the product removed it rather than keep
/// tuning it. What remains is the demand instrument: a fire is a fact about how often an agent
/// reaches for a matched tool (every `Bash` call, plus Kimi's `Grep`), and the row says which
/// harness and model ran, which project it ran in, and which tool was about to run.
/// `CORT_SUGGEST` no longer exists; nothing here reads it.
///
/// The row is decided before anything can fail: `no_payload` is installed first, and only a
/// payload that was actually read replaces it with `fired`. A payload the harness never finished
/// sending is not a fire, so that arm does not kick collection either.
fn cmd_hook_suggest(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let declared = HookSuggestArgs::try_parse_from(args.iter())
        .ok()
        .and_then(|a| a.harness)
        .unwrap_or_else(|| "unspecified".to_string());
    usage.args_summary = hook_args("no_payload", &declared, None, None);
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
    // A payload with no `tool_input` (absent or null) is one this hook could not read -- the
    // harness has not said what it is about to run. It stays `no_payload` and does not kick: there
    // is no fire to collect.
    if matches!(v.get("tool_input"), None | Some(Value::Null)) {
        return quiet();
    }
    let (harness, declared_differs) = harness_of(&v, &declared);
    let model = model_of_payload(&v);
    // The project is the directory the hook ran in, canonicalised and hashed -- the same id
    // `cort index` will take for it, so a fire row joins the projects table without a second
    // derivation. No git and no database: the old probe opened the project's index and read its
    // head to decide whether to suggest, and none of that decision is left to make. Not
    // `canonicalize_root`: it `expect`s a UTF-8 path, and a hook must never panic, so a directory
    // that is not UTF-8 simply has no project on its row.
    usage.project_id = std::fs::canonicalize(cwd())
        .ok()
        .and_then(|path| path.to_str().map(project_id_for));
    let row = hook_args_tag(
        &hook_args("fired", &harness, declared_differs.as_deref(), model),
        "tool",
        hook_tool_label(&v),
    );
    // Only a fired Bash call reaches the optional external gate.
    let hand_off = if v.get("tool_name").and_then(Value::as_str) == Some("Bash") {
        cort::gate::hand_off(&cort::gate::load(), payload.as_bytes())
    } else {
        cort::gate::HandOff {
            tag: "not_shell",
            elapsed_ms: None,
            deny_reason: None,
        }
    };
    let mut row = hook_args_tag(&row, "gate", hand_off.tag);
    if let Some(ms) = hand_off.elapsed_ms {
        row = hook_args_tag(&row, "gate_ms", &ms.to_string());
    }
    let row = hook_args_tag(&row, "collect", cort::collect::kick(&harness));
    usage.args_summary = hook_args_tag(&row, "notes", cort::notes_kick::kick(None));
    if let Some(reason) = hand_off.deny_reason {
        Ok(Emit {
            payload: json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }}),
            format: Format::Lean,
            render_command: Some("hook-suggest"),
        })
    } else {
        quiet()
    }
}

/// Recall reads shared memory without waiting on a local writer.
fn cmd_hook_recall(
    args: &[String],
    usage: &mut UsageEvent,
    started: Instant,
) -> Result<Emit, CortError> {
    let declared = HookSuggestArgs::try_parse_from(args.iter())
        .ok()
        .and_then(|a| a.harness)
        .unwrap_or_else(|| "unspecified".to_string());
    let mut input = String::new();
    let parsed = std::io::stdin()
        .read_to_string(&mut input)
        .ok()
        .and_then(|_| serde_json::from_str::<Value>(&input).ok());
    let (harness, differs) = parsed
        .as_ref()
        .map(|v| harness_of(v, &declared))
        .unwrap_or((declared, None));
    let model = parsed.as_ref().and_then(model_of_payload);
    let prompt = parsed
        .as_ref()
        .and_then(|v| v.get("hook_event_name"))
        .and_then(Value::as_str)
        == Some("UserPromptSubmit");
    let skip = if prompt {
        match parsed
            .as_ref()
            .and_then(|v| v.get("session_id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            None => Some("missing_session_id"),
            Some(id) => match cort::hook_recall::claim_prompt_session(id) {
                Ok(true) => None,
                Ok(false) => Some("already_injected"),
                Err(_) => Some("marker_unavailable"),
            },
        }
    } else {
        None
    };
    let root = parsed
        .as_ref()
        .and_then(|v| v.get("cwd"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(cwd);
    usage.project_id = std::fs::canonicalize(&root)
        .ok()
        .and_then(|p| p.to_str().map(project_id_for));
    let (outcome, content, count, notes_count) = if let Some(outcome) = skip {
        (outcome, None, 0, 0)
    } else if parsed.is_none() {
        ("failed", None, 0, 0)
    } else {
        let read = (|| {
            let Some((url, token)) = cort::memory_store::session_config().map_err(|_| "failed")?
            else {
                return Err("not_configured");
            };
            let projects = cort::memory_store::session_project_ids(&root).map_err(|_| "failed")?;
            let remaining = cort::remote::RECALL_DEADLINE.saturating_sub(started.elapsed());
            if remaining < Duration::from_millis(200) {
                return Err("remote_timeout");
            }
            let configured_ms = std::env::var("CORT_RECALL_TIMEOUT_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(3500)
                .min(3700);
            let db = cort::remote::RemoteDb::new(
                &url,
                token,
                Box::new(cort::remote::RecallTransport {
                    started,
                    max_timeout: Duration::from_millis(configured_ms),
                }),
            )
            .map_err(|e| match e.kind {
                cort::remote::ErrorKind::Blocked => "remote_blocked",
                cort::remote::ErrorKind::Timeout => "remote_timeout",
                _ => "remote_unreachable",
            })?;
            cort::memory_store::session_recall_with(
                &root,
                &db,
                cort::usage::machine_id(),
                (&projects.0, &projects.1),
                started,
            )
            .map_err(|kind| match kind {
                cort::remote::ErrorKind::Blocked => "remote_blocked",
                cort::remote::ErrorKind::Timeout => "remote_timeout",
                _ => "remote_unreachable",
            })
        })();
        match read {
            Ok((recall, omitted, notes, total)) => {
                match cort::hook_recall::render_with_notes(recall, omitted, &notes, total) {
                    Some((text, count, notes_count)) => {
                        ("injected", Some(text), count, notes_count)
                    }
                    None => ("none", None, 0, 0),
                }
            }
            Err(outcome) if outcome.starts_with("remote_") => (outcome, Some(format!("cort memory: the shared database could not be read at session start ({outcome}); run `cort memory recall \"<topic>\"` when you need it")), 0, 0),
            Err(outcome) => (outcome, None, 0, 0),
        }
    };
    let mut row = hook_row(outcome, &harness, differs.as_deref(), model);
    if outcome == "injected" {
        row["entries"] = json!(count);
        row["notes"] = json!(notes_count);
    }
    usage.args_summary = row.to_string();
    let payload = content
        .map(|text| {
            if prompt {
                json!({"message": text}).to_string()
            } else {
                json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}}).to_string()
            }
        })
        .unwrap_or_default();
    Ok(Emit {
        payload: Value::String(payload),
        format: Format::Lean,
        render_command: Some("hook-recall"),
    })
}

/// How long `cort projects` will wait for git before deciding it cannot tell.
///
/// `git rev-parse HEAD` is milliseconds on a warm local checkout and can be seconds on a network
/// mount or a cold worktree. The screen walks every project on the machine, and a `rev-parse` that
/// hangs would hold the whole report behind one unreachable checkout. Timing out is neither fresh
/// nor stale -- the row says `stale: null`, because a project that cannot be checked is not a
/// project that is fresh. The hooks no longer use this: `hook-suggest` stopped probing the index
/// when its suggestion was removed, and `hook-refresh` reaches git only through
/// `incremental_index`'s candidate set.
const HOOK_GIT_BUDGET_MS: u64 = 400;

/// `git rev-parse HEAD`, abandoned if it does not answer inside the budget.
///
/// The worker thread is left running when it times out. That is deliberate and bounded: the caller
/// (`cort projects`) is a one-shot command whose output does not depend on the abandoned thread,
/// and the alternative -- killing a child mid-`rev-parse` -- buys nothing.
fn git_head_quickly(root: &Path) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(git_head_of(&root));
    });
    rx.recv_timeout(std::time::Duration::from_millis(HOOK_GIT_BUDGET_MS))
        .ok()
        .flatten()
}

/// Wiring the PreToolUse hook into a Claude Code `settings.json`. `install.sh` calls this so the
/// hook ships with the skill instead of being a separate thing to remember per agent home; it is
/// not a verb anyone types either. `--remove` is what `install.sh --uninstall` calls.
fn map_json_settings_err(e: settings::SettingsError) -> CortError {
    match e {
        settings::SettingsError::Unparsable(m) => {
            CortError::new("bad_settings", json!({ "message": m }))
        }
        settings::SettingsError::Io(io) => {
            CortError::new("file_not_found", json!({ "message": io.to_string() }))
        }
    }
}

fn map_toml_settings_err(e: settings_toml::SettingsError) -> CortError {
    match e {
        settings_toml::SettingsError::Unparsable(m) => {
            CortError::new("bad_settings", json!({ "message": m }))
        }
        settings_toml::SettingsError::Io(io) => {
            CortError::new("file_not_found", json!({ "message": io.to_string() }))
        }
        e @ settings_toml::SettingsError::Unsupported(_) => {
            CortError::new("bad_flag", json!({ "message": e.to_string() }))
        }
    }
}

/// Which merge owns a settings file.
///
/// Extension alone answered this while `.toml` named exactly one harness (Codex; Claude Code and
/// Grok share one `settings.json`, `docs/2026-09-02-hook-wiring-correction.md` §6). Kimi's file is
/// also `config.toml`, with a shape that is not Codex's, so the extension rule survives only as the
/// default and Kimi has to name itself with `--format kimi`. The alternative -- sniffing the path
/// for `.kimi-code` -- would hang the answer on a string nobody controls, since `KIMI_CODE_HOME`
/// can point anywhere; `install.sh` already names all three files, so it can name the three formats.
#[derive(Clone, Copy, PartialEq)]
enum SettingsFormat {
    Json,
    CodexToml,
    KimiToml,
}

fn settings_format(
    path: Option<&Path>,
    declared: Option<&str>,
) -> Result<SettingsFormat, CortError> {
    match declared {
        None => Ok(
            if path.and_then(|p| p.extension()).and_then(|e| e.to_str()) == Some("toml") {
                SettingsFormat::CodexToml
            } else {
                SettingsFormat::Json
            },
        ),
        Some("json") => Ok(SettingsFormat::Json),
        Some("codex") => Ok(SettingsFormat::CodexToml),
        Some("kimi") => Ok(SettingsFormat::KimiToml),
        Some(other) => Err(CortError::new(
            "bad_flag",
            json!({ "message": format!("--format must be json, codex or kimi, not `{other}`") }),
        )),
    }
}

/// Where each dialect lives when `--settings` did not say. Each harness owns its own home variable
/// (`CLAUDE_SKILL_HOME`, `CODEX_HOME`, `KIMI_CODE_HOME`), so the message names the one that would
/// have answered rather than a generic "no HOME".
fn default_settings_path_for(fmt: SettingsFormat) -> Result<PathBuf, CortError> {
    let (path, vars, file) = match fmt {
        SettingsFormat::Json => (
            settings::default_settings_path(),
            "HOME or CLAUDE_SKILL_HOME",
            "settings.json",
        ),
        SettingsFormat::CodexToml => (
            settings_toml::default_settings_path(),
            "HOME or CODEX_HOME",
            "Codex config.toml",
        ),
        SettingsFormat::KimiToml => (
            settings_kimi::default_settings_path(),
            "HOME or KIMI_CODE_HOME",
            "Kimi config.toml",
        ),
    };
    path.ok_or_else(|| {
        CortError::new(
            "file_not_found",
            json!({ "message": format!("no {vars} to resolve {file} from") }),
        )
    })
}

/// Every hook entry that ships, in one place.
///
/// `install.sh` used to hold this table in bash and restate four things the binary already knew on
/// every call (`install.sh:542`): the settings path per harness, the `--format`/`--harness` pairing,
/// the event-to-subcommand mapping, and then a `sed` that re-parsed cort's own JSON reply. Bash's
/// copy was the one that shipped, so the Rust copy was never exercised and rotted -- which is
/// exactly how `--status --format kimi` came to answer `wired: false` about a file that was never
/// Kimi's. Two implementations of one rule, and the untested one is always the one that is wrong.
///
/// The same rule the hook's own history keeps paying for: a settings *parser* may be plural (one
/// module per dialect), because each speaks a different file format. The table that says which
/// harness reads which file, through which format, and which subcommand each event runs may not.
const HOOK_TARGETS: [(SettingsFormat, &str); 3] = [
    // The names come from `cort::settings::HOOK_HARNESSES` — the one home for the list
    // (`entry_shape_ok` dispatches on the same indices), not a second copy here.
    (SettingsFormat::Json, cort::settings::HOOK_HARNESSES[0]),
    (SettingsFormat::CodexToml, cort::settings::HOOK_HARNESSES[1]),
    (SettingsFormat::KimiToml, cort::settings::HOOK_HARNESSES[2]),
];

/// Wire, unwire or report every entry in `HOOK_TARGETS` in one call.
///
/// `--command-prefix` is required rather than defaulted, and that is not a convenience. The default
/// for a single `--command` is `current_exe()`, which inside the installed layout resolves to
/// `~/.local/share/cortexyoung/cort/cort` -- the real binary behind the shim. But `install.sh`
/// wires, and `check_hook_at` verifies, the *shim* at `~/.cargo/bin/cort`; a command naming the
/// real binary would read as "wired to a different binary" and fail `--check` on every machine.
/// Only the installer knows which of the two it wants named, so it has to say.
fn hook_install_all(a: &HookInstallArgs) -> Result<Emit, CortError> {
    let mut rows: Vec<Value> = Vec::new();
    for (fmt, harness) in HOOK_TARGETS {
        let path = default_settings_path_for(fmt)?;
        for &event in cort::settings::events_for(harness) {
            let mut row = json!({
                "harness": harness,
                "event": event.flag_name(),
                "settings": path.to_string_lossy(),
            });
            if a.status {
                let (wired, trusted) = status_of_entry(fmt, &path, event);
                row["wired"] = json!(wired.is_some());
                row["command"] = json!(wired);
                row["trusted"] = json!(trusted);
            } else if a.remove {
                // Removal takes all events at once in every dialect, so it runs once per file and
                // later events report what the first one did rather than re-editing.
                if event == cort::settings::HookEvent::Suggest {
                    match remove_from(fmt, &path) {
                        Ok(out) => {
                            row["change"] = json!(out.change.as_str());
                            row["backup"] =
                                json!(out.backup.map(|b| b.to_string_lossy().into_owned()));
                        }
                        Err(e) => row["error"] = json!(e.to_string()),
                    }
                } else {
                    row["change"] = json!("covered_by_pre");
                }
            } else {
                let Some(prefix) = a.command_prefix.as_deref() else {
                    return Err(CortError::new(
                        "bad_flag",
                        json!({ "message": "--all needs --command-prefix <path to the cort the harness should run>" }),
                    ));
                };
                let command = format!("{prefix} {} --harness {harness}", event.subcommand());
                match install_into(fmt, &path, &command, event) {
                    Ok(out) => {
                        row["command"] = json!(command);
                        row["change"] = json!(out.change.as_str());
                        row["backup"] = json!(out.backup.map(|b| b.to_string_lossy().into_owned()));
                    }
                    // One target's failure is reported and the rest still deploy. Independent
                    // calls had that property for free and it is worth keeping: a machine with an
                    // unparsable Codex config should still get its Claude Code hook.
                    Err(e) => row["error"] = json!(e.to_string()),
                }
            }
            rows.push(row);
        }
    }
    let payload = json!({ "entries": rows });
    if a.lean {
        return Ok(Emit {
            render_command: Some("hook-install-all-lean"),
            format: Format::Lean,
            payload: json!({ "lean": render_hook_entries_lean(&payload) }),
        });
    }
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload,
    })
}

/// One tab-separated line per entry: `harness  event  outcome  settings  detail  command`.
///
/// This exists so the installer stops re-parsing our JSON with `sed` (`install.sh:546`). A regex
/// over a serialised object is a second parser for a format we own, it is the one nobody tests, and
/// a field that grows a newline or a nested object silently changes what it captures. Five fields,
/// no quoting rules, `while IFS=$'\t' read` on the other side.
fn render_hook_entries_lean(payload: &Value) -> String {
    let mut out = String::new();
    let Some(entries) = payload.get("entries").and_then(Value::as_array) else {
        return out;
    };
    for e in entries {
        let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("");
        let outcome = if e.get("error").is_some() {
            "error"
        } else if let Some(c) = e.get("change").and_then(Value::as_str) {
            c
        } else if e.get("wired").and_then(Value::as_bool) == Some(true) {
            "wired"
        } else if e.get("wired").is_some() {
            "not_wired"
        } else {
            "unknown"
        };
        let detail = if let Some(err) = e.get("error").and_then(Value::as_str) {
            err.replace(['\t', '\n'], " ")
        } else if let Some(t) = e.get("trusted").and_then(Value::as_bool) {
            format!("trusted={t}")
        } else {
            String::new()
        };
        // `command` is last because it is the field with spaces in it, and it is present at all so
        // that `--check` can ask "is this wired to the binary I manage?" without reaching back into
        // the JSON -- which is the habit this whole format exists to break. Tabs and newlines are
        // stripped from every field: five separators, and nothing that can invent a sixth.
        //
        // No field is ever empty, and that is not cosmetic. Tab is an IFS *whitespace* character, so
        // `read` collapses a run of them into one delimiter and drops the empty field between --
        // `a\t\tb` reads as two fields, not three. An empty `detail` therefore shifted `command`
        // into `detail` on exactly the rows that had no trust to report, which is how the first
        // version of this passed for Codex and failed for the other two. `-` means empty.
        let clean = |v: &str| {
            let t = v.replace(['\t', '\n'], " ");
            if t.is_empty() {
                "-".to_string()
            } else {
                t
            }
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            clean(s("harness")),
            clean(s("event")),
            outcome,
            clean(s("settings")),
            clean(&detail),
            clean(s("command")),
        ));
    }
    out
}

fn status_of_entry(
    fmt: SettingsFormat,
    path: &Path,
    event: cort::settings::HookEvent,
) -> (Option<String>, Option<bool>) {
    match fmt {
        SettingsFormat::CodexToml => match settings_toml::installed_entry(path, event) {
            Some((command, trusted)) => (Some(command), Some(trusted)),
            None => (None, None),
        },
        SettingsFormat::KimiToml => (settings_kimi::installed_command(path, event), None),
        SettingsFormat::Json => (settings::installed_command(path, event), None),
    }
}

fn install_into(
    fmt: SettingsFormat,
    path: &Path,
    command: &str,
    event: cort::settings::HookEvent,
) -> Result<cort::settings::Outcome, CortError> {
    match fmt {
        SettingsFormat::CodexToml => {
            settings_toml::install_hook(path, command, event).map_err(map_toml_settings_err)
        }
        SettingsFormat::KimiToml => {
            settings_kimi::install_hook(path, command, event).map_err(map_toml_settings_err)
        }
        SettingsFormat::Json => {
            settings::install_hook(path, command, event).map_err(map_json_settings_err)
        }
    }
}

fn remove_from(fmt: SettingsFormat, path: &Path) -> Result<cort::settings::Outcome, CortError> {
    match fmt {
        SettingsFormat::CodexToml => {
            settings_toml::remove_hook(path).map_err(map_toml_settings_err)
        }
        SettingsFormat::KimiToml => settings_kimi::remove_hook(path).map_err(map_toml_settings_err),
        SettingsFormat::Json => settings::remove_hook(path).map_err(map_json_settings_err),
    }
}

fn cmd_internal_shim(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = InternalShimArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    Ok(Emit {
        render_command: Some("internal-shim-lean"),
        format: Format::Lean,
        payload: json!({ "lean": cort::install::render_shim(&a.cort_home) }),
    })
}

fn cmd_internal_ast_grep(_usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let prov = cort::install::ast_grep_provenance();
    Ok(Emit {
        render_command: Some("internal-ast-grep"),
        format: Format::Lean,
        payload: json!({
            "version": prov.version,
            "repo": prov.repo,
            "crate": prov.crate_name,
            "assets": prov.assets.iter().map(|(a, s)| json!([a, s])).collect::<Vec<_>>(),
        }),
    })
}

fn cmd_internal_manifest_keys(_usage: &mut UsageEvent) -> Result<Emit, CortError> {
    Ok(Emit {
        render_command: Some("internal-manifest-keys"),
        format: Format::Lean,
        payload: json!({
            "known": cort::install::MANIFEST_KEYS,
            "legacy": cort::install::MANIFEST_LEGACY_KEYS,
        }),
    })
}

fn cmd_hook_install(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = HookInstallArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    if a.all {
        return hook_install_all(&a);
    }
    // Format is decided before the path, not after it. The other order defaulted every
    // `--settings`-less invocation to Claude Code's `settings.json` and then read it as whichever
    // dialect `--format` named, so `--status --format kimi` reported `wired: false` against a file
    // that was never Kimi's -- the same false negative as `docs/2026-09-02-hook-wiring-correction.md`,
    // arriving by a different route. `install.sh` always passes `--settings` and so never saw it.
    let declared = a.settings.as_deref().map(Path::new);
    let fmt = settings_format(declared, a.format.as_deref())?;
    let path = match a.settings.as_deref() {
        Some(p) => PathBuf::from(p),
        None => default_settings_path_for(fmt)?,
    };
    let event = match a.event.as_deref() {
        None => cort::settings::HookEvent::Suggest,
        Some(s) => cort::settings::HookEvent::parse(s).ok_or_else(|| {
            CortError::new(
                "bad_flag",
                json!({ "message": format!("--event must be pre, post or session, not `{s}`") }),
            )
        })?,
    };
    // The harness for this dialect comes from HOOK_TARGETS, the one table that pairs them.
    let harness = HOOK_TARGETS
        .iter()
        .find(|(f, _)| *f == fmt)
        .map_or("", |(_, h)| *h);
    if !cort::settings::events_for(harness).contains(&event) {
        return Err(CortError::new(
            "bad_flag",
            json!({ "message": format!("{} is not supported for {harness}", event.flag_name()) }),
        ));
    }
    if a.status {
        // `trusted` is `null`, never `false`, wherever the question does not apply: Claude Code and
        // Grok have no trust gate, and a file with no entry of ours has nothing to be trusted. Only
        // a wired Codex entry can answer it, and `false` there is a hook that will not run.
        let (wired, trusted) = match fmt {
            SettingsFormat::CodexToml => match settings_toml::installed_entry(&path, event) {
                Some((command, trusted)) => (Some(command), Some(trusted)),
                None => (None, None),
            },
            SettingsFormat::KimiToml => (settings_kimi::installed_command(&path, event), None),
            SettingsFormat::Json => (settings::installed_command(&path, event), None),
        };
        return Ok(Emit {
            render_command: None,
            format: Format::Json,
            payload: json!({
                "settings": path.to_string_lossy(),
                "wired": wired.is_some(),
                "command": wired,
                "trusted": trusted,
            }),
        });
    }
    if a.remove {
        let out = match fmt {
            SettingsFormat::CodexToml => {
                settings_toml::remove_hook(&path).map_err(map_toml_settings_err)?
            }
            SettingsFormat::KimiToml => {
                settings_kimi::remove_hook(&path).map_err(map_toml_settings_err)?
            }
            SettingsFormat::Json => settings::remove_hook(&path).map_err(map_json_settings_err)?,
        };
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
            format!("{} {}", exe.to_string_lossy(), event.subcommand())
        }
    };
    let out =
        match fmt {
            SettingsFormat::CodexToml => settings_toml::install_hook(&path, &command, event)
                .map_err(map_toml_settings_err)?,
            SettingsFormat::KimiToml => settings_kimi::install_hook(&path, &command, event)
                .map_err(map_toml_settings_err)?,
            SettingsFormat::Json => {
                settings::install_hook(&path, &command, event).map_err(map_json_settings_err)?
            }
        };
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
    // Unlike the hooks, a foreground index WAITS (spec §3c: killing user foreground work
    // ranks worse than deferring — "前景動作等鎖,並告知原因"). Bounded retry with the same
    // 30s deadline the upgrader's drain uses; the two deadlines race, and whichever times
    // out first is the one that stands down — both directions are safe. On timeout the
    // index REFUSES (never proceeds unlocked: indexing through a migration mid-flip is the
    // mixed-generation read this whole protocol exists to prevent) with a structured error
    // naming why, so a human reruns instead of guessing.
    //
    // The cache dir is created first so the loop cannot spend 30s retrying a mere
    // missing-directory (open_lock does not mkdir). A cache dir that STILL cannot host a
    // lock file is not an upgrade — the storage layer names it right after, and masking
    // that with `upgrade_in_flight` would be the wrong-diagnosis sin; so that one error
    // proceeds without the guard (today's behavior, and the db open fails immediately).
    let cache = cache_dir();
    let _ = std::fs::create_dir_all(&cache);
    // The background self-healer claims the project's heal lock before anything else, so two
    // stale queries on a big repo cannot rebuild twice: the loser of the flock exits quietly
    // with the row that says it stood down. Held to the end of the function, like the upgrade
    // guard below -- dropping it early would let a second healer race a half-written graph.
    let _heal_guard: Option<std::fs::File> = if a.heal_background {
        match canonicalize_root(&root) {
            Ok(canon) => {
                usage.project_id = Some(canon.project_id.clone());
                let lock = cache.join(format!(".heal-{}.lock", canon.project_id));
                match cort::upgrade::try_exclusive_lock(&lock) {
                    Ok(guard) => {
                        usage.args_summary = json!({"v": 1, "heal": "background"}).to_string();
                        Some(guard)
                    }
                    Err(_) => {
                        usage.args_summary =
                            json!({"v": 1, "heal": "background", "skipped": "busy"}).to_string();
                        return Ok(Emit {
                            render_command: None,
                            format: Format::Json,
                            payload: json!({"heal": "background", "skipped": "busy"}),
                        });
                    }
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };
    // `_guard` reads as unused but is load-bearing: the activity flock releases when this
    // binding drops, at function end. An underscore-prefixed name keeps that lifetime while
    // telling the compiler the value is never read.
    let _guard: Option<cort::upgrade::ActivityGuard> = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match cort::upgrade::try_protected_entry(&cache) {
                Ok(g) => break Some(g),
                Err(cort::upgrade::LockError::LockFileUnavailable) => break None,
                Err(_) if std::time::Instant::now() >= deadline => {
                    return Err(CortError::new(
                        "upgrade_in_flight",
                        json!({
                            "message": "a cort upgrade is in progress and holds the index locks",
                            "next": "re-run `cort index` once the upgrade finishes",
                        }),
                    ));
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
    };
    // `guard` is held from here to the end of the function: across the db open and the index
    // work. There is no early drop — dropping it before `incremental_index` would index
    // through an upgrade that started in the gap.
    let (canon, mut db) = open_project_tracked(&root, usage)?;
    let stats = unwrap_busy(
        with_busy_retry(|| {
            if a.incremental {
                incremental_index(&mut db, &bin, &canon.path, RebuildPolicy::Allow).map_err(IdxWrap)
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
        .map_err(|e| cort::db::classify_sqlite(&e))?;
    // Spec §10 item 1, third instance: this read-only open ran with no busy timeout, so a
    // status poll under a concurrent refresh-hook write reported `storage_busy` for what is
    // contention, not corruption — six in one quarter's log, at ~1,500 refreshes a day.
    // Same 5s the rest of db.rs waits; contention now waits instead of lying.
    db.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(|e| cort::db::classify_sqlite(&e))?;
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
            "relationships": st.relationships,
            "extractor_version": st.extractor_version,
            "git_head": st.git_head,
            "last_indexed_at": st.last_indexed_at,
            "index_is_stale": stale.index_is_stale,
            "deleted_files": stale.deleted_files,
            "changed_files": stale.changed_files,
            // `deleted_files` and `changed_files` already say why a *tree* moved. This says why the
            // *index* is owed a full rebuild, which the other two cannot express: an extractor or
            // schema this binary no longer uses leaves both of them empty. Without it a person who
            // runs the obvious command learns that the index is stale and never learns that
            // re-indexing is what fixes it -- which is the whole point of the screen.
            "rebuild_required": stale.rebuild_required,
            // The narrowing fact raw, never folded into `rebuild_required`: it is a property of
            // the working tree and git, so a non-git index whose hashes all match must not read
            // as owed a rebuild. Beside the reasons, this is what the repair hook would decide.
            "candidates_narrowed": stale.candidates_narrowed,
        }),
    })
}

fn cmd_projects(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = ProjectsArgs::try_parse_from(args.iter()).map_err(clap_fail)?;

    let mut n_drifted = 0u64;
    let mut n_unreadable = 0u64;
    let mut rows: Vec<Value> = Vec::new();

    for entry in list_projects() {
        match entry {
            ProjectEntry::Unreadable { db_path, reason } => {
                n_unreadable += 1;
                rows.push(json!({ "db_path": db_path, "unreadable": reason }));
            }
            ProjectEntry::Indexed(r) => {
                // `stale` is what an installer or a person actually wants to know, and it is only
                // answerable here: the row stores the head it was built at, and the tree knows the
                // head it is on now. `null`, never `false`, when the two cannot be compared -- the
                // directory is gone, it is not a git tree, or `rev-parse` did not answer inside the
                // budget. A project that cannot be checked is not a project that is fresh.
                let exists = Path::new(&r.path).is_dir();
                let stale = match (r.git_head.as_deref(), git_head_quickly(Path::new(&r.path))) {
                    (Some(stored), Some(now)) => Some(stored != now),
                    _ => None,
                };
                let drifted = !cort::indexer::rebuild_reasons_of(
                    r.schema_version.as_deref(),
                    r.extractor_version.as_deref(),
                    r.graph_pending,
                )
                .map_err(map_index)?
                .is_empty();
                // Counted whether or not the directory still exists: the index is readable and
                // it does not match, and that is true regardless of whether anyone can act on it.
                if drifted {
                    n_drifted += 1;
                }
                rows.push(json!({
                    "project_id": r.project_id,
                    "name": r.name,
                    "path": r.path,
                    "git_head": r.git_head,
                    "last_indexed_at": r.last_indexed_at,
                    "db_path": r.db_path,
                    "exists": exists,
                    "stale": stale,
                    "schema_version": r.schema_version,
                    "extractor_version": r.extractor_version,
                    "drifted": drifted,
                }));
            }
        }
    }

    if a.verdict {
        // Drift is a fact; unreadable is the absence of one. A population with both is reported as
        // drifted, because that names the half we know -- and the other number is printed beside it
        // rather than folded into it.
        let word = if n_drifted > 0 {
            "drifted"
        } else if n_unreadable > 0 {
            "unknown"
        } else {
            "compatible"
        };
        return Ok(Emit {
            render_command: Some("projects"),
            format: Format::Lean,
            payload: json!({
                "indexes": word,
                "drifted": n_drifted,
                "unreadable": n_unreadable,
            }),
        });
    }

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
            // A row whose directory is gone is the one most worth deleting, and it is precisely
            // the one whose path cannot be canonicalised. The registry is a scan of the cache
            // directory, so it can still name the row and its database by the path recorded
            // inside it -- fall back to that before refusing.
            let want = root.to_string_lossy().trim_end_matches('/').to_string();
            if let Some(row) = cort::db::list_projects()
                .into_iter()
                .filter_map(|e| match e {
                    ProjectEntry::Indexed(r) => Some(r),
                    ProjectEntry::Unreadable { .. } => None,
                })
                .find(|r| r.path.trim_end_matches('/') == want)
            {
                usage.project_id = Some(row.project_id.clone());
                let r = cort::db::delete_project_db(std::path::Path::new(&row.db_path));
                return Ok(Emit {
                    render_command: None,
                    format: Format::Json,
                    payload: json!({
                        "deleted": r.deleted,
                        "db_path": r.db_path,
                        "path": row.path,
                        "note": "resolved from the project registry: the directory no longer exists",
                    }),
                });
            }
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
    let (canon, mut db) = open_project_tracked(&cwd(), usage)?;
    // 查詢即自癒: answer from a cache that is fresh by construction, never assign a rebuild
    // to the caller. See `heal.rs` for the measured reasons and the boundaries.
    let heal = cort::heal::ensure_fresh(&mut db, &bin, &canon.path, &canon.project_id);
    let depth = parse_i64_flag(a.depth.as_deref(), DEFAULT_DEPTH);
    // The busy-retry wrapper: a refresh hook's write can outlive the connection's busy
    // timeout, and `impact` alone used to surface that as `storage_busy` instead of waiting
    // it out.
    let mut out = unwrap_busy(
        with_busy_retry(|| {
            impact_command(&db, &bin, &canon.path, &canon.project_id, &symbol, depth)
                .map_err(CortWrap)
        }),
        |w| w.0,
    )?;
    // Recall is a separate question from cost, so it is opt-in: the default payload stays the small
    // answer the eval priced, and `--coverage` pays for a walk of the indexed files.
    if a.coverage {
        unwrap_busy(
            with_busy_retry(|| {
                coverage::attach(&db, &canon.project_id, Path::new(&canon.path), &mut out)
                    .map_err(CortWrap)
            }),
            |w| w.0,
        )?;
    }
    heal.attach_to(&mut out);
    fill_stale(usage, &out);
    usage.args_summary = heal.summarize(usage::args_summary(Some(&symbol), None, None, None));
    Ok(Emit {
        render_command: Some("impact"),
        format,
        payload: out,
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

fn cmd_memory(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = MemoryArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let value = match a.action {
        MemoryAction::Alias { action } => match action {
            AliasAction::Add(args) => serde_json::to_value(
                cort::memory_store::alias_add(&args.root, args.from())
                    .map_err(memory_alias_error)?,
            )
            .unwrap_or(Value::Null),
            AliasAction::List { root } => serde_json::to_value(
                cort::memory_store::alias_list(&root).map_err(memory_alias_error)?,
            )
            .unwrap_or(Value::Null),
            AliasAction::Remove(args) => serde_json::to_value(
                cort::memory_store::alias_remove(&args.root, args.from())
                    .map_err(memory_alias_error)?,
            )
            .unwrap_or(Value::Null),
        },
        MemoryAction::Notes { action } => match action {
            NotesAction::OptIn { root } => cort::memory_store::notes_opt_in(&root, true),
            NotesAction::OptOut { root } => cort::memory_store::notes_opt_in(&root, false),
            NotesAction::Status => cort::memory_store::notes_status(),
            NotesAction::Upload {
                dry_run,
                verbose,
                background,
            } => {
                if background {
                    cort::notes_kick::run_background();
                    Ok(json!({}))
                } else {
                    cort::memory_store::notes_upload(
                        dry_run,
                        verbose,
                        cort::memory_store::notes_lock_wait(),
                    )
                }
            }
            NotesAction::List { root } => {
                return Ok(Emit {
                    render_command: None,
                    format: Format::Json,
                    payload: cort::memory_store::notes_list(&root).map_err(memory_read_error)?,
                })
            }
            NotesAction::Show { note_id } => {
                return Ok(Emit {
                    render_command: None,
                    format: Format::Json,
                    payload: cort::memory_store::notes_show(&note_id).map_err(memory_read_error)?,
                })
            }
        }
        .map_err(memory_error)?,
        MemoryAction::Configure { url, token_stdin } => {
            if !token_stdin {
                return Err(CortError::new(
                    "memory_error",
                    json!({"message":"pass --token-stdin"}),
                ));
            }
            let mut token = String::new();
            std::io::stdin()
                .read_to_string(&mut token)
                .map_err(|e| CortError::new("memory_error", json!({"message":e.to_string()})))?;
            let path = cort::memory_store::configure(&url, &token).map_err(memory_error)?;
            json!({"status":"configured","path":path})
        }
        MemoryAction::ImportConfig { from } => {
            let path = cort::memory_store::import_config(&from).map_err(memory_error)?;
            json!({"status":"configured","path":path})
        }
        MemoryAction::Record {
            root,
            key,
            claim,
            source,
            line,
            supersedes,
            replace,
        } => serde_json::to_value(
            cort::memory_store::record_optional(
                &root,
                &key,
                &claim,
                source.as_deref().zip(line),
                &supersedes,
                replace,
            )
            .map_err(memory_ledger_error)?,
        )
        .unwrap_or(Value::Null),
        MemoryAction::Resolve {
            root,
            key,
            claim,
            source,
            line,
            supersedes,
        } => serde_json::to_value(
            cort::memory_store::resolve_optional(
                &root,
                &key,
                &claim,
                source.as_deref().zip(line),
                &supersedes,
            )
            .map_err(memory_ledger_error)?,
        )
        .unwrap_or(Value::Null),
        MemoryAction::Recall {
            query,
            root,
            harness,
            json: _,
        } => {
            usage.project_id = Some(cort::memory_store::project(&root).map_err(memory_error)?);
            usage.args_summary = json!({"v":1,"harness_declared":harness}).to_string();
            let mut report = serde_json::to_value(
                cort::memory_store::recall(&root, &query).map_err(memory_read_error)?,
            )
            .unwrap_or(Value::Null);
            report["harness_declared"] = json!(harness);
            report["remote"] = json!("ok");
            report["machine"] = json!({
                "id": cort::usage::machine_id(),
                "source": cort::usage::machine_id_source(),
            });
            report
        }
        MemoryAction::Status { root, json: _ } => {
            serde_json::to_value(cort::memory_store::status(&root).map_err(memory_read_error)?)
                .unwrap_or(Value::Null)
        }
        MemoryAction::Check { root } => {
            cort::memory_store::check(&root).map_err(memory_read_error)?
        }
    };
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: value,
    })
}

fn memory_error(message: String) -> CortError {
    CortError::new("memory_error", json!({"message":message}))
}

fn memory_read_error(error: cort::memory_store::LedgerError) -> CortError {
    use cort::memory_store::LedgerError;
    match error {
        LedgerError::Remote(kind) => CortError::new(
            "memory_remote_unavailable",
            json!({"remote":kind.as_str(),"message":format!("remote: {}",kind.as_str())}),
        ),
        other => memory_ledger_error(other),
    }
}

fn memory_alias_error(error: cort::memory_store::LedgerError) -> CortError {
    use cort::memory_store::LedgerError;
    match error {
        LedgerError::Remote(kind) => memory_read_error(LedgerError::Remote(kind)),
        LedgerError::NotRecorded { kind } => CortError::new(
            "memory_alias_not_recorded",
            json!({"kind":kind.as_str(), "message":"nothing changed; re-run when the database is reachable"}),
        ),
        LedgerError::OutcomeUnknown { kind, .. } => CortError::new(
            "memory_alias_outcome_unknown",
            json!({"kind":kind.as_str(), "message":"the alias change may or may not have been applied; run `cort memory alias list` before re-running"}),
        ),
        LedgerError::Message(message) => memory_error(message),
        other => memory_ledger_error(other),
    }
}

fn memory_ledger_error(error: cort::memory_store::LedgerError) -> CortError {
    use cort::memory_store::LedgerError;
    match error {
        LedgerError::Message(message) => memory_error(message),
        LedgerError::Remote(kind) | LedgerError::NotRecorded { kind } => CortError::new(
            "memory_not_recorded",
            json!({"kind":kind.as_str(),
                "message":format!("not recorded: {}; nothing was saved locally; re-run the same command when the database is reachable",kind.as_str())}),
        ),
        LedgerError::CommittedUnverified { id, kind } => CortError::new(
            "memory_committed_unverified",
            json!({"id":id,"kind":kind.as_str(),
                "message":format!("committed_unverified: {}; {id} was written but could not be read back; run `cort memory check` when the database is readable",kind.as_str())}),
        ),
        LedgerError::OutcomeUnknown {
            id,
            key: Some(key),
            kind,
        } => CortError::new(
            "memory_outcome_unknown",
            json!({"id":id,"key":key,"kind":kind.as_str(),
                "message":format!("outcome unknown: {}; {id} may or may not have been written; run `cort memory recall \"{key}\"` before re-running",kind.as_str())}),
        ),
        LedgerError::OutcomeUnknown { .. } => memory_error("invalid memory outcome".into()),
        LedgerError::SupersedesMissing { superseded } => {
            // Affected 0 proves only that the condition failed: for a resolution, that at least
            // one of the ids is missing, not which.
            let message = match superseded.as_slice() {
                [one] => format!(
                    "supersedes_missing: {one} is not in the shared database; nothing was recorded"
                ),
                many => format!(
                    "supersedes_missing: not all of {} are in the shared database; nothing was recorded",
                    many.join(", ")
                ),
            };
            let detail = if superseded.len() == 1 {
                json!({"superseded":superseded[0],"message":message})
            } else {
                json!({"superseded":superseded,"message":message})
            };
            CortError::new("memory_supersedes_missing", detail)
        }
        LedgerError::Conflict {
            id,
            key,
            active_ids,
        } => {
            let message = format!(
                "conflict: {id} was recorded, but key \"{key}\" now has {} active entries; settle them with `cort memory resolve`",
                active_ids.len()
            );
            CortError::new(
                "memory_conflict",
                json!({"id":id,"key":key,"active_ids":active_ids,"message":message}),
            )
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct MeasureArgs {
    #[command(subcommand)]
    action: MeasureAction,
}

#[derive(Subcommand, Debug)]
enum MeasureAction {
    Configure {
        #[arg(long)]
        url: String,
        #[arg(long, required = true)]
        token_stdin: bool,
    },
    /// Local detail for one transcript (session id) or one compaction (boundary uuid).
    Show { id: String },
    /// Read local collection state and check remote indexes.
    Status {
        #[arg(short = 'f', long = "format", default_value = "json", value_parser = ["json", "lean"])]
        format: String,
    },
    /// Scan local transcripts and upload them to the measurement store.
    Record,
    /// Scan local transcripts and upload batch events; a kicked child appends at most one line.
    Collect {
        #[arg(long)]
        background: bool,
    },
    /// Observe recently used indexes, read-only, and write local index_health_log; no network.
    Health {
        /// For the kicked child: append the result to measure.log without stdout.
        #[arg(long)]
        background: bool,
    },
    /// Manually install or remove the legacy cron entry; hooks' kick is the only automatic trigger.
    Cron {
        #[arg(long, group = "cron_action")]
        install: bool,
        #[arg(long, group = "cron_action")]
        uninstall: bool,
        #[arg(long, group = "cron_action")]
        status: bool,
        /// For manual install, skip when the measurement store is not configured.
        #[arg(long)]
        if_configured: bool,
        /// The command the crontab line runs: the installed shim. Required, never `current_exe()`.
        #[arg(long)]
        command_prefix: Option<String>,
    },
    /// Cross-machine report with compaction cost and gate handoff sections.
    Report {
        #[arg(long, default_value_t = 7)]
        days: i64,
        #[arg(long, default_value_t = 30)]
        compaction_days: i64,
        #[arg(long)]
        json: bool,
    },
}

/// Where events come from on this machine; `None` when even the default `usage.db` path is unknown.
fn event_inputs() -> Option<cort::telemetry::UploadInputs> {
    Some(cort::telemetry::UploadInputs {
        usage_db: usage::usage_db_path()?,
        cost_dir: cort::telemetry::default_cost_dir(),
        progress_path: cort::data_dir::data_dir().join("measure-progress.json"),
        machine_id: usage::machine_id().to_string(),
    })
}

/// A failed event step does not fail `record` (the compaction rows are independent),
/// so it must not pass silently either: say so on stderr.
fn warn_events(scan: &cort::measure::ScanSummary) {
    if let Some(events) = &scan.events {
        if events["status"] != "uploaded" {
            eprintln!("measure record: events not uploaded: {events}");
        }
    }
}

fn manual_error(error: cort::collect::ManualError, ordinary_code: &'static str) -> CortError {
    let code = if error.code() == "measure_collect_busy" {
        error.code()
    } else {
        ordinary_code
    };
    CortError::new(code, json!({"message":error.message()}))
}

/// A store failure from a human-run measure command. A remote one is named by its kind only;
/// anything the kind cannot place is not called a remote outage, and keeps its message.
fn measure_remote_error(message: String, local: Option<Value>) -> CortError {
    let kind = cort::measure_store::remote_error_kind(&message);
    let (code, mut detail) = if kind == "other" {
        ("measure_error", json!({"message": message}))
    } else {
        ("measure_remote_unavailable", json!({"kind": kind}))
    };
    if let Some(local) = local {
        detail["local"] = local;
    }
    CortError::new(code, detail)
}

/// `code` is the one each command reports for an unconfigured store, checked before any lock:
/// `measure_not_configured` for `status`,
/// `measure_error` everywhere else.
fn require_measure_config(code: &'static str) -> Result<(), CortError> {
    if cort::measure_store::is_configured() {
        Ok(())
    } else {
        Err(CortError::new(
            code,
            json!({
                "message": "Turso measurement is not configured; run `cort measure configure --url URL --token-stdin`"
            }),
        ))
    }
}

fn cmd_measure(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = MeasureArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    let value = match a.action {
        MeasureAction::Configure { url, token_stdin } => {
            if !token_stdin {
                return Err(CortError::new(
                    "measure_error",
                    json!({"message":"pass --token-stdin"}),
                ));
            }
            let mut token = String::new();
            std::io::stdin()
                .read_to_string(&mut token)
                .map_err(|e| CortError::new("measure_error", json!({"message":e.to_string()})))?;
            let path = cort::measure_store::configure(&url, &token)
                .map_err(|m| CortError::new("measure_error", json!({"message":m})))?;
            // Collection is kicked by the hook, so the legacy cron entry is no longer wanted:
            // say whether one is still there and how to retire it. Never touch the crontab here
            // and never fail configure over it.
            let mut value = json!({"status":"configured","path":path});
            match cort::cron::read_crontab() {
                Ok(text) => {
                    let installed = text.lines().any(cort::cron::is_measure_entry);
                    value["cron_installed"] = json!(installed);
                    if installed {
                        value["next"] = json!("cort measure cron --uninstall");
                    }
                }
                Err(e) => {
                    value["cron_installed"] = Value::Null;
                    value["cron_error"] = json!(e.message());
                }
            }
            value
        }
        MeasureAction::Show { id } => {
            cort::measure::compaction_details(&cort::measure::claude_projects_dir(), &id)
                .map_err(|m| CortError::new("measure_error", json!({"message":m})))?
        }
        MeasureAction::Status { format } => {
            require_measure_config("measure_not_configured")?;
            let paths = cort::collect::KickPaths::current().ok_or_else(|| {
                CortError::new(
                    "measure_error",
                    json!({"message":"usage.db path unavailable"}),
                )
            })?;
            let mut value = cort::collect::collection_status(&paths);
            value["progress"] = json!(match cort::measure_progress::load(
                &paths.data_dir.join("measure-progress.json")
            ) {
                cort::measure_progress::Load::Loaded(_) => "loaded",
                cort::measure_progress::Load::Missing => "missing",
                cort::measure_progress::Load::Corrupt => "corrupt",
            });
            match cort::measure_store::read_measure_store(|store| store.index_names()) {
                Ok(names) => {
                    value["remote"] = json!({
                        "missing_indexes": cort::measure_store::missing_indexes(&names)
                    })
                }
                Err(message) => {
                    value["remote"] =
                        json!({"error": cort::measure_store::remote_error_kind(&message)});
                    return Err(measure_remote_error(message, Some(value)));
                }
            }
            if format == "lean" {
                return Ok(Emit {
                    render_command: Some("internal-shim-lean"),
                    format: Format::Json,
                    payload: json!({"lean": cort::measure::render_status(&value)}),
                });
            }
            value
        }
        MeasureAction::Cron {
            install,
            uninstall,
            status,
            if_configured,
            command_prefix,
        } => {
            let fail = |m: &str| CortError::new("measure_error", json!({"message":m}));
            let need_prefix = || {
                command_prefix
                    .clone()
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| fail("pass --command-prefix"))
            };
            let action = if install {
                cort::cron::CronAction::Install {
                    prefix: need_prefix()?,
                    if_configured,
                }
            } else if uninstall {
                cort::cron::CronAction::Uninstall
            } else if status {
                cort::cron::CronAction::Status {
                    prefix: need_prefix()?,
                }
            } else {
                return Err(fail("pass one of --install, --uninstall, --status"));
            };
            // The error envelope goes to stdout; callers that capture only stderr (the upgrader)
            // would otherwise warn with an empty reason.
            let value = cort::cron::run(action).map_err(|m| {
                eprintln!("measure cron: {m}");
                fail(&m)
            })?;
            if value["status"] == "skipped" {
                return Ok(Emit {
                    render_command: Some("internal-shim-lean"),
                    format: Format::Json,
                    payload: json!({"lean": value["message"]}),
                });
            }
            value
        }
        MeasureAction::Record => {
            require_measure_config("measure_error")?;
            let paths = cort::collect::KickPaths::current().ok_or_else(|| {
                CortError::new("measure_error", json!({"message":"cannot locate usage.db"}))
            })?;
            let cfg = cort::collect::CollectConfig::from_env();
            let now_ms = usage::now_ms();
            let input = cort::measure::RecordInputs {
                projects_dir: cort::measure::claude_projects_dir(),
                marks_path: cort::measure::Watermarks::default_path(),
                machine_id: usage::machine_id().to_string(),
                host: cort::measure::host_name(),
                now_ms,
                events: event_inputs(),
                health: None,
            };
            let (scan, outcome) = cort::collect::run_manual(
                &mut || {
                    cort::measure_store::open_measure_store_within_timeout(
                        std::time::Duration::from_millis(cfg.manual_wait_ms),
                        std::time::Duration::from_millis(cfg.net_timeout_ms),
                    )
                },
                &paths,
                now_ms,
                &cfg,
                &mut |store| {
                    let (scan, outcome) =
                        cort::measure::record_with_scan(store, &input).map_err(|e| {
                            let kind = cort::collect::classify_error(&e);
                            (e, kind)
                        })?;
                    let failure = match &outcome {
                        cort::measure::RecordOutcome::Uploaded => None,
                        cort::measure::RecordOutcome::UploadFailed { kind, .. } => Some(*kind),
                    };
                    Ok(cort::collect::ManualStep {
                        events: scan.events.clone(),
                        failure,
                        value: (scan, outcome),
                    })
                },
            )
            .map_err(|e| manual_error(e, "measure_error"))?;
            warn_events(&scan);
            match outcome {
                cort::measure::RecordOutcome::Uploaded => {
                    json!({"status":"recorded","scan":scan.to_json()})
                }
                cort::measure::RecordOutcome::UploadFailed { kind: _, error } => {
                    return Err(CortError::new(
                        "measure_upload_failed",
                        json!({"message": error, "scan": scan.to_json()}),
                    ));
                }
            }
        }
        MeasureAction::Collect { background } => {
            let now_ms = usage::now_ms();
            let paths = cort::collect::KickPaths::current().ok_or_else(|| {
                CortError::new(
                    "measure_collect_failed",
                    json!({"message":"cannot locate usage.db"}),
                )
            })?;
            let cfg = cort::collect::CollectConfig::from_env();
            let harness = std::env::var(cort::collect::HARNESS_ENV)
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".to_string());
            // run_collect holds measure-collect.lock before this opener obtains measure.lock.
            let mut opener = || {
                cort::measure_store::open_measure_store_within_timeout(
                    std::time::Duration::from_millis(cfg.manual_wait_ms),
                    std::time::Duration::from_millis(cfg.net_timeout_ms),
                )
            };
            let run = if background {
                cort::collect::run_background_collect(&mut opener, &paths, &harness, now_ms, &cfg)
                    .map_err(|message| {
                    CortError::new("measure_collect_failed", json!({"message":message}))
                })?
            } else {
                cort::collect::run_collect(&mut opener, &paths, &harness, now_ms, &cfg, false)
            };
            run.summary
        }
        MeasureAction::Health { background } => {
            let now_ms = usage::now_ms();
            let result = cort::collect::KickPaths::current()
                .ok_or_else(|| "cannot locate usage.db".to_string())
                .and_then(|paths| {
                    cort::health::run_health(
                        &paths,
                        usage::machine_id(),
                        usage::machine_id_source(),
                        now_ms,
                        &cort::collect::CollectConfig::from_env(),
                        &mut cort::collect::DetachedSpawner,
                    )
                });
            // Every worker Err means this pass did not finish and did not advance its cursor.
            let (summary, error) = match result {
                Ok(run) => (
                    json!({"lane":"health","ts_ms":now_ms,"observed":run.observed,
                        "written":run.written,"unmapped":run.unmapped,
                        "skipped_busy":run.skipped_busy}),
                    None,
                ),
                Err(message) => (
                    json!({"lane":"health","ts_ms":now_ms,"error":message}),
                    Some(message),
                ),
            };
            if background {
                cort::collect::append_measure_log(
                    &cort::data_dir::data_dir(),
                    &summary.to_string(),
                )
                .map_err(|message| {
                    CortError::new("measure_health_failed", json!({"message":message}))
                })?;
            }
            if let Some(message) = error {
                return Err(CortError::new(
                    "measure_health_failed",
                    json!({"message":message}),
                ));
            }
            summary
        }
        MeasureAction::Report {
            days,
            compaction_days,
            json: as_json,
        } => {
            require_measure_config("measure_error")?;
            let now_ms = usage::now_ms();
            // Always a value when run here, so the first section is never silently missing.
            let collection = Some(match cort::collect::KickPaths::current() {
                Some(paths) => cort::collect::collection_status(&paths),
                None => json!({"unavailable": "cannot locate usage.db"}),
            });
            let value = cort::measure_store::with_turso_store(|store| {
                cort::measure::report(
                    store,
                    &cort::measure::ReportInputs {
                        days,
                        compaction_days,
                        now_ms,
                        collection,
                    },
                )
            })
            .map_err(|m| measure_remote_error(m, None))?;
            if !as_json {
                use std::io::IsTerminal;
                let color =
                    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
                return Ok(Emit {
                    render_command: Some("internal-shim-lean"),
                    format: Format::Json,
                    payload: json!({"lean": cort::measure::render_report(&value, color)}),
                });
            }
            value
        }
    };
    Ok(Emit {
        render_command: None,
        format: Format::Json,
        payload: value,
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
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let recall_hook = args.first().is_some_and(|a| a == "hook-recall");
    let claim = Arc::new(AtomicBool::new(false));
    if recall_hook {
        // A timeout exits before the normal usage writer; that missing row is intentional.
        // The process must never hold up a session start, even if stdin or storage stalls.
        // It fires 900 ms after the recall deadline, so a read the deadline allows still gets to
        // print its notice and row. Losing the claim means the main path is writing stdout and
        // its row; it gets 300 ms more for that, then the process exits anyway (a stuck pipe or
        // usage.db). The two add up to the harnesses' 5 s hook limit, as 3 s + 2 s did before.
        let watchdog_claim = Arc::clone(&claim);
        if std::thread::Builder::new()
            .name("hook-recall-watchdog".into())
            .spawn(move || {
                std::thread::sleep(
                    cort::remote::RECALL_DEADLINE + std::time::Duration::from_millis(900),
                );
                if watchdog_claim
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    std::process::exit(0);
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                std::process::exit(0);
            })
            .is_err()
        {
            let mut usage_ev = usage_from_args(&args);
            usage_ev.args_summary = hook_row("failed", "unspecified", None, None).to_string();
            finish_record(&usage_ev, "ok", None, 0);
            return;
        }
    }
    // A kicked `measure {health,collect} --background` child prints nothing on any path, the early
    // `--help`/`--version` returns included: its stdio is null and `measure.log` is its only output.
    let background_measure = args.first().is_some_and(|a| a == "measure")
        && args.get(1).is_some_and(|a| a == "health" || a == "collect")
        && args
            .iter()
            .any(|a| a == "--background" || a.starts_with("--background="));
    let background_notes = args.first().is_some_and(|a| a == "memory")
        && args.get(1).is_some_and(|a| a == "notes")
        && args.get(2).is_some_and(|a| a == "upload")
        && args.iter().any(|a| a == "--background");
    let background_measure = background_measure || background_notes;
    if wants_help(&args) {
        let usage = if args.first().is_some_and(|a| a == "memory") {
            narrow_usage(memory_usage_value(), &args[1..])
        } else {
            narrow_usage(usage_value(), &args)
        };
        if !background_measure {
            print!("{}", render(None, Format::Json, &usage));
        }
        return;
    }
    // Before `usage_from_args`, for the reason `--help` is: asking a binary its version must not
    // create a cache directory, a project database or a usage row.
    if wants_version(&args) {
        if !background_measure {
            println!("cort {} (rust)", env!("CARGO_PKG_VERSION"));
        }
        return;
    }
    let mut usage_ev = usage_from_args(&args);
    match dispatch(&args, &mut usage_ev, started) {
        Ok(emitted) => {
            let rendered = render_emit(&emitted);
            if recall_hook
                && claim
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                loop {
                    std::thread::park();
                }
            }
            if !background_measure {
                if recall_hook {
                    let mut stdout = std::io::stdout().lock();
                    let _ = stdout.write_all(rendered.as_bytes());
                    let _ = stdout.flush();
                } else {
                    print!("{rendered}");
                }
            }
            finish_record(
                &usage_ev,
                "ok",
                None,
                if background_measure {
                    0
                } else {
                    rendered.len()
                },
            );
        }
        Err(err) => {
            let format = peek_format(&args);
            let rendered = render_error(format, &err);
            if !background_measure {
                print!("{rendered}");
            }
            finish_record(
                &usage_ev,
                "error",
                Some(&err.code),
                if background_measure {
                    0
                } else {
                    rendered.len()
                },
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{memory_alias_error, memory_ledger_error};
    use cort::memory_store::LedgerError;
    use cort::remote::ErrorKind;
    use serde_json::json;

    #[test]
    fn alias_errors_use_alias_specific_guidance() {
        let cases = [
            (
                LedgerError::Remote(ErrorKind::Timeout),
                "memory_remote_unavailable",
            ),
            (
                LedgerError::NotRecorded {
                    kind: ErrorKind::Timeout,
                },
                "memory_alias_not_recorded",
            ),
            (
                LedgerError::OutcomeUnknown {
                    id: "old".into(),
                    key: None,
                    kind: ErrorKind::Timeout,
                },
                "memory_alias_outcome_unknown",
            ),
            (
                LedgerError::Message("alias is not linked".into()),
                "memory_error",
            ),
            (
                LedgerError::SupersedesMissing {
                    superseded: vec!["id".into()],
                },
                "memory_supersedes_missing",
            ),
        ];
        for (input, code) in cases {
            let error = memory_alias_error(input);
            assert_eq!(error.code, code);
            let message = error.detail["message"].as_str().unwrap();
            for forbidden in ["import-legacy", "recall", "invalid memory outcome"] {
                assert!(!message.contains(forbidden), "{message}");
            }
        }
    }

    #[test]
    fn one_missing_supersedes_names_the_id_and_records_nothing() {
        let error = memory_ledger_error(LedgerError::SupersedesMissing {
            superseded: vec!["abc123".into()],
        });
        assert_eq!(error.code, "memory_supersedes_missing");
        assert_eq!(
            error.detail,
            json!({
                "superseded": "abc123",
                "message": "supersedes_missing: abc123 is not in the shared database; nothing was recorded"
            })
        );
    }

    #[test]
    fn several_missing_supersedes_do_not_claim_which_one_is_missing() {
        let error = memory_ledger_error(LedgerError::SupersedesMissing {
            superseded: vec!["a1".into(), "b2".into()],
        });
        assert_eq!(error.code, "memory_supersedes_missing");
        assert_eq!(error.detail["superseded"], json!(["a1", "b2"]));
        assert_eq!(
            error.detail["message"],
            "supersedes_missing: not all of a1, b2 are in the shared database; nothing was recorded"
        );
    }
}
