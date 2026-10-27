# 集中式 Turso：拿掉本機副本，直接讀寫遠端

狀態：設計，2026-10-27，經 Muse 兩輪設計審查後修訂（第 11 節）。尚未實作。

## 1. 起因

2026-10-27 下午，Turso 帳號（`personal`，starter 方案，未開超額）的 embedded syncs 用到
3.03 GB／3 GB。Turso 因此封鎖整個帳號所有資料庫的 SQL 讀取，到 2026-11-01 08:00（台灣）才解除。
- 被封鎖的包括使用者其他 app 的庫：`travel-2026`、`ai-board` 都已實測確認。
- 同一時間，其他額度用得很少：reads 5.4M／500M、writes 1.17M／10M、storage 244 MB／5 GB。

embedded syncs 是 turso sync replica 的傳輸量，也就是 push、pull，以及開副本時的 bootstrap。
`cort-measure` 一個庫就佔 2.8 GB，庫本身只有 178 MB；`harnessuse` 佔 247 MB。其他 8 個庫都是
SQLite 型，沒有用副本。

副本是 10/23 量測搬進 Turso 時我們自己選的（`turso::sync::Builder::new_remote` 加
`bootstrap_if_empty`），使用者從來沒有要過。使用者的原話：「我本就不要副本，要用集中的」、
「真的沒要 sync 呀」。

`70bf926e` 先把背景收集改成只推不拉，作為止血；本設計取代那個止血。使用者決定先付一個月的錢解除封鎖
（2026-10-27），之後再評估要不要把 cort 搬到 Cloudflare D1，好讓 cort 不再跟其他 app 共用同一個
帳號的額度。

## 2. 目標與界線

- 兩個庫（`cort-measure`、`harnessuse`）只存在遠端。每台機器以 HTTP 直接讀寫，本機不留任何資料庫
  副本，也沒有快取或待寫佇列（outbox），embedded syncs 用量歸零。
- 產品裡不再出現「sync」這個詞：
  - 記憶檔上傳改名為 `cort memory notes upload`（C0，第 9 節）；
  - `cort memory sync` 改名為 `cort memory check`；
  - `cort measure sync` 移除，它的 `--dry-run` 首次上傳把關改由 `cort measure verify` 承接
    （第 4.4 節）。
- 週期性的工作（hook、背景、collect）一律零讀取；只有在「重建本機進度」時，才各自允許一次讀取
  （第 4.1、5.2 節）。每月用量估算見第 7 節。
- 傳輸層只有一個接口，之後要換成 D1 時，只需要換掉這一層。
- 資料表結構只新增索引（第 7 節），不改欄位。對外的指令與 JSON 只動副本、sync 相關的部分（第 6 節）。

## 3. 傳輸：沿用 gwebcdb 的 Hrana HTTP 模式

gwebcdb 的 `crates/bridge-core/src/turso.rs` 已經用 Rust 直連 Turso，`price-registry` 等庫一直靠它
運作：
- 對 `https://<db>/v2/pipeline` POST JSON，帶 `Authorization: Bearer <token>`；
- 傳輸用 `ureq`，錯誤只帶 HTTP 狀態碼；
- 只有一個 IO 接口 `TursoTransport::post`。

它證明的只有：編碼正確、單一語句的請求可行。它沒有用過多語句的交易。

2026-10-27 已實測：
- `turso://` 型的兩個庫都回應 `/v2/pipeline`，回的是 `BLOCKED`，不是協定錯誤；
- 讀取被封鎖時，寫入照常：`INSERT … ON CONFLICT DO UPDATE` 回 `rows_written: 1`、`rows_read: 0`。

本 repo 不 path 相依 gwebcdb（CI 拉不到），照它的形狀寫 `rust/src/remote.rs`。

### 3.1 `remote.rs`

- 相依：`ureq = { version = "2", default-features = false, features = ["tls"] }`。base64 沿用
  lockfile 既有的 0.22。`Cargo.lock` 跟著同一個 commit 更新，因為 CI 用 `--locked`。
