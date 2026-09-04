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
use cort::indexer::{
    canonicalize_root, full_index, git_head_of, status_of, CanonicalRoot, IndexError,
};
use cort::r#struct::{struct_command, StructOptions};
use cort::readings::{parse_content_mode, read_fragment, recall_readings, ContentMode};
use cort::render::{parse_format, render, render_error, Format};
use cort::settings;
use cort::settings_kimi;
use cort::settings_toml;
use cort::staleness::compute_stale;
use cort::usage::{self, CommandRecord};
use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// `hook-suggest` is here so its rows are recorded under their own name rather than `_unknown`:
// whether the harness hook fires, and whether a fire is followed by an `impact` call, is the one
// measurement that has a numerator. It is not a verb anyone types.
const KNOWN_COMMANDS: [&str; 13] = [
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
    "hook-refresh",
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
            "hook-refresh": "cort hook-refresh  (PostToolUse hook: brings this project's index up to the tree after an edit; silent, never creates an index; not a verb to type)",
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
        IndexError::Sqlite(e) => cort::db::classify_sqlite(&e),
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
            let db =
                open_db(db_path_for(&canon.path_str)).map_err(|e| cort::db::classify_sqlite(&e))?;
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
    } else if emit.render_command == Some("hook-install-all-lean") {
        // Emitted raw. Wrapping these lines in a JSON string would put the installer back where it
        // started -- reaching into a serialised object with a regex -- which is the whole thing
        // `--all --lean` exists to stop.
        emit.payload
            .get("lean")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
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
    /// Which of the two hooks to act on: `pre` (the search suggestion) or `post` (the index
    /// refresh). Defaults to `pre`, so every existing caller keeps its meaning. `--remove` ignores
    /// it and takes both out, because uninstalling is one act.
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
        Some("hook-suggest") => cmd_hook_suggest(&args[1..], usage),
        Some("hook-refresh") => cmd_hook_refresh(&args[1..], usage),
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
fn hook_row(outcome: &str, harness: &str, declared: Option<&str>, model: Option<&str>) -> Value {
    let mut v = json!({ "v": 3, "hook": outcome, "harness": harness });
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

/// The model that answered, as the harness named it in its own payload. Never inferred: a guess
/// here is worse than the absence, because absence is visible and a wrong model name is not.
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
/// Whether to wire Kimi at all is a separate decision, not a leftover. Its `PreToolUse` keeps only
/// results whose `action` is `block` and discards every allow-shaped one before the model sees it
/// (`blockDecision`; every other hook event drops its result outright), and `cort hook-suggest`
/// never blocks -- so an entry there would have to trade that rule for a deny. That is a contract
/// change, not a third call to the same installer. `docs/2026-09-02-hook-wiring-correction.md` §14.
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

/// The search a hook payload describes, on either surface.
///
/// `tool_input.command` is a shell line and is parsed as one. A payload without it may still be a
/// search: Kimi's `Grep` tool carries `pattern`/`path`/`glob` and the context flags as fields, and
/// on that harness it is the *majority* surface -- 834 structured calls against 32 shell greps in
/// the local corpus (`docs/2026-09-02-hook-wiring-correction.md` §15). Both parsers live in
/// `cort::hook` beside the one `judge` they feed, so adding a surface never adds a second verdict.
fn search_of_payload(v: &Value) -> Option<cort::hook::Search> {
    let input = v.get("tool_input")?;
    if let Some(command) = input.get("command").and_then(Value::as_str) {
        return cort::hook::search_from_shell(command);
    }
    if v.get("tool_name").and_then(Value::as_str) != Some("Grep") {
        return None;
    }
    let field = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    cort::hook::search_from_grep_fields(
        field("pattern")?,
        field("path"),
        field("glob"),
        ["-A", "-B", "-C"].iter().any(|k| input.get(*k).is_some()),
    )
}

/// Has this session already been told about this symbol?
///
/// Only Kimi needs the question, and it needs it because of what its `PreToolUse` will carry: it
/// keeps only results whose `action` is `block` and discards every allow-shaped one before the
/// model sees it, so on that harness a suggestion has to arrive as a deny or not at all. A deny
/// that repeats is a loop -- the agent re-issues the search, gets stopped again, and never gets its
/// answer. Firing once per session per symbol turns the cost of a false positive into one extra
/// turn, which is what a suggestion already costs everywhere else.
///
/// A live probe on 2026-09-02 recorded the intended sequence: `Grep cmd_hook_install` denied, then
/// `cort impact --symbol cmd_hook_install …` run by the model of its own accord, then the same grep
/// re-issued and allowed. State lives beside the index rather than in `usage.db` because it is a
/// gate, not a measurement: losing it costs one extra deny, and nothing reads it afterwards.
fn gate_already_fired(session_id: &str, symbol: &str) -> bool {
    let Some(dir) =
        cort::usage::usage_db_path().and_then(|p| p.parent().map(|d| d.join("hook-gate")))
    else {
        // No cache directory means no memory of a previous fire, and a deny we cannot remember is
        // the loop this gate exists to prevent. Treat it as already fired: stay silent.
        return true;
    };
    let safe: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(128)
        .collect();
    if safe.is_empty() {
        return true;
    }
    let file = dir.join(safe);
    let seen = std::fs::read_to_string(&file).unwrap_or_default();
    if seen.lines().any(|l| l == symbol) {
        return true;
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return true;
    }
    let mut line = seen;
    line.push_str(symbol);
    line.push('\n');
    // A failed write is the same unrememberable deny as a missing directory.
    std::fs::write(&file, line).is_err()
}

/// A `PostToolUse` hook on the edit tools, not a verb anyone types. It brings this project's index
/// up to the tree it now describes, and says nothing either way.
///
/// Why it exists. The `PreToolUse` hook could already tell that the index was behind, and all it
/// could do was say so -- and measured, saying so is worth almost nothing: 19 `cort index` runs
/// against 2,700+ hook fires in 90 days on this machine, and in the one live run where a model read
/// the warning it re-ran its grep instead. Worse, the staleness that hook reports compares git
/// heads, so the window in which a file is edited and not yet committed -- most of the time anyone
/// is working -- reads as fresh while the answers are already wrong. That window is where the
/// 2026-09-02 `impact` run put a caller at line 467 that was really at 482.
///
/// Three rules, and each one is a refusal:
///
/// * **It never creates an index.** No row in `projects` means this tree was never indexed on
///   purpose, and a hook that starts indexing whatever directory an agent edited in would be a
///   side effect nobody asked for. Same gate, same reason, as `hook-suggest`'s.
/// * **It gives up rather than wait.** Another refresh, or a real `cort index`, may hold the write
///   lock; a busy database is a reason to do nothing, not to block the agent's next tool call.
///   Nothing is lost -- the next edit refreshes it.
/// * **It is silent and always succeeds.** Exit 0, `{}` on stdout, whatever happened. A
///   `PostToolUse` hook that reports failure would put an error in front of the user for a cache
///   they did not ask about, on a tool call that already succeeded.
///
/// The cost was measured before it was built: 23-37ms when nothing changed (the common case, since
/// most edits touch one file that is already current by the time a second tool call lands), ~206ms
/// after one edited file. `incremental_index` does its work in transactions, so the harness killing
/// this at its timeout rolls back rather than leaving a half-written graph.
fn cmd_hook_refresh(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    // Same `--harness` the installer already passes on this entry's command line (`install.sh`
    // builds both events' commands from one template), and same `v: 2` row shape `hook-suggest`
    // writes. It was being discarded: the argument list arrived as `_args` and the summary was a
    // bare `outcome=`, so once three harnesses were wired every reindex became an anonymous row and
    // "is Kimi's post-hook actually firing?" had no answer in the data. That is the defect
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
    let quiet = |outcome: &str, usage: &mut UsageEvent| {
        usage.args_summary = hook_args(
            outcome,
            &harness,
            declared_differs.as_deref(),
            model.as_deref(),
        );
        Ok(Emit {
            payload: json!({}),
            format: Format::Lean,
            render_command: Some("hook-refresh"),
        })
    };

    if index_state() == IndexState::Missing {
        return quiet("no_index", usage);
    }
    // The same helpers `cmd_index` uses, deliberately: opening the database by anything other than
    // `db_path_for` silently addresses a different file, which is exactly how the first version of
    // this function reported success while refreshing nothing.
    let Ok(bin) = pin_bin() else {
        return quiet("no_ast_grep", usage);
    };
    let Ok((canon, mut db)) = open_project_tracked(&cwd(), usage) else {
        return quiet("db_unavailable", usage);
    };
    match incremental_index(&mut db, &bin, &canon.path) {
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
            usage.args_summary = row.to_string();
            Ok(Emit {
                payload: json!({}),
                format: Format::Lean,
                render_command: Some("hook-refresh"),
            })
        }
        Ok(_) => quiet("already_current", usage),
        Err(_) => quiet("busy_or_failed", usage),
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
fn cmd_hook_suggest(args: &[String], usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let declared = HookSuggestArgs::try_parse_from(args.iter())
        .ok()
        .and_then(|a| a.harness)
        .unwrap_or_else(|| "unspecified".to_string());
    // Every invocation used to record the same `hook` summary whether it injected or stayed
    // silent, so `usage` could only ever report how often the hook *ran* -- which is how often the
    // agent ran any Bash command. The injection count then had exactly one source, the transcript,
    // and the 09-01 "two independent sources agree" cross-check was two readings of the same
    // number. The outcome splits the row by what the hook did, which makes `hit` a second source
    // for the numerator and names the one silence that is a missed opportunity rather than a
    // correct pass: `no_index`, the rule fired on a project cort has never indexed.
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
    // A payload with no `tool_input` at all is one this hook could not read, and stays
    // `no_payload`. One it can read but has nothing to say about is `no_shape`. Conflating the two
    // would leave the funnel unable to tell "the hook never saw the command" from "the hook saw it
    // and correctly stayed quiet" -- and the second is the number that says the rule is selective.
    if v.get("tool_input").is_none() {
        return quiet();
    }
    let (harness, declared_differs) = harness_of(&v, &declared);
    let model = model_of_payload(&v);
    let harness_args =
        |outcome: &str| hook_args(outcome, &harness, declared_differs.as_deref(), model);
    usage.args_summary = harness_args("no_shape");
    let Some(search) = search_of_payload(&v) else {
        return quiet();
    };
    let hit = match cort::hook::judge(&search, |_| cort::hook::Evidence::Unknown) {
        cort::hook::Verdict::Fire(hit) => hit,
        cort::hook::Verdict::Silent(_) => return quiet(),
    };
    // The gate is "this project has an index", which is what `cort status` means by
    // `indexed: true` -- a row in `projects`. It used to be "a db file exists", and those are not
    // the same claim: opening a project creates the schema, so a db with 0 chunks satisfied the
    // file test and the hook then told the agent `cort has an index for this project` on a tree
    // where `impact` can only answer `no_seed_resolved / stale=true`. That is the exact failure
    // the doc comment above forbids, and it was live on this machine on 2026-09-02.
    usage.args_summary = harness_args("no_index");
    let state = index_state();
    if state == IndexState::Missing {
        return quiet();
    }
    // A stale index is still worth suggesting -- most seeds resolve, and `impact` discloses
    // `stale=true` itself -- but the suggestion must not arrive claiming more than it has. Every
    // `impact` row recorded on this machine up to 2026-09-02 ran against a stale index, and the
    // injected line said only "cort has an index", which is the half of the sentence that flatters
    // the tool. The outcome is recorded separately so the mining can tell the two apart.
    let stale = state == IndexState::BehindHead;
    usage.args_summary = harness_args(if stale { "hit_stale" } else { "hit" });
    let context = format!(
        "cort has an index for this project{}. `cort impact --symbol '{}' --depth 1 --coverage -f lean` \
answers who calls it in one call, and `--coverage` lists what the enumeration could not see -- which \
a grep cannot tell you. Use it before concluding nothing else uses this; keep the grep for anything \
literal.",
        if stale {
            ", but it was built on an older commit, so it will answer `stale=true` and may miss \
edges added since -- re-run `cort index` first if the answer has to be complete"
        } else {
            ""
        },
        hit.symbol
    );
    // Kimi is the one harness where a suggestion cannot arrive as a suggestion. Its `PreToolUse`
    // keeps only results whose `action` is `block` and drops every allow-shaped one before the
    // model sees it, so `additionalContext` there reaches nobody. The contract is therefore
    // deliberately different on this harness and only on it: deny once per session per symbol,
    // carrying the same sentence as the reason, and yield on every later attempt. The reason text
    // says so explicitly, because a stop the agent cannot get past is a worse failure than any
    // false positive this rule can produce.
    if harness == "kimi-code" {
        let session = v
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if gate_already_fired(session, &hit.symbol) {
            usage.args_summary = harness_args("hit_yielded");
            return quiet();
        }
        return Ok(Emit {
            payload: json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": format!(
                        "{context} This search was not run. Issue exactly the same search again and \
            it will run -- this fires once per symbol per session."
                    ),
                },
            }),
            format: Format::Json,
            render_command: Some("hook-suggest"),
        });
    }
    // `suppressOutput` keeps the raw JSON out of the user's transcript view. Codex 0.152.1 rejects
    // the whole output when it is present -- the hook is reported `Failed` and the context never
    // reaches the model -- even though its own embedded `pre-tool-use.command.output` schema lists
    // the field. Bisected on 2026-09-02 by emitting the shapes one at a time: `{}`, `continue`,
    // `hookSpecificOutput` alone, and `hookSpecificOutput` with `additionalContext` all report
    // `Completed` and deliver; adding `suppressOutput` is the single change that fails. Claude Code
    // and Grok both accept it and both suppress the noise, so it is dropped only where it breaks.
    let mut payload = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": context,
        },
    });
    if harness != "codex" {
        payload["suppressOutput"] = json!(true);
    }
    Ok(Emit {
        payload,
        format: Format::Json,
        render_command: Some("hook-suggest"),
    })
}

