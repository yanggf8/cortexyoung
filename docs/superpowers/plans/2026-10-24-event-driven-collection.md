# 事件觸發收集 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 hook 觸發的兩條非同步收集（批次上傳、逐筆健康觀察）取代 cron 與每日快照。

**Architecture:** 兩個 hook 結尾呼叫常數成本的 `collect::kick`，它依本機狀態檔與 `usage.db` 計數判斷是否到期，到期就以 detached 子程序啟動 `cort measure collect` 或 `cort measure health`。健康觀察只讀 index、寫本機 `usage.db` 的 `index_health_log`；批次收集是唯一碰網路的路徑，負責 transcript 掃描、`cort_command` 與新來源 `cort_index_health` 的上傳。

**Tech Stack:** Rust（`rust/` crate）、rusqlite、turso sync、flock（`upgrade::try_exclusive_lock`）、`heal.rs` 的 detached spawn。

**Spec:** `docs/superpowers/specs/2026-10-24-event-driven-collection-design.md`（第 7 版）。本計畫取代 `docs/superpowers/plans/2026-10-23-measurement-in-turso.md` 的 Task 8；那份計畫的 Task 9、10 依設計第 8 節調整，不在本計畫內。

## Global Constraints

- 純 Rust；不得新增任何腳本語言的可執行檔；bash 只留 `install.sh`、`tests/install-smoke.sh`。
- repo 內不得有任何開發者機器的絕對路徑（含測試、註解）。
- 儲存失敗一律回傳，不 panic（產品程式碼不得 `unwrap`/`expect` 在 I/O 或 SQL 上）。
- hook 契約：安靜、exit 0、忙就放棄、不做網路、`hook-refresh` 永不建立 index。
- `command_log` 的欄位不得增刪改名（欄位名稱進 claudecat 相容的 event_id）；可以加表、加索引、加 `_usage_meta` 列。
- 測試絕不碰真的 crontab（`CORT_CRONTAB_FILE`）、真的 data dir（`CORT_MEMORY_DIR` 指向暫存目錄）、真的 index cache（`CORT_CACHE_DIR`）、網路與真的 Turso。
- 預設值（都可用環境變數覆寫，無法解析時用預設）：
  `CORT_COLLECT_BASE_SECS=300`、`CORT_COLLECT_MAX_SECS=3600`、`CORT_COLLECT_BATCH_ROWS=2000`、
  `CORT_COLLECT_IDLE_SECS=900`、`CORT_HEALTH_INTERVAL_SECS=600`、`CORT_COLLECT_NET_TIMEOUT_SECS=120`、
  `CORT_COLLECT_DEADLINE_SECS=600`、`CORT_COLLECT_MANUAL_WAIT_SECS=60`、`CORT_HEALTH_ROOT_TIMEOUT_SECS=30`。
- 檔案位置（都在 `data_dir()` 底下）：`measure-collect-state.json`、`measure-health-state.json`、
  `measure-collect.lock`、`measure-health.lock`、`measure-batch.kick.lock`、`measure-health.kick.lock`、
  `measure-batch.spawned`、`measure-health.spawned`、`measure.log`。
- 鎖的順序：`measure-collect.lock` → `measure.lock` → `legacy-telemetry.lock`，倒序放；`*.kick.lock` 不與它們同時持有。
- 大型 `usage.db` fixture（20 萬列）：先經一次 `record_command_at` 建 schema，再用**一個連線、一個 transaction** 直接 `INSERT`；不要逐列呼叫 `record_command_at`（每次都開連線、跑 schema、跑 prune，會讓之後每張單的 gate 變慢）。
- 每個任務結束：`cargo fmt --all` 於 `rust/` 與 `evals/`，再 `cargo fmt --all --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked --all-targets`（兩個 crate），測試一律帶 `CORT_CRONTAB_FILE=<暫存檔>`；cargo 一次只跑一個。

## Review Focus

1. **一個很大的 `usage.db`（數十萬列）上的 hook 延遲**：kick 不能隨列數變慢——Task A1 的 `kick_counters_cost` 量測、Task C2 的 `kick_median_cost` 量測。
2. **沙箱裡 `data_dir()` 唯讀**：kick 不生任何程序、hook 照常 exit 0——Task C2 的 `stamp_write_failure_spawns_nothing`。
3. **同時兩個 hook 觸發**：只生一個程序——Task C2 的 `concurrent_kicks_spawn_once`。
4. **index 在觀察途中被刪或被另一個程序重建**：記 `unreadable`/`no_index`，不 panic、不寫 index——Task D2 的 `index_deleted_mid_observation_is_recorded`。
5. **`usage.db` 被換掉或還原舊檔**：不卡住、不重傳——Task B 的 `instance_change_resets_cursors`、`rewind_resets_cursors`，Task F 的 `reobservation_after_replacement_is_one_event`。

---

### Task A1: `usage.db` 的健康表、部分索引、instance_id、kick 計數

**Files:**
- Modify: `rust/src/usage_schema.sql`、`rust/src/usage.rs`（`ensure_schema` ~752、`prune_best_effort` ~882）
- Test: `rust/tests/usage_health.rs`

**Interfaces:**
- Produces:
  ```rust
  pub const INSTANCE_KEY: &str = "INSTANCE_ID";
  pub struct KickCounters { pub instance_id: String, pub max_command_id: i64, pub max_any_command_id: i64, pub max_health_id: i64 }
  // max_any_command_id 由 B-fix 加入：不分 command 的 MAX(id)，只給倒退偵測用（設計 §1 第 4 步）
  pub fn kick_counters_at(path: &Path) -> Option<KickCounters>; // 唯讀、busy 25 ms；檔案/表/索引/instance 缺任何一個或出錯 → None
  ```