- 設定：`turso://`、`libsql://`、`https://` 都接受，前兩個換成 `https://`。gwebcdb 的
  `resolve_turso_config` 不接受 `turso://`，照抄的話，已安裝的設定全部會失效。
- `Cell`：沿用既有的 Null／Int／Text。Hrana 的整數以字串傳送，解碼時 `"2164"` 要轉成 `Int(2164)`；
  遇到不認得的型別就回錯誤。
- 每個結果項目都要檢查，錯誤分類成封閉的詞彙：
  `blocked`、`unauthorized`、`unreachable`、`timeout`、`busy`、`constraint`、`sql`。
  - 分類依 Hrana 的錯誤碼與 HTTP 狀態碼；gwebcdb 的 `entry_error` 會丟掉錯誤碼，不能照抄。
  - 伺服器回的錯誤訊息只寫進 log，不進入 JSON 輸出或 hook 注入的文字。
- 每個語句的結果都帶 `affected_row_count`，呼叫端負責檢查（第 5.1、5.2 節）。
- 逾時：ureq 的總逾時。要測一次「接受連線後不回應」的假伺服器，確認逾時涵蓋整個請求。

### 3.2 交易：Hrana `batch` 加步驟條件

在同一個 pipeline 裡依序放 `BEGIN`…`COMMIT`，**不構成**交易：每個 `execute` 各自回結果，前一個出錯
不會阻止 `COMMIT` 執行。

改用 `batch` 請求，形狀照 libsql 的 `Batch::transactional`
（`libsql-hrana-0.2.0/src/proto.rs:295`）。步驟從 0 起算，n 個語句：

| 步 | 語句 | 條件 |
|---|---|---|
| 0 | `BEGIN TRANSACTION` | 無 |
| 1…n | 第 i 個語句 | `ok(i-1)` |
| n+1 | `COMMIT` | `ok(n)`（鏈狀傳遞，等於前面全部成功） |
| n+2 | `ROLLBACK` | `not(ok(n+1))` |

這裡用的是 libsql 的 `BEGIN`，不是 `BEGIN IMMEDIATE`，免得多一個待驗證的變數。

錯誤以「第一個失敗的步驟」為準，依步驟順序找。`BEGIN` 遇 busy 時，後面的步驟全被跳過，`ROLLBACK`
在沒有交易的狀態下執行會回錯誤，這個錯誤不能蓋掉 `BEGIN` 的 `busy`。

`busy` 最多重試 3 次（100／300／900 ms），之後回 `busy`。

**驗收探測**（C1 的門檻，只用寫入，封鎖期間也能跑；`#[ignore]`、需要環境變數）：

0. 前置：`DROP TABLE IF EXISTS cort_spike_tx`，再建 `cort_spike_tx(k INTEGER PRIMARY KEY)`。
1. 一個 batch：`BEGIN`、`INSERT k=1`、`INSERT k=1`（必定撞主鍵）、`COMMIT`、`ROLLBACK`。斷言逐步結果：
   - 步 1 成功；
   - 步 2 的錯誤分類是 `constraint`；
   - 步 3 被跳過；
   - 步 4 執行了。

   batch 若整批被拒（例如編碼錯誤），探測判為失敗，不能當成「已回滾」。
2. 再送一個普通的 `INSERT k=1`：
   - 成功 → 已回滾，通過；
   - 錯誤分類是 `constraint` → 被部分提交，失敗；
   - 其他錯誤（例如 `blocked`）→ 探測無效，重跑。
3. 同一個 pipeline POST 裡放：`execute BEGIN`、`execute INSERT k=2`、`close`，不送 `COMMIT`。然後用
   步驟 2 的方式確認 k=2 不存在。
4. 測被忽略的 `INSERT OR IGNORE` 會不會計入 `rows_written`。第 7 節的估算取決於這個結果。
5. 最後一定 `DROP TABLE`，失敗路徑也要；殘留的空表無害，下次的步驟 0 會清掉。

每個 POST 都會關閉 stream，所以跨連線的 busy 排除不驗，交給上面的重試兜底。

