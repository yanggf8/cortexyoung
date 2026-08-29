//! ast-grep subprocess bridge.
//! Spawn only — never `sg`, never in-process. Pin is string-equal `"0.45.2"`.

use crate::errors::CortError;
use serde_json::json;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub const AST_GREP_PINNED: &str = "0.45.2";
pub const SUBPROCESS_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Default, Clone)]
pub struct ExecOpts {
    pub cwd: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Explicit override means *exactly this binary*, with no fallback: `CORT_AST_GREP_BIN` is the knob
/// tests and the smoke suite use to pin a fixture, and silently falling back to a real ast-grep a
/// few directories away would turn those fail-closed tests into accidental passes.
fn explicit_override() -> Option<String> {
    match std::env::var("CORT_AST_GREP_BIN") {
        Ok(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Where `ast-grep` may live, in probe order.
///
/// cort used to ask for a bare `ast-grep` and rely on PATH. That broke for its main audience: an
/// agent shell does not necessarily inherit the user's PATH (Claude Code normalises it to
/// `/usr/local/bin:/usr/bin:/bin:~/.local/bin`), and `install.sh` puts the binary under
/// `$CARGO_HOME/bin` by default, which is not in that list — so a correctly installed cort answered
/// `ast_grep_missing` from inside an agent session (audit F-13). Probing the install locations
/// directly removes the dependency on whoever launched us.
pub fn ast_grep_candidates() -> Vec<String> {
    let mut out: Vec<String> = vec!["ast-grep".to_string()];

    let mut push_dir = |dir: Option<PathBuf>| {
        if let Some(d) = dir {
            let cand = d.join("ast-grep");
            let s = cand.to_string_lossy().into_owned();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    };

    // Next to the running cort: the installer's payload directory and the bin directory that
    // shipped the shim.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push_dir(Some(dir.to_path_buf()));
            push_dir(dir.parent().map(|p| p.join("bin")).clone());
        }
    }
    push_dir(
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".local").join("bin")),
    );
    push_dir(
        std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .map(|c| c.join("bin")),
    );
    push_dir(
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".cargo").join("bin")),
    );
    push_dir(Some(PathBuf::from("/usr/local/bin")));
    push_dir(Some(PathBuf::from("/opt/homebrew/bin")));
    out
}

/// `--version` probe has no 30s timeout (JS spawnSync default).
fn run_version(bin: &str) -> std::io::Result<std::process::Output> {
    Command::new(bin).arg("--version").output()
}

fn probe_failed(output: &std::io::Result<std::process::Output>) -> bool {
    match output {
        Err(_) => true,
        Ok(o) => !o.status.success(),
    }
}

pub fn resolve_ast_grep_bin() -> Result<String, CortError> {
    if let Some(explicit) = explicit_override() {
        let probe = run_version(&explicit);
        if probe_failed(&probe) {
            return Err(CortError::new(
                "ast_grep_missing",
                json!({ "candidate": explicit, "source": "CORT_AST_GREP_BIN" }),
            ));
        }
        return Ok(explicit);
    }

    let probed = ast_grep_candidates();
    let mut reachable: Option<(String, Option<String>)> = None;
    for candidate in &probed {
        let probe = match run_version(candidate) {
            Ok(ok) => ok,
            Err(_) => continue,
        };
        if !probe.status.success() {
            continue;
        }
        let found = first_xyz(&String::from_utf8_lossy(&probe.stdout)).map(str::to_string);
        if reachable.is_none() {
            reachable = Some((candidate.clone(), found.clone()));
        }
        // Prefer a binary that actually matches the pin over merely the first one found; a stale
        // 0.44.x on PATH must not shadow the pinned copy sitting next to cort.
        if found.as_deref() == Some(AST_GREP_PINNED) {
            return Ok(candidate.clone());
        }
    }

    match reachable {
        // Something is installed, but it is not the pinned parser: still fail closed, and say which
        // version was found so the fix is obvious instead of mysterious.
        Some((candidate, found)) => Err(CortError::new(
            "ast_grep_version_mismatch",
            json!({
                "found": found.unwrap_or_else(|| "unparsable".to_string()),
                "expected": AST_GREP_PINNED,
                "candidate": candidate,
            }),
        )),
        None => Err(CortError::new(
            "ast_grep_missing",
            json!({
                "candidate": "ast-grep",
                "probed": probed,
                "hint": "install ast-grep 0.45.2 (`./install.sh` or `cargo install ast-grep --version 0.45.2 --locked`), or point CORT_AST_GREP_BIN at it",
            }),
        )),
    }
}

