//! The legacy crontab entry that ran `cort measure record` hourly: manual install and removal only;
//! the installer and upgrader retire it, and the hook kick is the automatic collection trigger.
//!
//! Same discipline as `hook-install`: the binary owns the merge (the installer holds no logic),
//! our own entry is recognised by tokens and never by a line suffix, and the command prefix is
//! supplied by the caller -- the installed shim, not `current_exe()`.
//!
//! `crontab` is per-user and system-wide, not scoped to `HOME`, so every test goes through the
//! `CORT_CRONTAB_FILE` seam: when it is set and non-empty, that file stands in for the crontab.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Env seam: a file that replaces `crontab -l` / `crontab -`.
pub const CRONTAB_FILE_ENV: &str = "CORT_CRONTAB_FILE";

/// Quote `s` for /bin/sh only when it needs it, so ordinary paths keep the documented shape.
/// Single quotes, with an embedded single quote written as `'\''`.
fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@+=:,./-".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Split a crontab line into shell words: whitespace separates, single and double quotes group
/// (and vanish), a backslash outside quotes escapes the next character. Enough to read back
/// what [`shell_quote`] wrote and what a person would plausibly have typed.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut live = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                live = true;
            }
            None if c == '\\' => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    live = true;
                }
            }
            None if c.is_whitespace() => {
                if live {
                    words.push(std::mem::take(&mut cur));
                    live = false;
                }
            }
            None => {
                cur.push(c);
                live = true;
            }
        }
    }
    if live {
        words.push(cur);
    }
    words
}

/// One line, hourly at minute 17. `command_prefix` is the installed shim path. Both paths are
/// shell-quoted when needed: cron hands the line to /bin/sh, and a space would split them.
pub fn measure_cron_line(command_prefix: &str, log: &Path) -> String {
    format!(
        "17 * * * * {} measure record >> {} 2>&1",
        shell_quote(command_prefix),
        shell_quote(&log.display().to_string())
    )
}

/// True for a crontab line whose command is `cort measure record`: the first word after the
/// schedule (one `@` word, or five time fields) has the file name exactly `cort` (quoted or not),
/// and the next two are `measure` and `record`. Comments never match, and whatever follows is
/// irrelevant -- a token test, not a suffix test, because anchoring to the end of the line is what
/// made `hook-install --status` lie twice. The start is anchored because uninstall drops the whole
/// line: `echo cort measure record` or `backup && cort measure record` is someone else's job.
pub fn is_measure_entry(line: &str) -> bool {
    let line = line.trim();
    if line.starts_with('#') {
        return false;
    }
    let words = shell_words(line);
    let start = if words.first().is_some_and(|w| w.starts_with('@')) {
        1
    } else {
        5
    };
    matches!(
        words.get(start..start + 3),
        Some([cmd, m, r]) if m == "measure"
            && r == "record"
            && Path::new(cmd).file_name().and_then(|n| n.to_str()) == Some("cort")
    )
}

