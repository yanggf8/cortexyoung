//! Pack enumeration + hash.
//! cort does not parse YAML — ast-grep reads `sgconfig.yml`; we only walk and hash.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Absolute path of the ast-grep pack directory.
///
/// The installed binary cannot lean on `CARGO_MANIFEST_DIR` — that is a compile-time
/// path, valid only on the machine that built it. The installer therefore sets
/// `CORT_PACK_DIR`; without it the build-tree layout is the fallback, and a caller
/// that sets the variable to a directory without an `sgconfig.yml` gets a
/// fail-closed error from `sgconfig()` instead of an empty hash over nothing.
pub fn pack_dir() -> PathBuf {
    if let Ok(over) = std::env::var("CORT_PACK_DIR") {
        if !over.is_empty() {
            return PathBuf::from(over);
        }
    }
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("src")
        .join("pack");
    fs::canonicalize(&p).unwrap_or(p)
}

pub fn sgconfig() -> PathBuf {
    let p = pack_dir().join("sgconfig.yml");
    if !p.is_file() {
        panic!(
            "cort pack is missing {}: the installer must deploy src/pack (see CORT_PACK_DIR)",
            p.display()
        );
    }
    p
}

/// Recurse `pack_dir()`, keep files whose path ends with `.yml`, sort.
///
/// Every failure is returned. It used to swallow all of them -- `read_dir` returned silently and an
/// entry whose `file_type` failed was skipped -- which is far worse here than a missing file
/// normally is: the list feeds `extractor_version`, so a pack that is unreadable, or half-copied by
/// an installer mid-swap, produced a *shorter* list and therefore a hash that looks perfectly
/// legitimate. Stamped into an index by `full_index`, that identity never matches again and never
/// explains itself. An absent pack is a refusal, not an empty one.
pub fn pack_files() -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(&pack_dir(), &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if entry.file_type()?.is_dir() {
            walk(&p, out)?;
        } else if entry.file_type()?.is_file() {
            let as_str = p.to_string_lossy();
            if as_str.ends_with(".yml") {
                out.push(p);
            }
        }
    }
    Ok(())
}

/// SHA-256 of each pack file's raw bytes, in `pack_files()` order, mixed with the scan engine's
/// identity. 64 lowercase hex.
///
/// The pack bytes alone stopped describing extraction when the scan moved in-process
/// (2026-09-01): the same pack through the crate and through the CLI is parity-proven identical,
/// but they are still different engines, and an engine that could flip via `CORT_SCAN_BACKEND`
/// without moving this version would make staleness lie. The engine string changes whenever the
/// `ast-grep-*` crate entries move; the parity probe is the discipline that re-answers
/// byte-identity when it does.
pub fn extractor_version() -> std::io::Result<String> {
    let mut h = Sha256::new();
    for f in pack_files()? {
        h.update(&fs::read(&f)?);
    }
    h.update(crate::scan::SCAN_ENGINE.as_bytes());
    Ok(format!("{:x}", h.finalize()))
}