**探測通過前**，`transaction()` 只能用在冪等的語句組（`OR IGNORE`／`OR REPLACE`／`MAX` upsert）。

另外，請求逾時但伺服器其實已經提交時，呼叫端會重送。所以每個交易的語句組都必須可以重跑；目前所有的
呼叫點都滿足這一點。

`SqliteTransport` 用 rusqlite 的 in-memory 庫，照上面的規則執行 pipeline 與 batch。它驗證我們的 SQL
與編解碼；伺服器的交易語意只有探測能證明。

## 4. 量測（`cort-measure`）

- `TursoStore` 換成 `RemoteStore`。
- `RemoteStore` 有兩種建法：
  - 量測庫：可以建索引；
  - 舊 harnessuse 的 `telemetry_events`（`copy-events` 的來源）：唯讀、不跑任何 DDL，並保留「來源與
    目的是同一個庫就拒絕」的檢查。
- 同一台機器的寫入者由 `measure.lock` 序列化，鎖持有到 store 的整個生命週期。
- 不支援複製出來的 machine id（整機複製會連 `/etc/machine-id` 一起複製）：歸屬會混在一起。以下的保護
  只防同一台機器上的競態。

### 4.1 收集只寫不讀

- 本機的 `measure-progress.json`（0600，原子寫入）是**唯一的權威**：
  - 每個 `(machine_id, source)` 的水位，也就是 max_rowid 加上那一列的 row fingerprint；
  - 已上傳 compaction 的 `boundary_uuid → session_id`。
- `measure-collect-state.json` 的 `uploaded_through_id` 鏡像欄位移除，`status` 改從進度檔推算。
- 主機對應每次重算並以 `MAX` upsert，不需要進度。
- 遠端的 `event_watermarks` 表不再寫入，留著不刪。
- 去重交給遠端：event id 是內容的決定性雜湊（`telemetry.rs:78`），所以用 `INSERT OR IGNORE`。
  - full-range 讀遠端去重那一步拿掉，改成全部重送；
  - compaction 用 `INSERT OR REPLACE`；health、cost 用 `INSERT OR IGNORE`。
- 每個 chunk 一個寫入 batch。順序是先寫遠端、成功後才更新本機進度檔；中間當掉的話，下次從舊水位
  重送，重複的由 `OR IGNORE` 吸收。

**進度檔遺失或損壞時重建**（唯一允許讀遠端的時機）：
- 水位：
  - 讀遠端 `event_watermarks` 裡這台機器的列，**用本機 `usage.db` 那個 rowid 的 fingerprint 核對**；
  - 不符、該列已不存在（例如被 prune 掉）、或讀不到，就歸零重送。
  - 遠端水位停在搬遷當天，所以這個種子只在搬遷初期有用，之後重建幾乎等於歸零重送。結果正確，只是
    多耗 writes。
- compaction 對應：
  - 讀遠端 `compactions` 中這台機器的列（主鍵前綴 `machine_id`），補回 `boundary_uuid → session_id`；
  - 讀不到時，compaction 的上傳暫停，**不推進** `measure-transcripts.json`，以保住「先到的 session
    擁有這個 boundary」的保證；
  - 等下一次讀得到時再補。
- `status` 標示 `progress: rebuilt`。

### 4.2 失敗、遺失與停滯

- `RecordOutcome` 簡化成：
  - `Uploaded`；
  - `UploadFailed(kind)`。

  `SavedLocallyPushFailed`、`classify_record` 的 push 分支、`measure_push_failed` 一併換掉。
- `push_pending` 改名為 `upload_pending`。新增 `pending_since_ms`：pending 第一次變成 true 時寫入，
  完全成功時清除；從來沒成功過的機器也有值。