/// Does the cwd's project have an index with a `projects` row -- the same question `cort status`
/// answers as `indexed`. Every failure is a `false`: a hook that cannot read the index has nothing
/// to suggest, and must never turn a broken cache into a broken tool call.
/// How long the hook will wait for git before deciding it cannot tell.
///
/// `git rev-parse HEAD` is milliseconds on a warm local checkout and can be seconds on a network
/// mount or a cold worktree. This runs inside a PreToolUse hook the harness gives 5 seconds, and
/// spending that budget to decide whether to add one sentence would cost the injection entirely --
/// which the funnel would then read as the hook correctly staying silent. Timing out is neither
/// fresh nor stale; it is `HeadUnknown`.
const HOOK_GIT_BUDGET_MS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexState {
    /// No index the hook may speak for. Every failure lands here.
    Missing,
    /// Indexed, and `git rev-parse HEAD` returned the commit the index was built on.
    ///
    /// Deliberately not called `Fresh`. It is "the same commit was observed", which is weaker: a
    /// tree at that commit with uncommitted edits is stale and lands here, and so does a tree whose
    /// head could not be read at all (no git, or git too slow -- see `HOOK_GIT_BUDGET_MS`). The
    /// recorded `hit` outcome inherits exactly that meaning and no more; `impact` still computes
    /// real staleness and still reports `stale=true` on those trees.
    HeadMatches,
    /// Indexed on a different commit. Provably behind: the full `compute_stale` walk is too
    /// expensive for a hook with a 5s budget, and one `git rev-parse HEAD` catches the case that
    /// actually bites -- an index left behind by commits made since it was built.
    BehindHead,
}

