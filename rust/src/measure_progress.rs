//! Durable local upload progress. Remote rows are consulted only to rebuild a lost file.

use crate::measure_store::MeasureStore;
use crate::telemetry::{health_row_fingerprint, row_fingerprint, SOURCE_COMMAND, SOURCE_HEALTH};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watermark {
    pub rowid: i64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    version: u32,
    pub watermarks: BTreeMap<String, BTreeMap<String, Watermark>>,
    pub compactions: BTreeMap<String, String>,
    pub compactions_complete: bool,
    /// Whether the event watermarks have been seeded from the remote after a rebuild. A rebuild
    /// without a usage.db to check fingerprints against, or whose read failed, leaves this false
    /// so a later run retries the seed instead of settling on no watermark.
    #[serde(default)]
    pub watermarks_checked: bool,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            version: VERSION,
            watermarks: BTreeMap::new(),
            compactions: BTreeMap::new(),
            compactions_complete: false,
            watermarks_checked: false,
        }
    }
}

pub enum Load {
    Loaded(Progress),
    Missing,
    Corrupt,
}

pub fn load(path: &Path) -> Load {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Progress>(&text) {
            Ok(progress) if progress.version == VERSION => Load::Loaded(progress),
            _ => Load::Corrupt,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Load::Missing,
        Err(_) => Load::Corrupt,
    }
}

impl Progress {
    pub fn watermark(&self, machine: &str, source: &str) -> Option<&Watermark> {
        self.watermarks.get(machine)?.get(source)
    }

    pub fn set_watermark(&mut self, machine: &str, source: &str, rowid: i64, fingerprint: String) {
        self.watermarks
            .entry(machine.into())
            .or_default()
            .insert(source.into(), Watermark { rowid, fingerprint });
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = path.with_extension(format!("cort.{}.{}.tmp", std::process::id(), stamp));
        let result = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, path)?;
            // The rename itself is durable only once the directory entry is.
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

/// The returned label describes how this run obtained progress.
pub fn open(
    store: &mut dyn MeasureStore,
    path: &Path,
    machine: &str,
    usage_db: Option<&Path>,
) -> Result<(Progress, &'static str), String> {
    let (mut progress, rebuilt) = match load(path) {
        Load::Loaded(progress) => (progress, false),
        Load::Missing | Load::Corrupt => (Progress::default(), true),
    };
    let unchecked = !progress.watermarks_checked && usage_db.is_some();
    if unchecked {
        let mut checked = true;
        for (source, fingerprint) in [
            (SOURCE_COMMAND, row_fingerprint as fn(&Path, i64) -> _),
            (SOURCE_HEALTH, health_row_fingerprint as fn(&Path, i64) -> _),
        ] {
            if progress.watermark(machine, source).is_some() {
                continue;
            }
            match store.event_watermark(machine, source) {
                // `unchecked` implies a usage.db. A local fingerprint that cannot be read settles
                // nothing either, so it is retried like a failed remote read.
                Ok(Some((rowid, prior))) => match usage_db.map(|db| fingerprint(db, rowid)) {
                    Some(Ok(Some(current))) if current == prior => {
                        progress.set_watermark(machine, source, rowid, prior);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => checked = false,
                },
                Ok(None) => {}
                Err(_) => checked = false,
            }
        }
        progress.watermarks_checked = checked;
    }
    let incomplete = !progress.compactions_complete;
    if incomplete {
        if let Ok(rows) = store.compaction_owners(machine) {
            progress.compactions = rows.into_iter().collect();
            progress.compactions_complete = true;
        }
    }
    if rebuilt || incomplete || unchecked {
        progress.save(path)?;
    }
    // Unchecked watermarks only make this run partial when it has events to upload.
    let label =
        if !progress.compactions_complete || (usage_db.is_some() && !progress.watermarks_checked) {
            "rebuilt_partial"
        } else if rebuilt || incomplete || unchecked {
            "rebuilt"
        } else {
            "loaded"
        };
    Ok((progress, label))
}
