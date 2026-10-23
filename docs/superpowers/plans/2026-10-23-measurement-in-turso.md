# 量測收進 cort Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** cort 自己記錄「壓縮後的代價」、cort 用量事件、index 健康快照，存進專用的 Turso 量測資料庫，`cort measure report` 印出全部機器的趨勢；claudecat 因此能完全退場。

**Architecture:** 新模組 `transcript`（純函式：Claude Code JSONL → 壓縮列）、`telemetry`（逐字移植 claudecat 的 event_id）、`measure`（記錄、快照、匯入、報告）都只透過 `MeasureStore` trait 讀寫；正式實作是 Turso 同步副本（`TursoStore`，比照 `memory_store.rs` 的開檔與鎖），測試用 rusqlite 的 `SqliteStore`。所有指令掛在 `cort measure <子命令>`，排程每小時第 17 分跑 `cort measure record`。

**Tech Stack:** Rust（rust/ crate）、`turso` 0.9.0-pre.1 sync、rusqlite（只在測試與讀本機 `usage.db`／index）、serde_json（開 `preserve_order`）、cron。

**Spec:** `docs/superpowers/specs/2026-10-23-measurement-in-turso-design.md`（第二版，`97ca4acd`）

## Global Constraints

- 全部 Rust，不加腳本；Bash 只在 `install.sh`、`tests/install-smoke.sh`。
- Turso 同步副本只能用 `turso` crate 開，不能用 rusqlite 或 Turso CLI；測試用的 `SqliteStore` 開的是暫存目錄裡的普通 SQLite 檔，不是副本。
- hook 不碰網路；`hook-suggest`／`hook-refresh` 不改。`usage.db` 的 schema 不改。
- 寫進 Turso 的只有：`command_log` 整列、健康數字、壓縮列的計數與 id。對話文字、指令內容、檔名不上傳。
- `day` 一律 UTC；活躍＝30 天；report 預設 7 天，壓縮一節預設 30 天。
- 量測用自己的資料庫與設定檔，不碰 `harnessuse` 的 `cort memory`；`harnessuse` 舊表的清除要先問使用者。
- 遠端事件永久保留，不刪、不彙總。
- 儲存失敗回傳錯誤，不 panic。
- 每個 commit 前：兩個 crate 都 `cargo fmt --all`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked --all-targets`。本機 9.7Gi：**一次只跑一個 cargo**，先 `pgrep -af 'cargo (build|test|install)'`。
- 使用者 gate 每一個 commit：每個 Task 的 Commit 步驟是「準備好 diff 與訊息，請使用者核准後才 commit」，不 push。
- commit 訊息結尾：`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。

## Review Focus

1. **被寫到一半的記錄檔**：掃描時 Claude Code 正在 append，最後一行不完整——要略過那行、不報錯，下次水位變了再算。→ Task 2 `partial_last_line_is_ignored`。
2. **壞掉或格式不明的行**（非 UTF-8、不是 JSON、未知的 `type`）——略過並計數，整份不能因此失敗。→ Task 2 `malformed_line_is_skipped_and_counted`。
3. **cron 跑 `record` 時量測還沒設定**——非零碼結束、印怎麼設定，不建立任何副本或水位檔。→ Task 4 `record_without_config_fails_cleanly`。
4. **記錄檔被 Claude Code 清掉**——已記的列留著，水位裡那筆移除，不報錯。→ Task 3 `deleted_transcript_keeps_rows`。
5. **同一台同時跑兩個量測動作**（cron 的 `record` 與手動 `report`）——共用一把鎖，後到的等，不會兩個同時寫副本。→ Task 1 `store_lock_serializes_two_holders`。

---

### Task 1: 量測儲存與設定