- `upload_stalled_days` 由 `pending_since_ms` 算出。
- pending 超過 7 天時，每次 collect 在 `measure.log` 寫一行 `warning: upload stalled <n> days`。
- 會遺失的情況要照實寫：
  - `usage.db` 的 prune（`usage.rs:1158`）改成**不刪水位以上、還沒上傳的列**，硬上限 180 天，以免
    無限成長。超過 180 天還沒上傳的才會遺失，而且 prune 會在 log 寫一行記下遺失了多少列。
  - transcript：Claude Code 自己會刪舊的 transcript。上傳失敗時不推進 marks，但若 transcript 在重試前
    就被刪了，那筆 compaction 就沒了。
  - cost JSON、`CORT-AUDIT.md`：只要還在，就能重送或重跑。

### 4.3 人執行的讀取指令

- `report`、`status` 的遠端摘要、`copy-events`、`import-audit`、`verify`：讀取被擋時以 `blocked`
  結束、exit code 不是 0，不印假的 0。
- `report`、`import-audit` 開頭的 `measure::sync`（pull）拿掉。
- `upsert_imported_usage`：**只有**錯誤分類是 `constraint` 時，才重讀那一列、分類成 `unchanged` 或
  `conflict`；其他錯誤照原樣回報。
- `copy-events` 的 `from.pull()`／`to.push()` 變成單純的讀與寫，JSON 的 `source_pushed` 移除。
- `measure status` 用一次遠端查詢 `sqlite_master` 回報缺少的索引（`missing_indexes`）。這是人執行的
  讀取。本機的 schema 標記只決定要不要跑 DDL，不保證遠端真的有索引。

### 4.4 首次上傳的把關

現行機制：沒有水位的機器，必須先由人跑 `measure sync --dry-run` 留下 marker（`marker_valid`，
`telemetry.rs:1055`），collect 才會上傳。它防的是被帶到另一台機器的 `usage.db` 用錯的 id 直接上傳。
這個把關保留：
- dry-run 改名為 `cort measure verify`：人執行，讀遠端比對 id，寫入同一格式的 marker；
- 既有機器的 marker 繼續有效，所以兩台現有機器不受影響；
- 沒有 marker 的新機器，collect 照舊拒絕上傳，直到有人跑過 `verify`；封鎖期間 `verify` 讀不到遠端，
  新機器就等到解封。

## 5. 記憶（`harnessuse`）

### 5.1 ledger（人執行）

- `record`、`recall`、`status`、`resolve`、`import-legacy`、`check` 全部改透過 `RemoteDb`。
- `record`、`resolve` 的保證：
  - 「active」是在 Rust 裡從關係圖算出來的，SQL 條件表達不了。兩台機器同時記同一個 key，結果是兩筆
    active，也就是既有的 conflict 狀態。
  - 保證的是**不覆蓋**（`(project, id)` 加 `INSERT OR IGNORE`），以及**一定偵測得到**：寫入後必定重讀，
    發現超過一筆 active 就回報 conflict。這次重讀是必要步驟。
  - `--supersedes <id>` 寫成 `INSERT … SELECT … WHERE EXISTS (…id…)`。`affected_row_count == 0` 時回
    `supersedes_missing`，exit code 不是 0；不能把「影響 0 列」當成成功。
  - `resolve` 寫入之後才發現有新的 active 時，那筆 resolution 已經提交，就留著並回報 conflict。
- 三種結果：
  - **寫入失敗**：`not recorded: <kind>; nothing was saved locally; re-run the same command when the
    database is reachable`，exit code 不是 0，不印 entry。
  - **寫入成功、但寫後重讀失敗**（例如讀取被封鎖）：`committed_unverified: <kind>`，exit code 不是 0，
    印出寫入的 id，請人在可讀時跑 `memory check` 確認。這時不能說「沒記錄」。
  - 成功，或 conflict。
- 不增加本機待寫佇列。安全網是 `record` 原有的規則：來源行必須已經 commit。
- `memory check`：
  - 讀得到時，印 entry 數與 conflict；
  - 讀不到時，只印 `remote: <kind>`，不印任何數字。

### 5.2 記憶檔上傳（`notes upload`）

- 把這台機器 Claude Code 的記憶檔（`projects/*/memory/*.md`）上傳到 `harness_notes`，每筆標註機器。
  這是單向上傳，沒有下載。
