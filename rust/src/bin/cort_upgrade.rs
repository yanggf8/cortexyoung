//! `cort-upgrade` — diagnose an existing install, then bring it to what THIS tree requires.
//!
//! Repo-local, never installed: it runs the new tree's code, so it inherently knows what the
//! new release needs; its only question about the machine is "what's installed" (spec §1).
//! The bin sequences and prints; every decision is a function in `cort::upgrade`, tested
//! there — this file holds no logic worth unit-testing, only the order the plan fixes:
//! diagnose (unlocked) → stage (unlocked, invisible) → locks → activate → re-verify →
//! rewire → migrate → release → verdict. `--check` is step one plus the verdict, and takes
//! no lock and writes nothing by construction.

use clap::Parser;
use cort::upgrade::{
    acquire_upgrade_locks, check_hooks, check_skills, diagnose_for_check, first_upgrade_note,
    load_acks, migrate_indexes, repair_skill, run_capture_with_deadline, run_status_with_deadline,
    save_ack, verdict, Component, ComponentState, DiagnoseInputs, LockError, UpgradeExit,
};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "cort-upgrade",
    about = "Bring an installed cort to what this tree requires"
)]
struct Args {
    /// Diagnose only: no locks, no writes, no repair. Same exit taxonomy.
    #[arg(long)]
    check: bool,
    /// Accept one drifted component by name, this run and every later one.
    #[arg(long = "ack")]
    acks: Vec<String>,
    /// Keep my edited skills (defer their redeploy; reported DeferredByUser).
    #[arg(long = "keep-mine")]
    keep_mine: bool,
    /// Record index debt without rebuilding (passes --defer to the migration).
    #[arg(long)]
    defer: bool,
}

/// The repo this binary was built from, derived from the executable's own location
/// (`rust/target/<profile>/cort_upgrade` → three levels up) and verified by the two
/// directories staging needs. A moved or stripped copy refuses rather than guessing.
fn repo_root() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut cur = exe.as_path();
    for _ in 0..3 {
        cur = cur
            .parent()
            .ok_or_else(|| "executable has no parent chain".to_string())?;
    }
    let root = cur.to_path_buf();
    if root.join("src/pack/sgconfig.yml").is_file() && root.join("skills").is_dir() {
        Ok(root)
    } else {
        Err(format!(
            "{} does not look like the cortexyoung tree (no src/pack, no skills)",
            root.display()
        ))
    }
}

fn manifest_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    base.join("cortexyoung")
}

fn manifest_value(manifest: &Path, key: &str) -> Option<String> {
    let raw = std::fs::read_to_string(manifest).ok()?;
    raw.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}:")))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn state_word(s: &ComponentState) -> &'static str {
    match s {
        ComponentState::Current => "current",
        ComponentState::Drifted => "drifted",
        ComponentState::Unreadable => "unreadable",
        ComponentState::Absent => "absent",
        ComponentState::DeferredByUser => "deferred-by-user",
    }
}

fn next_action(c: &Component) -> &'static str {
    match c.state {
        ComponentState::Current => "",
        ComponentState::Drifted => " (rerun cort-upgrade, or --ack to accept)",
        ComponentState::Unreadable => " (inspect the named file — this never passes silently)",
        ComponentState::Absent => " (debt recorded; nothing to do)",
        ComponentState::DeferredByUser => " (--keep-mine; drop the flag to redeploy)",
    }
}

fn print_verdict(v: &cort::upgrade::Verdict) {
    for c in &v.components {
        let detail = if c.detail.is_empty() {
            String::new()
        } else {
            format!(" — {}", c.detail)
        };
        println!(
            "{}: {}{}{}",
            c.name,
            state_word(&c.state),
            detail,
            next_action(c)
        );
    }
}

fn fatal(msg: &str) -> ! {
    eprintln!("cort-upgrade: fatal: {msg}");
    exit(2)
}

/// One bounded status poll against the installed shim, shared by every phase.
fn status_runner(shim: &Path) -> impl Fn() -> Result<String, String> + '_ {
    move || run_status_with_deadline(shim, Duration::from_secs(10))
}