/// First `(\d+\.\d+\.\d+)` in stdout — no regex crate (not on the dep whitelist).
fn first_xyz(stdout: &str) -> Option<&str> {
    let b = stdout.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < n && b[i].is_ascii_digit() {
                i += 1;
            }
            if i < n && b[i] == b'.' {
                i += 1;
                if i < n && b[i].is_ascii_digit() {
                    while i < n && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i < n && b[i] == b'.' {
                        i += 1;
                        if i < n && b[i].is_ascii_digit() {
                            while i < n && b[i].is_ascii_digit() {
                                i += 1;
                            }
                            return std::str::from_utf8(&b[start..i]).ok();
                        }
                    }
                }
            }
            i = start + 1;
        } else {
            i += 1;
        }
    }
    None
}

pub fn ast_grep_version(bin: &str) -> Result<String, CortError> {
    let r = run_version(bin);
    if probe_failed(&r) {
        return Err(CortError::new(
            "ast_grep_missing",
            json!({ "candidate": bin }),
        ));
    }
    let stdout = String::from_utf8_lossy(&r.unwrap().stdout).into_owned();
    match first_xyz(&stdout) {
        Some(v) => Ok(v.to_string()),
        None => Err(CortError::new(
            "ast_grep_version_unreadable",
            json!({ "stdout": stdout }),
        )),
    }
}

pub fn assert_ast_grep_version(bin: &str) -> Result<(), CortError> {
    let found = ast_grep_version(bin)?;
    if found != AST_GREP_PINNED {
        return Err(CortError::new(
            "ast_grep_version_mismatch",
            json!({ "found": found, "expected": AST_GREP_PINNED }),
        ));
    }
    Ok(())
}

fn timeout_err(args: &[String], timeout_ms: u64) -> CortError {
    CortError::new(
        "ast_grep_timeout",
        json!({ "args": args, "timeoutMs": timeout_ms }),
    )
}

fn send_sigterm(pid: u32) {
    // JS spawnSync timeout kills with SIGTERM (then r.signal === 'SIGTERM' or ETIMEDOUT).
    #[cfg(unix)]
    {
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        const SIGTERM: i32 = 15;
        // SAFETY: pid is the child we spawned and have not reaped yet.
        unsafe {
            kill(pid as i32, SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

pub fn exec_ast_grep(bin: &str, args: &[&str], opts: ExecOpts) -> Result<ExecResult, CortError> {
    let timeout_ms = opts.timeout_ms.unwrap_or(SUBPROCESS_TIMEOUT_MS);
    let args_owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();

    let mut cmd = Command::new(bin);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = opts.cwd {
        cmd.current_dir(cwd);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Err(CortError::new(
                "ast_grep_spawn_failed",
                json!({ "args": args_owned, "message": e.to_string() }),
            ));
        }
    };

    let pid = child.id();
    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");

    let out_h = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let err_h = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(status)) => {
            let stdout = String::from_utf8_lossy(&out_h.join().unwrap_or_default()).into_owned();
            let stderr = String::from_utf8_lossy(&err_h.join().unwrap_or_default()).into_owned();
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if status.signal() == Some(15) {
                    return Err(timeout_err(&args_owned, timeout_ms));
                }
            }
            Ok(ExecResult {
                code: status.code().unwrap_or(0),
                stdout,
                stderr,
            })
        }
        Ok(Err(e)) => Err(CortError::new(
            "ast_grep_spawn_failed",
            json!({ "args": args_owned, "message": e.to_string() }),
        )),
        Err(_) => {
            send_sigterm(pid);
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let _ = out_h.join();
            let _ = err_h.join();
            Err(timeout_err(&args_owned, timeout_ms))
        }
    }
}
