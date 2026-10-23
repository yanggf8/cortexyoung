//! The crontab entry that runs `cort measure record` hourly.
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

/// One line, hourly at minute 17. `command_prefix` is the installed shim path.
pub fn measure_cron_line(command_prefix: &str, log: &Path) -> String {
    format!(
        "17 * * * * {command_prefix} measure record >> {} 2>&1",
        log.display()
    )
}

/// True for a crontab line that runs `cort measure record`: some token whose file name is exactly
/// `cort`, immediately followed by the tokens `measure` and `record`. Comments never match, and
/// whatever follows (redirects, a trailing comment) is irrelevant -- this is a token test, not a
/// suffix test, because anchoring to the end of the line is what made `hook-install --status`
/// lie twice.
pub fn is_measure_entry(line: &str) -> bool {
    let line = line.trim();
    if line.starts_with('#') {
        return false;
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    tokens.windows(3).any(|w| {
        w[1] == "measure"
            && w[2] == "record"
            && Path::new(w[0]).file_name().and_then(|n| n.to_str()) == Some("cort")
    })
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

/// The merge `install` uses: [`merge_whole_line`] (verbatim) plus the one thing it does not do --
/// collapse byte-identical duplicates of the entry down to a single line.
pub fn merge_measure_entry(existing: &str, line: &str) -> (String, bool) {
    let mut seen = false;
    let deduped: Vec<&str> = existing
        .lines()
        .filter(|l| {
            if !(is_measure_entry(l) && l.trim() == line.trim()) {
                return true;
            }
            !std::mem::replace(&mut seen, true)
        })
        .collect();
    let collapsed = if deduped.len() == existing.lines().count() {
        existing.to_string()
    } else {
        format!("{}\n", deduped.join("\n"))
    };
    let (out, changed) = merge_whole_line(&collapsed, line, is_measure_entry);
    (out, changed || collapsed != existing)
}

/// Drop every entry `is_entry` recognises; everything else is returned untouched.
pub fn remove_entries(existing: &str, is_entry: fn(&str) -> bool) -> String {
    if !existing.lines().any(is_entry) {
        return existing.to_string();
    }
    let kept: Vec<&str> = existing.lines().filter(|l| !is_entry(l)).collect();
    if kept.is_empty() {
        String::new()
    } else {
        format!("{}\n", kept.join("\n"))
    }
}

fn seam_file() -> Option<PathBuf> {
    std::env::var_os(CRONTAB_FILE_ENV)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// Current crontab text. A user with no crontab ("no crontab for <user>", exit 1) is empty, not
/// an error; a missing seam file likewise.
pub fn read_crontab() -> Result<String, String> {
    if let Some(file) = seam_file() {
        return match std::fs::read_to_string(&file) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("cannot read {}: {e}", file.display())),
        };
    }
    let out = Command::new("crontab")
        .arg("-l")
        .output()
        .map_err(|e| format!("cannot run crontab: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("no crontab") {
        return Ok(String::new());
    }
    Err(format!("crontab -l failed: {}", stderr.trim()))
}

/// Replace the crontab: always `crontab -` with the text on stdin.
pub fn write_crontab(content: &str) -> Result<(), String> {
    if let Some(file) = seam_file() {
        return crate::data_dir::write_atomic(&file, content)
            .map_err(|e| format!("cannot write {}: {e}", file.display()));
    }
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run crontab: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| format!("cannot write to crontab: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("crontab did not finish: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "crontab - failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
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

fn skipped(message: String) -> Value {
    json!({"status":"skipped","message":message})
}

/// Run one action. A `"status":"skipped"` value carries a one-line `message` for the caller to
/// print; everything else is a JSON report.
pub fn run(action: CronAction) -> Result<Value, String> {
    match action {
        CronAction::Install {
            prefix,
            if_configured,
        } => {
            if if_configured && !crate::measure_store::is_configured() {
                return Ok(skipped("measure: not configured; skipping cron".into()));
            }
            match install(&prefix) {
                Ok(v) => Ok(v),
                // The installer must not fail over a crontab it cannot reach; a person who asked
                // for the entry by name (plain --install) gets the error.
                Err(e) if if_configured => Ok(skipped(format!(
                    "measure: crontab unavailable ({e}); skipping cron"
                ))),
                Err(e) => Err(e),
            }
        }
        CronAction::Uninstall => {
            let existing = read_crontab()?;
            let out = remove_entries(&existing, is_measure_entry);
            let changed = out != existing;
            if changed {
                write_crontab(&out)?;
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

fn install(prefix: &str) -> Result<Value, String> {
    let line = measure_cron_line(prefix, &log_path());
    let existing = read_crontab()?;
    let (out, changed) = merge_measure_entry(&existing, &line);
    if changed {
        write_crontab(&out)?;
    }
    Ok(json!({"status": if changed {"installed"} else {"unchanged"}, "line": line}))
}