- 本機的 `notes-uploaded.json`（0600，原子寫入）是權威。每筆存**完整的鍵與雜湊**：
  `(project, harness, source_dir, file_name) → sha256`。note_id 是這些欄位的單向雜湊，反推不回來，
  所以檔案刪除後，要靠這些欄位產生 tombstone。
- 背景上傳零讀取：
  - 比對本機檔與記憶目錄，算出 upsert 與 tombstone；
  - 分 chunk 送出（每 chunk 最多 50 筆或 256 KB），每個 chunk 成功後才更新本機檔；
  - tombstone 放在最後一個 chunk。
- tombstone 要能重跑：
  - 改成**保留** `sha256`，只設 `deleted=1` 並清空內容：
    `UPDATE … SET deleted=1, body='' WHERE <鍵> AND sha256=?`。
  - 現行寫法會把 `sha256` 清成空字串（`notes_sync.rs:351`）。那樣的話，第一次 tombstone 成功、本機檔
    還沒更新就當掉，重送時會影響 0 列。保留之後，重送仍然影響 1 列。
  - 影響 0 列，代表遠端是別的內容或沒有這一列。這時記 `tombstone_skipped`、照樣從本機檔移除，並寫
    log。
- upsert 同樣檢查影響列數：0 列就回報錯誤。
- 本機檔遺失或損壞時重建：
  - 全部 upsert 重送，遠端冪等；
  - 以前上傳過、現在已刪除的 note 需要 tombstone，這要讀一次遠端（這台機器自己的分區，索引
    `(machine, harness, project)`）；
  - 讀不到時，把 `tombstones_unknown: true` 存進本機檔、顯示在 `notes status`，之後每次上傳都再試一次
    那個讀取，直到成功。已刪的記憶內容不會無聲地一直留在遠端。
- tombstone 只來自完整讀到結尾的目錄。
- 觸發方式照舊：編輯記憶檔時由 hook 啟動，加上每 30 分鐘一次，都在背景跑，hook 不等它。

### 5.3 SessionStart（`hook-recall`）上網讀

推翻 AGENTS.md「SessionStart 只讀已同步的本機副本」那條。新規則：**hook 可以讀網路，絕不寫網路**
（不跑 DDL，不寫入），本機鎖只用 try-lock。

- 期限是預算分配，從進程啟動算起（現行 watchdog：`main.rs:2749`、`:2765`、`:2829`）：
  - dispatch 的截止時間是啟動後 2500 ms；
  - 網路逾時是 `min(CORT_RECALL_TIMEOUT_MS, 截止時間 − 已經用掉的時間 − 100 ms)`；
    `CORT_RECALL_TIMEOUT_MS` 預設 1500，上限夾在 2400。
  - stdin 解析與 `project()` 的 `git remote` 都算在這份預算裡。剩餘預算不到 200 ms 就不發請求，直接
    回 `remote_timeout`。
  - watchdog 3 秒時，若主流程還沒 claim，進程直接結束，不寫 usage 列（現狀）。已經 claim 的話，另有
    2 秒寬限，寬限內照樣寫 usage 列。
- 一次 pipeline 讀兩樣東西：
  - 這個專案的 ledger：要整個專案讀，因為算 active 需要完整的關係圖，所以讀取量隨 entry 數成長；
  - 其他機器的 notes 名稱。
- 讀不到時：只注入一行「記憶暫時讀不到（<原因>）」，原因只用固定詞彙；照樣寫 usage 列。
- `cmd_hook_recall` 必須是全函數：沒有 `?` 往外傳，每個錯誤都變成一個 outcome。
- outcome 詞彙比照 suggest 的既有做法，拆成兩份：
  - `RECALL_OUTCOMES`（現行）：新增 `remote_unreachable`、`remote_blocked`、`remote_timeout`；
  - `LEGACY_RECALL_OUTCOMES`（只給 census 用）：放 `no_replica`、`unsynced_replica`。
  - `usage.rs` 的 census 把兩份都當成已知。