- `index_health_log` 欄位：`id INTEGER PRIMARY KEY`、`ts INTEGER NOT NULL`、`machine_id TEXT NOT NULL`、`machine_source TEXT NOT NULL`、`root TEXT`、`local_project_id TEXT NOT NULL`、`project_id TEXT`、`status TEXT NOT NULL CHECK(status IN ('ok','unreadable','missing_dir','no_index'))`、`error TEXT`、`fresh TEXT`、`chunks INTEGER`、`relationships INTEGER`、`not_chunked INTEGER`、`real_gap INTEGER`、`fts TEXT`；索引 `idx_index_health_log_local ON index_health_log(local_project_id, id)`。

- [x] **Step 1: 寫失敗的測試**（暫存 `usage.db`；健康列用測試內的原始 SQL 插入）
  - `schema_adds_health_table_and_partial_index`：`sqlite_master` 有 `index_health_log`、`idx_index_health_log_local`、`idx_command_log_not_measure`。
  - `command_log_columns_unchanged`：`telemetry::command_log_columns` 的結果與加表前相同（把目前的字串寫死在測試裡）。
  - `instance_id_is_stamped_once`：同一檔開兩次 instance 不變；另一個新檔 instance 不同；32 個 hex 字元。
  - `kick_counters_excludes_measure_rows`：寫 `impact`、`hook-refresh`、`measure`、`measure` 四列 → `max_command_id` 是第 2 列的 id。
  - `kick_counters_none_when_missing`：檔案不存在 → None，且沒有建出檔案。
  - `partial_index_is_used`：`EXPLAIN QUERY PLAN SELECT MAX(id) FROM command_log WHERE command <> 'measure'` 含 `idx_command_log_not_measure`。
  - `kick_counters_cost`：20 萬列（尾端 5 萬列是 `measure`）上呼叫 100 次，印中位數；斷言中位數 < 20 ms（寬鬆上限防 CI 抖動，實際數字寫進回報）。
  - `health_rows_pruned_with_command_log`：ts 為 91 天前的健康列在下一次 `record_command_at` 後消失，89 天前的留著。
- [x] **Step 2: 跑 `cargo test --locked --test usage_health`，預期失敗。**
- [x] **Step 3: 實作。** schema 用 `CREATE TABLE/INDEX IF NOT EXISTS`（不升 `USAGE_SCHEMA_VERSION`，因為沒有 migration）。instance_id 在 `ensure_schema` 以 `INSERT OR IGNORE` 寫入 `_usage_meta`，值取 sha256（目前時間的奈秒、pid、檔案路徑）的前 32 個 hex 字元。prune 對 `index_health_log` 用同一個 cutoff 與分批刪除。
- [x] **Step 4: 跑測試至綠，再跑全套 gate（Global Constraints）。**

### Task A2: 健康列的讀寫函式

**Files:**
- Modify: `rust/src/usage.rs`
- Test: `rust/tests/usage_health.rs`（補）

