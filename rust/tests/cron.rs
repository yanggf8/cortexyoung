//! `cort measure cron`: the crontab entry that runs `cort measure record`.
//! Every test that touches a crontab goes through `CORT_CRONTAB_FILE`; none may reach the real one.

use cort::cron::{is_measure_entry, measure_cron_line, merge_measure_entry, remove_entries};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PREFIX: &str = "/home/u/.local/bin/cort";
const LOG: &str = "/home/u/.local/share/cortexyoung/memory/measure.log";

fn line() -> String {
    measure_cron_line(PREFIX, Path::new(LOG))
}

fn merge(existing: &str, l: &str) -> (String, bool) {
    merge_measure_entry(existing, l)
}

const FOREIGN: &str = "# claudecat\n9 17 * * * /home/u/.cargo/bin/claudecat analyze\n29 9 * * * /home/u/.cargo/bin/claudecat track\nMAILTO=me@example.com\n";

#[test]
fn line_has_the_documented_shape() {
    assert_eq!(
        line(),
        format!("17 * * * * {PREFIX} measure record >> {LOG} 2>&1")
    );
}

#[test]
fn adds_entry_when_absent() {
    let (out, changed) = merge("", &line());
    assert!(changed);
    assert_eq!(out, format!("{}\n", line()));
}

#[test]
fn replaces_stale_variant() {
    let old_log = format!("17 * * * * {PREFIX} measure record >> /old/measure.log 2>&1\n");
    let (out, changed) = merge(&old_log, &line());
    assert!(changed);
    assert_eq!(out, format!("{}\n", line()));

    let old_prefix = format!("17 * * * * /old/cort measure record >> {LOG} 2>&1\n");
    let (out, changed) = merge(&old_prefix, &line());
    assert!(changed);
    assert_eq!(out, format!("{}\n", line()));
}

#[test]
fn collapses_duplicates() {
    let existing = format!("{0}\n{0}\n17 * * * * /old/cort measure record\n", line());
    let (out, changed) = merge(&existing, &line());
    assert!(changed);
    assert_eq!(out.lines().filter(|l| is_measure_entry(l)).count(), 1);
    assert_eq!(out, format!("{}\n", line()));
}

#[test]
fn keeps_foreign_lines_byte_identical() {
    let (out, _) = merge(FOREIGN, &line());
    assert!(out.starts_with(FOREIGN), "foreign prefix changed: {out:?}");
    assert_eq!(out, format!("{FOREIGN}{}\n", line()));
}

#[test]
fn install_is_idempotent() {
    let (once, changed) = merge(FOREIGN, &line());
    assert!(changed);
    let (twice, changed_again) = merge(&once, &line());
    assert!(!changed_again);
    assert_eq!(once, twice);
}

#[test]
fn uninstall_restores_foreign_lines() {
    let (installed, _) = merge(FOREIGN, &line());
    assert_eq!(remove_entries(&installed, is_measure_entry), FOREIGN);
    assert_eq!(remove_entries(FOREIGN, is_measure_entry), FOREIGN);
    let (only, _) = merge("", &line());
    assert_eq!(remove_entries(&only, is_measure_entry), "");
}

#[test]
fn entry_test_is_token_not_suffix() {
    // Recognised wherever the command sits, whatever follows it.
    assert!(is_measure_entry(&line()));
    assert!(is_measure_entry(
        "17 * * * * /x/cort measure record >> /l 2>&1 # tail"
    ));
    assert!(is_measure_entry("*/5 * * * * cort measure record"));
    // Not ours.
    assert!(!is_measure_entry(
        "# 17 * * * * /x/cort measure record >> /l 2>&1"
    ));
    assert!(!is_measure_entry("17 * * * * /x/cort measure report"));
    assert!(!is_measure_entry("17 * * * * /x/notcort measure record"));
    assert!(!is_measure_entry("17 * * * * /x/cort-old measure record"));
    assert!(!is_measure_entry("17 * * * * /x/cort index measure record"));
    assert!(!is_measure_entry("9 17 * * * /x/claudecat analyze"));
    assert!(!is_measure_entry(""));
}

// ── CLI, always against a temp crontab file ───────────────────────────────

fn cort(file: &Path, data: &Path, args: &[&str]) -> Output {
    Command::new(PathBuf::from(env!("CARGO_BIN_EXE_cort")))
        .args(args)
        .env("CORT_CRONTAB_FILE", file)
        .env("CORT_MEMORY_DIR", data)
        .output()
        .unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("crontab");
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    (dir, file, data)
}

fn count_entries(file: &Path) -> usize {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter(|l| is_measure_entry(l))
        .count()
}