- 不留本機快取。代價：斷線期間，每次 SessionStart 最多多等約 1.5 秒。
- 上限（20 筆、6000 bytes；notes 15 筆、2000 bytes）照舊。

### 5.4 鎖

- `memory.lock`、`measure.lock`、`legacy-telemetry.lock` 移到 `${memory data dir}/locks/`。
- 過渡期：只要舊副本目錄還在，新版**同時拿新舊兩把鎖**。這樣部署當下，還在跑的舊版背景進程不會和
  新版交錯寫入同一個分區（例如把 `machine_hosts` 蓋回較舊的值）。
- 第 8.4 節刪除副本目錄之後，自然只剩新鎖。

## 6. 對外可見的改變

- 指令：
  - `memory notes sync` → `memory notes upload`（C0）；
  - `memory sync` → `memory check`；
  - `measure sync` 移除，`--dry-run` 改為 `measure verify`。
- 環境變數與檔名（C0）：
  - `CORT_NOTES_SYNC_INTERVAL_SECS` → `CORT_NOTES_UPLOAD_INTERVAL_SECS`；
  - `CORT_NOTES_SYNC_DEADLINE_SECS` → `CORT_NOTES_UPLOAD_DEADLINE_SECS`；
  - `memory-sync.log` → `memory-upload.log`；
  - `notes-sync.pending` → `notes-upload.pending`；
  - 模組 `notes_sync.rs` → `notes_upload.rs`。

  兩台機器都沒有設這些環境變數，所以直接改名，不留舊名。
- 欄位：
  - `memory status`：`sync_status` 改成 `remote: ok|<kind>`；
  - `measure status`：`push_pending` 改成 `upload_pending`，另加 `pending_since_ms`、
    `upload_stalled_days`、`progress`、`missing_indexes`；
  - `copy-events`：移除 `source_pushed`。
- `memory recall` 的 header 改成「shared database」。
- `--help`、README、`docs/memory.md`、AGENTS.md 裡的 replica、sync、push／pull 敘述都改掉。
- `install.sh`／`cort_upgrade` 不碰副本目錄，所以裝新版不會清副本。

## 7. 每月用量與索引

### 7.1 用量估算

以三台機器估算（免費方案：reads 500M、writes 10M）：

- **collect**：每台每 20 分鐘一次，零 reads；寫入就是新事件的列數。
  - 例外是進度重建與 full-range 重送，一次最多約 20 萬列的寫入嘗試。
  - **C2b 第一次在每台機器上跑時，進度檔不存在**。解封前讀不到遠端水位，所以會歸零重送，約 20 萬列／台。
    這是搬遷成本，不是故障。
  - 被忽略的列算不算 writes，看第 3.2 節探測的第 4 步：不算，就可以忽略；算，就收緊 full-range 的
    觸發條件，並把 C2b 的首次部署排到解封之後（解封後能種回水位）。
- **SessionStart**：讀整個專案的 ledger（目前不到 20 列），加上 notes 名稱（有索引時約 15 列）。
  一個月不到 10 萬列。
- **記憶檔上傳**：零 reads，只有重建時讀一次自己的分區。
- **schema DDL**：只在 `configure` 時跑，以及本機沒有 `schema-v<N>.ok` 標記時跑一次（每個設定 digest
  各一份）。缺少的索引由 `measure status` 回報（第 4.3 節）。

### 7.2 索引（只加索引，不改欄位）

Turso 的 rows read 以「掃描的列」計，不是回傳的列。

- `telemetry_events (source, machine_id, time_ms)`：`SELECT_REMOTE`、`SELECT_RANGE`／
  `SELECT_NO_TIME`，以及 `COUNT_EVENTS` 的前綴。
- `harness_notes (machine, harness, project)`：重建時讀自己的分區。
- `harness_notes (note_id)`：`notes show`，非唯一。
- `harness_notes (project, deleted, updated_at)`：SessionStart 的 `ORDER BY updated_at DESC LIMIT 15`。

一次性成本：在約 20 萬列上建索引，於解封後執行 `configure` 時發生。