**Files:**
- Create: `rust/src/measure_store.rs`
- Modify: `rust/src/memory_store.rs`（`digest`、`runtime`、`project_from_git_origin` 與寫私有設定檔、開副本、拿鎖的程式改成 `pub(crate)` 共用，行為不變）
- Modify: `rust/src/lib.rs`、`rust/src/main.rs`（`KNOWN_COMMANDS` 加 `measure`；`cmd_measure` 與 `measure configure`）
- Modify: `rust/Cargo.toml`：`serde_json = { version = "1", features = ["preserve_order"] }`
- Test: `rust/tests/measure_store.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct WindowCounts { pub tool_calls: i64, pub rereads: i64, pub repeated_commands: i64,
      pub transcript_lookups: i64, pub human_messages: i64, pub corrections: i64 }
  pub struct CompactionRow { pub machine_id: String, pub boundary_uuid: String, pub session_id: String,
      pub time_ms: i64, pub project_id: Option<String>, pub trigger: String,
      pub pre_tokens: Option<i64>, pub post_tokens: Option<i64>,
      pub pre: WindowCounts, pub post: WindowCounts, pub post_complete: bool }
  pub struct MachineHost { pub machine_id: String, pub host: String, pub last_seen_ms: i64 }
  pub trait MeasureStore {
      fn upsert_compaction(&mut self, row: &CompactionRow) -> Result<(), String>;
      fn compaction_session(&mut self, machine_id: &str, boundary_uuid: &str) -> Result<Option<String>, String>;
      fn compactions_since(&mut self, since_ms: i64) -> Result<Vec<CompactionRow>, String>;
      fn upsert_machine_host(&mut self, machine_id: &str, host: &str, seen_ms: i64) -> Result<(), String>;
      fn machine_hosts(&mut self) -> Result<Vec<MachineHost>, String>;
      fn push(&mut self) -> Result<(), String>;
      fn pull(&mut self) -> Result<(), String>;
  }
  pub const MEASURE_SCHEMA: &[&str];      // 全部 CREATE TABLE/INDEX，兩種實作都執行
  pub struct SqliteStore;                 // SqliteStore::open(path) ; .fail_push(bool) 供測試
  pub fn configure(url: &str, token: &str) -> Result<PathBuf, String>;   // 寫 data_dir()/turso-measure.json，0600、拒絕 symlink
  pub fn with_turso_store<T>(f: impl FnOnce(&mut dyn MeasureStore) -> Result<T, String>) -> Result<T, String>;
  ```
- 副本：`data_dir()/turso-measure/<digest(url)>/measure.db`，鎖 `measure.lock`（同目錄）。開副本時明確設定 bootstrap-if-empty（查 `~/.cargo/registry` 裡 turso 0.9.0-pre.1 `sync::Builder` 的確切方法名，寫進註解）。
- 表：`compactions`（PK `machine_id, boundary_uuid`，`pre_*`／`post_*` 各六欄）、`machine_hosts`（PK `machine_id, host`）。後續 Task 往 `MEASURE_SCHEMA` 與 trait 加。

- [ ] **Step 1: 寫失敗的測試**：`compaction_upsert_replaces_same_key`（同 key 寫兩次，`compactions_since(0)` 只回一列且是第二次的值）、`compaction_session_reports_first_writer`、`machine_host_upsert_keeps_latest_seen`、`configure_refuses_symlink_and_empty_token`、`config_file_is_private`（mode 0o600）、`store_lock_serializes_two_holders`（兩個執行緒拿 `measure.lock`，斷言臨界區不重疊）、`push_failure_is_returned`（`fail_push(true)` 時 `push()` 回 Err、資料仍在）。
- [ ] **Step 2: 跑 `cargo test --locked --test measure_store`，預期編譯失敗（模組不存在）。**
- [ ] **Step 3: 實作 `measure_store.rs` 與 `memory_store.rs` 的共用抽取、`measure configure --url URL --token-stdin`。**
- [ ] **Step 4: 跑 `cargo test --locked --test measure_store` 與 `--test memory`，全綠（memory 行為不變）。**
- [ ] **Step 5: 全套 fmt／clippy／test，請使用者核准 commit。**

### Task 2: 記錄檔解析與壓縮視窗（純函式）

**Files:**
- Create: `rust/src/transcript.rs`
- Test: `rust/tests/transcript.rs`（測試內用 helper 組 JSONL 字串，不讀真記錄）

