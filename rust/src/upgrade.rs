//! cort-upgrade: diagnose an existing install, then bring it to what THIS tree requires.
//!
//! Repo-local binary, never installed. It runs the new tree's code, so it inherently knows what
//! the new release needs; its only question about the machine is "what's installed" (spec §1).
//! Every inventory it checks is consumed from `cort::install` or the shipped binary's own verbs
//! — restating one is the HOOK_TARGETS sin this module exists to end.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentState {
    Current,
    Drifted,
    Unreadable,
    Absent,
    DeferredByUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    // Owned: per-project index components carry the project path, which is never 'static.
    pub name: String,
    pub state: ComponentState,
    pub detail: String,
}

/// Pack identity for a directory: same construction as `pack::extractor_version` but over an
/// arbitrary dir, so tests can build two packs that differ by one byte. Sorted file list, hashed
/// contents, MIXED WITH the scan engine identity — exactly like `extractor_version` does:
/// the same bytes through a different engine are a different extractor.
pub fn pack_identity(dir: &Path) -> std::io::Result<String> {
    let mut files: Vec<_> = walk_yaml(dir)?;
    files.sort();
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for f in files {
        h.update(&fs::read(&f)?);
    }
    h.update(crate::scan::SCAN_ENGINE.as_bytes());
    Ok(format!("{:x}", h.finalize()))
}

fn walk_yaml(dir: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            out.extend(walk_yaml(&p)?);
        } else if p.extension().map(|e| e == "yml").unwrap_or(false) {
            out.push(p);
        }
    }
    Ok(out)
}

/// The shim check: read the manifest, resolve cort_bin, diff against render_shim. Unreadable
/// manifest (read ERROR, not empty) is Unreadable — never Absent, never Current.
pub fn check_shim(manifest: &Path, cort_home: &Path) -> ComponentState {
    let contents = match fs::read_to_string(manifest) {
        Ok(c) => c,
        Err(_) => return ComponentState::Unreadable,
    };
    let Some(bin_line) = contents.lines().find(|l| l.starts_with("cort_bin:")) else {
        return ComponentState::Absent;
    };
    let bin_path = Path::new(bin_line.trim_start_matches("cort_bin:"));
    let on_disk = match fs::read_to_string(bin_path) {
        Ok(s) => s,
        Err(_) => return ComponentState::Unreadable,
    };
    let expected = crate::install::render_shim(&cort_home.to_string_lossy());
    if on_disk.trim_end() == expected.trim_end() {
        ComponentState::Current
    } else {
        ComponentState::Drifted
    }
}

/// Installed ast-grep CLI output vs the tree pin. Pure string comparison: the caller (Task 5
/// sequencing the installed binary's `--version`) owns subprocess policy; this function only
/// judges. Empty or unparseable installed output is Unreadable — an empty string never equals
/// the pin, so without this arm a missing binary would read as Drifted and demand an update to
/// a parser that was never there.
pub fn check_version_pin(installed: &str, pinned: &str) -> Component {
    let name = "ast_grep".to_string();
    let installed = installed.trim();
    if installed.is_empty() {
        return Component {
            name,
            state: ComponentState::Unreadable,
            detail: "no version output to read".into(),
        };
    }
    if installed == pinned {
        Component {
            name,
            state: ComponentState::Current,
            detail: String::new(),
        }
    } else {
        Component {
            name,
            state: ComponentState::Drifted,
            detail: format!("installed reports {installed}, tree pins {pinned}"),
        }
    }
}

/// Live manifest key set vs the tree authority. `live` is every `xxx:` key prefix present in the
/// manifest file (the caller reads the file; this function diffs). Unknown keys are Drifted with
/// names, never failures here — Task 5 maps states to exit codes.
pub fn check_manifest_keys(live: &[&str]) -> Component {
    let unknown: Vec<&&str> = live
        .iter()
        .filter(|k| {
            !crate::install::MANIFEST_KEYS.contains(k)
                && !crate::install::MANIFEST_LEGACY_KEYS.contains(k)
        })
        .collect();
    if unknown.is_empty() {
        Component {
            name: "manifest_keys".to_string(),
            state: ComponentState::Current,
            detail: String::new(),
        }
    } else {
        let names: Vec<String> = unknown.iter().map(|k| k.to_string()).collect();
        Component {
            name: "manifest_keys".to_string(),
            state: ComponentState::Drifted,
            detail: format!("keys no release knows: {}", names.join(", ")),
        }
    }
}

/// Stored usage-schema version vs the tree constant. `None` (file unreadable or key absent)
/// is Unreadable: absence of evidence, reported, never passed and never silently drifted.
pub fn check_usage_schema(stored: Option<&str>, current: &str) -> Component {
    const NAME: &str = "usage_db";
    match stored {
        None => Component {
            name: NAME.to_string(),
            state: ComponentState::Unreadable,
            detail: "usage.db unreadable or version key absent".into(),
        },
        Some(v) if v == current => Component {
            name: NAME.to_string(),
            state: ComponentState::Current,
            detail: String::new(),
        },
        Some(v) => Component {
            name: NAME.to_string(),
            state: ComponentState::Drifted,
            detail: format!("usage.db schema {v}, tree expects {current}"),
        },
    }
}

