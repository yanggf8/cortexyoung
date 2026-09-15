//! cortexyoung/cort 索引整合：唯讀存取 cort 的 SQLite（~/.cache/cortex-ng/<sha256>.db）
//! 相容 cort schema v5–v7（chunks / projects / relationships / _cortex_meta / file_state；
//! v6 的 `file_state.indexed_uncommitted`、v7 的 `file_state.chunk_count`）。
//! 新欄位在舊 DB 上整句查詢會失敗 → 對應欄位回 `None`＝「無法判讀」，
//! 而不是拿 0 假裝健康；claudecat 是消費者，靠欄位查不查得到判斷，不靠版本號。
//! 不做任何寫入；DB 不存在或 schema 不符時回退到 None。
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 與 cort 完全一致：project_id = sha256(real_path) hex
pub fn project_id(real_path: &str) -> String {
    let mut h = Sha256::new();
    h.update(real_path.as_bytes());
    let out = h.finalize();
    let mut hex = String::with_capacity(64);
    for b in out.iter() {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

pub fn cache_dir() -> PathBuf {
    match std::env::var("CORT_CACHE_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".cache").join("cortex-ng"))
            .unwrap_or_else(|| PathBuf::from(".cortex-ng")),
    }
}

pub fn db_path_for(real_path: &str) -> PathBuf {
    cache_dir().join(format!("{}.db", project_id(real_path)))
}

#[derive(Debug, Clone, Serialize)]
pub struct CortIndexInfo {
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub git_head: Option<String>,
    pub last_indexed_at: Option<i64>,
    pub extractor_version: String,
    pub chunk_count: i64,
    pub relationships_count: i64,
    /// cort 的 `_cortex_meta.SCHEMA_VERSION`（純呈現；claudecat 是消費者，
    /// 不硬編碼「期望版本」——那個常數只有 cort 自己知道，寫死必然像文件一樣爛掉）
    pub schema_version: Option<String>,
    /// derived graph 是否落後 chunks（None = 無法判讀，見 [`read_meta`]）
    pub graph_pending: Option<bool>,
    /// HEAD 相符 + 索引 ≤7 天 **且** 圖不是已知落後（`graph_pending != Some(true)`）。
    /// `None`（讀不到 `_cortex_meta`）不翻布林——沒讀到不等於壞掉。
    pub fresh: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CortHit {
    pub symbol: Option<String>,
    pub chunk_type: String,
    pub file: String,
    pub start_line: i64,
    pub end_line: i64,
    pub language: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CortDependent {
    pub source_file: String,
    pub source_symbol: Option<String>,
    pub source_start_line: i64,
    pub rel_type: String,
    pub call_site_line: Option<i64>,
    pub confidence_score: f64,
}

/// SQLite URI：路徑裡的 `%` `?` `#` 空白 會被 URI 語法吃掉（query/fragment 邊界、
/// percent-decode），必須 percent-encode；其餘位元組（含 UTF-8）原樣。
pub fn sqlite_uri(path: &Path) -> String {
    let mut out = String::from("file:");
    for c in path.to_string_lossy().chars() {
        match c {
            '%' | '?' | '#' | ' ' => out.push_str(&format!("%{:02X}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push_str("?immutable=1");
    out
}

/// cort 的 `_cortex_meta`（key/value）：升級與圖重建狀態的唯一權威。
/// 回傳 `(schema_version, graph_pending)`，兩者的 `None` 一律是「無法判讀」。
///
/// `graph_pending` 是 cort 的 derived-graph 旗標（cort `db.rs` 判準為 `== Some("1")`，
/// 在 `rebuild_reasons_of` 轉成 `graph_incomplete`）：
/// - 每次 per-file 增量都會先設 1，但**與 `git_head`/`last_indexed_at` 在同一個
///   transaction 裡清 0**（cort `incremental.rs`），所以外部讀到持續為 1 只有兩種情況：
///   ①schema 遷移後還沒跑過任何索引（此時時戳與 HEAD 仍是「新的」——這正是會被誤報
///   fresh 的洞）②增量跑到一半死掉（此時時戳沒前進，本來就會 STALE）。
/// - 表讀得到但沒這個鍵 → `Some(false)`（對齊 cort 的判準；「沒這個鍵」是讀到的事實）
/// - 表不存在／查詢失敗 → `None`；**絕不寫成 `Some(false)` 假裝圖是新的**
fn read_meta(conn: &Connection) -> (Option<String>, Option<bool>) {
    // 外層 Option：Err（表不存在／查詢失敗）→ None＝無法判讀
    // 內層 Option：Ok(None)＝表在、鍵不存在
    let get = |key: &str| -> Option<Option<String>> {
        conn.query_row(
            "SELECT value FROM _cortex_meta WHERE key = ?1",
            [key],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
    };
    let schema_version = get("SCHEMA_VERSION").flatten();
    let graph_pending = get("graph_pending").map(|v| v.as_deref() == Some("1"));
    (schema_version, graph_pending)
}

/// cort 的 DB 檔案是否存在（區分「尚未索引」與「存在但讀取失敗」）
pub fn db_exists(root: &Path) -> bool {
    std::fs::canonicalize(root)
        .ok()
        .and_then(|r| r.to_str().map(db_path_for))
        .is_some_and(|p| p.is_file())
}

/// 對 cort DB 跑唯讀查詢：先試一般唯讀（sidecar 齊全時最準）；若開檔失敗
/// （唯讀檔案系統缺 -shm/-wal、sandbox 擋 lock 等）或查詢時 BUSY（cort 正持有寫鎖），
/// 自動退回 `immutable=1`（SQLite 完全不碰 sidecar/lock，直接讀主檔；代價是 cort 若有
/// 未 checkpoint 的 WAL 內容會讀不到——這是可接受的誠實取捨）。claudecat 全程不寫入。
fn with_readonly<T>(real_path: &str, f: impl Fn(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let db = db_path_for(real_path);
    if !db.is_file() {
        return None;
    }
    let uri = sqlite_uri(&db);
    let candidates = [
        Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY),
        Connection::open_with_flags(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        ),
    ];
    for conn in candidates.into_iter().flatten() {
        if let Ok(v) = f(&conn) {
            return Some(v);
        }
    }
    None
}

/// 取得 cort 索引狀態（DB 不存在或 projects 表無此專案 → None）
pub fn index_info(root: &Path) -> Option<CortIndexInfo> {
    let real = std::fs::canonicalize(root).ok()?;
    let real_str = real.to_str()?;
    let pid = project_id(real_str);

    let row = with_readonly(real_str, |conn| {
        conn.query_row(
            "SELECT name, path, git_head, last_indexed_at, extractor_version \
             FROM projects WHERE project_id = ?1",
            [&pid],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
    })??;

    let (schema_version, graph_pending) =
        with_readonly(real_str, |conn| Ok(read_meta(conn))).unwrap_or((None, None));

    let (chunk_count, relationships_count) = with_readonly(real_str, |conn| {
        let chunks: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE project_id = ?1",
            [&pid],
            |r| r.get(0),
        )?;
        let rels: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks c JOIN relationships r \
             ON r.source_chunk_id = c.chunk_id WHERE c.project_id = ?1",
            [&pid],
            |r| r.get(0),
        )?;
        Ok((chunks, rels))
    })
    .unwrap_or((0, 0));

    // 新鮮度：git head 相符 + 索引在 7 天內 + 圖不是已知落後
    let fresh = freshness(real_str, row.2.as_deref(), row.3) && graph_pending != Some(true);
    Some(CortIndexInfo {
        project_id: pid,
        name: row.0,
        path: row.1,
        git_head: row.2,
        last_indexed_at: row.3,
        extractor_version: row.4,
        chunk_count,
        relationships_count,
        schema_version,
        graph_pending,
        fresh,
    })
}

/// 索引新鮮度：git head 相符 + 索引時間在 7 天內。
/// `last_indexed_at` 與 cort 的寫入一致，是**毫秒**（epoch ms）——
/// 曾因誤當秒數比較，7 天關卡永不觸發（cort-status 對 40 天前的索引仍報 fresh）。
const FRESH_WINDOW_MS: i64 = 7 * 24 * 3600 * 1000;

fn is_fresh(head_matches: bool, last_indexed_at: Option<i64>) -> bool {
    head_matches && last_indexed_at.is_some_and(|t| now_ms() - t <= FRESH_WINDOW_MS)
}

fn freshness(real_str: &str, indexed_head: Option<&str>, last_indexed_at: Option<i64>) -> bool {
    // git head 相符（非 git repo 或索引未記錄 head 時不追究）
    let head_matches = match (&git_head(real_str), indexed_head) {
        (Some(now), Some(ih)) => now == ih,
        _ => true,
    };
    is_fresh(head_matches, last_indexed_at)
}

fn git_head(real_str: &str) -> Option<String> {
    let head = std::path::Path::new(real_str).join(".git").join("HEAD");
    let content = std::fs::read_to_string(head).ok()?;
    let content = content.trim();
    if let Some(ref_path) = content.strip_prefix("ref: ") {
        let full = std::path::Path::new(real_str)
            .join(".git")
            .join(ref_path.trim());
        std::fs::read_to_string(full)
            .ok()
            .map(|s| s.trim().to_string())
    } else {
        Some(content.to_string())
    }
}

/// 搜尋 cort 索引的符號（比 tree-sitter 的 top-N 更完整：全 project）
pub fn search_symbols(root: &Path, query: &str) -> Option<Vec<CortHit>> {
    let real = std::fs::canonicalize(root).ok()?;
    let real_str = real.to_str()?;
    let pid = project_id(real_str);
    let pat = format!("%{}%", query.to_lowercase());

    let hits = with_readonly(real_str, |conn| {
        let mut stmt = conn.prepare(
            "SELECT symbol_name, chunk_type, file_path, start_line, end_line, language, content \
             FROM chunks WHERE project_id = ?1 AND lower(symbol_name) LIKE ?2 \
             ORDER BY start_line LIMIT 50",
        )?;
        let rows = stmt.query_map([&pid, &pat], |r| {
            Ok(CortHit {
                symbol: r.get(0)?,
                chunk_type: r.get(1)?,
                file: r.get(2)?,
                start_line: r.get(3)?,
                end_line: r.get(4)?,
                language: r.get(5)?,
                content: r.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<CortHit>>>()
    })?;
    if hits.is_empty() {
        None
    } else {
        Some(hits)
    }
}

/// 把使用者查詢轉成 FTS5 MATCH 字串：拆成 alnum token、逐個加雙引號（字面 token，
/// 不讓 FTS 運算子影響）、以 AND 連接。例：`user creation` → `"user" AND "creation"`。
fn fts_match_query(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" AND "))
    }
}

/// FTS 全文 fallback（`cort recall` 的資料源 `chunks_fts`）：symbol_name 未命中時，
/// 搜 content / symbol / file 全文（external-content FTS5，唯讀 join 回 chunks）。
pub fn search_fts(root: &Path, query: &str) -> Option<Vec<CortHit>> {
    let real = std::fs::canonicalize(root).ok()?;
    let real_str = real.to_str()?;
    let pid = project_id(real_str);
    let match_q = fts_match_query(query)?;

    let hits = with_readonly(real_str, |conn| {
        let mut stmt = conn.prepare(
            "SELECT c.symbol_name, c.chunk_type, c.file_path, c.start_line, c.end_line, c.language, c.content \
             FROM chunks_fts f JOIN chunks c ON c.rowid = f.rowid \
             WHERE chunks_fts MATCH ?1 AND c.project_id = ?2 \
             ORDER BY f.rank LIMIT 20",
        )?;
        let rows = stmt.query_map([&match_q, &pid], |r| {
            Ok(CortHit {
                symbol: r.get(0)?,
                chunk_type: r.get(1)?,
                file: r.get(2)?,
                start_line: r.get(3)?,
                end_line: r.get(4)?,
                language: r.get(5)?,
                content: r.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<CortHit>>>()
    })?;
    if hits.is_empty() {
        None
    } else {
        Some(hits)
    }
}

/// 摘要 cort content：壓縮空白、取前 max_chars 字元（省 read 用，單行可讀）。
pub fn content_summary(content: &str, max_chars: usize) -> String {
    let collapsed: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = collapsed.trim();
    let mut out: String = collapsed.chars().take(max_chars).collect();
    if collapsed.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// 「誰呼叫/import 這個符號」——反向依賴（cort impact 的資料來源）
pub fn dependents(root: &Path, symbol: &str) -> Option<Vec<CortDependent>> {
    let real = std::fs::canonicalize(root).ok()?;
    let real_str = real.to_str()?;
    let pid = project_id(real_str);

    let deps = with_readonly(real_str, |conn| {
        let mut stmt = conn.prepare(
            "SELECT sc.file_path, sc.symbol_name, sc.start_line, r.rel_type, r.call_site_line, r.confidence_score \
             FROM relationships r \
             JOIN chunks tc ON r.target_chunk_id = tc.chunk_id AND tc.project_id = ?1 \
             JOIN chunks sc ON r.source_chunk_id = sc.chunk_id AND sc.project_id = ?1 \
             WHERE tc.symbol_name = ?2 \
             ORDER BY sc.file_path LIMIT 50",
        )?;
        let rows = stmt.query_map([&pid, symbol], |r| {
            Ok(CortDependent {
                source_file: r.get(0)?,
                source_symbol: r.get(1)?,
                source_start_line: r.get(2)?,
                rel_type: r.get(3)?,
                call_site_line: r.get(4)?,
                confidence_score: r.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<CortDependent>>>()
    })?;
    if deps.is_empty() {
        None
    } else {
        Some(deps)
    }
}

// ---------------------------------------------------------------------------
// cort-audit：收集「整合是否達成」的驗證數據（唯讀）
// 索引健康 + 覆蓋缺口 + FTS 同步 + 用量（usage.db），全部不寫入
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CortAuditIndex {
    /// 索引健康（index_info 已有欄位）
    pub name: String,
    pub path: String,
    pub fresh: bool,
    pub git_head_matches: bool,
    /// 索引距今幾天；無 last_indexed_at 時為 None
    pub index_age_days: Option<i64>,
    pub chunk_count: i64,
    pub relationships_count: i64,
    /// 覆蓋：file_state 有、chunks 沒有的檔案（completeness 缺口）。
    /// 查詢失敗（schema 差異等）一律 None＝「無法判讀」，絕不用 0 假裝「無缺口」
    /// ——當初 ?1 參數 bug 就是被 unwrap_or(0) 吞成「無缺口」。
    pub file_state_files: Option<i64>,
    pub chunked_files: Option<i64>,
    /// 未 chunk 檔案的「總數」（清單只保留前 20 筆，避免輸出過長）
    pub not_chunked_total: Option<i64>,
    pub not_chunked_files: Vec<String>,
    /// 未 chunk 檔裡 `chunk_count = 0` 的數量：extractor 掃過、檔內沒有可 chunk 的宣告
    /// ——cortexyoung#2 定義的**正確沉默**，不是缺口。
    /// `None` = 這個 DB 沒有 v7 的 `chunk_count` 欄（或查詢失敗）＝無法判讀。
    pub not_chunked_scanned_empty: Option<i64>,
    /// 未 chunk 檔裡 `chunk_count = -1` 的數量：v7 之前寫入、之後從未重寫的列。
    /// **不是掃描結果**——把它算進正確沉默等於拿舊資料當證據。
    /// `None` 同上，意思是「這個 DB 根本沒有這個欄位可讀」。
    pub not_chunked_unknown: Option<i64>,
    /// 整個 `file_state` 裡 `indexed_uncommitted <> 0` 的列數（v6）：索引建立在未提交
    /// 內容上，git 還原後增量因 diff 為空而永不重看（cortexyoung#5 的成因本身）。
    /// `None` = 沒有 v6 的這個欄位（或查詢失敗）＝無法判讀，不等於 0。
    pub indexed_uncommitted_files: Option<i64>,
    /// 含至少一個 unparsed chunk 的檔案數（純資訊欄）
    pub files_with_unparsed_chunks: i64,
    /// chunks_fts 列數（表不存在 → None）
    pub fts_docs: Option<i64>,
    /// FTS 與 chunks 的 rowid 雙向差異數（None = 無法判讀）。
    /// 0 才是同步——「數量相等」會在一多一少時偽稱 synced。
    pub fts_drift: Option<i64>,
    /// cort 的 `_cortex_meta.SCHEMA_VERSION`（純呈現，見 [`CortIndexInfo::schema_version`]）
    pub schema_version: Option<String>,
    /// derived graph 是否落後 chunks（None = 無法判讀，見 [`read_meta`]）——
    /// 與 `index_info` 同一口徑，兩邊的 `fresh` 不得分岔
    pub graph_pending: Option<bool>,
    /// cortexyoung f4ad4c7d 的 repair 三態（`none`/`refreshable`/`rebuild_required`），
    /// 問 PATH 上的 `cort status`（見 [`query_repair`]）。本地 `fresh` 看不見
    /// extractor/schema 變更——34e33a1d 換身分識別後所有舊索引都會讀成
    /// `extractor_changed`，本地仍說 fresh；只有 binary 看得見 rebuild_required。
    /// `None` = binary 不在 PATH 或判讀失敗，報 `?`。
    pub repair: Option<String>,
}

impl CortAuditIndex {
    /// 扣掉「正確沉默」與「未知」之後剩下的缺口：`file_state` 說檔裡有宣告
    /// （`chunk_count > 0`）、`chunks` 卻一列都沒有——這正是 cortexyoung#5 的形狀
    /// （索引停在一個從未提交、後來被 git 還原的版本，增量因 diff 為空永不重看）。
    ///
    /// 三個數字任一 `None` 就回 `None`：少一個事實就不准下結論。負數夾回 0——
    /// 兩邊 SQL 的 `NOT IN` 子查詢之間若有寫入進來，算術可能短暫倒掛，
    /// 但「負的缺口」是沒有意義的斷言。
    pub fn real_gap(&self) -> Option<i64> {
        let total = self.not_chunked_total?;
        let scanned_empty = self.not_chunked_scanned_empty?;
        let unknown = self.not_chunked_unknown?;
        Some((total - scanned_empty - unknown).max(0))
    }
}

/// 單一 harness 的 router 切面（`harness` 只出現在 hook payload：
/// hook-suggest / hook-refresh；`impact`/`context` 等動詞命令**沒有**這個欄位，
/// 所以這個切面只能講「router 對誰開了口」，不能講「誰真的用了 cort」）
#[derive(Debug, Clone, Serialize, Default)]
pub struct HarnessStat {
    pub suggests: i64,
    /// hit + hit_yielded + hit_stale（與命中率同一組口徑）
    pub hits: i64,
    pub no_shape: i64,
    pub refreshes: i64,
    /// 可行動的 decline 之最（排除 `not_a_search_tool` baseline，語意同 decline-top）
    pub top_decline: Option<(String, i64)>,
    /// `harness` 與 `harness_declared` 不符的列數——實測 grok 會宣告成 claude-code，
    /// 任何信任 declared 值的歸因都會被它汙染，所以要看得見
    pub declared_mismatch: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UsageWindow {
    pub window_days: u32,
    pub total_commands: i64,
    pub by_command: BTreeMap<String, i64>,
    pub suggest_outcomes: BTreeMap<String, i64>,
    /// hook-suggest 的 decline 歸因（鍵如 "no_shape/context_flag"）——
    /// cortexyoung c290c383（2026-09-06）起的新列才帶 decline，舊列自然缺席
    pub declines: BTreeMap<String, i64>,
    /// 可行動 no_shape 的 `shape` 分佈（cortexyoung 09f55136 起）：`工具名|top-level key`，
    /// 永不含 payload 內容。口徑與 `declines` 排序一致——`not_a_search_tool` baseline
    /// 不混進來，否則本機兩千多筆「本來就不是搜尋」會把唯一的行動靶心擠掉。
    /// 沒有 `shape` 欄的舊列直接跳過：記成 "unparsed" 只會汙染排行。
    pub no_shape_shapes: BTreeMap<String, i64>,
    pub refresh_outcomes: BTreeMap<String, i64>,
    /// hook fire 的窮盡分割（cortexyoung 6623113d 口徑，見 [`census_bucket`]）：
    /// command → bucket → count，`_total` 桶記 fire 總數、其餘每列恰落一桶，
    /// 加總恆等 `_total`——「分割閉合」一眼可查，不用再做對帳 session。
    /// claudecat 自算（不呼叫 binary）；詞彙靠上面的複製品常數同步。
    pub census: BTreeMap<String, BTreeMap<String, i64>>,
    /// router 的 harness 切面（cortexyoung v3 payload 起才有 `harness`）
    pub by_harness: BTreeMap<String, HarnessStat>,
    /// 沒有 `harness` 欄的 hook 列（v3 之前的歷史列）——不計進任何 harness，
    /// 否則各 harness 加總會悄悄對不上 hook 總數
    pub harness_unknown: i64,
    pub errors: i64,
    pub index_stale_queries: i64,
    /// 省下的位元組。自 cortexyoung 3f1d3d96 起來源變寬：除了 receipt cache 命中，
    /// 還含 ranged `read` 少傳的檔案位元組——所以它不再等於「快取命中省下的量」。
    pub saved_bytes: i64,
    /// query-time self-heal 採樣（cortexyoung 5f6d5267 起，impact/context 回答前自癒
    /// index）：`heal_scanned` 是分母（impact+context 列數）。heal 欄位只在「有話要說」
    /// 的列上（綠路 payload 完全不加 key，heal.rs `attach_to`），沒有任何 heal key 的
    /// 歷史列計 `heal_legacy`——「0 次自癒」與「還沒資料」必須分得開，不得互相冒充。
    pub heal_scanned: i64,
    pub heal_self_healed: i64,
    /// self_healed=true 的列按 heal_mode 分桶（詞彙見 [`HEAL_MODES`]；輕量 breakdown——
    /// 樣體預期極小，不做窮盡分割，詞彙外的值照樣顯示自身字串）
    pub heal_modes: BTreeMap<String, i64>,
    /// heal_deferred 列按理由字串分桶（詞彙見 [`HEAL_DEFERRED_REASONS`]；同上不窮盡）
    pub heal_deferred: BTreeMap<String, i64>,
    pub heal_ms_total: i64,
    pub heal_ms_max: i64,
    /// 背景重建事件數：`cort index --heal-background` 的 usage 列（args_summary 帶
    /// `"heal":"background"`，heal.rs `spawn_background_healer` 記自己的列）；
    /// 一般 index 列（無 heal key）不計
    pub heal_background: i64,
    pub heal_legacy: i64,
    /// args_summary 為 NULL 或非法 JSON 的 impact/context 列——照 hook census 對
    /// unparseable 的既有態度：可見、入自己的桶，不 panic、不中斷掃描
    pub heal_unparseable: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CortAudit {
    pub root: String,
    /// 主機名（/etc/hostname）——usage.db 是每台機器各自的，多機的列靠 host 区分
    pub host: String,
    pub window_days: u32,
    pub index: Option<CortAuditIndex>,
    /// DB 檔案存在但 index=None → 「讀取失敗」，不是「尚未索引」
    pub db_exists: bool,
    pub usage: Option<UsageWindow>,
    /// 固定 7 天窗口（早期訊號；與 `--window` 的長期趨勢互補）
    pub usage_7d: Option<UsageWindow>,
}

/// 主機名：讀 /etc/hostname（WSL/Linux），讀不到則 "unknown"
fn host_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 索引健康 + 覆蓋缺口 + FTS 同步（單一唯讀連線內完成）
pub fn audit_index(root: &Path) -> Option<CortAuditIndex> {
    let real = std::fs::canonicalize(root).ok()?;
    let real_str = real.to_str()?;
    let pid = project_id(real_str);
    let head_now = git_head(real_str);

    with_readonly(real_str, |conn| {
        let row = conn
            .query_row(
                "SELECT name, path, git_head, last_indexed_at FROM projects WHERE project_id = ?1",
                [&pid],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;

        let (name, path, indexed_head, last_indexed_at) = match row {
            Some(r) => r,
            None => return Ok(None), // 專案尚未被 cort 索引
        };

        let chunk_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE project_id = ?1",
            [&pid],
            |r| r.get(0),
        )?;
        let relationships_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks c JOIN relationships r \
                 ON r.source_chunk_id = c.chunk_id WHERE c.project_id = ?1",
                [&pid],
                |r| r.get(0),
            )
            .unwrap_or(0);
        // 以下健康聲明欄位：查詢失敗 → None（無法判讀），絕不 unwrap_or(0) 假裝健康
        let file_state_files: Option<i64> = conn
            .query_row(
                "SELECT COUNT(*) FROM file_state WHERE project_id = ?1",
                [&pid],
                |r| r.get(0),
            )
            .ok();
        let chunked_files: Option<i64> = conn
            .query_row(
                "SELECT COUNT(DISTINCT file_path) FROM chunks WHERE project_id = ?1",
                [&pid],
                |r| r.get(0),
            )
            .ok();
        let not_chunked_sql = "SELECT file_path FROM file_state WHERE project_id = ? \
             AND file_path NOT IN (SELECT DISTINCT file_path FROM chunks WHERE project_id = ?) \
             ORDER BY file_path";
        let not_chunked_total: Option<i64> = conn
            .query_row(
                "SELECT COUNT(*) FROM file_state WHERE project_id = ? \
                 AND file_path NOT IN (SELECT DISTINCT file_path FROM chunks WHERE project_id = ?)",
                [&pid, &pid],
                |r| r.get(0),
            )
            .ok();
        let mut not_chunked: Vec<String> = Vec::new();
        if not_chunked_total.is_some() {
            if let Ok(mut stmt) = conn.prepare(not_chunked_sql) {
                let rows = stmt.query_map([&pid, &pid], |r| r.get::<_, String>(0));
                if let Ok(rows) = rows {
                    for r in rows.flatten() {
                        if not_chunked.len() < 20 {
                            not_chunked.push(r);
                        }
                    }
                }
            }
        }
        // v7 的 `chunk_count` 把「正確沉默」與「真缺口」分開（在此之前 claudecat 只能
        // 印「兩種成因都要查」的猜測）。一次查兩個 split：舊 DB 沒這欄時整句失敗 →
        // 兩者皆 None＝無法判讀，這正是要的行為，絕不 unwrap_or(0) 假裝掃過了。
        // SUM 在 0 列時回 NULL，但查詢本身成功＝「讀到 0 筆」是事實，所以攤成 Some(0)。
        let (not_chunked_scanned_empty, not_chunked_unknown) = conn
            .query_row(
                "SELECT SUM(CASE WHEN chunk_count = 0 THEN 1 ELSE 0 END), \
                        SUM(CASE WHEN chunk_count = -1 THEN 1 ELSE 0 END) \
                 FROM file_state WHERE project_id = ? AND file_path NOT IN \
                      (SELECT DISTINCT file_path FROM chunks WHERE project_id = ?)",
                [&pid, &pid],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                        r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    ))
                },
            )
            .map(|(empty, unknown)| (Some(empty), Some(unknown)))
            .unwrap_or((None, None));
        // v6：索引自未提交內容的檔（git 還原後增量永不重看——cortexyoung#5 的成因）
        let indexed_uncommitted_files: Option<i64> = conn
            .query_row(
                "SELECT COUNT(*) FROM file_state \
                 WHERE project_id = ?1 AND indexed_uncommitted <> 0",
                [&pid],
                |r| r.get(0),
            )
            .ok();
        let files_with_unparsed_chunks: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT file_path) FROM chunks \
                 WHERE project_id = ?1 AND chunk_source = 'unparsed'",
                [&pid],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let fts_docs: Option<i64> = conn
            .query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0))
            .ok();
        // drift 用 rowid 雙向差異，不用數量相等（一多一少會偽稱 synced）
        let fts_drift: Option<i64> = conn
            .query_row(
                "SELECT \
                   (SELECT COUNT(*) FROM chunks c LEFT JOIN chunks_fts f ON f.rowid = c.rowid \
                     WHERE f.rowid IS NULL AND c.project_id = ?1) + \
                   (SELECT COUNT(*) FROM chunks_fts f LEFT JOIN chunks c ON c.rowid = f.rowid \
                     WHERE c.rowid IS NULL)",
                [&pid],
                |r| r.get(0),
            )
            .ok();

        // 新鮮度：git head 相符 + ≤7 天
        let git_head_matches = match (&head_now, &indexed_head) {
            (Some(now), Some(idx)) => now == idx,
            _ => true, // 非 git repo 時不追究
        };
        let age_days = last_indexed_at.map(|t| (now_ms() - t).max(0) / (24 * 3600 * 1000));
        let (schema_version, graph_pending) = read_meta(conn);
        let fresh = is_fresh(git_head_matches, last_indexed_at) && graph_pending != Some(true);

        Ok(Some(CortAuditIndex {
            name,
            path,
            fresh,
            git_head_matches,
            index_age_days: age_days,
            chunk_count,
            relationships_count,
            file_state_files,
            chunked_files,
            not_chunked_total,
            not_chunked_files: not_chunked,
            not_chunked_scanned_empty,
            not_chunked_unknown,
            indexed_uncommitted_files,
            files_with_unparsed_chunks,
            fts_docs,
            fts_drift,
            schema_version,
            graph_pending,
            repair: None,
        }))
    })?
}

fn open_usage_readonly() -> Option<Connection> {
    let db = cache_dir().join("usage.db");
    if !db.is_file() {
        return None;
    }
    let uri = sqlite_uri(&db);
    Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .or_else(|_| {
            Connection::open_with_flags(
                &uri,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )
        })
        .ok()
}

/// cortexyoung hook.rs 的 SUGGEST_OUTCOMES 複製品（6623113d 引入）。
/// **會漂**：上游動了詞彙，這裡要跟——census 的 `unknown/<hook>` 桶就是為了
/// 在跟丟時發出訊號，而不是把新值靜默混進既有桶。
/// 078a0c5c（2026-09-15）加了第十個 `no_evidence_hinted`：`no_evidence` 的
/// 邊界 5 指路（chunks 內容 LIKE 出現處，per session 每符號一次、上限三）。
pub const SUGGEST_OUTCOMES: [&str; 10] = [
    "no_payload",
    "no_shape",
    "upgrade_stood_down",
    "no_index",
    "no_index_hinted",
    "no_evidence",
    "no_evidence_hinted",
    "hit",
    "hit_stale",
    "hit_yielded",
];

/// cortexyoung hook.rs 的 REFRESH_OUTCOMES 複製品（4b895589 補齊）。
pub const REFRESH_OUTCOMES: [&str; 8] = [
    "upgrade_stood_down",
    "no_index",
    "db_unavailable",
    "no_ast_grep",
    "refreshed",
    "already_current",
    "rebuild_required",
    "busy_or_failed",
];

/// cortexyoung heal.rs 的 heal_mode 詞彙（5f6d5267）：`incremental_index` 的
/// `stats.mode` 只有這兩個值（上游 incremental.rs:313 `"full"`、:498 `"incremental"`；
/// heal.rs:138 以 `stats.mode == "full"` 判別）。
/// **會漂**：上游動了詞彙，這裡要跟——採樣桶按實際字串落鍵，詞彙外的值
/// 照樣顯示自身字串（輕量 breakdown，不做窮盡分割），不會靜默混進既有桶。
pub const HEAL_MODES: [&str; 2] = ["incremental", "full"];

/// cortexyoung heal.rs 的 heal_deferred 理由詞彙（5f6d5267）：`deferred()` 的全部
/// `&'static str` 呼叫點——`upgrade_in_flight`(heal.rs:131)、`heal_failed`(:148)、
/// `no_cache_dir`(:159)、`background_already_running`(:163)、`spawn_failed`(:166/:171)、
/// `background_spawned`(:169)。
/// **會漂**：同上——理由桶按實際字串落鍵，詞彙外照樣顯示自身字串。
pub const HEAL_DEFERRED_REASONS: [&str; 6] = [
    "upgrade_in_flight",
    "heal_failed",
    "no_cache_dir",
    "background_already_running",
    "spawn_failed",
    "background_spawned",
];

/// 一列 hook command_log 落進哪個 census 桶。口徑照 cortexyoung usage.rs 的
/// `hook_census_at`（6623113d）：status 非 ok → `status_error`；args_summary 不是
/// 合法 JSON → `unparseable_summary`；JSON 沒有 hook 欄 → `legacy_unsplit`；
/// hook-suggest 的 `no_shape` 展開 decline（欄位缺席 = `decline_absent`）；
/// 其餘 hook 在該命令的詞彙內照名落桶，詞彙外 → `unknown/<hook>`。
/// 分割互斥且窮盡：每列恰落一桶，buckets 加總 == fires。
fn census_bucket(command: &str, status: &str, raw: &str) -> String {
    if status != "ok" {
        return "status_error".to_string();
    }
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) else {
        return "unparseable_summary".to_string();
    };
    let (known, splits_decline): (&[&str], bool) = if command == "hook-refresh" {
        (&REFRESH_OUTCOMES, false)
    } else {
        (&SUGGEST_OUTCOMES, true)
    };
    match parsed.get("hook").and_then(|h| h.as_str()) {
        None => "legacy_unsplit".to_string(),
        Some("no_shape") if splits_decline => format!(
            "no_shape/{}",
            parsed
                .get("decline")
                .and_then(|d| d.as_str())
                .unwrap_or("decline_absent")
        ),
        Some(h) if known.contains(&h) => h.to_string(),
        Some(h) => format!("unknown/{h}"),
    }
}

/// cortexyoung f4ad4c7d 的 repair 判定，從 `cort status` 的 JSON 鏡射 impact.rs
/// 的推導（`incremental::forbid_refuses`）：`!index_is_stale` → `none`；
/// hook 會拒絕增量（`rebuild_required` 理由非空，或 `candidates_narrowed` 失守）
/// → `rebuild_required`（只有前景全量 `cort index` 會修）；其餘 → `refreshable`。
/// 關鍵欄位缺席或未索引 → `None`＝無法判讀——這個判定的輸入（pack hash 比對）
/// 不在 claudecat 讀得到的 DB 裡，自算必然是假的。
pub fn repair_from_status_json(v: &serde_json::Value) -> Option<String> {
    let indexed = v.get("indexed").and_then(|b| b.as_bool())?;
    let stale = v.get("index_is_stale").and_then(|b| b.as_bool())?;
    if !indexed {
        return None;
    }
    let repair = if !stale {
        "none"
    } else {
        let rebuild_required = v.get("rebuild_required").and_then(|r| r.as_array());
        let narrowed = v.get("candidates_narrowed").and_then(|b| b.as_bool())?;
        let hook_refuses = rebuild_required.is_some_and(|r| !r.is_empty()) || !narrowed;
        if hook_refuses {
            "rebuild_required"
        } else {
            "refreshable"
        }
    };
    Some(repair.to_string())
}

/// 問 PATH 上的 `cort status <root>` 拿 repair 判定；binary 不在、非零退出、
/// 輸出不是 JSON，一律 `None`（報告顯示 `?`）。唯讀動詞，不會動索引。
fn query_repair(root: &Path) -> Option<String> {
    let out = std::process::Command::new("cort")
        .arg("status")
        .arg(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    repair_from_status_json(&v)
}

/// 用量統計（cort 自己的 usage.db command_log，唯讀）：window 天內
pub fn audit_usage(window_days: u32) -> Option<UsageWindow> {
    let conn = open_usage_readonly()?;
    let since = now_ms() - (window_days as i64) * 24 * 3600 * 1000;

    let mut u = UsageWindow {
        window_days,
        ..Default::default()
    };
    if let Ok(mut stmt) =
        conn.prepare("SELECT command, COUNT(*) FROM command_log WHERE ts >= ?1 GROUP BY command")
    {
        if let Ok(rows) = stmt.query_map([&since], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        }) {
            for r in rows.flatten() {
                u.by_command.insert(r.0, r.1);
                u.total_commands += r.1;
            }
        }
    }
    // hook 結果分佈（args_summary 是 JSON）+ census（同一次掃描，多讀一個 status 欄）
    // + harness 切面（不多開查詢）
    let mut harness_declines: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for (cmd, target) in [
        ("hook-suggest", &mut u.suggest_outcomes),
        ("hook-refresh", &mut u.refresh_outcomes),
    ] {
        if let Ok(mut stmt) = conn
            .prepare("SELECT status, args_summary FROM command_log WHERE command = ?1 AND ts >= ?2")
        {
            if let Ok(rows) = stmt.query_map(rusqlite::params![cmd, since], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            }) {
                for (status, raw) in rows.flatten() {
                    // census（6623113d 口徑）：每列恰落一桶，`_total` 之外加總恆等 fires。
                    // args_summary 為 NULL 時上游會整個 census 中止；這裡照「不是合法 JSON」
                    // 落 unparseable_summary——分割窮盡是這個數字存在的全部意義。
                    let bucket = census_bucket(cmd, &status, raw.as_deref().unwrap_or(""));
                    let cb = u.census.entry(cmd.to_string()).or_default();
                    *cb.entry("_total".to_string()).or_insert(0) += 1;
                    *cb.entry(bucket).or_insert(0) += 1;
                    let r = raw.unwrap_or_default();
                    let parsed = serde_json::from_str::<serde_json::Value>(&r).ok();
                    let hook = parsed
                        .as_ref()
                        .and_then(|v| v.get("hook").and_then(|h| h.as_str()).map(String::from))
                        .unwrap_or_else(|| "unparsed".to_string());
                    *target.entry(hook.clone()).or_insert(0) += 1;
                    let str_field = |k: &str| {
                        parsed
                            .as_ref()
                            .and_then(|v| v.get(k).and_then(|s| s.as_str()).map(String::from))
                    };
                    let decline = str_field("decline");
                    let is_suggest = cmd == "hook-suggest";
                    if is_suggest {
                        if let Some(d) = &decline {
                            *u.declines.entry(format!("{hook}/{d}")).or_insert(0) += 1;
                        }
                        // shape 排行（09f55136 起）：只收可行動的 no_shape。baseline 與
                        // 沒有 shape 欄的舊列都不進——前者會淹掉靶心，後者會造出一個
                        // 假鍵（"unparsed"）讓排行看起來有資料。
                        if hook == "no_shape" && decline.as_deref() != Some("not_a_search_tool") {
                            if let Some(shape) = str_field("shape") {
                                *u.no_shape_shapes.entry(shape).or_insert(0) += 1;
                            }
                        }
                    }
                    // harness 切面：v3 payload 起才有；沒有這欄的歷史列另計，
                    // 否則各 harness 加總會悄悄對不上 hook 總數
                    match str_field("harness") {
                        None => u.harness_unknown += 1,
                        Some(h) => {
                            let st = u.by_harness.entry(h.clone()).or_default();
                            if is_suggest {
                                st.suggests += 1;
                                // hit / hit_yielded / hit_stale——與命中率同一組口徑
                                if hook.starts_with("hit") {
                                    st.hits += 1;
                                }
                                if hook == "no_shape" {
                                    st.no_shape += 1;
                                }
                            } else {
                                st.refreshes += 1;
                            }
                            if str_field("harness_declared").is_some_and(|d| d != h) {
                                st.declared_mismatch += 1;
                            }
                            if is_suggest {
                                if let Some(d) = decline
                                    .filter(|d| d != "not_a_search_tool")
                                    .filter(|_| hook == "no_shape")
                                {
                                    *harness_declines
                                        .entry(h)
                                        .or_default()
                                        .entry(d)
                                        .or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    for (h, tags) in harness_declines {
        if let Some(st) = u.by_harness.get_mut(&h) {
            st.top_decline = tags.into_iter().max_by_key(|(_, c)| *c);
        }
    }
    // query-time self-heal（cortexyoung 5f6d5267）：impact/context 回答前自癒 index。
    // heal 欄位只在「有話要說」的列上（heal.rs `attach_to`：healed 帶
    // self_healed/heal_mode/heal_ms、deferred 帶 self_healed=false/heal_deferred，
    // 綠路完全不加 key），沒有任何 heal key 的列是 5f6d5267 之前的歷史 → legacy，
    // 不混進新桶（否則「0 次自癒」與「還沒資料」互相冒充）。
    // 詞彙見上面的 HEAL_MODES／HEAL_DEFERRED_REASONS 複製品——桶按實際字串落鍵，
    // 詞彙外照樣顯示自身字串（輕量 breakdown，樣體預期極小，不做窮盡分割）。
    if let Ok(mut stmt) = conn.prepare(
        "SELECT args_summary FROM command_log \
         WHERE command IN ('impact', 'context') AND ts >= ?1",
    ) {
        if let Ok(rows) = stmt.query_map([&since], |r| r.get::<_, Option<String>>(0)) {
            for raw in rows.flatten() {
                u.heal_scanned += 1;
                let parsed = raw
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
                // NULL／非法 JSON 照 hook census 的態度：進自己的桶，不 panic、不中斷掃描
                let Some(v) = parsed else {
                    u.heal_unparseable += 1;
                    continue;
                };
                let has_heal_key = ["self_healed", "heal_mode", "heal_ms", "heal_deferred"]
                    .iter()
                    .any(|k| v.get(k).is_some());
                if !has_heal_key {
                    u.heal_legacy += 1;
                } else if v.get("self_healed").and_then(|b| b.as_bool()) == Some(true) {
                    u.heal_self_healed += 1;
                    // healed 列契約上必帶 heal_mode/heal_ms；缺欄時可見化（mode_absent），
                    // 不靜默丟棄——與 census 的 decline_absent 同一態度
                    let mode = v
                        .get("heal_mode")
                        .and_then(|m| m.as_str())
                        .unwrap_or("mode_absent");
                    *u.heal_modes.entry(mode.to_string()).or_insert(0) += 1;
                    if let Some(ms) = v.get("heal_ms").and_then(|m| m.as_i64()) {
                        u.heal_ms_total += ms;
                        u.heal_ms_max = u.heal_ms_max.max(ms);
                    }
                } else {
                    let reason = v
                        .get("heal_deferred")
                        .and_then(|r| r.as_str())
                        .unwrap_or("reason_absent");
                    *u.heal_deferred.entry(reason.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    // 背景重建事件（heal.rs `defer_to_background` → `cort index --heal-background`
    // 記自己的 usage 列）：command='index'、args_summary 帶 `"heal":"background"`。
    // 一般 index 列不計——這裡是絕對次數，不進 scanned 分母。
    if let Ok(mut stmt) =
        conn.prepare("SELECT args_summary FROM command_log WHERE command = 'index' AND ts >= ?1")
    {
        if let Ok(rows) = stmt.query_map([&since], |r| r.get::<_, Option<String>>(0)) {
            for raw in rows.flatten() {
                let is_background = raw
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                    .and_then(|v| v.get("heal").and_then(|h| h.as_str()).map(String::from))
                    .is_some_and(|h| h == "background");
                if is_background {
                    u.heal_background += 1;
                }
            }
        }
    }
    u.errors = conn
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE status = 'error' AND ts >= ?1",
            [&since],
            |r| r.get(0),
        )
        .unwrap_or(0);
    u.index_stale_queries = conn
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE index_stale = 1 AND ts >= ?1",
            [&since],
            |r| r.get(0),
        )
        .unwrap_or(0);
    u.saved_bytes = conn
        .query_row(
            "SELECT COALESCE(SUM(saved_bytes), 0) FROM command_log WHERE ts >= ?1",
            [&since],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Some(u)
}

/// 完整審計：索引健康/覆蓋 + 用量（window 天）+ 固定 7 天早期訊號
pub fn audit(root: &Path, window_days: u32) -> CortAudit {
    let real = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let usage = audit_usage(window_days);
    let usage_7d = if window_days == 7 {
        usage.clone()
    } else {
        audit_usage(7)
    };
    // repair 只有 binary 判得出來（見 query_repair），DB 讀得到才問——
    // 沒索引的專案沒有 repair 可講
    let mut index = audit_index(&real);
    if let Some(i) = index.as_mut() {
        i.repair = query_repair(&real);
    }
    CortAudit {
        root: real.to_string_lossy().into_owned(),
        host: host_name(),
        window_days,
        index,
        db_exists: db_exists(&real),
        usage,
        usage_7d,
    }
}
