//! claudecat 自己的每機資料目錄（XDG Data Home 慣例）：`update` 產生的地圖等 derived
//! 資料放這裡，不進 repo；repo 內的 CLAUDE.md/AGENTS.md 只放手寫規則。
//! 與 cort 模組（唯讀消費 cort 的 SQLite）無關——這是 claudecat 唯一寫「自己資料」的地方。
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 資料根目錄：`CLAUDECAT_DATA_DIR` > `${XDG_DATA_HOME:-$HOME/.local/share}/claudecat`。
/// 空字串視同未設（同 `cort::cache_dir` 對 `CORT_CACHE_DIR` 的口徑）；HOME 也缺席時退回
/// 相對路徑 `.claudecat`——`walk::ALWAYS_EXCLUDE` 已排除該名，意外落在專案內也不進掃描。
pub fn data_dir() -> PathBuf {
    if let Some(v) = std::env::var_os("CLAUDECAT_DATA_DIR") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("claudecat");
        }
    }
    match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h).join(".local/share/claudecat"),
        _ => PathBuf::from(".claudecat"),
    }
}

/// 專案資料目錄：`data_dir()/projects/<cort::project_id(real_path)>`。
/// id 與 cort 的 DB 檔同名同源（sha256(canonical path)），一個 `ls projects/`
/// 同時解釋兩個工具的 per-project 檔。
pub fn project_data_dir(real_path: &str) -> PathBuf {
    data_dir()
        .join("projects")
        .join(crate::cort::project_id(real_path))
}

/// 地圖檔位置。
pub fn map_path_for(real_path: &str) -> PathBuf {
    project_data_dir(real_path).join("map.md")
}

/// 地圖檔內容：固定標頭 + 既有 outline render 原樣置入。
pub fn map_file_body(section: &str) -> String {
    format!(
        "# claudecat Project Map\n\n\
         <!-- 由 `claudecat update` 產生並整檔改寫；放在各機器的資料目錄（不進 git）。\
         路徑＝<資料目錄>/projects/<專案根目錄 sha256>/map.md。 -->\n\n\
         {section}"
    )
}

/// 原子寫入：create_dir_all + 同目錄唯一 tmp（`claudecat.<pid>.<nanos>.tmp`）+ fsync + rename。
/// 目標是 claudecat 自有檔（HOME 下、永不 symlink），不需要 claude_md 的 read_link 穿透。
pub fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "claudecat.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    {
        use std::io::Write;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}