**Interfaces:**
- Consumes: Task 1 的 `WindowCounts`。
- Produces:
  ```rust
  pub const WINDOW: usize = 50;
  pub const CORRECTION_TERMS: &[&str] = &["不對", "不是", "錯了", "講錯", "搞錯", "wrong", "that's not", "no,"];
  pub const KNOWN_FILE_TOOLS: &[&str] = &["Read", "Write", "Edit", "MultiEdit", "NotebookEdit"];
  pub struct BoundaryMeta { pub uuid: String, pub session_id: String, pub time_ms: i64, pub cwd: Option<String>,
      pub trigger: String, pub pre_tokens: Option<i64>, pub post_tokens: Option<i64> }
  pub enum Step { Tool { name: String, file_path: Option<String>, command: Option<String>, input: String },
      Human { text: String }, Boundary(BoundaryMeta) }
  pub struct Parsed { pub steps: Vec<Step>, pub skipped_lines: usize }
  pub struct WindowDetails { pub tool_calls: usize, pub rereads: Vec<String>, pub repeated_commands: Vec<String>,
      pub transcript_lookups: Vec<String>, pub human_messages: usize, pub corrections: Vec<String> }
  pub struct Compaction { pub boundary: BoundaryMeta, pub pre: WindowDetails, pub post: WindowDetails, pub post_complete: bool }
  pub fn parse_transcript(jsonl: &str) -> Parsed;
  pub fn compactions(steps: &[Step]) -> Vec<Compaction>;
  pub fn is_human(line: &serde_json::Value) -> bool;
  pub fn is_correction(text: &str) -> bool;      // 英文詞不分大小寫
  impl WindowDetails { pub fn counts(&self) -> WindowCounts }
  ```
- 計數一律由 `WindowDetails` 推出（上傳用 counts，本機明細用 details），規則只有一份。
- 視窗與「已知」的定義照 spec「兩個對稱的視窗」，人類訊息照 spec「使用者親手打的訊息」。`isCompactSummary` 那列不是 Human。

- [ ] **Step 1: 寫失敗的測試**：
  `boundary_row_is_parsed_with_metadata`（trigger、tokens、cwd、session、uuid）、
  `written_file_counts_as_known_for_reread`（Write a.rs → 壓縮 → Read a.rs ⇒ `post.rereads == ["a.rs"]`）、
  `reread_counts_each_path_once`、`first_touch_inside_window_is_not_a_reread`、
  `repeated_command_is_exact_match_only`（`git status` 與 `git status ` 不同）、
  `transcript_lookup_matches_full_input`（`.jsonl` 在第 200 個字元之後也算）、
  `origin_human_counts`、`task_notification_and_peer_are_not_human`、`legacy_line_starting_with_lt_is_not_human`、`compact_summary_is_not_human`、`tool_result_is_not_human`、
  `correction_terms_match`（每個詞一例，`Wrong` 也算）、
  `post_window_stops_at_next_boundary_and_is_complete`、`short_post_window_is_incomplete`、`full_post_window_is_complete`（剛好 50 個）、`pre_window_stops_at_previous_boundary`、`empty_pre_window_when_boundaries_adjacent`、
  `partial_last_line_is_ignored`、`malformed_line_is_skipped_and_counted`。
- [ ] **Step 2: 跑 `cargo test --locked --test transcript`，預期失敗。**
- [ ] **Step 3: 實作 `transcript.rs`。** 逐行 `serde_json::from_str`；tool_use 從 assistant 訊息的 `content[]` 取 `name`、`input.file_path`、`input.command`，`input` 存整段 JSON 字串（給 lookup 比對）。
- [ ] **Step 4: 跑測試，全綠；再對本機真記錄跑一次 debug 輸出（不進 repo），確認 `ad08d1d3…` 那個壓縮點的 `post.rereads` 含設計文件路徑。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**

### Task 3: 掃描記錄檔進量測儲存，本機明細

**Files:**
- Create: `rust/src/measure.rs`
- Modify: `rust/src/main.rs`（`measure show <session-id|boundary-uuid>`）
- Test: `rust/tests/measure_scan.rs`