**Interfaces:**
- Consumes: A1 的 `index_health_log`。
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq)]
  pub struct HealthRow { pub id: i64, pub ts: i64, pub machine_id: String, pub machine_source: String,
      pub root: Option<String>, pub local_project_id: String, pub project_id: Option<String>,
      pub status: String /* ok|unreadable|missing_dir|no_index */, pub error: Option<String>,
      pub fresh: Option<String>, pub chunks: Option<i64>, pub relationships: Option<i64>,
      pub not_chunked: Option<i64>, pub real_gap: Option<i64>, pub fts: Option<String> }
  pub fn insert_health_row_at(path: &Path, row: &HealthRow) -> Result<i64, String>; // 忽略 row.id，回新 id
  pub fn last_health_rows_at(path: &Path, machine_id: &str) -> Result<HashMap<String, HealthRow>, String>; // 只看該 machine_id 的列，每個 local_project_id 最新一列
  pub fn health_rows_after_at(path: &Path, after_id: i64, limit: usize) -> Result<Vec<HealthRow>, String>; // id 遞增
  pub fn touched_project_ids_at(path: &Path, after_id: i64) -> Result<(Vec<String>, i64), String>;
      // command <> 'measure' 且 project_id 非 NULL、非 '_unknown' 的不重複 project_id，以及讀到的最大 id（沒有列 → after_id）
  ```

- [x] **Step 1: 寫失敗的測試**
  - `health_row_roundtrip`：insert 後 `health_rows_after_at(0, 10)` 讀回相同欄位（id 除外）。
  - `health_rows_after_pages_in_id_order`：5 列、`after_id` = 第 2 列、limit 2 → 第 3、4 列。
  - `last_health_rows_returns_latest_per_project`。
  - `last_health_rows_ignore_other_machines`：同一個 `local_project_id` 有 machine A 的較新列與 machine B 的較舊列 → 以 B 查只回 B 的那列；以 C 查回空。
  - `touched_project_ids_skip_measure_null_unknown`：混合 `measure`、NULL、`_unknown`、兩個真 id（各兩列）→ 回兩個 id，最大 id 是最後一列非 measure 的 id。
  - `invalid_status_is_an_error_not_a_panic`：`status="bogus"` → `Err`。
- [x] **Step 2: 跑 `cargo test --locked --test usage_health`，預期失敗。**
- [x] **Step 3: 實作。** 每個函式開一個連線；寫入沿用 `record_command_at` 的 busy timeout。
- [x] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task B: 收集狀態與到期規則（純函式）

**Files:**
- Create: `rust/src/collect.rs`（`lib.rs` 加 `pub mod collect;`）
- Test: `rust/tests/collect_due.rs`

**Interfaces:**
- Consumes: Task A1 的 `KickCounters`。
- Produces:
  ```rust
  pub struct CollectConfig { pub base_interval_ms: i64, pub max_interval_ms: i64, pub batch_rows: i64,
      pub idle_due_ms: i64, pub health_interval_ms: i64, pub net_timeout_ms: u64, pub run_deadline_ms: u64,
      pub manual_wait_ms: u64, pub root_timeout_ms: u64 }
  impl CollectConfig { pub fn from_env() -> Self; pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self; }
  #[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
  pub struct Failure { pub count: u32, pub last_error_kind: String, pub last_failed_ms: i64 }
  #[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
  pub struct CollectState { pub instance_id: Option<String>, pub last_run_ms: Option<i64>, pub last_ok_ms: Option<i64>,
      pub failures: BTreeMap<String, Failure>, pub push_pending: bool, pub uploaded_through_id: i64,
      pub health_uploaded_through_id: i64, pub refused: bool }
  #[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
  pub struct HealthState { pub instance_id: Option<String>, pub health_cursor: i64, pub last_run_ms: Option<i64>,
      pub unmapped_count: u64, pub unmapped_recent: Vec<String> /* 最多 5 個，最新在前，不重複 */ }
  pub fn collect_state_path() -> PathBuf; pub fn health_state_path() -> PathBuf;
  impl CollectState { pub fn load(p: &Path) -> Self; pub fn save(&self, p: &Path) -> Result<(), String>; }
  impl HealthState  { pub fn load(p: &Path) -> Self; pub fn save(&self, p: &Path) -> Result<(), String>; }
      // load：缺檔或壞檔 → Default；save：data_dir::write_atomic
  pub fn batch_cursors(s: &CollectState, c: &KickCounters) -> (i64, i64); // (command, health)；instance 不同 → 0；command 游標 > max_any_command_id → 0；health 游標 > max_health_id → 0
  pub fn health_cursor(s: &HealthState, c: &KickCounters) -> i64;
  pub fn min_interval_ms(s: &CollectState, harness: &str, cfg: &CollectConfig) -> i64; // base × 2^count(該 harness)，上限 max
  pub fn batch_due(s: &CollectState, c: &KickCounters, harness: &str, now_ms: i64, cfg: &CollectConfig) -> bool;
  pub fn health_due(s: &HealthState, c: &KickCounters, now_ms: i64, cfg: &CollectConfig) -> bool;
  ```

- [x] **Step 1: 寫失敗的測試**（`from_lookup` 注入環境，不動真的環境變數）
  - `defaults_match_spec`：九個預設值逐一斷言（Global Constraints 的數字）。
  - `garbage_env_keeps_default`：`"abc"` → 預設；`" 7 "` → 7 秒。
  - `never_run_with_pending_is_due`：預設 state、counters `max_command_id=1` → due。
  - `within_min_interval_not_due`：`last_run_ms = now-299_000` → 不 due；`now-300_000` 且 pending ≥ 1 但距上次 < 15 分鐘 → 不 due。
  - `big_backlog_due_after_min_interval`：pending 2000、距上次 301 秒 → due；pending 1999 → 不 due。
  - `idle_rule`：pending 1、距上次 900 秒 → due。
  - `push_pending_due_with_zero_pending`。
  - `refused_excludes_command_pending`：refused、command pending 5000、health pending 0 → 不 due；health pending 1 且距上次 900 秒 → due。
  - `pending_terms_clamped`：`uploaded_through_id=101`、`max_command_id=100`、health pending 1、距上次 900 秒 → due（Codex 第三輪 #1）。
  - `backoff_is_per_harness`：codex 失敗 3 次 → codex 的 `min_interval` 2400 秒、claude-code 的 300 秒；失敗 10 次上限 3600 秒。
  - `instance_change_resets_cursors`、`rewind_resets_cursors`（游標 > MAX → 0）。
  - `health_due_rules`：cursor < MAX 且距上次 ≥ 600 秒 → due；任一不成立 → 不 due。
  - `state_roundtrip_and_corrupt_file_is_default`。
- [x] **Step 2: 跑 `cargo test --locked --test collect_due`，預期失敗。**
- [x] **Step 3: 實作。** `batch_due` 依設計第 2 節到期規則；`pending = max(0, cmd) + max(0, health)`，refused 時 cmd 項為 0。
- [x] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task C1: detached spawn 共用、`Lane`／`Spawner`、`KickPaths`

**Files:**
- Modify: `rust/src/heal.rs`（`spawn_background_healer` ~212/~235 改成呼叫新的 `pub(crate) fn spawn_detached(cmd: std::process::Command) -> io::Result<()>`，由它補上 setsid 與 null stdio，兩個 cfg 分支都要；healer 自己組 `Command` 後交給它）、`rust/src/measure_store.rs`（`config_path` ~763 改成 `pub(crate)`）、`rust/src/collect.rs`
- Test: `rust/tests/collect_kick.rs`

**Interfaces:**
- Consumes: Task B 的 `collect.rs`。
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Lane { Batch, Health }
  pub trait Spawner { fn spawn(&mut self, lane: Lane, harness: &str) -> std::io::Result<()>; }
  pub struct DetachedSpawner; // heal::spawn_detached(detached_command(current_exe(), lane, harness))
  pub const HARNESS_ENV: &str = "CORT_COLLECT_HARNESS";
  pub fn detached_command(exe: &Path, lane: Lane, harness: &str) -> std::process::Command; // 引數 spawn_args(lane)、環境加 HARNESS_ENV=harness；只組指令不執行
  pub fn spawn_args(lane: Lane) -> [&'static str; 3]; // ["measure","collect","--background"] / ["measure","health","--background"]
  pub struct KickPaths { pub data_dir: PathBuf, pub usage_db: PathBuf, pub cache_dir: PathBuf, pub measure_config: PathBuf }
  impl KickPaths { pub fn current() -> Option<Self>; } // usage_db 為 None → None
  ```
- `measure_config` 一律取自 `measure_store::config_path()`，`collect.rs` 不得自己拼 `turso-measure.json`；`cache_dir` 取自 `db::cache_dir()`；`usage_db` 取自 `usage::usage_db_path()`。

- [x] **Step 1: 寫失敗的測試**
  - `spawn_args_are_stable`。
  - `detached_command_carries_lane_and_harness`：`collect::detached_command(exe, Lane::Batch, "codex")` 的 `get_args()` 等於 `spawn_args(Lane::Batch)`、`get_envs()` 含 `(CORT_COLLECT_HARNESS, codex)`。不實際生程序。
  - `kick_paths_follow_env`：`CORT_MEMORY_DIR`、`CORT_CACHE_DIR` 指暫存目錄 → `data_dir`、`cache_dir` 相符，`measure_config == data_dir.join("turso-measure.json")`。
