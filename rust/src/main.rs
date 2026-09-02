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
use cort::settings_toml;
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
            let db = open_db(db_path_for(&canon.path_str))
                .map_err(|e| cort::db::classify_sqlite(&e))?;
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
        Some("hook-suggest") => cmd_hook_suggest(&args[1..], usage),
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
fn hook_args(outcome: &str, harness: &str, declared: Option<&str>) -> String {
    let mut v = json!({ "v": 2, "hook": outcome, "harness": harness });
    if let Some(d) = declared {
        v["harness_declared"] = json!(d);
    }
    v.to_string()
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
/// Kimi (`kimi-code`) is deliberately absent from the list below. Its `PreToolUse` payload carries
/// `tool_name` / `tool_input` / `tool_call_id` and nothing else: neither spelling of a transcript
/// path occurs anywhere in the shipped `@moonshot-ai/kimi-code` bundle (grep over `dist/main.mjs`,
/// 2026-09-02). A `/.kimi-code/` arm sat here until that was checked -- it could never match, and
/// an arm that cannot fire made this list look like it covered a harness that is not wired at all.
/// Kimi is also the one harness this hook has nothing to say to: its `PreToolUse` keeps only
/// results whose `action` is `block` and drops every allow-shaped one before the model sees it
/// (`blockDecision`, same bundle), and `cort hook-suggest` never blocks. Re-add the arm when a Kimi
/// payload actually carries the path -- not on the assumption that it does.
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
    usage.args_summary = hook_args("no_payload", &declared, None);
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
    let (harness, declared_differs) = harness_of(&v, &declared);
    let harness_args = |outcome: &str| hook_args(outcome, &harness, declared_differs.as_deref());
    usage.args_summary = harness_args("no_shape");
    let Some(hit) = cort::hook::suggests_impact(command) else {
        return quiet();
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

/// TOML for Codex's `config.toml`, JSON for everything else (Claude Code's and Grok's shared
/// `settings.json` -- Grok reads that file for Claude Code compatibility and needs no wiring of its
/// own, `docs/2026-09-02-hook-wiring-correction.md` §6). Decided by the target path's extension
/// rather than a second flag: `install.sh` already has to name two different files for the two
/// homes, and a `.toml` path is unambiguous.
fn is_toml_settings(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("toml")
}

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
    let toml = is_toml_settings(&path);
    if a.status {
        // `trusted` is `null`, never `false`, wherever the question does not apply: Claude Code and
        // Grok have no trust gate, and a file with no entry of ours has nothing to be trusted. Only
        // a wired Codex entry can answer it, and `false` there is a hook that will not run.
        let (wired, trusted) = if toml {
            match settings_toml::installed_entry(&path) {
                Some((command, trusted)) => (Some(command), Some(trusted)),
                None => (None, None),
            }
        } else {
            (settings::installed_command(&path), None)
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
        let out = if toml {
            settings_toml::remove_hook(&path).map_err(map_toml_settings_err)?
        } else {
            settings::remove_hook(&path).map_err(map_json_settings_err)?
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
            format!("{} hook-suggest", exe.to_string_lossy())
        }
    };
    let out = if toml {
        settings_toml::install_hook(&path, &command).map_err(map_toml_settings_err)?
    } else {
        settings::install_hook(&path, &command).map_err(map_json_settings_err)?
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