**Interfaces:**
- Consumes: Task 1 `MeasureStore`、`CompactionRow`；Task 2 `parse_transcript`、`compactions`、`WindowDetails::counts`；`memory_store::project_from_git_origin`。
- Produces:
  ```rust
  pub struct Watermarks;   // load(path)/save(path)；key = 檔案路徑，值 = (size, mtime_ms)；存 data_dir()/measure-transcripts.json（write_atomic）
  pub struct ScanSummary { pub files_seen: usize, pub files_scanned: usize, pub compactions: usize, pub skipped_lines: usize }
  pub fn scan_transcripts(store: &mut dyn MeasureStore, projects_dir: &Path, marks: &mut Watermarks, machine_id: &str) -> Result<ScanSummary, String>;
  pub fn compaction_details(projects_dir: &Path, id: &str) -> Result<serde_json::Value, String>;  // 只讀本機，不碰 store
  pub fn claude_projects_dir() -> PathBuf;   // $CLAUDE_CONFIG_DIR/projects，否則 ~/.claude/projects
  ```
- 只掃 `projects_dir/*/*.jsonl`，不進 `<session>/subagents/`。依 mtime 由舊到新處理；`compaction_session` 已有別的 session 時保留原值（先寫者勝），計數仍以先寫那份為準、不覆蓋。
- `project_id`：壓縮點的 `cwd` → `project_from_git_origin`，失敗為 `None`；同一次掃描內依 `cwd` 快取。

- [ ] **Step 1: 寫失敗的測試**（暫存目錄當 projects_dir，`SqliteStore`）：`subagent_files_are_skipped`、`unchanged_file_is_not_rescanned`、`grown_file_is_rescanned_and_upserted`（後窗由未滿變滿，`post_complete` 翻 true）、`shared_boundary_counted_once_first_writer_wins`、`cwd_without_origin_gives_null_project`、`deleted_transcript_keeps_rows`、`details_print_paths_and_corrections_locally`。
- [ ] **Step 2: 跑 `cargo test --locked --test measure_scan`，預期失敗。**
- [ ] **Step 3: 實作 `measure.rs` 的掃描與 `measure show`。**
- [ ] **Step 4: 跑測試，全綠。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**

### Task 4: `record` 與 `report`（壓縮一節）——第一個可用的版本

**Files:**
- Modify: `rust/src/measure.rs`、`rust/src/main.rs`（`measure record`、`measure report [--days N] [--json]`）
- Test: `rust/tests/measure_report.rs`

**Interfaces:**
- Consumes: Task 1–3。
- Produces:
  ```rust
  pub struct RecordInputs { pub projects_dir: PathBuf, pub marks_path: PathBuf, pub machine_id: String, pub host: String, pub now_ms: i64 }
  pub enum RecordOutcome { Pushed, SavedLocallyPushFailed(String) }
  pub fn record(store: &mut dyn MeasureStore, input: &RecordInputs) -> Result<RecordOutcome, String>;
  pub struct ReportInputs { pub days: i64, pub compaction_days: i64, pub now_ms: i64, pub offline: Option<String> /* 最後同步時間 */ }
  pub fn report(store: &mut dyn MeasureStore, input: &ReportInputs) -> Result<serde_json::Value, String>;
  pub fn render_report(v: &serde_json::Value, color: bool) -> String;
  ```
- `record` 順序：先在本機寫完（host 對照、壓縮列；之後的 Task 再加快照與事件），再 `push`、`pull`。push 失敗：資料留著，CLI 印 `saved locally, push failed: …`，**非零碼**。沒設定：非零碼並說明 `cort measure configure`。
- `report`：先 push 再 pull；失敗就讀本機，第一行「離線：讀本機副本，最後同步於 …」（最後成功同步時間存 `data_dir()/measure-last-sync`）。壓縮一節放最前：每台機器、預設 30 天，壓縮次數、四個計數「後窗減前窗」的中位數與總和、按 UTC ISO 週列趨勢；沒有壓縮印「這段期間沒有壓縮」。host 以 `machine_hosts` 為準，沒有對照的印 `machine <前 8 碼>`；同 host 兩個 machine_id、同 machine_id 兩個 host 各印警告。`NO_COLOR` 或非 TTY 不上色。