#[test]
fn cli_install_status_uninstall_round_trip() {
    let (_d, file, data) = fixture();
    std::fs::write(&file, FOREIGN).unwrap();
    let o = cort(
        &file,
        &data,
        &["measure", "cron", "--install", "--command-prefix", PREFIX],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(count_entries(&file), 1);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with(FOREIGN));
    assert!(text.contains(&format!(
        "{PREFIX} measure record >> {} 2>&1",
        data.join("measure.log").display()
    )));

    let o = cort(
        &file,
        &data,
        &["measure", "cron", "--status", "--command-prefix", PREFIX],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["installed"], true);
    assert_eq!(v["entries"], 1);

    // Idempotent.
    cort(
        &file,
        &data,
        &["measure", "cron", "--install", "--command-prefix", PREFIX],
    );
    assert_eq!(count_entries(&file), 1);

    let o = cort(&file, &data, &["measure", "cron", "--uninstall"]);
    assert!(o.status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), FOREIGN);
}

#[test]
fn cli_status_reports_stale_entry_as_not_installed() {
    let (_d, file, data) = fixture();
    std::fs::write(&file, "17 * * * * /old/cort measure record\n").unwrap();
    let o = cort(
        &file,
        &data,
        &["measure", "cron", "--status", "--command-prefix", PREFIX],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["installed"], false);
    assert_eq!(v["entries"], 1);
    assert!(v["line"].as_str().unwrap().contains(PREFIX));
}

#[test]
fn cli_if_configured_skips_when_unconfigured() {
    let (_d, file, data) = fixture();
    let o = cort(
        &file,
        &data,
        &[
            "measure",
            "cron",
            "--install",
            "--if-configured",
            "--command-prefix",
            PREFIX,
        ],
    );
    assert!(o.status.success());
    assert!(out(&o).contains("measure: not configured; skipping cron"));
    assert_eq!(count_entries(&file), 0);
    assert!(!file.exists(), "an unconfigured install must not write");
}

#[test]
fn cli_if_configured_installs_when_configured() {
    let (_d, file, data) = fixture();
    std::fs::write(data.join("turso-measure.json"), "{}").unwrap();
    let o = cort(
        &file,
        &data,
        &[
            "measure",
            "cron",
            "--install",
            "--if-configured",
            "--command-prefix",
            PREFIX,
        ],
    );
    assert!(o.status.success());
    assert_eq!(count_entries(&file), 1);
}

#[test]
fn cli_install_requires_command_prefix() {
    let (_d, file, data) = fixture();
    let o = cort(&file, &data, &["measure", "cron", "--install"]);
    assert!(!o.status.success());
    assert_eq!(count_entries(&file), 0);
}

#[test]
fn cli_plain_install_fails_when_crontab_unwritable() {
    let (d, _f, data) = fixture();
    // The seam points at a path whose parent is a regular file: writing must fail loudly.
    let blocker = d.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let bad = blocker.join("crontab");
    let o = cort(
        &bad,
        &data,
        &["measure", "cron", "--install", "--command-prefix", PREFIX],
    );
    assert!(!o.status.success());
}

#[test]
fn cli_if_configured_fails_loudly_on_a_crontab_error() {
    let (d, _f, data) = fixture();
    std::fs::write(data.join("turso-measure.json"), "{}").unwrap();
    let blocker = d.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let bad = blocker.join("crontab");
    let o = cort(
        &bad,
        &data,
        &[
            "measure",
            "cron",
            "--install",
            "--if-configured",
            "--command-prefix",
            PREFIX,
        ],
    );
    assert!(!o.status.success(), "a real failure must not exit 0");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        out(&o).contains("cannot read"),
        "envelope lost: {}",
        out(&o)
    );
    assert!(err.contains("cannot read"), "message lost: {err}");
    // Status degrades to "absent" instead of failing.
    let o = cort(
        &bad,
        &data,
        &["measure", "cron", "--status", "--command-prefix", PREFIX],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["installed"], false);
    assert_eq!(v["entries"], 0);
    assert_eq!(v["crontab_available"], false);
}

