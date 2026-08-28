//! Pack enumeration + hash. JS `src/pack.js`.
//! cort does not parse YAML — ast-grep reads `sgconfig.yml`; we only walk and hash.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Absolute path of repo `src/pack` (JS `import.meta.url` of `src/pack.js`).
pub fn pack_dir() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("src")
        .join("pack");
    fs::canonicalize(&p).unwrap_or(p)
}

/// Absolute path of `src/pack/sgconfig.yml`.
pub fn sgconfig() -> PathBuf {
    pack_dir().join("sgconfig.yml")
}

/// Recurse `pack_dir()`, keep files whose path ends with `.yml`, sort.
pub fn pack_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&pack_dir(), &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            walk(&p, out);
        } else if ft.is_file() {
            let as_str = p.to_string_lossy();
            if as_str.ends_with(".yml") {
                out.push(p);
            }
        }
    }
}

/// SHA-256 of each pack file's raw bytes, in `pack_files()` order. 64 lowercase hex.
pub fn extractor_version() -> String {
    let mut h = Sha256::new();
    for f in pack_files() {
        let bytes = fs::read(&f).unwrap_or_else(|e| panic!("read pack file {}: {e}", f.display()));
        h.update(&bytes);
    }
    format!("{:x}", h.finalize())
}