- [ ] **Step 1: 寫失敗的測試**：`report_compaction_section_shows_post_minus_pre`（兩次壓縮，斷言 JSON 的中位數與總和）、`report_weekly_trend_uses_utc_iso_weeks`、`report_without_compactions_says_so`、`unmapped_machine_shown_by_prefix`、`same_host_two_machines_warns`、`renamed_machine_warns`、`record_order_writes_before_push`（`fail_push(true)` 後列仍在、回 `SavedLocallyPushFailed`）、`offline_report_first_line`、`record_without_config_fails_cleanly`（CLI 測試：暫存 `CORT_MEMORY_DIR`、非零碼、目錄裡沒有建出副本）。
- [ ] **Step 2: 跑 `cargo test --locked --test measure_report`，預期失敗。**
- [ ] **Step 3: 實作。**
- [ ] **Step 4: 跑測試，全綠。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**
- [ ] **Step 6（實機，使用者在場）：向 `armo` 申請量測資料庫與憑證 → 兩台 `cort measure configure` → 各跑一次 `cort measure record`（回填全部現存記錄）→ `cort measure report`。把兩台的壓縮次數與 spec 的實查數字對一次。**

### Task 5: 排程條目

**Files:**
- Create: `rust/src/cron.rs`
- Modify: `rust/src/main.rs`（`measure cron --install|--uninstall|--status --command-prefix <path>`）、`install.sh`（裝完 binary 後呼叫 `--install`）、`rust/src/upgrade.rs`（升級後呼叫同一個合併）
- Test: `rust/tests/cron.rs`、`tests/install-smoke.sh`（斷言條目存在且只有一條）

**Interfaces:**
- Produces:
  ```rust
  pub fn measure_cron_line(command_prefix: &str, log: &Path) -> String;   // "17 * * * * <prefix> measure record >> <log> 2>&1"
  pub fn is_measure_entry(line: &str) -> bool;                            // token 判定：含 cort 的 binary token 且接 `measure record`；不是後綴判定
  pub fn merge_whole_line(existing: &str, line: &str, is_entry: fn(&str) -> bool) -> (String, bool);  // 逐字移植 claudecat doctor.rs:143（c768644）
  pub fn remove_entries(existing: &str, is_entry: fn(&str) -> bool) -> String;
  ```
- 讀 `crontab -l`、寫一律 `crontab -`（stdin）。`--command-prefix` 必填，不預設 `current_exe()`（同 `hook-install` 的理由）。

- [ ] **Step 1: 寫失敗的測試**：`adds_entry_when_absent`、`replaces_stale_variant`（舊 log 路徑或舊 prefix）、`collapses_duplicates`、`keeps_foreign_lines_byte_identical`（含 claudecat 的 09:17／09:29 兩條與註解行）、`install_is_idempotent`、`uninstall_restores_foreign_lines`、`entry_test_is_token_not_suffix`。
- [ ] **Step 2: 跑 `cargo test --locked --test cron`，預期失敗。**
- [ ] **Step 3: 實作 `cron.rs`、子命令、`install.sh` 一行呼叫、`upgrade.rs` 呼叫。**
- [ ] **Step 4: 跑測試與 `tests/install-smoke.sh`，全綠。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**（實際裝到兩台的 crontab 放在退場步驟，不在這裡。）

### Task 6: 事件同步（逐字移植 event_id）

**Files:**
- Create: `rust/src/telemetry.rs`
- Modify: `rust/src/measure_store.rs`（`telemetry_events`、`telemetry_project_aliases` 表沿用 claudecat DDL 原文，見 `claudecat/src/turso_memory.rs:13-14`；加索引 `(source, time_ms)`；加 `event_watermarks(machine_id, source, max_rowid, row_fingerprint)`）、`rust/src/measure.rs`（`record` 加上傳事件）、`rust/src/main.rs`（`measure sync [--dry-run]`）、`rust/src/usage_schema.sql`（只加一行註解：欄位名稱會進 event_id 雜湊；key 是字母序，不是欄位順序）
- Test: `rust/tests/telemetry.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct TelemetryEvent { pub source: String, pub event_id: String, pub machine_id: String, pub machine_source: String,
      pub project_id: Option<String>, pub time_ms: Option<i64>, pub payload: String, pub rowid: Option<i64> }
  pub fn event_id(source: &str, machine_id: &str, rowid: i64, payload: &str) -> String;
  pub fn cort_events(usage_db: &Path, after_rowid: Option<i64>) -> Result<Vec<TelemetryEvent>, String>;
  pub fn cost_events(dir: &Path) -> Result<Vec<TelemetryEvent>, String>;     // ~/.local/share/cort-evals/claude-cost/*.json，移植 claudecat read_cost
  pub fn project_aliases(usage_db: &Path) -> Result<Vec<(String, String, String)>, String>;  // 移植 claudecat telemetry.rs:257
  pub struct DryRun { pub matched: usize, pub mismatched: Vec<(i64, String)>, pub local_only: usize, pub remote_only: usize }
  // MeasureStore 加：
  fn insert_events(&mut self, events: &[TelemetryEvent]) -> Result<usize, String>;     // INSERT OR IGNORE
  fn remote_ids(&mut self, machine_id: &str, source: &str) -> Result<HashMap<i64, String>, String>;  // rowid → event_id
  fn event_watermark(&mut self, machine_id: &str, source: &str) -> Result<Option<i64>, String>;
  fn set_event_watermark(&mut self, machine_id: &str, source: &str, max_rowid: i64) -> Result<(), String>;
  fn upsert_aliases(&mut self, rows: &[(String, String, String)]) -> Result<(), String>;
  ```