- [x] **Step 2: 跑 `cargo test --locked --test collect_kick`，預期失敗。**
- [x] **Step 3: 實作。** healer 的行為不得改變（同樣的 setsid、null stdio、引數）。
- [x] **Step 4: 跑測試至綠，再跑全套 gate；`heal` 既有測試必須全綠。**

### Task C2: `kick` 與 hook 接線

**Files:**
- Modify: `rust/src/collect.rs`（kick）、`rust/src/main.rs`（`cmd_hook_refresh` ~1154、`cmd_hook_suggest` ~1314）
- Test: `rust/tests/collect_kick.rs`（補）、`rust/tests/` 既有 hook 測試檔補斷言

**Interfaces:**
- Consumes: Task A1 `kick_counters_at`、Task B 全部、Task C1 全部。
- Produces:
  ```rust
  pub fn kick_at(paths: &KickPaths, harness: &str, now_ms: i64, cfg: &CollectConfig, spawner: &mut dyn Spawner) -> &'static str;
  pub fn kick(harness: &str) -> &'static str; // current() + from_env() + now + DetachedSpawner；吞掉一切錯誤
  ```
  回傳的標籤（記進 hook 列 `args_summary` 的 `"collect"`）只會是：`not_configured`、`upgrade`、`no_counters`、`not_due`、`busy`、`throttled`、`stamp_failed`、`spawned_batch`、`spawned_health`、`spawned_both`、`spawn_failed`。

- [x] **Step 1: 寫失敗的測試**（暫存目錄 + 假 `Spawner` 記錄呼叫）
  - `not_configured_touches_nothing`：沒有 measure 設定檔 → `not_configured`，data dir 內容前後相同。
  - `due_batch_spawns_once_and_writes_stamp`。
  - `throttled_within_interval`：剛寫過 stamp → 第二次 `throttled`，假 spawner 只被呼叫一次。
  - `concurrent_kicks_spawn_once`：兩個執行緒以 `Barrier` 同時 `kick_at` → 合計一次 spawn。
  - `stamp_write_failure_spawns_nothing`：data dir 設成 0500 → `stamp_failed`，零次 spawn（以 root 執行時印 `SKIP:`）。
  - `busy_lane_lock_spawns_nothing`：測試先持有 `measure-collect.lock` → `busy`。
  - `kick_releases_lane_lock`：kick 之後測試能立刻拿到 `measure-collect.lock`。
  - `both_lanes_due_spawn_both`。
  - `kick_median_cost`：20 萬列 `usage.db`、不到期的狀態下呼叫 200 次，印中位數，斷言 < 20 ms（實際數字寫進回報；設計目標 < 2 ms）。
  - hook 的 CLI 測試：`hook-refresh` 與 `hook-suggest` 在未設定 measure 時 stdout 為空、exit 0，寫進 `command_log` 的那列 `args_summary` 含 `"collect":"not_configured"`。
- [x] **Step 2: 跑 `cargo test --locked --test collect_kick`，預期失敗。**
- [x] **Step 3: 實作。** 步驟照設計第 1 節；升級檢查用 `upgrade::try_protected_entry(&paths.cache_dir)`；試鎖用 `upgrade::try_exclusive_lock` 試完立刻 drop；stamp 讀寫在 `*.kick.lock` 內。兩個 hook 在每個「已經知道 harness」的返回路徑都呼叫 `kick`，並把標籤放進該次的 hook row JSON；`kick` 自己做升級檢查，所以 `hook-suggest` 在升級檢查之前就返回的 `no_shape`（大部分 Bash 呼叫走這條）也要 kick，只有讀不到 payload／沒有 `tool_input` 這兩條還不知道 harness 的路徑不 kick。
- [x] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task D1: 唯讀開檔、`repair_label`、`fresh_label`

**Files:**
- Create: `rust/src/health.rs`（`lib.rs` 加 `pub mod health;`）
- Modify: `rust/src/db.rs`（加唯讀開檔）、`rust/src/staleness.rs`（加 `repair_label`，`rust/src/impact.rs:211-217` 改用它）、`rust/src/heal.rs`（`heal_max_files` ~37 改成 `pub(crate)`）
- Test: `rust/tests/health_observe.rs`

**Interfaces:**
- Produces:
  ```rust
  // db.rs
  pub fn open_db_readonly(db_path: &Path) -> rusqlite::Result<Db>; // SQLITE_OPEN_READ_ONLY，不建檔、不建表、不 migrate
  // staleness.rs
  pub fn repair_label(stale: &StaleReport) -> &'static str; // "none" | "refreshable" | "rebuild_required"
  // health.rs
  pub fn fresh_label(repair: Option<&str>, rebuild_reasons: &[String], stale: bool, graph_pending: Option<bool>) -> &'static str;
  ```
- `fresh_label` 對應照 claudecat `src/cort_audit.rs` `row_md`（~562），但 rebuild 先看原因（設計第 3 節）：`rebuild_reasons` 非空且全是 `graph_incomplete` → `STALE/graph`；否則 `rebuild_required`→`STALE/rebuild`、`refreshable`→`STALE/refresh`、否則 graph_pending `Some(true)`→`STALE/graph`、stale→`STALE`、未 stale 且 `None`→`fresh?`、未 stale 且 `Some(false)`→`fresh`。**沒有 7 天門檻。**

- [x] **Step 1: 寫失敗的測試**
  - `fresh_label_covers_every_label`：六個標籤（`STALE/rebuild`、`STALE/refresh`、`STALE/graph`、`STALE`、`fresh?`、`fresh`）各一例。沒有 index 不是標籤，由 `status=no_index` 表達。
  - `graph_reason_alone_is_stale_graph`：`repair=Some("rebuild_required")`、原因 `["graph_incomplete"]` → `STALE/graph`；原因 `["graph_incomplete","schema_changed"]` → `STALE/rebuild`。
  - `open_db_readonly_creates_nothing`：不存在的路徑 → `Err`，且檔案與父目錄都沒被建出來。
  - `open_db_readonly_refuses_writes`：對既有 index 開唯讀後 `INSERT` → `Err`。
  - `impact_repair_unchanged`：`impact` 既有 repair 相關測試全綠（重構後）。