/// `git rev-parse HEAD`, abandoned if it does not answer inside the hook's budget.
///
/// The worker thread is left running when it times out. That is deliberate and bounded: this
/// process is a hook that exits within milliseconds of returning, so the thread cannot outlive
/// anything, and the alternative -- killing a child mid-`rev-parse` -- buys nothing.
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

fn index_state() -> IndexState {
    let Ok(canon) = canonicalize_root(cwd()) else {
        return IndexState::Missing;
    };
    let path = db_path_for(&canon.path_str);
    if !path.exists() {
        return IndexState::Missing;
    }
    let Ok(db) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return IndexState::Missing;
    };
    let Ok(status) = status_of(&db, &canon.path) else {
        return IndexState::Missing;
    };
    if !status.indexed {
        return IndexState::Missing;
    }
    match (status.git_head.as_deref(), git_head_quickly(&canon.path)) {
        // Only a definite disagreement is called stale. A tree with no git, or a head that could
        // not be read inside the budget, is not evidence of staleness and must not be reported as
        // if it were -- but it is not evidence of freshness either, which is why the other arm is
        // named for what was observed rather than for what the reader would like it to mean.
        (Some(stored), Some(now)) if stored != now => IndexState::BehindHead,
        _ => IndexState::HeadMatches,
    }
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
/// This is the same reason `judge` is single and `hook-probe` replays it rather than
/// reimplementing it. The parsers may be plural; the table may not.
const HOOK_TARGETS: [(SettingsFormat, &str); 3] = [
    (SettingsFormat::Json, "claude-code"),
    (SettingsFormat::CodexToml, "codex"),
    (SettingsFormat::KimiToml, "kimi-code"),
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
        for event in cort::settings::EVENTS {
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
                // Removal takes both events at once in every dialect, so it runs once per file and
                // the second event reports what the first one did rather than re-editing.
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
                    // One target's failure is reported and the rest still deploy. Six independent
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
                json!({ "message": format!("--event must be pre or post, not `{s}`") }),
            )
        })?,
    };
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
            // `stale` is what an installer or a person actually wants to know, and it is only
            // answerable here: the row stores the head it was built at, and the tree knows the head
            // it is on now. `null`, never `false`, when the two cannot be compared -- the directory
            // is gone, it is not a git tree, or `rev-parse` did not answer inside the budget. A
            // project that cannot be checked is not a project that is fresh, and saying so is the
            // same discipline `index_state` and the coverage screen already keep.
            let exists = Path::new(&r.path).is_dir();
            let stale = match (r.git_head.as_deref(), git_head_quickly(Path::new(&r.path))) {
                (Some(stored), Some(now)) => Some(stored != now),
                _ => None,
            };
            json!({
                "project_id": r.project_id,
                "name": r.name,
                "path": r.path,
                "git_head": r.git_head,
                "last_indexed_at": r.last_indexed_at,
                "db_path": r.db_path,
                "exists": exists,
                "stale": stale,
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
            // A row whose directory is gone is the one most worth deleting, and it is precisely
            // the one whose path cannot be canonicalised. The registry is a scan of the cache
            // directory, so it can still name the row and its database by the path recorded
            // inside it -- fall back to that before refusing.
            let want = root.to_string_lossy().trim_end_matches('/').to_string();
            if let Some(row) = cort::db::list_projects()
                .into_iter()
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