- 算法逐項照 spec「原始事件」一節；`dry_run` 比對單位是 `(source, machine_id, payload 的 id 欄)`，重疊為 0 回 Err（`no overlap: nothing proves the ids match`），不符回 Err 並列前 5 筆。正式同步：有水位只算水位之後；沒水位走全範圍再設水位。
- `measure sync --help` 與 README 寫明保留邊界：本機 `command_log` 只留 90 天，每次 `record` 先寫進本機副本；只有連續 90 天沒跑 `record`，事件才會在上傳前被刪掉。

- [ ] **Step 1: 產生 golden 值。** 在測試裡用 helper 建一份固定的 fixture `usage.db`（v2 列、pre-v2 legacy 列＋`_usage_meta`、NULL、浮點、BLOB、非 ASCII 文字），先寫出到暫存檔；在 `/home/yanggf/a/claudecat`（已封存，只在本機，**不 commit**）加一個臨時測試呼叫 `claudecat::telemetry::scan` 讀同一份檔、印出每列 payload 與 id；跑完把值抄進 cort 的測試，刪掉臨時測試（`git -C /home/yanggf/a/claudecat status` 要回到乾淨）。
- [ ] **Step 2: 寫失敗的測試**：`event_ids_match_claudecat_golden`、`payload_keys_are_sorted_not_in_column_order`、`command_event_ids_hash_rowid_zero`、`legacy_row_takes_machine_from_meta_and_drops_stamp`、`dry_run_refuses_on_mismatch`、`dry_run_refuses_empty_overlap`、`dry_run_counts_local_only_and_remote_only`、`watermark_skips_already_uploaded`、`missing_watermark_full_range_inserts_no_duplicates`。
- [ ] **Step 3: 跑 `cargo test --locked --test telemetry`，預期失敗。**
- [ ] **Step 4: 實作。**
- [ ] **Step 5: 跑測試，全綠。**
- [ ] **Step 6: 全套檢查，請使用者核准 commit。**

### Task 7: 從 `harnessuse` 複製舊事件

**Files:**
- Modify: `rust/src/measure.rs`、`rust/src/measure_store.rs`（`TursoStore` 可開指定設定與檔名的第二個副本）、`rust/src/main.rs`（`measure copy-events`）
- Test: `rust/tests/measure_copy.rs`

**Interfaces:**
- Consumes: Task 6 的表與 `insert_events`、`upsert_aliases`。
- Produces: `pub fn copy_events(from: &mut dyn MeasureStore, to: &mut dyn MeasureStore) -> Result<CopyReport, String>;`（`groups` 每組 `(source, machine_id, 來源列數, 目的列數)`，加 `conflicts` 與前 5 筆 `conflict_examples`：同 key 但欄位不同的列，不覆寫；被擋下又查不到的列回 Err）；`MeasureStore` 加 `fn events_page(&mut self, after: Option<(String, String)>, limit: usize) -> Result<Vec<TelemetryEvent>, String>;`、`fn aliases(&mut self)`、`fn event_counts(&mut self)`、`fn events_in_key_range(&mut self, first, last)`。兩份設定指向同一個資料庫（比對主機）時拒絕執行，什麼都不開。（2026-10-24 依 Codex 三輪審查更新）
- 來源設定讀 `cort memory` 的設定（`harnessuse`），副本檔另開 `legacy-telemetry.db`，不動 `memory.db`。只讀來源，不刪任何東西。