- [x] **Step 2: 跑 `cargo test --locked --test health_observe`，預期失敗。**
- [x] **Step 3: 實作。**
- [x] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task D2: 觀察單一 index 的健康（只讀）

**Files:**
- Modify: `rust/src/health.rs`
- Test: `rust/tests/health_observe.rs`（補）

**Interfaces:**
- Consumes: D1 全部、Task B `CollectConfig`。
- Produces:
  ```rust
  pub struct Observation { pub status: &'static str, pub error: Option<String>, pub fresh: Option<String>,
      pub chunks: Option<i64>, pub relationships: Option<i64>, pub not_chunked: Option<i64>,
      pub real_gap: Option<i64>, pub fts: Option<String> }
  pub fn observe(root: &Path, local_project_id: &str, cfg: &CollectConfig) -> Observation;
  pub fn bounded<F: FnOnce() -> Observation + Send + 'static>(f: F, timeout: Duration) -> Observation;
      // 在新執行緒上跑 f，recv_timeout；逾時 → status=unreadable、error=Some("timeout")、指標全 None；被放棄的執行緒不 join
  pub fn observe_bounded(root: &Path, local_project_id: &str, cfg: &CollectConfig) -> Observation; // bounded(observe, root_timeout_ms)
  ```
- 指標 SQL 照 claudecat `src/cort.rs` ~595-680（not_chunked、real_gap、fts_drift）與 `read_meta`（~94-118，graph_pending）逐字移植，每段註解標原始 `claudecat src/<file>:<line>`（repo 內不寫絕對路徑）；FTS：drift 0 → `"synced"`、N → `"drift=N"`、讀不到 → NULL；`-1` 一律存 NULL。

- [x] **Step 1: 寫失敗的測試**（暫存目錄建小型 git repo，用既有的 `fake_ast_grep` 與 `cort index` 建 index；`CORT_CACHE_DIR` 指暫存；同一個 index 在多個測試間共用時各自複製一份；實作改用真的 ast-grep 經 `cort index` 建 index，同 `tests/cli.rs`；沒有 ast-grep 時測試失敗而不是 SKIP，同 `tests/staleness.rs`）
  - `fresh_index_observed_ok`：剛建好的 index → `status=ok`、`fresh=Some("fresh")`、chunks ≥ 1、`fts=Some("synced")`。
  - `no_age_gate`：把 index 的 `last_indexed_at` 改成 40 天前、HEAD 不動 → 仍是 `fresh`。
  - `edited_file_is_stale_refresh`：改一個檔 → `STALE/refresh`。
  - `observation_never_writes_the_index`：觀察前後 index db 檔的 bytes 與 mtime 相同；cache 目錄的新檔只可能是該 index 的 `-shm`／`-wal` 這一對（唯讀開 WAL 資料庫時 SQLite 自己建的，D1 `open_db_readonly` 的文件與測試已釘住），且在沒有其他寫入者的 fixture 裡 `-wal` 為 0 bytes。
  - `missing_index_is_no_index`、`missing_dir_is_missing_dir`。
  - `unreadable_index_records_error`：index 檔寫入垃圾 bytes → `status=unreadable`、`error` 非空、所有指標 None。
  - `large_tree_skips_staleness`：`CORT_HEAL_MAX_FILES=0` → `fresh=None`、`error=Some("staleness_skipped_large_tree")`，其他指標照讀。
  - `graph_pending_index_is_stale_graph`：建好的 index 把 graph_pending 的 meta 設成 true（照 `read_meta` 讀的那個鍵）→ `fresh=Some("STALE/graph")`。
  - `bounded_times_out`：`bounded(|| { sleep(2s); ok_obs }, 100ms)` 在 1 秒內回 `status=unreadable`、`error=Some("timeout")`。
  - `index_deleted_mid_observation_is_recorded`：測試注入點在讀完 chunks 後刪檔（用 `observe` 的內部分段函式測），結果 `unreadable` 或 `no_index`，不 panic。
- [x] **Step 2: 跑 `cargo test --locked --test health_observe`，預期失敗。**
- [x] **Step 3: 實作。** 開 index 一律 `open_db_readonly`，不用 `immutable=1`（連 `CANTOPEN` 的退路也不用：它承諾檔案不變、略過鎖與 WAL，會在 hook 正在寫時讀到不一致或漏掉 WAL 內容；開不起來就照實記 `no_index`／`unreadable`）；staleness 走 `compute_stale` 前先用與 `heal.rs` 相同的候選檔計數判斷是否超過 `heal_max_files()`；逾時一律由 `bounded` 處理（`compute_stale` 內部沒有可停下的點，不做階段間檢查）。
- [x] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task E: 逐筆健康收集 `cort measure health`

**Files:**
- Modify: `rust/src/health.rs`（worker）、`rust/src/main.rs`（`MeasureAction::Health { background: bool }`）
- Test: `rust/tests/health_worker.rs`

**Interfaces:**
- Consumes: Task A1（`kick_counters_at`）、A2（`touched_project_ids_at`、`last_health_rows_at`、`insert_health_row_at`）、Task B（`HealthState`）、Task C1／C2（`KickPaths`、`Spawner`、`kick_at`）、Task D2（`observe_bounded`）。
- Produces:
  ```rust
  pub struct HealthRun { pub observed: usize, pub written: usize, pub unmapped: usize, pub skipped_busy: bool }
  pub fn run_health(paths: &KickPaths, machine_id: &str, machine_source: &str, now_ms: i64, cfg: &CollectConfig,
      spawner: &mut dyn Spawner) -> Result<HealthRun, String>; // CLI 傳 DetachedSpawner，測試傳假的
  ```