/// 兩種條目共用的整行合併：`is_entry` 認出的行中，trim 後與 `line` 不同者移除；
/// 剩下仍有相符者就不附加，否則把 `line` 附在最後。只留一份實作，兩條才不會再分岔。
/// (Ported verbatim from claudecat `doctor.rs:143`, c768644.)
pub fn merge_whole_line(existing: &str, line: &str, is_entry: fn(&str) -> bool) -> (String, bool) {
    let stale = |l: &str| is_entry(l) && l.trim() != line.trim();
    let kept: Vec<&str> = existing.lines().filter(|l| !stale(l)).collect();
    let up_to_date = kept.iter().any(|l| is_entry(l));
    if up_to_date {
        let out = format!("{}\n", kept.join("\n"));
        let changed = existing != out;
        return (out, changed);
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    (out, true)
}

/// The merge `install` uses. Same whole-line semantics as [`merge_whole_line`] (a different
/// variant of our entry is replaced, an identical one is left alone, none means append), plus two
/// things that port does not do: it collapses byte-identical duplicates to one line, and it
/// rebuilds the text from `split_inclusive('\n')` segments instead of `lines()` + join, so every
/// line it does not own comes back byte for byte (the port loses a lone blank line and any
/// carriage return).
///
/// Cron ignores an unterminated last line, so such a line never ran; appending our entry after it
/// would silently fuse the two, so a final newline is always added. That one case does not
/// round-trip through install then uninstall, and cannot be made to without gluing lines.
pub fn merge_measure_entry(existing: &str, line: &str) -> (String, bool) {
    let mut out = String::with_capacity(existing.len() + line.len() + 1);
    let mut have_current = false;
    for seg in existing.split_inclusive('\n') {
        let text = seg.trim_end_matches(['\n', '\r']);
        if is_measure_entry(text) {
            if text.trim() != line.trim() || have_current {
                continue;
            }
            have_current = true;
        }
        out.push_str(seg);
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !have_current {
        out.push_str(line);
        out.push('\n');
    }
    let changed = out != existing;
    (out, changed)
}

/// Drop every entry `is_entry` recognises; every other byte is returned untouched.
pub fn remove_entries(existing: &str, is_entry: fn(&str) -> bool) -> String {
    existing
        .split_inclusive('\n')
        .filter(|seg| !is_entry(seg.trim_end_matches(['\n', '\r'])))
        .collect()
}

fn seam_file() -> Option<PathBuf> {
    std::env::var_os(CRONTAB_FILE_ENV)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// Why the crontab could not be reached. `Absent` is the only one `--if-configured` forgives.
#[derive(Debug)]
pub enum CronError {
    /// The `crontab` program is not installed (spawn failed with NotFound).
    Absent(String),
    Failed(String),
}

impl CronError {
    pub fn message(&self) -> &str {
        match self {
            CronError::Absent(m) | CronError::Failed(m) => m,
        }
    }
}

fn spawn_error(e: std::io::Error) -> CronError {
    let m = format!("cannot run crontab: {e}");
    if e.kind() == std::io::ErrorKind::NotFound {
        CronError::Absent(m)
    } else {
        CronError::Failed(m)
    }
}

/// Current crontab text. A user with no crontab ("no crontab for <user>", exit 1) is empty, not
/// an error; a missing seam file likewise.
pub fn read_crontab() -> Result<String, CronError> {
    if let Some(file) = seam_file() {
        return match std::fs::read_to_string(&file) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(CronError::Failed(format!(
                "cannot read {}: {e}",
                file.display()
            ))),
        };
    }
    let out = Command::new("crontab")
        .arg("-l")
        .output()
        .map_err(spawn_error)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("no crontab") {
        return Ok(String::new());
    }
    Err(CronError::Failed(format!(
        "crontab -l failed: {}",
        stderr.trim()
    )))
}

/// Replace the crontab: always `crontab -` with the text on stdin.
pub fn write_crontab(content: &str) -> Result<(), CronError> {
    if let Some(file) = seam_file() {
        return crate::data_dir::write_atomic(&file, content)
            .map_err(|e| CronError::Failed(format!("cannot write {}: {e}", file.display())));
    }
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_error)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| CronError::Failed(format!("cannot write to crontab: {e}")))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| CronError::Failed(format!("crontab did not finish: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(CronError::Failed(format!(
            "crontab - failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

pub fn log_path() -> PathBuf {
    crate::data_dir::data_dir().join("measure.log")
}

pub enum CronAction {
    Install { prefix: String, if_configured: bool },
    Uninstall,
    Status { prefix: String },
}

fn skipped(message: &str) -> Value {
    json!({"status":"skipped","message":message})
}

/// Run one action. A `"status":"skipped"` value carries a one-line `message` for the caller to
/// print; everything else is a JSON report.
///
/// Under `--if-configured` only two things exit 0 without installing: measurement is not
/// configured, or the `crontab` program does not exist. Any other failure is returned, so the
/// caller (installer, upgrader) can print it as a warning instead of losing it.
pub fn run(action: CronAction) -> Result<Value, String> {
    match action {
        CronAction::Install {
            prefix,
            if_configured,
        } => {
            if if_configured && !crate::measure_store::is_configured() {
                return Ok(skipped("measure: not configured; skipping cron"));
            }
            match install(&prefix) {
                Ok(v) => Ok(v),
                Err(CronError::Absent(_)) if if_configured => {
                    Ok(skipped("measure: no crontab program; skipping cron"))
                }
                Err(e) => Err(e.message().to_string()),
            }
        }
        CronAction::Uninstall => {
            // No `crontab` program means no entry can exist: the goal state, not a failure.
            let existing = match read_crontab() {
                Ok(text) => text,
                Err(CronError::Absent(_)) => {
                    return Ok(json!({"status":"not_present","crontab_available":false}))
                }
                Err(e) => return Err(e.message().to_string()),
            };
            let out = remove_entries(&existing, is_measure_entry);
            let changed = out != existing;
            if changed {
                write_crontab(&out).map_err(|e| e.message().to_string())?;
            }
            Ok(json!({"status": if changed {"removed"} else {"not_present"}}))
        }
        CronAction::Status { prefix } => {
            let line = measure_cron_line(&prefix, &log_path());
            let (text, available) = match read_crontab() {
                Ok(t) => (t, true),
                Err(_) => (String::new(), false),
            };
            let entries = text.lines().filter(|l| is_measure_entry(l)).count();
            let installed = entries == 1 && text.lines().any(|l| l.trim() == line.trim());
            Ok(json!({
                "installed": installed,
                "entries": entries,
                "line": line,
                "crontab_available": available,
            }))
        }
    }
}

fn install(prefix: &str) -> Result<Value, CronError> {
    let line = measure_cron_line(prefix, &log_path());
    let existing = read_crontab()?;
    let (out, changed) = merge_measure_entry(&existing, &line);
    if changed {
        write_crontab(&out)?;
    }
    Ok(json!({"status": if changed {"installed"} else {"unchanged"}, "line": line}))
}