閘門規則：**成長中的表不得有無上限的掃描**。小的維度表（hosts、aliases、imported_usage）的全表讀取
可以接受。這條寫成測試：用 rusqlite 對 schema 跑 `EXPLAIN QUERY PLAN`，斷言上述查詢用到索引。

## 8. 搬遷與清理

### 8.1 封鎖期間

讀取失敗，寫入可以：
- 量測收集與記憶檔上傳可以運作；
- 記憶的 recall、record、SessionStart 要等解封。

使用者已決定付費解除封鎖，所以這段時間可能會提早結束。

### 8.2 副本檔的格式

2026-10-27 實測：一般 SQLite 打不開副本檔。
- 檔頭寫 `SQLite format 3`，但第 18、19 位元組是 `255 255`，是 turso 自己的格式版本；
- 內容實際放在 `-log`，主檔停在 10-23。

所以不做 rusqlite 對帳：只開主檔、忽略 `-log`／`-changes` 的讀取端，會少報沒推上去的列，還把刪除當成
安全的。

### 8.3 刪除副本會失去什麼

- `project_memory`：寫進本機但 push 失敗的列。這是唯一需要搶救的。
- `harness_notes`：可以從 `.md` 重新上傳。例外是檔案已刪掉的 note 的 tombstone，由第 5.2 節的重建
  讀取補上。
- 量測：可以重新推導，但 `measure-transcripts.json` 已經跳過那些 compaction 只寫進副本的 transcript。
  所以要一起刪掉它，讓新版全部重掃一次（`OR REPLACE`，冪等）。
- `legacy-telemetry.db` 與各個 `-log`：內容都來自遠端，刪掉不會失去任何東西。

### 8.4 步驟（兩台都跑 C4 版本、且已解封之後；每台機器、每個 digest 目錄都要做）

1. 部署 C2b 之前，把一份 `70bf926e` 的 binary 留在 `~/.local/share/cortexyoung/replica-flush/`。
2. 停掉背景工作：沒有 collect 或上傳進程在跑，鎖都是空的。
3. 用舊 binary 跑 `cort memory sync`，必須回 `sync_status: synced`（代表這次 push 成功）。
4. `-changes` 檔的大小**不能**當成依據。2026-10-27 17:26 實測：NUC11i5 的 push 成功、`push_pending`
   變成 false 之後，`measure.db-changes` 仍是 135,563 bytes，沒有被截斷。依據改成第 3 步的結果，加上
   第 5 步。
5. 用新版 `cort memory check` 從遠端那一側確認 entry 數，與舊 binary `cort memory status` 的本機
   entry 數相同。
6. 刪除 `turso-memory/`、`turso-measure/`、`measure-transcripts.json`。
7. 刪除 `replica-flush/`。**刪掉副本之後，絕不再執行舊 binary**：它會 bootstrap 一份新副本，重演這次
   的事故。

### 8.5 拿掉的與留下的

- **拿掉**：
  - `turso`、`turso_sync_engine` 相依；`tokio` 若沒有其他用途也拿掉；
  - `network_timeout.rs` 為副本設的部分（replace-base、poisoned store）。要等 C2、C3 都上線才能拿，
    commit 訊息要逐項列出。
- **留下**：`collect.rs` 的進程 watchdog（transcript 掃描是本機 CPU 工作），以及 hook-recall 的
  watchdog。

## 9. 分包（每包都能單獨編譯、單獨測試）

- **C0 改名**（行為不變）：
  - `notes sync` → `notes upload`，包括：
    - spawner（`notes_kick.rs:53`）；
    - 背景模式的字串辨識（`main.rs:2783`）；
    - `run_background`；
    - 第 6 節列的環境變數、log 檔名、pending 檔名、模組名。
  - 測試與文件一起改。
  - 部署：等舊的背景上傳進程結束才裝。因為舊進程結束時，會 spawn 已經不存在的舊指令名。
- **C1** `remote.rs`：
  - Hrana 編解碼、`batch` 步驟條件、錯誤分類、busy 重試、`SqliteTransport`、單元測試；
  - 以 public 模組加入，不接任何呼叫點；
  - 驗收包括第 3.2 節的探測，我在 C1 審完後親自跑。