- 流程照設計第 3 節：拿 `measure-health.lock`（拿不到 → `skipped_busy`）；instance／倒退 → 游標歸零；`touched_project_ids_at` → 對回 root（`list_projects()` 的 `Indexed`；`Unreadable` 由 `db_path` 檔名取 project_id 記 `unreadable`；`last_health_rows_at(path, machine_id)` 的本機歷史 root；都沒有 → `unmapped`，`unmapped_count` 加一、id 放到 `unmapped_recent` 最前面並去重，只留 5 個）；逐一 `observe_bounded`；**寫入條件**：與該 project 上一列所有指標與 status 相同且同一 UTC 日 → 不寫；`project_id` 用 `memory_store::project_from_git_origin(root)`（與壓縮列同一函式，回 `Result<String, String>`），`Err` 存 NULL；寫完推進 cursor、存 state；`written > 0` 時以同一個 `spawner` 呼叫一次 `kick_at(..., "measure-health", ..., spawner)`。
- CLI：`cort measure health [--background]`；`--background` 時把一行 JSON 摘要附加到 `measure.log`、stdout 不輸出；沒有 `--background` 時印 JSON。兩者都 exit 0，除非 `usage.db` 寫入失敗（`measure_health_failed`）。

- [x] **Step 1: 寫失敗的測試**（實作拆成 E1 `run_health`＋`tests/health_worker.rs`、E2 CLI＋`tests/health_cli.rs`；審查後另加：不同原因的 unreadable 同日重寫、升級中跳過、origin 查詢卡住不佔鎖、背景 `--help`／`--version` 不輸出、並發 append 不黏行）
  - `touched_project_is_observed_and_written`。
  - `same_state_same_day_not_rewritten`；`same_state_next_day_rewritten`；`changed_state_rewritten`。
  - `deleted_index_maps_back_via_history`：先觀察一次，刪 index，再寫一筆 command 列 → 第二列 `status=no_index`、root 相同。
  - `unreadable_entry_recorded_not_unmapped`。
  - `never_observed_deleted_index_is_unmapped`：`unmapped_count=1`、`unmapped_recent` 含該 id、沒有寫列。
  - `unmapped_recent_keeps_five_newest`：7 個不同的未對應 id → 留最後 5 個、最新在前；同一個 id 重複出現只留一次。
  - `measure_rows_do_not_trigger_observation`。
  - `other_machine_history_is_ignored`：`usage.db` 裡有另一個 machine_id 的同日同狀態列 → 本機照樣寫一列。
  - `busy_lock_skips`。
  - `handoff_kicks_batch_when_written`：假 spawner 收到一次 `Lane::Batch`（若到期）；沒寫列時不 kick。
  - `background_writes_log_not_stdout`（CLI）。
- [x] **Step 2: 跑 `cargo test --locked --test health_worker`，預期失敗。**
- [x] **Step 3: 實作。**
- [x] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task F: 健康來源上傳 `cort_index_health`

**Files:**
- Modify: `rust/src/telemetry.rs`（新來源）、`rust/src/measure_store.rs`（若需要新的 trait 方法）
- Test: `rust/tests/health_upload.rs`

**Interfaces:**
- Consumes: Task A2 `health_rows_after_at`；Task 6 既有的 `event_watermarks`、`insert_events`。
- Produces:
  ```rust
  pub const SOURCE_HEALTH: &str = "cort_index_health";
  pub fn health_event_id(machine_id: &str, utc_day: &str, root: &str, payload_without_ts: &str) -> String;
      // sha256("cort_index_health\0" + machine + "\0" + day + "\0" + root + "\0" + payload_without_ts)，root 為 None 時用 ""
  pub fn health_events(usage_db: &Path, after_id: i64, limit: usize) -> Result<Vec<TelemetryEvent>, String>;
  pub fn upload_health_events(store: &mut dyn MeasureStore, usage_db: &Path, machine_id: &str) -> EventsReport;
      // 不經 dry-run 閘門；水位 source=cort_index_health、指紋 "ts|root"；每頁 2000；每頁寫進副本後推進水位
      // 水位先驗證（同 upload_events）：該列指紋不同或不在 → 全範圍讀，回報 reason watermark_row_changed / watermark_row_missing
  ```
- payload：該列所有欄位的緊湊 JSON，key 字母序，用 `cort_command` 同一個序列化器；`payload_without_ts` 去掉 `id` 與 `ts`。`time_ms` = 列的 `ts`；`project_id` = 列的 `project_id`。

- [x] **Step 1: 寫失敗的測試**（`SqliteStore`）
  - `health_event_id_is_content_keyed`：同 day、root、指標但 ts 不同 → 同 id；任一指標不同 → 不同 id；不同 day → 不同 id。
  - `uploads_without_dry_run_marker`：沒有 marker 也上傳。
  - `uploads_while_command_source_refused`：同一次 `cort_command` 回 `Refused`、健康照樣 `Uploaded`。
  - `watermark_skips_uploaded_health_rows`。
  - `reobservation_after_replacement_is_one_event`：換一個新 `usage.db`、寫一列同日同狀態的觀察 → 上傳後遠端該 (day, root) 只有一列，且回報 `full_range=true`。
  - `replacement_below_old_watermark_still_uploads`：舊庫上傳到水位 id 5；換新庫寫 3 列**不同狀態**的觀察（id 1..3）→ 三列都進遠端，回報 reason `watermark_row_missing`。
  - `changed_row_at_watermark_reads_full_range`：新庫有 ≥ 舊水位 id 的列但該列 `ts|root` 不同 → `watermark_row_changed`、全範圍。
  - `payload_keys_sorted`。
- [x] **Step 2: 跑 `cargo test --locked --test health_upload`，預期失敗。**
- [x] **Step 3: 實作。**
- [x] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task G1a: 批次收集核心 `run_collect`（成功、失敗、當機安全）

