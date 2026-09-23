//! CLAUDE.md 的規則檔維護。v2.1 起本檔只放手寫規則＋claudecat 的兩個種子區；
//! 自動生成的 Project Map 改放各機器的資料目錄（`data_dir::map_path_for`），
//! 不再寫進 CLAUDE.md——這裡只剩「剝除舊版 auto 區塊」與「播種」兩件事。
use std::fs;
use std::path::{Path, PathBuf};

/// 舊版（<=2.0）auto 區塊的標記。今天只剩 strip 認得它們，沒有任何程式路徑再寫入。
pub const BEGIN_MARKER: &str = "<!-- claudecat:auto:begin -->";
pub const END_MARKER: &str = "<!-- claudecat:auto:end -->";

/// 地圖位置指標的種子標記。內容機器中立——只描述慣例路徑，不含本機絕對路徑或 hash，
/// 因此提交後在每台機器 byte-identical。
pub const PTR_BEGIN: &str = "<!-- claudecat:map-pointer:begin -->";
pub const PTR_END: &str = "<!-- claudecat:map-pointer:end -->";

pub fn find_claude_md(root: &Path) -> PathBuf {
    // V2: 只寫 --root/CLAUDE.md，不做向上搜尋（避免子目錄汙染父專案）。
    // 需要父專案時請自行指定，或之後提供 --discover-claude-md。
    root.join("CLAUDE.md")
}

/// 地圖位置指標種子：播種一次、之後永不改寫（同 guardrails 的契約）。
pub fn pointer_block() -> String {
    format!(
        "{PTR_BEGIN}\n- 專案地圖（Project Map）不在本檔：`claudecat update` 會把它寫到各機器的 `~/.local/share/claudecat/<project-id>/map.md`（`CLAUDECAT_DATA_DIR` 可覆蓋），並在輸出印出 `map ->` 的確切路徑。本檔只放手寫規則，產生的地圖不進 git。本區由 claudecat 播種一次，之後永不改寫。\n{PTR_END}\n"
    )
}

pub fn has_pointer(content: &str) -> bool {
    content.contains(PTR_BEGIN) && content.contains(PTR_END)
}

/// 移除全部舊版 auto 區塊，回傳 (新內容, 是否移除過)。
///
/// 邊界（皆有意為之）：
/// - 標記「整行」一起移除——吃掉殘留字元（claudecat repo 歷史上 begin 行行首有個反引號）。
/// - 有 begin 無 end（截斷檔）：自 begin 行移除到檔尾。
/// - 前後以恰一個空行接回；全文剝空則回空字串。
/// - 循環剝除：多個（含手動複製出的）區塊一次清乾。
/// - 用戶正文若含字面 marker 字串會被一併移除——與 2.0 的 replace 同等暴露，接受。
pub fn strip_auto_block(content: &str) -> (String, bool) {
    let mut out = content.to_string();
    let mut removed = false;
    loop {
        let Some(begin) = out.find(BEGIN_MARKER) else {
            return (out, removed);
        };
        let line_start = out[..begin].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let Some(end_rel) = out[begin..].find(END_MARKER) else {
            let mut head = out[..line_start].to_string();
            while head.ends_with('\n') || head.ends_with('\r') {
                head.pop();
            }
            if !head.is_empty() {
                head.push('\n');
            }
            return (head, true);
        };
        let end = begin + end_rel + END_MARKER.len();
        let line_end = out[end..]
            .find('\n')
            .map(|i| end + i + 1)
            .unwrap_or(out.len());
        let before = out[..line_start].trim_end_matches(['\n', '\r']);
        let after = out[line_end..].trim_start_matches(['\n', '\r']);
        out = match (before.is_empty(), after.is_empty()) {
            (true, true) => String::new(),
            (true, false) => format!("{after}\n"),
            (false, true) => format!("{before}\n"),
            (false, false) => format!("{before}\n\n{after}\n"),
        };
        removed = true;
    }
}

/// v2.1：CLAUDE.md 只放手寫規則＋種子。移除舊版 auto 區塊、確保 guardrails 與地圖
/// 指標兩個種子（都只在缺時播種、之後永不改寫）。
/// 回傳 (changed, new_content)；dry_run 只算差異不寫檔。
pub fn update_section(path: &Path, dry_run: bool) -> std::io::Result<(bool, String)> {
    // 讀取失敗必須傳播：整檔改寫的世界裡，把讀不到當成空檔等於銷毀它。
    let existing = if path.is_file() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    let (stripped, _) = strip_auto_block(&existing);
    let mut new_content = stripped;
    let mut changed = new_content != existing.as_str();

    // First-time setup: seed a guardrails block (never overwrite after creation)
    if !crate::guardrails::has_marker(&new_content) {
        let mut with_seed = new_content.clone();
        if !with_seed.ends_with('\n') {
            with_seed.push('\n');
        }
        with_seed.push_str(&crate::guardrails::seed_block());
        changed |= with_seed != existing.as_str();
        if changed && !dry_run {
            new_content = with_seed;
        }
    }

    // First-time setup: seed the map-pointer rule (same never-overwrite contract)
    if !has_pointer(&new_content) {
        let mut with_seed = new_content.clone();
        if !with_seed.ends_with('\n') {
            with_seed.push('\n');
        }
        with_seed.push_str(&pointer_block());
        changed |= with_seed != existing.as_str();
        if changed && !dry_run {
            new_content = with_seed;
        }
    }

    if changed && !dry_run {
        // CLAUDE.md 常見是 symlink（如 cortexyoung：CLAUDE.md -> AGENTS.md，讓兩個
        // harness 永不漂移）。rename 直接蓋 path 會把 symlink 換成普通檔——
        // 寫入目標必須是解析後的本體；tmp 也放目標目錄，rename 才是同檔案系統原子操作。
        let target = match fs::read_link(path) {
            Ok(link) if link.is_absolute() => link,
            Ok(link) => path.parent().unwrap_or_else(|| Path::new(".")).join(link),
            Err(_) => path.to_path_buf(),
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        // atomic write: temp file + rename（唯一 tmp 名 + fsync）
        let tmp = target.with_extension(format!(
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
            f.write_all(new_content.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &target)?;
    }
    Ok((changed, new_content))
}
