//! ast-grep subprocess bridge. JS `src/ast-grep.js`.
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

fn candidate_bin() -> String {
    match std::env::var("CORT_AST_GREP_BIN") {
        Ok(s) if !s.is_empty() => s,
        _ => "ast-grep".to_string(),
    }
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
    let candidate = candidate_bin();
    let probe = run_version(&candidate);
    if probe_failed(&probe) {
        return Err(CortError::new(
            "ast_grep_missing",
            json!({ "candidate": candidate }),
        ));
    }
    Ok(candidate)
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