#[test]
fn cli_if_configured_exits_zero_when_the_crontab_program_is_absent() {
    let (d, _f, data) = fixture();
    std::fs::write(data.join("turso-measure.json"), "{}").unwrap();
    // No seam, and a PATH holding nothing: `crontab` cannot be spawned (NotFound), so the real
    // crontab is unreachable by construction.
    let empty = d.path().join("empty-path");
    std::fs::create_dir_all(&empty).unwrap();
    let run = |extra: &[&str]| {
        let mut args = vec!["measure", "cron", "--command-prefix", PREFIX];
        args.extend_from_slice(extra);
        Command::new(PathBuf::from(env!("CARGO_BIN_EXE_cort")))
            .args(args)
            .env_remove(cort::cron::CRONTAB_FILE_ENV)
            .env("PATH", &empty)
            .env("CORT_MEMORY_DIR", &data)
            .output()
            .unwrap()
    };
    let o = run(&["--install", "--if-configured"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(out(&o).contains("skipping cron"));
    // Without --if-configured the same absence is an error.
    assert!(!run(&["--install"]).status.success());
    let v: serde_json::Value = serde_json::from_str(&out(&run(&["--status"]))).unwrap();
    assert_eq!(v["installed"], false);
    assert_eq!(v["crontab_available"], false);
}

// ── quoting ───────────────────────────────────────────────────────────────

#[test]
fn paths_with_spaces_and_quotes_are_shell_quoted() {
    let l = measure_cron_line("/home/a b/it's/cort", Path::new("/data dir/measure.log"));
    assert_eq!(
        l,
        "17 * * * * '/home/a b/it'\\''s/cort' measure record >> '/data dir/measure.log' 2>&1"
    );
    assert!(is_measure_entry(&l));
    // cron runs the line through /bin/sh: the shell must see exactly the two original paths.
    let o = Command::new("sh")
        .arg("-c")
        .arg("for a in '/home/a b/it'\\''s/cort' '/data dir/measure.log'; do printf '%s\\n' \"$a\"; done")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "/home/a b/it's/cort\n/data dir/measure.log\n"
    );
}

#[test]
fn entry_test_accepts_quoted_and_unquoted_forms() {
    assert!(is_measure_entry(
        "17 * * * * '/a b/cort' measure record >> '/l m/x.log' 2>&1"
    ));
    assert!(is_measure_entry(
        "17 * * * * \"/a b/cort\" measure record >> /l 2>&1"
    ));
    assert!(is_measure_entry("17 * * * * /a/cort measure record"));
    assert!(!is_measure_entry(
        "17 * * * * '/a b/cort-old' measure record"
    ));
    assert!(!is_measure_entry("17 * * * * '/a b/cort measure' record"));
}

#[test]
fn stale_unquoted_entry_is_replaced_by_the_quoted_one() {
    let spaced = "/home/a b/cort";
    let l = measure_cron_line(spaced, Path::new("/data dir/measure.log"));
    let stale = "17 * * * * /home/a b/cort measure record >> /data dir/measure.log 2>&1\n";
    // The unquoted spelling is not even recognisable (its words are split), so it stays foreign;
    // the previous, correctly spelled unquoted variant for a safe path is what gets replaced.
    let safe_old = "17 * * * * /old/cort measure record >> /old/m.log 2>&1\n";
    let (out, changed) = merge(&format!("{safe_old}{stale}"), &l);
    assert!(changed);
    assert!(!out.contains("/old/cort"));
    assert_eq!(out.lines().filter(|x| *x == l).count(), 1);
    let (again, changed) = merge(&out, &l);
    assert!(!changed);
    assert_eq!(again, out);
}

#[test]
fn cli_round_trip_with_spaces_in_prefix_and_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("crontab");
    let data = dir.path().join("da ta");
    std::fs::create_dir_all(&data).unwrap();
    let prefix = format!("{}/bin dir/cort", dir.path().display());
    std::fs::write(&file, "17 * * * * /old/cort measure record\n").unwrap();
    let o = cort(
        &file,
        &data,
        &["measure", "cron", "--install", "--command-prefix", &prefix],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 1, "stale entry not replaced: {text}");
    assert!(text.contains(&format!("'{prefix}' measure record >> '")));
    assert!(text.contains(&format!("'{}'", data.join("measure.log").display())));
    let o = cort(
        &file,
        &data,
        &["measure", "cron", "--status", "--command-prefix", &prefix],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["installed"], true);
    assert_eq!(v["entries"], 1);
}

// ── byte fidelity ─────────────────────────────────────────────────────────

#[test]
fn newline_terminated_foreign_text_round_trips_byte_for_byte() {
    for foreign in [
        "",
        "\n",
        "\n\n",
        "# top\n\nMAILTO=me@example.com\n\n0 3 * * * /bin/backup --all\n\n\n# end\n",
        FOREIGN,
        "  \t\n0 0 * * * x # comment with cort measure report\n",
    ] {
        let (installed, changed) = merge(foreign, &line());
        assert!(changed);
        assert!(
            installed.starts_with(foreign),
            "{foreign:?} -> {installed:?}"
        );
        assert_eq!(
            remove_entries(&installed, is_measure_entry),
            foreign,
            "round trip lost bytes for {foreign:?}"
        );
    }
}

#[test]
fn unterminated_last_line_gets_a_newline_before_our_entry() {
    // Cron ignores an unterminated last line, so it never ran; the newline keeps it from being
    // glued to our entry. This case deliberately does not round-trip.
    let (out, _) = merge("0 3 * * * /bin/backup", &line());
    assert_eq!(out, format!("0 3 * * * /bin/backup\n{}\n", line()));
}
