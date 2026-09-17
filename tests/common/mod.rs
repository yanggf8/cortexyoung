#![allow(dead_code)] // shared helpers: 不同 test binary 不一定用到全部
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn temp_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!("claudecat-test-{}", std::process::id()));
    // 純 nanos 會在平行測試同 tick 起跑時撞位（兩個 fixture 共用一個目錄、
    // 互相覆寫對方的檔案），疊一個進程內單調序號保證唯一。
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) as u128;
    let d = base.join(format!("{nanos}{}", (seq << 20)));
    fs::create_dir_all(&d).unwrap();
    d
}

pub struct Tmp(pub PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}