- [ ] **Step 1: 寫失敗的測試**：`copy_is_idempotent`、`copy_reports_counts_by_source_and_machine`、`copy_keeps_event_ids_byte_identical`、`copy_never_writes_to_source`。
- [ ] **Step 2–4: 跑、實作、跑到綠。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**

### Task 8: index 健康快照

**Files:**
- Modify: `rust/src/measure.rs`、`rust/src/measure_store.rs`（`index_snapshots` 表，PK `day, machine_id, root`）
- Test: `rust/tests/measure_snapshot.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum SnapshotStatus { Ok, Unreadable, MissingDir, NoIndex }
  pub struct SnapshotRow { pub day: String, pub machine_id: String, pub root: String, pub project_id: Option<String>, pub host: String,
      pub first_recorded_ms: i64, pub recorded_ms: i64, pub status: SnapshotStatus, pub error: Option<String>,
      pub fresh: Option<String>, pub chunks: Option<i64>, pub relationships: Option<i64>, pub not_chunked: Option<i64>,
      pub real_gap: Option<i64>, pub fts: Option<String> /* "synced" | "drift=N" | NULL=讀不到 */,
      pub source: String /* "cort" | "cort-audit-md" */ }
  pub fn snapshot_rows(machine_id: &str, host: &str, day: &str, now_ms: i64, prior_roots: &[String]) -> Vec<SnapshotRow>;
  pub fn fresh_label(repair: Option<&str>, stale: bool, graph_pending: Option<bool>) -> &'static str;
  // MeasureStore 加：upsert_snapshot（保留既有 first_recorded_ms）、snapshots_since(day)、roots_seen_since(machine_id, day)
  ```
- `fresh_label` 照 `claudecat/src/cort_audit.rs:562` `row_md` 的對應，但 `fresh` 改用 cort 自己的 `index_is_stale`，沒有 7 天門檻；repair 用 cort 內部算（等同 `claudecat/src/cort.rs:812` `repair_from_status_json` 對 cort status 的判定），不呼叫外部 binary。
- 專案清單＝`db::list_projects()` 聯集 `roots_seen_since(machine, 今天-30)`；`Unreadable` 記 `status=unreadable`＋`error`。真缺口＝`file_state.chunk_count > 0` 卻不在 chunks；`-1` 記 NULL。
- `record` 每次 upsert 今天的列。

- [ ] **Step 1: 寫失敗的測試**：`fresh_label_covers_every_label`（七個標籤各一例）、`no_age_gate`（一個月前建、HEAD 沒動 ⇒ `fresh`）、`deleted_index_becomes_no_index_row`、`missing_dir_row_recorded`、`unreadable_list_entry_recorded_with_error`、`same_day_upsert_keeps_first_recorded_ms`、`null_metric_means_unreadable_not_zero`。
- [ ] **Step 2–4: 跑、實作、跑到綠。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**

### Task 9: report 的健康表、用量表與異常

**Files:**
- Modify: `rust/src/measure.rs`
- Test: `rust/tests/measure_report.rs`（加測試）

**Interfaces:**
- Consumes: Task 4 `report`、Task 6 事件、Task 8 快照、Task 10 的匯入表（Task 10 先做也可；沒有表時當作空）。
- 健康表一個專案一張（列＝機器×日），用量表一張（列＝機器×日：命令數/30d、命令數/7d、`impact`/30d、decline-top）；視窗＝截至 UTC 日 D 結束；decline-top 用 `declines` 計數（`hook/decline` 為 key、排除 `not_a_search_tool`，口徑見 `claudecat/src/cort.rs:906`、`claudecat/src/cort_audit.rs:619`）。
- 異常：(1) `STALE`／`STALE/rebuild`／`STALE/graph`／`fresh?`／`no-index` 當天標；`STALE/refresh` 連續 ≥2 天才標；(2) FTS 不是 `synced`；(3) 真缺口 >0 或 NULL；(4) 活躍 host 那天一列快照都沒有（今天不檢查）。`missing_dir` 不標。
- 活躍＝30 天內有快照或事件的 host；不活躍的每次在摘要列「不活躍，最後資料於 YYYY-MM-DD」。同一天有事件就用事件算的用量；只有匯入值時標「匯入」。同 host 同日同 root，真 machine_id 的列勝過 `imported:<host>`。

