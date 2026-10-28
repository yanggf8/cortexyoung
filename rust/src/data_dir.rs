//! Durable cort data, including memory credentials, locks, and upload state.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("CORT_MEMORY_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_DATA_HOME").filter(|p| !p.is_empty()) {
        return PathBuf::from(path).join("cortexyoung/memory");
    }
    std::env::var_os("HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/cortexyoung/memory")
}

pub fn project_data_dir(real_path: &str) -> PathBuf {
    data_dir()
        .join("projects")
        .join(crate::db::project_id_for(real_path))
}

pub fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("cort.{}.{}.tmp", std::process::id(), stamp));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    fs::rename(tmp, path)
}
