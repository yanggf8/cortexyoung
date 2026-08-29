//! B-1..B-5 — ast-grep.test.js

use cort::ast_grep::{
    assert_ast_grep_version, ast_grep_candidates, ast_grep_version, exec_ast_grep,
    resolve_ast_grep_bin, ExecOpts, AST_GREP_PINNED,
};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn fake_ag() -> PathBuf {
    // Built by cargo as part of this package (see Cargo.toml [[bin]] fake_ast_grep).
    PathBuf::from(env!("CARGO_BIN_EXE_fake_ast_grep"))
}

fn with_var(key: &str, val: Option<&str>, f: impl FnOnce()) {
    let prev = std::env::var(key).ok();
    // SAFETY: tests in this file take ENV_LOCK so no other thread mutates env.
    unsafe {
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    // SAFETY: restoring the value we read above, still under ENV_LOCK.
    unsafe {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// B-1
#[test]
fn resolves_the_real_ast_grep_and_it_matches_the_pin() {
    let _g = env_guard();
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    assert_eq!(ast_grep_version(&bin).unwrap(), AST_GREP_PINNED);
    assert!(assert_ast_grep_version(&bin).is_ok());
}

/// B-2
#[test]
fn missing_binary_is_fail_closed() {
    let _g = env_guard();
    with_var("CORT_AST_GREP_BIN", Some("/nonexistent/ast-grep"), || {
        with_var("PATH", Some(""), || {
            let err = resolve_ast_grep_bin().unwrap_err();
            assert_eq!(err.code, "ast_grep_missing");
        });
    });
}

/// B-3
#[test]
fn wrong_version_is_fail_closed_with_found_expected_detail() {
    let _g = env_guard();
    with_var("FAKE_AG_MODE", Some("version:0.44.9"), || {
        let err = assert_ast_grep_version(fake_ag().to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "ast_grep_version_mismatch");
        assert_eq!(
            err.detail,
            serde_json::json!({ "found": "0.44.9", "expected": AST_GREP_PINNED })
        );
        assert_eq!(
            err.to_json(),
            serde_json::json!({
                "error": "ast_grep_version_mismatch",
                "detail": { "found": "0.44.9", "expected": AST_GREP_PINNED },
            })
        );
    });
}

/// B-4
#[test]
fn a_hung_subprocess_raises_ast_grep_timeout() {
    let _g = env_guard();
    with_var("FAKE_AG_MODE", Some("hang"), || {
        let err = exec_ast_grep(
            fake_ag().to_str().unwrap(),
            &["run"],
            ExecOpts {
                timeout_ms: Some(150),
                ..ExecOpts::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "ast_grep_timeout");
    });
}

/// B-5
#[test]
fn exec_ast_grep_returns_code_stdout_and_stderr_separately() {
    let _g = env_guard();
    with_var("FAKE_AG_MODE", Some("streams"), || {
        let r = exec_ast_grep(fake_ag().to_str().unwrap(), &["run"], ExecOpts::default())
            .expect("non-zero exit must not throw");
        assert_eq!(r.code, 1);
        assert_eq!(r.stdout, "OUT\n");
        assert_eq!(r.stderr, "ERR\n");
    });
}

/// Spec §2.3 remaining fake-ast-grep modes (empty / emit / preflight-*), used by later jobs.
#[test]
fn fake_ast_grep_modes_empty_emit_and_preflight() {
    let _g = env_guard();
    let bin = fake_ag();
    let bin_s = bin.to_str().unwrap();

    with_var("FAKE_AG_MODE", Some("empty"), || {
        let r = exec_ast_grep(bin_s, &["run"], ExecOpts::default()).unwrap();
        assert_eq!(r.code, 1);
        assert_eq!(r.stdout, "");
        assert_eq!(r.stderr, "");
    });

    // "hello" as standard base64 — matches JS Buffer.from(..., 'base64').
    with_var("FAKE_AG_MODE", Some("emit:aGVsbG8="), || {
        let r = exec_ast_grep(bin_s, &["run"], ExecOpts::default()).unwrap();
        assert_eq!(r.code, 0);
        assert_eq!(r.stdout, "hello");
    });

    with_var("FAKE_AG_MODE", Some("preflight-bad"), || {
        let r = exec_ast_grep(bin_s, &["run"], ExecOpts::default()).unwrap();
        assert_eq!(r.code, 0);
        assert!(r.stderr.contains("Pattern contains an ERROR node"));
    });

    with_var("FAKE_AG_MODE", Some("preflight-ok"), || {
        let r = exec_ast_grep(bin_s, &["run"], ExecOpts::default()).unwrap();
        assert_eq!(r.code, 0);
        assert!(r.stderr.contains("Debug AST:"));
        assert!(!r.stderr.contains("ERROR node"));
    });
}

// ── F-13: cort must not depend on whoever launched it having ast-grep on PATH ──────────────

/// Install a scripted stand-in for ast-grep at `<home>/<subdir>/ast-grep` and make it executable.
fn install_fake(home: &std::path::Path, subdir: &str, version: &str) -> PathBuf {
    let dir = home.join(subdir);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("ast-grep");
    fs::write(
        &target,
        format!("#!/bin/sh\necho \"ast-grep {version}\"\nexit 0\n"),
    )
    .unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    target
}

#[test]
fn candidates_probe_the_install_locations_and_not_only_path() {
    let _g = env_guard();
    let home = std::env::temp_dir().join(format!("cort-ag-home-{}", std::process::id()));
    with_var("HOME", Some(home.to_str().unwrap()), || {
        with_var("CARGO_HOME", Some("/opt/cargo"), || {
            let cands = ast_grep_candidates();
            assert_eq!(cands[0], "ast-grep", "PATH is still tried first");
            for expected in [
                home.join(".local/bin/ast-grep").to_str().unwrap(),
                "/opt/cargo/bin/ast-grep",
                home.join(".cargo/bin/ast-grep").to_str().unwrap(),
                "/usr/local/bin/ast-grep",
            ] {
                assert!(
                    cands.contains(&expected.to_string()),
                    "missing {expected} in {cands:?}"
                );
            }
        });
    });
}

#[test]
fn resolve_finds_a_pinned_binary_that_is_not_on_path() {
    let _g = env_guard();
    let home = std::env::temp_dir().join(format!("cort-ag-found-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    let installed = install_fake(&home, ".cargo/bin", AST_GREP_PINNED);
    with_var("HOME", Some(home.to_str().unwrap()), || {
        with_var(
            "CARGO_HOME",
            Some(home.join("nope").to_str().unwrap()),
            || {
                with_var("PATH", Some(""), || {
                    with_var("CORT_AST_GREP_BIN", None, || {
                        let got = resolve_ast_grep_bin().expect(
                            "a pinned ast-grep under ~/.cargo/bin must be found without PATH",
                        );
                        assert_eq!(
                            fs::canonicalize(&got).unwrap(),
                            fs::canonicalize(&installed).unwrap()
                        );
                    });
                });
            },
        );
    });
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_reachable_but_unpinned_binary_still_fails_closed_with_both_versions() {
    let _g = env_guard();
    let home = std::env::temp_dir().join(format!("cort-ag-old-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    install_fake(&home, ".local/bin", "0.44.9");
    with_var("HOME", Some(home.to_str().unwrap()), || {
        with_var(
            "CARGO_HOME",
            Some(home.join("nope").to_str().unwrap()),
            || {
                with_var("PATH", Some(""), || {
                    with_var("CORT_AST_GREP_BIN", None, || {
                        let err = resolve_ast_grep_bin().unwrap_err();
                        assert_eq!(err.code, "ast_grep_version_mismatch");
                        assert_eq!(err.detail["found"], "0.44.9");
                        assert_eq!(err.detail["expected"], AST_GREP_PINNED);
                    });
                });
            },
        );
    });
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_missing_binary_reports_everywhere_it_looked() {
    let _g = env_guard();
    let home = std::env::temp_dir().join(format!("cort-ag-none-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    with_var("HOME", Some(home.to_str().unwrap()), || {
        with_var(
            "CARGO_HOME",
            Some(home.join("nope").to_str().unwrap()),
            || {
                with_var("PATH", Some(""), || {
                    with_var("CORT_AST_GREP_BIN", None, || {
                        let err = resolve_ast_grep_bin().unwrap_err();
                        assert_eq!(err.code, "ast_grep_missing");
                        let probed = err.detail["probed"].as_array().expect("probed list");
                        assert!(
                            probed.len() >= 4,
                            "the error must show the search: {probed:?}"
                        );
                        assert!(err.detail["hint"]
                            .as_str()
                            .unwrap()
                            .contains("CORT_AST_GREP_BIN"));
                    });
                });
            },
        );
    });
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn an_explicit_override_never_falls_back_to_a_lucky_path() {
    let _g = env_guard();
    let home = std::env::temp_dir().join(format!("cort-ag-explicit-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    install_fake(&home, ".local/bin", AST_GREP_PINNED);
    with_var("HOME", Some(home.to_str().unwrap()), || {
        with_var("PATH", Some(""), || {
            // A bad override must stay bad: the probes that make cort usable for agents cannot be
            // allowed to rescue an explicit path, or the fail-closed tests would pass by accident.
            with_var("CORT_AST_GREP_BIN", Some("/nonexistent/ast-grep"), || {
                let err = resolve_ast_grep_bin().unwrap_err();
                assert_eq!(err.code, "ast_grep_missing");
                assert_eq!(err.detail["source"], "CORT_AST_GREP_BIN");
            });
        });
    });
    let _ = fs::remove_dir_all(&home);
}