**Files:**
- Modify: `rust/src/collect.rs`（worker）、`rust/src/measure.rs`（`record_with_scan` ~324 讓呼叫端能分辨 push 失敗、pull 失敗、儲存失敗與事件 `Refused`）、`rust/src/memory_store.rs`（`lock_in` ~391 旁加有期限的版本）、`rust/src/measure_store.rs`（給 collect 用的有期限開 store）
- Test: `rust/tests/collect_worker.rs`

**Interfaces:**
- Consumes: Task B、C1、F；既有 `record_with_scan`、`upload_events`、`EventsReport`。
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum ErrorKind { Network, Storage, Other }
  pub struct CollectRun { pub outcome: &'static str /* ok|failed|skipped_busy */, pub refused: bool,
      pub error_kind: Option<ErrorKind>, pub summary: serde_json::Value }
  // memory_store.rs
  pub(crate) enum LockWait { TimedOut, Failed(String) }
  pub(crate) fn lock_in_within(subdir: &str, config: &Config, name: &str, wait: Duration) -> Result<fs::File, LockWait>; // try_lock 輪詢到期限
  // measure_store.rs
  pub fn open_measure_store_within(wait: Duration) -> Result<Box<dyn MeasureStore>, OpenError>; // OpenError::{Busy, Other(String)}
  // collect.rs
  pub fn run_collect(store_opener: &mut dyn FnMut() -> Result<Box<dyn MeasureStore>, OpenError>,
      paths: &KickPaths, harness: &str, now_ms: i64, cfg: &CollectConfig, manual: bool) -> CollectRun;
  ```
- 流程照設計第 2 節：拿 `measure-collect.lock`（本單只做背景：拿不到 → `skipped_busy`）；拿到後把自己的 `pid` 與 `started_ms` 寫進鎖檔；instance／倒退 → 游標歸零；**先**存 `push_pending=true`，再呼叫 `store_opener`（正式路徑是 `open_measure_store_within(manual_wait_ms)`，拿不到 `measure.lock` 回 `Busy` → `skipped_busy`，不算失敗）做 scan + `cort_command` 上傳 + 健康上傳 + push + pull；push/pull/儲存失敗記到該 harness 的 `failures`（`last_error_kind` 為 `network`／`storage`／`other`）；全部成功清 `push_pending`、該 harness 歸零、寫 `last_ok_ms`。`uploaded_through_id` / `health_uploaded_through_id` 從兩個來源的水位抄出。
- `manual=true` 在本單直接回 `failed`/`Other`（G2 實作）。

- [ ] **Step 1: 寫失敗的測試**（`SqliteStore` 經 `store_opener` 注入，`fail_push`/`fail_pull`）
  - `push_pending_persisted_before_replica_write`：opener 被呼叫時讀 state 檔，斷言當下 `push_pending == true`。
  - `push_failure_keeps_push_pending_and_counts_harness`。
  - `success_clears_push_pending_and_only_that_harness`：codex 先失敗 2 次、claude-code 成功 → codex 仍 2、claude-code 0。
  - `instance_change_resets_and_reuploads_nothing_twice`。
  - `lock_file_names_holder`：執行中鎖檔內容含本程序 pid。
  - `lock_in_within_times_out`：測試先持有同一把 `measure.lock` → `lock_in_within(.., 300ms)` 在 1 秒內回 `TimedOut`；放開後立刻拿得到。
  - `busy_store_is_skipped_not_failed`：opener 回 `OpenError::Busy` → `outcome=skipped_busy`、`failures` 不變、`push_pending` 為 true（開 store 前已持久化，下一次 kick 照樣到期）。
- [ ] **Step 2: 跑 `cargo test --locked --test collect_worker`，預期失敗。**
- [ ] **Step 3: 實作。**
- [ ] **Step 4: 跑測試至綠，再跑全套 gate；`measure_report`、`telemetry`、`measure_copy` 既有測試全綠。**

### Task G1b: refused 轉換、背景安靜、鎖序、`cort measure collect` CLI

**Files:**
- Modify: `rust/src/collect.rs`、`rust/src/main.rs`（`MeasureAction::Collect { background: bool }`）
- Test: `rust/tests/collect_worker.rs`（補）

**Interfaces:**
- Consumes: G1a `run_collect`。
- 背景執行每次最多一行 JSON 摘要附加到 `measure.log`；`skipped_busy` 不寫；refused 只在 `refused` 由 false 變 true（或反過來）時寫一行，不計入 `failures`。
- CLI：`cort measure collect [--background]`；harness 讀 `collect::HARNESS_ENV`（C1 的 `DetachedSpawner` 會設），沒有就記成 `unknown`。

- [ ] **Step 1: 寫失敗的測試**
  - `refusal_is_not_failure_and_logs_on_transition_only`：連跑三次都 refused → `measure.log` 只有一行 refused；之後 marker 出現 → 一行恢復。
  - `background_busy_is_silent`：先持有鎖 → `skipped_busy`、`measure.log` 沒有新行。
  - `lock_order_collect_then_measure`：同時跑一個背景 collect 與一個只拿 `measure.lock` 的 `report` 不死鎖（10 秒內兩者結束）。
  - `collect_cli_background_prints_nothing`：`cort measure collect --background`（未設定 measure）stdout 為空、exit 0。
  - `harness_env_reaches_failure_bucket`：`CORT_COLLECT_HARNESS=codex` 且 push 失敗 → `failures["codex"].count == 1`。
- [ ] **Step 2: 跑 `cargo test --locked --test collect_worker`，預期失敗。**
- [ ] **Step 3: 實作。**
- [ ] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task G2: 手動 `record` / `sync` / `sync --dry-run` 改走同一套鎖與狀態

**Files:**
- Modify: `rust/src/collect.rs`（`manual=true` 路徑）、`rust/src/main.rs`（`MeasureAction::Record`、`Sync` ~2883）
- Test: `rust/tests/collect_manual.rs`

**Interfaces:**
- Consumes: G1a／G1b `run_collect`。
- Produces: 錯誤碼 `measure_collect_busy`（訊息：`measure collect is running (pid <pid>, started <UTC>); try again later`）。手動失敗記在 `failures["manual"]`，不動任何 harness 的計數。

- [ ] **Step 1: 寫失敗的測試**
  - `manual_busy_times_out_with_holder`：持有者寫了 pid → 手動在 `manual_wait_ms`（測試設 1 秒）後回 `measure_collect_busy` 且訊息含 pid。
  - `manual_runs_even_when_not_due`：剛跑過、pending 0 → 手動 `record` 照跑。
  - `manual_failure_counts_manual_only`。
  - `dry_run_pass_clears_refused_and_stamp`：`sync --dry-run` 通過 → `refused=false`、`measure-batch.spawned` 被刪。
  - `record_cli_output_unchanged`：既有 `measure record` CLI 測試的輸出欄位不變。
- [ ] **Step 2: 跑 `cargo test --locked --test collect_manual`，預期失敗。**
- [ ] **Step 3: 實作。**
- [ ] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task H: 網路逾時與整次期限

**Files:**
- Modify: `rust/Cargo.toml`（tokio 加 `time` feature，~50）、`Cargo.lock`、`rust/src/measure_store.rs`（`TursoStore` 的 open/push/pull 包 `tokio::time::timeout`）、`rust/src/collect.rs`（整次期限）
- Test: `rust/tests/collect_worker.rs`（補）、`rust/src/measure_store.rs` 單元測試

**Interfaces:**
- Produces: `TursoStore` 的網路操作逾時回 `Err("network timeout after Ns")`；`run_collect` 超過 `run_deadline_ms` 時停在下一個檢查點，回 `failed` + `ErrorKind::Network`，放掉所有鎖。

- [ ] **Step 1: 寫失敗的測試**
  - `turso_timeout_wraps_future`：對一個永不完成的 future 套用同一個包裝函式、逾時 50 ms → 回逾時錯誤（不連網）。
  - `deadline_stops_between_phases`：`run_deadline_ms=1`、opener 先睡 20 ms → `failed`/`Network`，之後能立刻拿到 `measure-collect.lock`。
- [ ] **Step 2: 跑測試，預期失敗。**
- [ ] **Step 3: 實作。**
- [ ] **Step 4: 跑測試至綠，再跑全套 gate。**

### Task I: `measure status`、拿掉 cron、文件

**Files:**
- Modify: `rust/src/main.rs`（`MeasureAction::Status`）、`rust/src/measure.rs`（`report` / `render_report` ~624/~732 在本機執行時第一段印同樣的兩條狀態）、`install.sh`（`deploy_measure_cron` 改成移除）、`rust/src/upgrade.rs`（發布後 best-effort 移除 cort 自己的 cron 條目）、`tests/install-smoke.sh`、`README.md`、`rust/src/main.rs` 的 measure usage／help
- Test: `rust/tests/measure_status.rs`、`tests/install-smoke.sh`

**Interfaces:**
- Consumes: Task B 狀態檔、Task A1 `kick_counters_at`、`cron::{read_crontab, remove_entries, is_measure_entry, write_crontab}`。
- Produces: `cort measure status` 印 JSON：`{"batch":{"pending","last_run_ms","last_ok_ms","push_pending","refused","failures":{...}},"health":{"pending","last_run_ms","unmapped_count","unmapped_recent"}}`，lean 格式一行一條；未設定 measure → 錯誤碼 `measure_not_configured`。

- [ ] **Step 1: 寫失敗的測試**
  - `status_reports_both_lanes`、`status_separates_refused_from_network_failure`、`status_unconfigured_errors`（未設定的判斷走 `measure_store::is_configured()`）。
  - `report_first_section_shows_collection_state`：本機有狀態檔時 `render_report` 第一段含 batch 與 health 兩條；沒有狀態檔時印 `collection: no local state`。
  - install-smoke：安裝後 `CORT_CRONTAB_FILE` 裡沒有 `measure record` 條目；預先放一條 cort 的條目與一條別人的條目 → 安裝後只剩別人的那條、bytes 相同。
  - `upgrade_removes_own_cron_entry`（`CORT_CRONTAB_FILE`）。
  - `help_states_the_retention_boundary`（既有測試）改成也斷言「閒置超過 90 天」的說法存在。
- [ ] **Step 2: 跑測試，預期失敗。**
- [ ] **Step 3: 實作。** README 與 help：kick 是唯一自動觸發；cron 已移除；保留期邊界照設計第 6 節。
- [ ] **Step 4: 跑測試至綠，再跑全套 gate 與 `bash tests/install-smoke.sh`。**

---

## 派工（多 AI 工作流，aib `claude-code` #855／#856）

- 每個 Task 一張單，依 A1 → A2 → B → C1 → C2 → D1 → D2 → E → F → G1a → G1b → G2 → H → I 順序派。依賴：C1 依 B；C2 依 A1、B、C1；D1 無；D2 依 B、D1；E 依 A1、A2、B、C2、D2；F 依 A2；G1a 依 B、C1、F；G1b 依 G1a；G2、H 依 G1b；I 依 G2。D1、D2、F 可在依賴滿足後平行派（看 `free -h`，cargo 不可同時兩支）。
- D1、D2 的單要在派工提示裡附 claudecat 的本機位置（唯讀參考，絕不寫入；路徑只寫在提示裡，不進 repo）。
- 實作：Codex，從該單的 worktree 目錄啟動 `codex-companion task --background --write`，不帶 `--model`，提示不得含 `-C <路徑>`，開頭寫「先讀 agent-portal-tpm 的 `dispatch/rules/agent-rules.md`（本機路徑寫在提示裡），第 2、3 節不適用於本 repo」並附本計畫路徑與單號。
- 審查：Muse 沒額度時由 K3.1 Pro 代審後端（kimi plugin `kimi_review`，`model: kimi-code/k3.1-pro`），不必等 Muse。
- 測試：zglmcode 子代理跑全套 gate，整合者看原始輸出核定。
- 整合者逐行審 diff → 使用者核准 → commit → `measure` 分支推送。