/// Read-only reasons for one project db: open WITHOUT migrating (no `ensure_schema` — the
/// upgrader must not migrate schema as a side effect of looking) and run plan 2's shared
/// reader. `None` = could not read = Unreadable downstream, never empty-debt.
pub fn read_reasons_readonly(db_path: &str) -> Option<Vec<String>> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    // Spec §10 item 1: the scan connection used to be opened with no busy timeout, so one
    // transient SQLITE_BUSY under a concurrent refresh-hook write reported a healthy index
    // Unreadable. Same 5s the rest of db.rs uses; contention now waits instead of lying.
    conn.busy_timeout(std::time::Duration::from_secs(5)).ok()?;
    crate::indexer::rebuild_reasons(&conn).ok()
}

pub struct DiagnoseInputs<'a> {
    /// Dir holding `manifest` + `cort/` (cort_home = install_root/cort, pack = cort_home/pack).
    pub install_root: &'a Path,
    /// The staged new generation's pack dir (Task 0 produces it).
    pub new_pack: &'a Path,
    /// Installed ast-grep `--version` stdout, gathered by the CALLER (Task 5 owns subprocess
    /// policy incl. deadlines; this function only judges — same seam as check_version_pin).
    pub installed_ast_grep_version: &'a str,
}

/// Compose the Task-1 components. Task 3 extends this fn with skill/hook components; the
/// per-component fns above stay the unit seams (the e2e test calls this, the break tests call
/// those — both levels observe something the other cannot).
pub fn diagnose(inputs: &DiagnoseInputs) -> Vec<Component> {
    let mut out = Vec::new();
    let manifest = inputs.install_root.join("manifest");
    let cort_home = inputs.install_root.join("cort");
    out.push(Component {
        name: "shim".to_string(),
        state: check_shim(&manifest, &cort_home),
        detail: String::new(),
    });
    out.push(
        match (
            pack_identity(&cort_home.join("pack")),
            pack_identity(inputs.new_pack),
        ) {
            (Ok(a), Ok(b)) if a == b => Component {
                name: "pack".into(),
                state: ComponentState::Current,
                detail: String::new(),
            },
            (Ok(a), Ok(b)) => Component {
                name: "pack".into(),
                state: ComponentState::Drifted,
                detail: format!("installed pack {a}, new pack {b}"),
            },
            _ => Component {
                name: "pack".into(),
                state: ComponentState::Unreadable,
                detail: "a pack dir could not be hashed".into(),
            },
        },
    );
    out.push(check_version_pin(
        inputs.installed_ast_grep_version,
        crate::install::AST_GREP_PINNED,
    ));
    // Manifest keys: every `xxx:` prefix in the live file. An unreadable manifest is its own
    // Unreadable component — the key-set cannot be diffed from bytes we could not read, and
    // empty-keys-would-read-Current is exactly the false-pass this plan keeps refusing.
    out.push(match fs::read_to_string(&manifest) {
        Err(_) => Component {
            name: "manifest_keys".into(),
            state: ComponentState::Unreadable,
            detail: "manifest unreadable".into(),
        },
        Ok(contents) => {
            let live: Vec<&str> = contents
                .lines()
                .filter_map(|l| l.split(':').next())
                .collect();
            check_manifest_keys(&live)
        }
    });
    // usage.db: no file yet → Absent (fresh machine, never a failure — Task 5's verdict
    // keeps that promise); present-but-unreadable → Unreadable.
    out.push(match crate::usage::usage_db_path() {
        Some(p) if p.exists() => check_usage_schema(
            crate::usage::read_schema_version(&p).as_deref(),
            &crate::usage::USAGE_SCHEMA_VERSION.to_string(),
        ),
        _ => Component {
            name: "usage_db".into(),
            state: ComponentState::Absent,
            detail: "no usage.db yet".into(),
        },
    });
    // Indexes, read-only: empty reasons → Current; non-empty → Drifted WITH the reasons named
    // (Task 4 repays them; diagnosis only names them). Unreadable db → Unreadable.
    for entry in crate::db::list_projects() {
        match entry {
            crate::db::ProjectEntry::Unreadable { db_path, reason } => {
                out.push(Component {
                    name: "index_unreadable".into(),
                    state: ComponentState::Unreadable,
                    detail: format!("{db_path}: {reason}"),
                });
            }
            crate::db::ProjectEntry::Indexed(row) => {
                let name = format!("index:{}", row.path);
                match read_reasons_readonly(&row.db_path) {
                    None => out.push(Component {
                        name,
                        state: ComponentState::Unreadable,
                        detail: format!("{}: metadata unreadable", row.db_path),
                    }),
                    Some(reasons) if reasons.is_empty() => out.push(Component {
                        name,
                        state: ComponentState::Current,
                        detail: String::new(),
                    }),
                    Some(reasons) => out.push(Component {
                        name,
                        state: ComponentState::Drifted,
                        detail: format!("needs: {}", reasons.join(", ")),
                    }),
                }
            }
        }
    }
    out
}