- **C2a** 量測 store：
  - `RemoteStore` 的兩種建法、鎖移到 `locks/` 加上雙鎖過渡、索引與 schema 標記；
  - trait 的 `push`／`pull` **暫時保留為 no-op**，讓 crate 能編譯；
  - `SqliteStore` 的 `fail_write`；
  - turso-local 的 SQL 測試（`measure_store.rs:1565` 那批）退役。
- **C2b** 量測呼叫點：
  - `measure-progress.json` 與重建；
  - 只寫不讀的 collect、`RecordOutcome`／`upload_pending`／`pending_since_ms`、prune 規則；
  - 人執行指令改讀遠端、`measure verify`、移除 `measure sync`；
  - 移除 trait 的 `push`／`pull`；
  - 測試。
- **C3a** 記憶（人執行）與記憶檔上傳：
  - ledger 的各指令、`check`、三種結果；
  - `notes upload` 的零讀取、完整鍵、可重跑的 tombstone、chunk、重建；
  - `recall_local` 暫時保留舊實作，讓 hook 能編譯；
  - 改寫 `rust/tests/notes_sync.rs`（C0 後改名為 `notes_upload.rs`）。
- **C3b** SessionStart：
  - 上網讀、預算分配、outcome 兩份詞彙與 census、AGENTS.md 改寫；
  - 移除 `recall_local`；
  - 改寫 `rust/tests/hook_recall.rs`、`memory_deadline.rs`。
- **C4** 清理：拿掉 turso／tokio 相依、`network_timeout.rs` 的副本部分；改 README、`docs/memory.md`、
  `--help`。

每包的流程：Codex 實作 → 我逐行審 → Muse 審（後端）→ gate（fmt、clippy、完整測試，含 `EXPLAIN` 測試）
→ 提交推送 → CI 綠燈。

部署時機：
- C0 先部署；
- C2b 完成後部署一次（量測改直寫遠端）；
- C4 完成後再部署一次；
- 每次部署前，等舊的背景進程結束；
- 第 8.4 節在 C4 部署完成、而且已解封之後才做。

## 10. CI

- CI 沒有 Turso 憑證，所有測試都走 `SqliteTransport`。
- 第 3.2 節的探測是 `#[ignore]`、需要環境變數的測試，由人手動跑，結果記在本文件。

## 11. 審查紀錄

**第一輪**（Muse，2026-10-27）：
- 交易改用 batch，所有語句組必須可以重跑；
- record／resolve 照實寫保證；
- import-audit 的並發處理；
- 遺失邊界；
- 記憶寫入失敗的文字；
- SessionStart 的期限、全函數、詞彙；
- 索引與閘門測試；
- 搬遷清單；
- 鎖移出副本目錄；
- 分包、ureq 的 `tls`、接受 `turso://`、tokio 測試。

**第二輪**（Muse，同日）：
- batch 的步驟表逐字照 libsql，改用 `BEGIN`；探測斷言逐步結果與錯誤分類（§3.2）；
- `supersedes` 影響 0 列時失敗；`committed_unverified`（§5.1）；
- 只有 `constraint` 才重讀分類（§4.3）；
- `pending_since_ms`、prune 不刪未上傳的列、log 警告（§4.2）；
- `memory check` 讀不到時不印數字（§5.1）；
- 期限改成預算分配；兩份 outcome 詞彙（§5.3）；
- 進度檔是唯一權威；重建時驗 fingerprint；compaction 對應的重建（§4.1）；
- notes 存完整鍵、可重跑的 tombstone、影響列數、chunk、`tombstones_unknown`（§5.2）；
- 不支援複製出來的 machine id（§4）；
- 首次上傳把關改由 `measure verify` 承接（§4.4）；
- `missing_indexes`（§4.3）；
- 雙鎖過渡（§5.4）；
- C2a 保留 no-op、C3a 保留 `recall_local`；C0 改名要涵蓋 spawner 與背景辨識（§9）；
- 第 8.4 節的時序。