fn main() {
    let args = Args::parse();

    let install_root = manifest_dir();
    let manifest = install_root.join("manifest");
    let Some(shim) = manifest_value(&manifest, "cort_bin").map(PathBuf::from) else {
        fatal("no installation found (no cort_bin in the manifest) — run ./install.sh first");
    };
    let root = match repo_root() {
        Ok(r) => r,
        Err(e) => fatal(&e),
    };
    let new_pack = root.join("src/pack");
    let new_tree = root.clone();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());

    // The installed ast-grep's own --version, gathered here and only judged in diagnose —
    // subprocess policy (deadlines) lives with the other invocations.
    let installed_version = manifest_value(&manifest, "ast_grep_bin")
        .and_then(|ag| {
            run_capture_with_deadline(Path::new(&ag), &["--version"], Duration::from_secs(10))
                .ok()
                .map(|(out, _)| out)
        })
        .unwrap_or_default();

    let cache = cort::db::cache_dir();
    let mut cli_acks = args.acks.clone();

    // ── Step 1: diagnose, unlocked. --check stops here. ─────────────
    let inputs = DiagnoseInputs {
        install_root: &install_root,
        new_pack: &new_pack,
        installed_ast_grep_version: &installed_version,
        new_tree: &new_tree,
        home: &home,
        keep_mine: args.keep_mine,
    };
    let shim_ref: &Path = &shim;
    let runner = status_runner(shim_ref);
    let mut components = diagnose_for_check(&inputs, shim_ref, &runner);
    if args.check {
        if let Some(note) = first_upgrade_note(&cache) {
            components.push(note);
        }
        let acks = load_acks(&cache);
        let borrowed: Vec<&str> = acks.iter().map(String::as_str).collect();
        let v = verdict(components, &borrowed);
        print_verdict(&v);
        exit(match v.exit {
            UpgradeExit::Ok => 0,
            UpgradeExit::Partial => 1,
            UpgradeExit::Fatal => 2,
        });
    }

    // The steady state must be cheap and touch nothing: everything Current → no locks, no
    // install.sh, exit 0.
    let all_current = components
        .iter()
        .all(|c| matches!(c.state, ComponentState::Current));
    if all_current {
        if let Some(note) = first_upgrade_note(&cache) {
            components.push(note);
        }
        let acks = load_acks(&cache);
        let borrowed: Vec<&str> = acks.iter().map(String::as_str).collect();
        let v = verdict(components, &borrowed);
        print_verdict(&v);
        exit(match v.exit {
            UpgradeExit::Ok => 0,
            UpgradeExit::Partial => 1,
            UpgradeExit::Fatal => 2,
        });
    }

    // ── Step 2: stage the new generation — unlocked, invisible, validated. ──
    let install_sh = root.join("install.sh");
    let install_bin = Path::new("bash");
    let stage_owned = [
        install_sh.to_string_lossy().into_owned(),
        "--stage-only".to_string(),
    ];
    let stage_args: Vec<&str> = stage_owned.iter().map(String::as_str).collect();
    let (stage_out, _) =
        match run_capture_with_deadline(install_bin, &stage_args, Duration::from_secs(15 * 60)) {
            Ok(x) => x,
            Err(e) => fatal(&format!("staging failed: {e}")),
        };
    let gen_id = stage_out.trim().to_string();
    if !gen_id.starts_with("cort-") {
        fatal(&format!(
            "staging produced no generation id (stdout: {stage_out:?})"
        ));
    }

    // ── Step 3: take both locks — 30s drain, killing nothing. ───────
    let _locks = match acquire_upgrade_locks(&cache, Duration::from_secs(30)) {
        Ok(l) => l,
        Err(e @ (LockError::DrainTimeout | LockError::AdmissionBusy)) => {
            fatal(&format!(
                "cannot take the upgrade locks ({e:?}): a worker is mid-operation; \
                 retry when quiet"
            ));
        }
        Err(e) => fatal(&format!("cannot take the upgrade locks ({e:?})")),
    };

    // ── Step 4: activate ONLY the payload — no skills, no hooks, so Task 3's policy
    // (keep-mine, hook shape) is never pre-empted by the installer. ──
    let act_owned = [
        install_sh.to_string_lossy().into_owned(),
        "--activate-only".to_string(),
        "--gen".to_string(),
        gen_id,
    ];
    let act_args: Vec<&str> = act_owned.iter().map(String::as_str).collect();
    if let Err(e) = run_capture_with_deadline(install_bin, &act_args, Duration::from_secs(5 * 60)) {
        fatal(&format!("activation failed: {e}"));
    }

    // ── Step 5: re-verify, then rewire. Every commit boundary re-diagnoses — §5's "diagnose
    // once and trust it" is the bug, not the discipline. ────────────
    let mut components = diagnose_for_check(&inputs, shim_ref, &runner);

    // Hooks: judge → repair → re-run status → re-judge, all inside check_hooks. Repair is
    // the product's own installer verb, aimed at the same shim.
    let shim_for_repair = shim.clone();
    let repair = move || {
        let _ = run_capture_with_deadline(
            &shim_for_repair,
            &[
                "hook-install",
                "--all",
                "--command-prefix",
                &shim_for_repair.to_string_lossy(),
            ],
            Duration::from_secs(60),
        );
    };
    components.push(check_hooks(shim_ref, &runner, &repair));

    // Skills: content-diff, repair only the managed-and-drifted, then re-check — a repair is
    // reported as landed only after the re-check says so.
    let mut skills = check_skills(&new_tree, &home, args.keep_mine);
    let needs_repair = |name: &str| -> Option<(PathBuf, PathBuf)> {
        let (src, dest) = match name {
            "skill_xgrep" => (
                new_tree.join("skills/xgrep/SKILL.md"),
                home.join(".claude/skills/xgrep/SKILL.md"),
            ),
            "skill_ast_grep" => (
                new_tree.join("skills/ast-grep/SKILL.md"),
                home.join(".claude/skills/ast-grep/SKILL.md"),
            ),
            "skill_ast_grep_codex" => (
                new_tree.join("skills/ast-grep/SKILL.md"),
                home.join(".codex/skills/ast-grep/SKILL.md"),
            ),
            _ => return None,
        };
        Some((src, dest))
    };
    for c in skills.clone() {
        if matches!(c.state, ComponentState::Drifted) && c.detail.contains("managed") {
            if let Some((src, dest)) = needs_repair(&c.name) {
                let _ = repair_skill(&src, &dest);
            }
        }
    }
    if skills
        .iter()
        .any(|c| matches!(c.state, ComponentState::Drifted))
    {
        skills = check_skills(&new_tree, &home, args.keep_mine);
    }
    components.append(&mut skills);

    // Indexes: eager for live directories, deferred for gone ones (Task 4).
    components.append(&mut migrate_indexes(args.defer));

    if let Some(note) = first_upgrade_note(&cache) {
        components.push(note);
    }

    // ── Step 6: verdict, print, release (drop _locks), then the marker. ──
    for a in &cli_acks {
        if let Err(e) = save_ack(&cache, a) {
            fatal(&format!("cannot persist --ack {a}: {e}"));
        }
    }
    let mut acks = load_acks(&cache);
    for a in &cli_acks {
        if !acks.iter().any(|n| n == a) {
            acks.push(a.clone());
        }
    }
    let borrowed: Vec<&str> = acks.iter().map(String::as_str).collect();
    let v = verdict(components, &borrowed);
    print_verdict(&v);

    if matches!(v.exit, UpgradeExit::Ok) {
        // Only a FULLY successful run marks itself done — the marker gates the one-time
        // partial-drain note, and a failed upgrade must not silence it.
        let _ = cort::upgrade::write_first_upgrade_marker(&cache);
    }
    let _ = &mut cli_acks;
    exit(match v.exit {
        UpgradeExit::Ok => 0,
        UpgradeExit::Partial => 1,
        UpgradeExit::Fatal => 2,
    });
}
