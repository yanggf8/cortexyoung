//! explore：量化 Claude 若無地圖時的探索成本 vs 地圖成本
//! 作為長期指標：`claudecat track <file>` 把指標寫進文件的長期指標表
use crate::model::{MapProfile, ProjectMap};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;

pub const TRACK_SECTION: &str = "## 長期指標 (claudecat explore)";

#[derive(Debug, Clone, Serialize)]
pub struct CoverageRow {
    pub k: usize,
    pub loc: usize,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExploreMetrics {
    pub root: String,
    pub date: String,
    pub total_files: usize,
    pub total_loc: usize,
    pub map_chars: usize,
    pub map_tokens: usize,
    pub read_tokens: usize,
    pub savings_pct: f64,
    pub coverage: Vec<CoverageRow>,
    pub top10_pct: f64,
    pub map_overhead: bool,
}

fn est_tokens_from_loc(loc: usize) -> usize {
    loc * 6 // 粗略：每行約 6 tokens
}

pub fn compute(map: &ProjectMap) -> ExploreMetrics {
    compute_with_profile(map, MapProfile::Full)
}

pub fn compute_with_profile(map: &ProjectMap, profile: MapProfile) -> ExploreMetrics {
    let map_md = crate::outline::render_with_profile(map, profile);
    let map_chars = map_md.chars().count();
    let map_tokens = map_chars / 4;
    let read_tokens = est_tokens_from_loc(map.total_loc);
    let savings_pct = if read_tokens > 0 {
        (1.0 - map_tokens as f64 / read_tokens as f64) * 100.0
    } else {
        0.0
    };
    // 標記地圖成本是否反而超過全讀（小專案）
    let map_overhead = map_tokens > read_tokens && read_tokens > 0;
    let mut coverage = Vec::new();
    let mut acc = 0usize;
    for (i, f) in map.key_files.iter().enumerate() {
        acc += f.loc;
        let k = i + 1;
        if [5, 10, 20, 50].contains(&k) {
            coverage.push(CoverageRow {
                k,
                loc: acc,
                pct: acc as f64 / map.total_loc.max(1) as f64 * 100.0,
            });
        }
    }
    let top10_pct = coverage
        .iter()
        .find(|r| r.k == 10)
        .map(|r| r.pct)
        .unwrap_or(0.0);
    ExploreMetrics {
        root: map.root.clone(),
        date: map.generated_at[..10].to_string(),
        total_files: map.total_files,
        total_loc: map.total_loc,
        map_chars,
        map_tokens,
        read_tokens,
        savings_pct,
        coverage,
        top10_pct,
        map_overhead,
    }
}

pub fn render(m: &ExploreMetrics) -> String {
    let mut s = String::new();
    s.push_str("# Exploration Cost Report (claudecat explore)\n\n");
    s.push_str(&format!("- **Root**: `{}`\n", m.root));
    s.push_str(&format!(
        "- **Code scale**: {} files, {} LOC\n",
        m.total_files, m.total_loc
    ));
    s.push_str(&format!(
        "- **Map token cost**: ~{} tokens ({} chars)\n",
        m.map_tokens, m.map_chars
    ));
    s.push_str(&format!(
        "- **Read-everything cost**: ~{} tokens (LOC×6)\n",
        m.read_tokens
    ));
    if m.map_overhead {
        s.push_str(&format!(
            "- **Map overhead**: +{:.0}% (地圖成本高於全讀，小專案建議 mini)\n",
            m.savings_pct.abs()
        ));
    } else {
        s.push_str(&format!(
            "- **Map vs full-read**: 地圖省 ~{:.1}% token\n",
            m.savings_pct
        ));
    }

    s.push_str("\n## 只看 top-K 檔案的覆蓋率\n\n");
    s.push_str("| K | 累計 LOC | 佔總 LOC % |\n|---|--------:|----------:|\n");
    for r in &m.coverage {
        s.push_str(&format!("| {} | {} | {:.1}% |\n", r.k, r.loc, r.pct));
    }
    if m.total_loc > 0 {
        s.push_str(&format!("| 全部 | {} | 100% |\n", m.total_loc));
    }

    s.push_str("\n## 解讀\n\n");
    s.push_str(
        "- **Map token cost** = `claudecat update` 寫進 CLAUDE.md 的地圖成本（不到 1k tokens）。\n",
    );
    s.push_str(
        "- **Read-everything cost** = 若 Claude 沒有地圖、只能把全部程式碼讀完的粗估（LOC×6）。\n",
    );
    s.push_str(
        "- 真實 session 資料顯示 64–96% 工具呼叫花在探索；地圖把「找結構」變成「看地圖」。\n",
    );
    s
}

pub fn row_md(m: &ExploreMetrics) -> String {
    format!(
        "| {} | `{}` | {} | {} | ~{} | ~{} | {:.1}% | {:.1}% |",
        m.date,
        m.root,
        m.total_files,
        m.total_loc,
        m.map_tokens,
        m.read_tokens,
        m.savings_pct,
        m.top10_pct,
    )
}

/// section 邊界：s 內第一個以 `#` 開頭之行的 byte offset（找不到 → None＝到 EOF）。
/// 用 `split_inclusive` 逐行累加，offset 是位元組位置，對 UTF-8 安全。
/// `pub(crate)` 是因為發現區塊（`cort_audit::findings_update`）要用同一套邊界規則——
/// 第二份拷貝正是 2026-09-06 那個「從 section 掃到 EOF、吃掉使用者筆記」的 bug 的溫床。
pub(crate) fn next_heading_offset(s: &str) -> Option<usize> {
    let mut off = 0usize;
    for line in s.split_inclusive('\n') {
        if line.starts_with('#') {
            return Some(off);
        }
        off += line.len();
    }
    None
}

/// 通用長期指標表更新：把一組新列（已含日期+root）寫進文件的某個 section
/// - 同一天 + root 精確相等 的舊列 -> 以新列取代
/// - 其餘歷史列保留；新 root 追加
/// - section 範圍 = 標題起至下一個 `#` 標題（或 EOF）；**範圍外的內容原樣保留**
///   （曾掃到 EOF 重寫，把表格後的使用者筆記/其他 section 靜默刪除）
///
/// 原子寫入；回傳 (changed, file_path)。
/// `header` 必須是完整區塊（含 section 標題 + 表頭 + 分隔列）。
pub fn track_table(
    path: &Path,
    section: &str,
    header: &str,
    new_rows: &[String],
    host_col: Option<usize>,
) -> std::io::Result<(bool, String)> {
    let existing = if path.is_file() {
        fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    };

    // 定位 section：從標題到下一個 `#` 標題（或 EOF）為止
    let zone: Option<(usize, usize)> = existing.find(section).map(|start| {
        let after_title = start + section.len();
        let end = after_title
            + next_heading_offset(&existing[after_title..]).unwrap_or(existing.len() - after_title);
        (start, end)
    });

    // 解析既有表格資料列（`|` 開頭）——只在 section 範圍內找
    let mut old_rows: Vec<String> = Vec::new();
    if let Some((b, e)) = zone {
        for line in existing[b..e].lines() {
            let t = line.trim();
            if t.starts_with('|') && !t.starts_with("|---") && !t.starts_with("| 日期") {
                old_rows.push(t.to_string());
            }
        }
    }

    fn row_root(row: &str) -> Option<String> {
        let cols: Vec<&str> = row.split('|').collect();
        cols.get(2).map(|c| c.trim().trim_matches('`').to_string())
    }

    // 欄數正規化：舊格式（例如多一個裝飾欄）對齊新 header
    fn normalize_row(row: &str, ncols: usize) -> String {
        let cols: Vec<&str> = row.split('|').collect();
        if cols.len() <= ncols + 2 {
            return row.to_string();
        }
        let kept: Vec<&str> = cols[1..=ncols].iter().map(|c| c.trim()).collect();
        format!("| {} |", kept.join(" | "))
    }
    let ncols = header.matches('|').count() - 1; // 欄數 = 管道數 - 1（前後各一）
                                                 // host 欄位（多機情境）：usage.db 是每台機器各自的，同日 + root 但 host
                                                 // 不同的列必須並存。舊格式列（沒有 host 欄）視為 host 相符 → 同日可被替換。
    let host_of = |row: &str| -> Option<String> {
        host_col
            .and_then(|i| row.split('|').nth(i))
            .map(|c| c.trim().to_string())
    };
    let keep: Vec<String> = old_rows
        .into_iter()
        .filter(|row| {
            // 只刪「同一天 + root 精確相等」（+ host 相符或舊列無 host 欄）的舊列
            // （避免 foo 誤刪 foo-bar）
            let r = row_root(row);
            for nr in new_rows {
                let cols: Vec<&str> = nr.split('|').collect();
                let date = cols.get(1).map(|c| c.trim()).unwrap_or("");
                let root = cols
                    .get(2)
                    .map(|c| c.trim().trim_matches('`'))
                    .unwrap_or("");
                if row.contains(date) && r.as_deref() == Some(root) {
                    match (host_of(row), host_of(nr)) {
                        (Some(oh), Some(nh)) if oh != nh => continue, // 他機的列，保留
                        _ => return false,
                    }
                }
            }
            true
        })
        .map(|row| normalize_row(&row, ncols))
        .collect();

    let (prefix, suffix) = match zone {
        Some((b, e)) => (existing[..b].to_string(), existing[e..].to_string()),
        None => {
            let mut p = existing.clone();
            if !p.is_empty() && !p.ends_with('\n') {
                p.push('\n');
            }
            if !p.is_empty() {
                p.push('\n');
            }
            (p, String::new())
        }
    };

    let mut table = String::new();
    // header 帶尾端換行（慣例），這裡不再多 push，避免資料列前多一個空行
    table.push_str(header);
    for r in &keep {
        table.push_str(r);
        table.push('\n');
    }
    for nr in new_rows {
        table.push_str(nr);
        table.push('\n');
    }
    let new_content = format!("{prefix}{table}{suffix}");
    let changed = new_content != existing;
    if changed {
        atomic_write(path, &new_content)?;
    }
    Ok((changed, path.to_string_lossy().into_owned()))
}

/// 原子寫入（同目錄 temp + fsync + rename）。長期指標表與發現區塊共用，
/// 兩份拷貝就會有一份忘了 fsync。
pub(crate) fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("track.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(content.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

/// 把多個 explore 指標更新進文件的「長期指標」表格（走通用 track_table）
pub fn track_update(path: &Path, metrics: &[&ExploreMetrics]) -> std::io::Result<(bool, String)> {
    let header = format!(
        "{}\n\n| 日期 | 專案 | 檔案 | LOC | map tokens | 全讀 tokens | 節省%(map-vs-read) | top-10 覆蓋% |\n|---|---|---:|---:|---:|---:|---:|---:|\n",
        TRACK_SECTION
    );
    let rows: Vec<String> = metrics.iter().map(|m| row_md(m)).collect();
    track_table(path, TRACK_SECTION, &header, &rows, None)
}

/// 相容舊 API（單 repo）
pub fn track_append(path: &Path, m: &ExploreMetrics) -> std::io::Result<(bool, String)> {
    track_update(path, &[m])
}