- [ ] **Step 1: 寫失敗的測試**：`stale_refresh_one_day_not_flagged`、`stale_refresh_two_days_flagged`、`each_immediate_stale_label_flagged`、`fts_drift_flagged`、`real_gap_null_flagged`、`missing_snapshot_flagged_per_host_not_today`、`inactive_host_listed_not_flagged`、`usage_window_ends_at_utc_day_end`、`decline_top_excludes_baseline`、`events_override_imported_usage`、`real_machine_row_beats_imported`、`usage_table_is_per_machine_not_per_project`。
- [ ] **Step 2–4: 跑、實作、跑到綠。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**

### Task 10: 匯入 `CORT-AUDIT.md`

**Files:**
- Modify: `rust/src/measure.rs`、`rust/src/measure_store.rs`（`imported_usage` 表，PK `day, host`）、`rust/src/main.rs`（`measure import-audit <path>`）
- Test: `rust/tests/measure_import.rs`（fixture：從真檔挑 4 列，含 09-06 那列）

**Interfaces:**
- Produces: `pub fn parse_audit_md(md: &str) -> Result<Vec<(SnapshotRow, ImportedUsage)>, String>;`、`pub struct ImportedUsage { pub day: String, pub host: String, pub commands_window: Option<i64>, pub core_window: Option<i64>, pub deep_window: Option<i64>, pub commands_7d: Option<i64>, pub deep_7d: Option<i64>, pub decline_top: Option<String>, pub window_days: i64 }`
- 健康欄進 `index_snapshots`（`machine_id = imported:<host>`、`source = cort-audit-md`），用量欄進 `imported_usage`；09-06 依它自己的欄數解析，host 記 `unknown`，不參與活躍與缺列。`project_id` 用 claudecat repo origin 的正規化雜湊。可重跑。

- [ ] **Step 1: 寫失敗的測試**：`parses_full_rows`、`parses_short_0906_row_as_unknown_host`、`unknown_host_excluded_from_activity`、`reimport_is_idempotent`、`row_count_matches_table`（真檔 53 列在實機驗證步驟比對，不進測試）。
- [ ] **Step 2–4: 跑、實作、跑到綠。**
- [ ] **Step 5: 全套檢查，請使用者核准 commit。**

### Task 11: 文件

**Files:**
- Modify: `AGENTS.md`（「claudecat (separate repo)」一節與 `claudecat telemetry sync` 那句改寫成 cort 自己的量測；寫明這是 claudecat **退場**取代 2026-09-23 的「不要併進來」規則）、`README.md`（指令清單加 `measure`）、`docs/memory.md`（量測與記憶分用兩個資料庫）
- [ ] **Step 1: 改文件；`rust/tests/skill_format.rs` 與 `tests/install-smoke.sh` 照跑。**
- [ ] **Step 2: 請使用者核准 commit。**

### 退場（不是程式碼，使用者在場逐步做；照 spec「claudecat 退場步驟」）

- [ ] 移除 claudecat 的 09:17、09:29 條目（crontab 先備份到 scratchpad）。
- [ ] 兩台 `claudecat telemetry configure` 指向**量測資料庫**，各跑最後一次 `claudecat telemetry sync`。
- [ ] `cort measure copy-events`，比對各組列數。
- [ ] 兩台 `cort measure sync --dry-run`：重疊非空、100% 相符；不符就停，修 Task 6。
- [ ] NUC11i5 `cort measure import-audit /home/yanggf/a/claudecat/CORT-AUDIT.md`，對 53 列（加上封存後新增的列）。
- [ ] 兩台 `cort measure cron --install --command-prefix <cort shim>`。
- [ ] 隔天：遠端兩台都有昨天的快照、事件持續增加、無重複；壓縮列有新增。
- [ ] 移除 `~/.cargo/bin/claudecat`（兩台）。
- [ ] **問使用者後**才清 `harnessuse` 的 `telemetry_events` 與 alias。
