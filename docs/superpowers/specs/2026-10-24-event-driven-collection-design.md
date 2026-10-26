# 量測改成事件觸發：hook 開排、兩條非同步收集

日期：2026-10-24。分支 `measure`（HEAD `f99ef6c8`）。這份設計**取代**
`2026-10-23-measurement-in-turso-design.md` 的「每天的快照：index 健康」與「記錄：每天的排程」兩節，
以及計畫 `docs/superpowers/plans/2026-10-23-measurement-in-turso.md` 的 Task 8；其餘章節（量測資料庫、
原始事件、event_id、dry-run、copy-events、report 的壓縮一節）不變。第 2 版：併入 Codex 設計審查
（8 點）與 builder check（4 題）；第 3 版：併入 Codex 第二輪（8 點）；第 4 版：併入 Codex 第三輪（3 點）；第 5 版：併入 Codex 第四輪（`usage.db` 被換掉）；第 6 版：併入 Codex 第五輪（approve with changes：倒退偵測、換庫後健康重複觀察）；第 7 版：併入 GLM 設計審查（代 Muse；concern 1 中 2 低＋2 附註）；第 8 版：併入 K3.1 Pro 補審（代 Muse；block，7 點：健康水位驗證、`measure.lock` 等待上限、只用本機歷史、單一 root 逾時改執行緒、戳記檔名、`STALE/graph` 看原因、計畫裡的絕對路徑）。

## 為什麼改

舊設計以**日曆**為單位：cron 每小時跑 `cort measure record`，每台機器、每個 index、每個 UTC 日記一列
快照，report 再判斷「活躍機器那天沒有快照＝沒有資料」。問題是沒有任何常駐程式保證那一列會寫出來——
cron 要等機器開著、WSL 起來、cron daemon 在跑，三台都是 WSL2（2026-10-24 查過：`systemd=true`、
cron `enabled`/`active`，但 Windows 開機後 WSL 要等有人開終端才會起來）。結果是設計的一大塊都在處理
「排程沒跑」：缺列判斷、30 天 root 清單、`no_index`/`missing_dir` 補列。

使用者 2026-10-24 的判斷：「這設計不優」「要走 event base」。資料本來就只在有人用的時候產生——
transcript、`command_log`、index 變動都發生在 agent 工作時——觸發也應該在那時。

## 使用者決定的部分（2026-10-24，原話摘要）

- **觸發由 cort 的 hook 開排**：hook 不做收集，只決定要不要啟動收集，收集非同步進行。
- **批次**：未收的資料累積到合理程度，或累積較多時，整批收。
- **逐筆**：需要一筆一筆收的，走另一條非同步。
- **cort 自己收**：批次收集是 cort 接手原本 claudecat 的角色；claudecat 不啟用，真的需要再啟。
- **cron 拿掉**：「cron 沒必要，WSL 的 cron 沒太多用」。

## 目標、非目標、反目標

- 目標：有人用 cort 的機器，資料在使用當下或之後幾分鐘內被收進本機並上傳；沒人用的時候不需要、
  也不產生任何資料。
- 目標：hook 的成本不因量測而明顯增加——kick 是常數時間、純本機、不等鎖；額外成本要量測並寫進
  實作的測試或報告（目標：中位數 < 2 ms）。
- 非目標：讓遠端分得出「機器關著」與「機器開著但收集壞了」。遠端只看得到資料有沒有來；分辨要在
  那台機器本機看（第 7 節）。
- 非目標：記錄沒人用的 index 的健康。沒被開過的 index 不會變，記了也沒資訊。
- **反目標**：hook 變慢、hook 輸出任何東西、hook 失敗、hook 做網路工作、hook 等鎖——任何一項都違反
  `AGENTS.md` 對兩個 hook 的契約（安靜、exit 0、忙就放棄、不在 hook 裡做網路、`hook-refresh` 永不
  建立 index）。
- **反目標**：每次 hook 觸發都生一個程序（hook-refresh 一個月約 6.6 萬次）。
- **反目標**：收集自己餵自己——收集程序自己的 `command_log` 列讓下一次收集到期，沒人用也停不下來。
- **反目標**：上傳被拒（dry-run 還沒過）時，每次都重試並寫 log。

## 設計

### 1. 觸發：`kick`

`hook-suggest` 與 `hook-refresh` 在主工作完成後、回傳之前各呼叫一次 `measure::kick(harness)`
（`harness` 是 hook 已經判定好的 harness 名稱）。步驟：

1. measure 沒設定（設定檔不存在）→ 返回。一次 `stat`。
2. 升級進行中（`try_protected_entry` 拿不到）→ 返回。`hook-refresh` 已經判斷過；`hook-suggest` 也要判斷。
3. 以**唯讀**、busy timeout 25 ms（跟 `usage.rs` 既有的讀取一樣）開 `usage.db`，取三個數：
   `SELECT MAX(id) FROM command_log WHERE command <> 'measure'`、
   `SELECT MAX(id) FROM command_log`（**不分** `command`；rowid 的 `MAX` 是 O(1)）與
   `SELECT MAX(id) FROM index_health_log`。第一個數決定「有多少待收」（pending），第三個數同理
   管健康那條；第二個數只用在第 4 步的倒退偵測——它回答的是「游標是否超過資料庫現有的任何一列」，
   跟待收量無關。第一個查詢靠新加的部分索引 `CREATE INDEX IF NOT EXISTS idx_command_log_not_measure
   ON command_log(id) WHERE command <> 'measure'`（只是索引、不是欄位，不影響 event_id；由
   `usage.db` 既有的開檔建表流程建立），O(log n)；實作要量測。忙、失敗、表或索引還
   不存在 → 返回。
4. 讀兩條收集各自的狀態檔（第 2、3 節）；讀不到當「從沒跑過」。
   **`usage.db` 被換掉的偵測**：`usage.db` 開檔建表時，若 `_usage_meta` 沒有 `instance_id` 就寫入一個
   隨機值（`INSERT OR IGNORE`；這是 meta 表的一列，不是 `command_log` 的欄位，不影響 event_id）。
   兩條收集的狀態檔都記下自己上次看到的 `instance_id`。kick 在第 3 步一併以主鍵讀出 `instance_id`；
   跟某條狀態檔記的不同（包括狀態檔還沒有這個值）→ 那條的游標（`uploaded_through_id`、
   `health_uploaded_through_id`、`health_cursor`）一律當 0，視為到期（仍受節流戳記限制）。收集程序
   看到 `instance_id` 不同時，把自己的游標歸零、記下新的 `instance_id` 再做事；`cort_command` 的去重
   仍由 Task 6 既有的水位指紋與全範圍去重負責。既有的 `usage.db` 第一次補上 `instance_id` 時，最多多
   跑一次收集。
   **倒退的偵測**（還原同一份舊的 `usage.db`，`instance_id` 相同但 rowid 倒退）：`cort_command`
   的游標（`uploaded_through_id`）與健康那條的 `command_log` 游標（`health_cursor`）大於**全部**
   `command_log` 的 `MAX(id)`（不分 `command`）才算倒退；`health_uploaded_through_id` 比
   `MAX(index_health_log.id)`。倒退的那個游標當 0、視為到期，收集程序同樣歸零再做。這裡必須用
   不分 `command` 的 `MAX`，因為游標是「已寫進副本的最大本機 rowid」，**包含已上傳的 `measure`
   列**——`measure` 列不計 pending，但它們確實佔用 rowid，所以「最後幾列是 `measure` 自己的」這種
   最常見的狀態下，那些游標正常地大於 `MAX_non_measure_id`（`uploaded_through_id` 每批都會這樣，
   因為每批都把自己的 `measure` 列一起上傳）；拿 `MAX_non_measure_id` 判斷會把它
   誤判成倒退，游標歸 0、整個歷史又算成 pending，收集就自己養活自己。hook 寫的每個新列都會在同一個
   hook 裡觸發 kick，所以只要新列都來自 hook，真倒退在資料庫長回舊游標之前就會被發現。**hook 以外的
   寫入**（手動下的 `cort impact`、`measure record`/`sync` 等）會寫列卻不 kick；它們若在下一次 hook
   之前把 `MAX(id)` 推回舊游標之上，倒退就不再被偵測。後果有界：批次那條的上傳以副本裡的水位為準，
   Task 6 的水位驗證（`watermark_row_changed`／`watermark_row_missing` → 全範圍重讀、`INSERT OR
   IGNORE` 去重）照樣把還原後的檔案整份重掃，只是晚一點；健康那條的 `health_cursor` 純本機、沒有
   水位可退，還原帶（還原後的 `MAX(id)` 到舊游標之間）裡用過的專案那一天不會有健康觀察。
   **健康觀察在換庫或倒退後不會在遠端重複**：新的 `index_health_log` 沒有上一列可比，同一天同一個
   root 可能在本機再記一次，但第 4 節的健康 event_id 是內容鍵（UTC 日＋root＋不含 `ts` 的指標），同日
   同狀態的重觀察算出同一個 id，被 `INSERT OR IGNORE` 擋掉。
5. 依到期規則判斷兩條各自是否到期。對到期的那條：
   - 先**試**它的單一執行鎖（`flock`，不等待），被佔用 → 返回（有人在收）；能拿到 → **立刻放掉**。
     鎖的擁有者是子程序，不是 kick（照 `heal.rs` `defer_to_background` 的作法：父程序的試探只是
     減少白生程序，子程序自己拿鎖，拿不到就安靜退出）。
   - **節流戳記**：先拿一把不等待的短鎖 `data_dir()/measure-<lane>.kick.lock`（拿不到 → 返回，
     別的 hook 正在做這件事），在鎖內讀 `measure-<lane>.spawned`（上次生程序的 ms），距今 <
     該條的 `min_interval` → 放鎖返回；否則寫入現在的 ms，**寫成功才生程序**，生完放鎖。寫不了
     （例如沙箱不讓寫 `data_dir()`）就不生。檢查與更新在同一把鎖內，所以任何環境下每條收集每個
     `min_interval` 最多生一個程序，不可寫的環境一個都不生。這是 kick 唯一的寫入。
     戳記先寫、生程序在後，所以生程序失敗（fork 資源不足、`current_exe` 讀不到）也算用掉這一次：
     下一次嘗試要等該條的 `min_interval`，期間 hook 記 `throttled`。（時鐘撥回、`now < 戳記` 時不算節流，會立刻重寫戳記再試——與到期規則對未來時間的處理一致，避免撥回的時鐘讓這條停擺。）刻意不回滾戳記——一直失敗的
     環境若每個 hook 都重試 fork，代價落在 hook 上（2026-10-26 Muse 補審 C2 後確認）。
   - 用 `spawn_background_healer` 同一套方式生子程序：`setsid`、stdin/stdout/stderr 全部 null、
     不等待、不保留 handle。子程序結果自己寫 `measure.log`。
6. 任何錯誤都吞掉，不輸出、不改 hook 的 exit code 與輸出。kick 的結果記進 hook 自己的
   `args_summary` JSON（例如 `"collect":"spawned_batch"`、`"throttled"`、`"busy"`）——這個值會進該列
   自己的 event_id（整列都會），但列寫入後不再改、本來就還沒上傳，所以不影響已上傳的列，也不改
   `command_log` 的欄位佈局。

kick 不開量測副本、不起 tokio、不碰網路。

### 2. 批次收集：`cort measure collect`

做現在 `measure record` 做的事（Task 3 的 transcript 掃描、Task 6 的 `cort_command` 事件上傳、
push/pull），在 `measure.lock` 底下跑，外加第 4 節的 `cort_index_health` 上傳。

- 單一執行鎖 `measure-collect.lock`（拿不到就安靜退出，不寫 log）。
- **時間上限**：每個網路操作（開副本的 bootstrap、push、pull）包在 120 秒逾時裡；整次執行上限 10
  分鐘（都可覆寫）。逾時記成 `network` 失敗、正常結束程序、放掉所有鎖——cron 拿掉之後沒有別的機制
  會替卡住的程序收尾，所以不能依賴 turso client 自己的逾時。**`measure.lock` 的等待也有上限**（背景與手
  動都是 60 秒，可覆寫）：現有的 `memory_store::lock_in` 是無期限阻塞的 `flock`，持有 `measure-collect.lock`
  的背景程序若卡在它上面，之後每一次收集都會因第一把鎖忙碌而被跳過。背景等不到記成 `skipped_busy`
  （不算失敗、不加退避）；手動等不到以錯誤碼 `measure_collect_busy` 結束。

  整次期限只在階段之間檢查，網路逾時是包在 future 外的 `tokio::time::timeout`，打斷不了不讓出
  執行權的 CPU 迴圈：2026-10-26 一次 turso sync pull（套用遠端新底庫）在單一階段吃滿一顆 CPU 約
  31 分鐘，期間一直占著 `measure-collect.lock`。所以背景批次收集另有一條獨立執行緒的硬性監看：
  從開啟 store 前的狀態儲存成功之後啟動，一直監看到整支程序結束（正常結束時它隨程序消失）；
  更早的本機鎖與狀態步驟不在監看範圍。從執行起算的期限加 60 秒仍未結束，就盡力記一筆 `network`
  失敗（`error` 以 `watchdog:` 開頭）、保留 `push_pending`。啟動後無論正常路徑的最後本機寫入或補救寫入是否卡住，
  程序都會在期限加 60 秒，再加一段 60 秒寬限內結束；這些寫入卡住時可能記不下失敗。鎖由核心放掉，
  副本與 watermark 由下一次收集接續。只要 store 已開啟，開啟前已成功將 `push_pending = true`
  寫入狀態。watchdog 失敗與其他網路失敗一樣，會增加該 harness 的失敗次數，下一次 kick 的退避
  時間因此加倍。正好在期限邊界完成的批次，包括原本會是 `skipped_busy` 的執行，也可能被記為
  網路失敗並觸發退避；資料不會遺失，下次收集會重新驗證 watermark。與正常結束以一次性旗標
  互斥，最後的紀錄最多只由先搶到的一方寫（第二階段結束時可能雙方都沒寫完）。
- **鎖的順序（所有路徑一致）**：要同時持有多把鎖的路徑，一律依序拿 `measure-collect.lock` →
  `measure.lock` →（`copy-events` 才有的）`legacy-telemetry.lock`，倒序放。背景收集、手動
  `record`、`sync`、`sync --dry-run` 都照這個順序；只拿 `measure.lock` 的路徑（`report`、
  `copy-events` 的前兩把以外）不受影響。kick 的 `*.kick.lock` 只在 kick 內短暫持有，從不與上述鎖
  同時持有。
- 狀態檔 `data_dir()/measure-collect-state.json`，**只有持有 `measure-collect.lock` 的程序寫**
  （背景收集，或拿到這把鎖的手動 `record`/`sync`/`sync --dry-run`）：`last_run_ms`、`last_ok_ms`、
  `failures`（**每個 harness 一筆**：`{harness: {count, last_error_kind, last_failed_ms}}`，
  `last_error_kind` 是 `network`／`storage`／`other`；手動執行記成 `manual`）、`push_pending`、
  `uploaded_through_id`（`cort_command` 已寫進副本的最大本機 rowid）、`health_uploaded_through_id`
  （`cort_index_health` 同上）、`refused`。用 `data_dir::write_atomic` 寫。
- **寫入順序（當機安全）**：要把列寫進副本之前，先把 `push_pending = true` 寫進狀態檔並落地；然後
  寫副本、推進水位與 `*_uploaded_through_id`；push 成功之後才把 `push_pending` 清成 false。任一步
  之間當機，下次一定看得到「欠 push」，最壞是多 push 一次（push 本身是冪等的）。
- **成敗依結構化結果判斷，不看 exit code**：
  - push、pull、副本讀寫、網路失敗 → 失敗：這次觸發它的 harness 那一筆 `count += 1`，記
    `last_error_kind`；`push_pending` 維持 true（見寫入順序）。
  - `cort_command` 被 dry-run 閘門拒絕**不是失敗**，是持續性的狀態：`refused = true`，這個來源不計入
    pending；其他工作（transcript 掃描、`cort_index_health` 上傳、push）照常做、照常算成功。
    `measure.log` 只在 `refused` 由 false 變 true（或反過來）時寫一行，不是每次都寫。
  - **`refused` 的恢復**：`cort measure sync --dry-run` 通過時（它拿同一把鎖）把 `refused` 清成
    false，並把節流戳記 `measure-batch.spawned` 刪掉，讓下一個 kick 立刻到期；手動 `sync` 本身
    也會直接上傳。
  - 全部成功 → 這次觸發它的 harness 那一筆歸零、`push_pending = false`、`last_ok_ms = now`。別的
    harness 的失敗紀錄不因此歸零。
- 一般情況寫一行摘要到 `measure.log`（每次執行最多一行）。

**到期規則**：

- `pending = command_pending + health_pending`，**兩項各自先取下限 0**：
  `command_pending = refused ? 0 : max(0, MAX_non_measure_id − uploaded_through_id)`、
  `health_pending = max(0, MAX(index_health_log.id) − health_uploaded_through_id)`。**這個下限不是
  防禦**：`uploaded_through_id` 包含已上傳的 `measure` 列，合法地可以大於 `MAX_non_measure_id`
  （最後幾列是 `measure` 自己的，就是最常見的狀態），不取下限的話負數會吃掉健康那項。健康觀察的列算進待收，所以健康那條
  寫的列一定會被某次批次送出去（另見第 3 節結尾的交接）。
- `min_interval = 5 分鐘 × 2^count`，`count` 是**這次 kick 的 harness** 自己的連續失敗次數，上限 60
  分鐘。沙箱裡一直失敗的 harness 最多每 60 分鐘試一次網路，別的 harness 不受它影響。
- 距離上次執行 < `min_interval` → 不到期。否則以下任一成立就到期：
  - `push_pending`（副本還欠 push，即使 `pending = 0`）；
  - `pending ≥ 2000`（累積較多）；
  - `pending ≥ 1` 且距離上次執行 ≥ 15 分鐘（累積到合理程度）；
  - 有新的 transcript 變動也算進「合理程度」那條：transcript 掃描本來只重讀有變動的檔案，跟著
    任一次到期的批次一起跑即可，不單獨判斷。
- 數字可用環境變數覆寫（比照 `CORT_HEAL_MAX_FILES`），預設值寫在程式裡並有測試。
- **手動執行**的 `cort measure record` 與 `cort measure sync` 不看到期規則、不看退避，一律照做；
  它們拿同一把 `measure-collect.lock`，**最多等 60 秒**（可覆寫）；等不到就以非零結束、錯誤碼
  `measure_collect_busy`，訊息帶上鎖檔裡持有者寫下的 pid 與開始時間（持有者拿到鎖時把這兩個值寫進
  鎖檔；只是說明用，不拿來判斷鎖）。結束時一樣更新狀態檔。

### 3. 逐筆收集：index 健康觀察：`cort measure health`

**要觀察的對象是「cort 實際開過的 index」**：一個 hook 或指令真的開了某個專案的 index，它寫進
`command_log` 的那一列就帶著該專案的 `project_id`（`rust/src/db.rs` 的 realpath 雜湊，由
`open_project_unmigrated` 等設定）。hook 在開 index 之前就返回的那些觸發（`no_index`、形狀不符、
payload 讀不到）不算使用，也不需要觀察。`project_id` 是 NULL 或 `_unknown` 的列略過。

- 單一執行鎖 `measure-health.lock`；狀態檔 `data_dir()/measure-health-state.json`，**只有這條寫**：
  `health_cursor`（已處理到的 `command_log` id）、`last_run_ms`、`unmapped`（對不回 root 而略過的
  project_id 累計數與最近 5 個）。
- 讀 `id > health_cursor` 的列，取不重複的 `project_id`，對回 root：
  1. `db::list_projects()` 現有的 index；
  2. 對不到的，用 `index_health_log` 裡這台機器記過的 `(local_project_id → root)`——index 被刪掉的
     專案靠這個對回 root，記一列 `status=no_index`（目錄也不在就 `missing_dir`）；
  2b. `list_projects()` 回 `ProjectEntry::Unreadable` 的，從它的 `db_path` 取 project_id（檔名就是
     `<project_id>.db`，`rust/src/db.rs:111` `db_path_for`），**記一列 `status=unreadable` 加
     `reason`**，不算進 `unmapped`；root 用第 2 步的對照，對不到就 NULL（report 以
     `project:<前 8 碼>` 顯示）。讀不到可能是暫時忙碌也可能是真的壞了（`db.rs:479`），兩者都要看得見；
  3. 兩邊都對不到（index 在第一次觀察之前就被刪掉）→ 略過，計入 `unmapped`。**觀察範圍因此明確
     縮小為「被觀察到之前沒被刪掉的 index」**：開了又在 10 分鐘內刪掉的 index 不記健康；略過的數量
     在第 7 節的 `measure status` 看得到，不是默默漏掉。
- 對每個 root **一個一個**觀察。**只讀的邊界**：
  - 專案的 index 資料庫一律以 SQLite 唯讀模式開（`SQLITE_OPEN_READ_ONLY`），不跑 `ensure_schema`、
    不跑 migration、不走 `heal.rs`、不跑 `incremental_index`，絕不建立 index。
  - staleness 用 cort 自己的判定（`index_is_stale` 與 repair，**沒有 7 天門檻**）。它會走訪檔案、
    重抽候選檔；候選檔數超過 `CORT_HEAL_MAX_FILES` 同一個上限時，**不算** staleness：`fresh` 記 NULL、
    `error` 記 `staleness_skipped_large_tree`。每個 root 的觀察有時間上限（預設 30 秒，可覆寫）：
    觀察在一條獨立執行緒上跑，主迴圈只等這麼久；超過就記 `status=unreadable`、`error=timeout` 換下一個，
    被放棄的執行緒只讀、隨 worker 程序結束而結束。階段之間檢查做不到這件事——`compute_stale` 的走訪、
    git 與重抽都在單一呼叫裡，沒有可以停下來的點。
  - 樹在觀察途中變動造成的讀取錯誤記 `status=unreadable` 與 `error`，不重試、不略過。
- 指標與舊設計相同：`fresh` 標籤（`fresh`／`STALE`／`STALE/refresh`／`STALE/rebuild`／
  `STALE/graph`／`fresh?`；`no-index` 由 `status` 表達）、chunks、relationships、未 chunk 檔數、
  真缺口、FTS（`synced`／`drift=N`）。NULL 一律代表讀不到，原因在 `status`／`error`；0 就是 0。
  `graph_pending` 一定會讓 rebuild 原因含 `graph_incomplete`（`rust/src/indexer.rs` `rebuild_reasons_of`），
  而任何 rebuild 原因都分類成 `rebuild_required`，所以標籤看的是**原因**：原因只有 `graph_incomplete`
  → `STALE/graph`；還有其他原因 → `STALE/rebuild`。
- **寫入條件**：跟這台機器對這個 root 上一次觀察的指標與狀態完全相同（帶到別台的 `usage.db` 裡別台的列不算），而且上一次是同一個 UTC 日 → 不寫。否則
  寫一列。有用到的那天至少一列，狀態變化時多一列；沒用到的那天沒有列。
- 存在本機 `usage.db` 的**新表** `index_health_log`：`id INTEGER PRIMARY KEY`、`ts`、`machine_id`、
  `machine_source`、`root`、`local_project_id`（realpath 雜湊，給第 2 步對回 root 用）、`project_id`
  （正規化 origin 雜湊，與壓縮列同一個函式；沒有 origin 為 NULL）、`status`、`error`、上述指標。
  **不動 `command_log` 的欄位**：它的欄位名稱會進 claudecat 相容的 event_id，改了會讓還沒跑過的
  dry-run 對不上。
- 本機寫入跟專案 index 的讀取分開：`usage.db` 是這條唯一寫的資料庫。
- 處理完才把 `health_cursor` 推進到本批讀過的最大 id。
- **交接給批次**：這次寫了至少一列，結束前呼叫一次 `kick`（harness 記成 `measure-health`）。
  健康的列已算進批次的 pending，所以這個 kick 會依批次的到期規則生出批次收集；即使批次正在跑、
  漏掉了這幾列，它們也還在 pending 裡，下一次到期就會送。

**到期規則**：`MAX_non_measure_id > health_cursor` 且距離上次健康收集 ≥ 10 分鐘（可覆寫）。這條純本機、
不碰網路；失敗（例如 `usage.db` 寫不進去）只是下次再試，不需要退避以外的保護——節流戳記已經限制了
生程序的頻率。

### 4. 健康觀察的上傳：來源 `cort_index_health`

由批次收集負責，跟 `cort_command` **分開的路徑**：

- 不受 dry-run 閘門限制（這個來源沒有 claudecat 的歷史）。`cort_command` 被拒時它照樣上傳。
- event_id 是**內容鍵**：`sha256("cort_index_health\0" + machine_id + "\0" + UTC 日 + "\0" + root +
  "\0" + payload_without_ts)`，`payload_without_ts` 是該列去掉 `id`、`ts` 後的緊湊 JSON（key 字母序，
  與 `cort_command` 同一個序列化器）；上傳的 payload 本身仍含 `ts`。同一天同一狀態的觀察（包括換庫
  或倒退後的重觀察）算出同一個 id，`INSERT OR IGNORE` 擋掉，遠端不會重複。這個來源沒有 claudecat
  歷史，可以自由選算法。
- 水位沿用 Task 6 的 `event_watermarks`（`source = 'cort_index_health'`），指紋用該列原始的
  `ts|root`。水位在列寫進**副本**後才前進；push 是否成功由第 2 節的 `push_pending` 負責。
  **水位要先驗證**，跟 `cort_command` 同一套（`rust/src/telemetry.rs` `upload_events`）：水位那一列在
  本機的指紋不同（`watermark_row_changed`）或不在了（`watermark_row_missing`）→ 這次全範圍讀。水位存在
  副本裡、不會隨本機 state 歸零，不驗證的話換庫後 `id` 不大於舊水位的新觀察會永遠被跳過；全範圍讀
  的重複由內容鍵 id 吸收。
- 本機保留：`index_health_log` 跟 `command_log` 用同一套 90 天清理。邊界跟事件一樣：只有連續 90 天
  都沒上傳成功才會在上傳前被刪掉。
- report 的健康表從 `telemetry_events` 裡 `source='cort_index_health'` 的 payload 讀（同一天取最後
  一筆）。`index_snapshots` 表不建。

### 5. 量測自己的 `command_log` 列

`measure collect`、`measure health`、手動的 `measure record`/`sync` 照舊由 `main()` 寫自己的
`command_log` 列（`command='measure'`），所以「收集何時跑過」可以追。這些列：

- **不計入** kick 的 pending 與兩條的到期判斷（第 1 節第 3 步的 `WHERE command <> 'measure'`）；
  但它們仍是 `command_log` 的列、照樣佔 rowid，所以第 3 步另外讀的不分 `command` 的 `MAX(id)`
  **包含**它們——那是倒退偵測用的天花板，正是為了讓「已上傳的 measure 列」不會被誤判成倒退（第 1 節
  第 4 步）。這兩個 `MAX` 不能互相取代；
- 照常被上傳（它們是使用紀錄的一部分），但只有在真有使用觸發收集時才會產生，不會自己養活自己
  （包含不會讓游標看起來倒退、因而每 `min_interval` 再生一次收集）；
- 沒有 `project_id`，不會成為健康觀察的對象。

### 6. cron：拿掉

`kick` 是**唯一**的自動觸發。最後一次 hook 之後還沒到期的那一小段資料（不到一個 idle 間隔的量，預設 15 分鐘）先存在
本機，下次使用時第一個到期的 kick 就會送，只是晚到。**邊界**：`command_log` 在寫入時清掉 90 天前的
列（`rust/src/usage.rs` 既有行為，`index_health_log` 照同一套），清理發生在新指令寫入的當下、
早於非同步收集——所以機器**閒置超過 90 天**時，閒置前那一小段未到期的資料會在下次使用時先被清掉。
這個損失有上限（不到一個 idle 間隔的使用量；預設 15 分鐘，`CORT_COLLECT_IDLE_SECS` 可改），不為它改清理規則；README 與 `--help` 的保留期說明照這句
補上。

- `install.sh` 不再安裝 `measure record` 的 crontab 條目（原本 `install.sh` 的
  `deploy_measure_cron` 與 `cort_upgrade.rs` 的發布後步驟都會裝）；本單把兩者都改成移除，安裝與升級時若看到
  cort 自己的條目就移除
  （Task 5 既有的 `measure cron --uninstall` 邏輯，只動 cort 自己那一行）。
- 三台（NUC11i5、Thinkpade15、i51149R3050）原本都裝了這個條目。NUC11i5（2026-10-25 約 02:45Z，`cort measure cron --uninstall`）與 i51149R3050（2026-10-24，`crontab -r`）已移除；Thinkpade15 2026-10-26 查時仍在（`17 * * * * …/cort measure record >> …/measure.log 2>&1`），部署 I2（`80b56743`）時的安裝或升級會自動移除，不必另外手動刪；部署時使用者在場。 2026-10-26 13:35Z 以 `9483306a` 部署後已移除（三台都沒有了）。
- `cort measure cron` 子命令暫時保留（手動用得到），之後再決定刪不刪。

### 7. 本機自檢

遠端分不出機器關著還是收集壞了。新子命令 **`cort measure status`**（`measure show` 已經被壓縮明細
用掉，不改它）與 report 在本機執行時的第一段，印兩條收集的狀態：`pending`、上次執行／成功時間、
每個 harness 的連續失敗次數與錯誤種類、`push_pending`、`refused`、健康那條的 `unmapped`。
`refused` 與網路失敗分開顯示：前者要人去跑 dry-run，後者等它自己恢復。

### 8. 對其他 Task 的影響

- **Task 8**：改成本設計第 1–6 節。舊 Task 8 的 `SnapshotRow`／`snapshot_rows`／`roots_seen_since`／
  `index_snapshots` 不做。
- **Task 9**：健康表改成「每個專案 × 有使用的日期」，空白日是沒使用、不是異常；刪除「活躍機器那天
  沒有快照」這條規則。`STALE/refresh` 連續 2 天之類的規則改成「連續 2 個有使用的日子」。
- **Task 10**：`CORT-AUDIT.md` 的健康欄匯入**直接寫進遠端** `telemetry_events`（不經本機
  `index_health_log`，因為這些列不屬於執行匯入的那台機器）：`source='cort_index_health'`、
  `machine_id='imported:<host>'`、`machine_source='cort-audit-md'`、payload 多一個
  `"origin":"cort-audit-md"`（含 7 天門檻的舊口徑，report 標明）。event_id
  `sha256("cort_index_health\0imported:<host>\0" + day + "\0" + root + "\0" + payload)`，同一份檔重跑
  得到同樣的 id，`INSERT OR IGNORE` 擋掉重複。用量欄照原計畫進 host × 日的匯入用量表，不變。
- **退場步驟 7**「裝 cort 的排程」刪除；**步驟 8**「隔天檢查：遠端每台機器都有昨天的快照」改成
  「隔天檢查：有使用的機器都有新的事件與健康觀察，且沒有重複」。

## 限制

- 所有既有規則照舊：純 Rust、無腳本、測試絕不碰真的 crontab（`CORT_CRONTAB_FILE`）、儲存失敗回傳
  不 panic、hook 契約。
- 測試不可以在背景生出程序去碰真的資料目錄：kick 的「生什麼程序」要能在不真的生的情況下驗證（例如
  注入 spawner），真的生程序的測試一律指向暫存的 data dir 與 cache dir。
- 沙箱裡跑的 harness（例如 Codex）：生出來的子程序繼承沙箱，可能連不上網或寫不了 `data_dir()`。
  寫不了 → 節流戳記寫不成 → 不生程序；連不上網 → 失敗、記 `failed_harness`，別的 harness 不受退避
  影響。哪些沙箱實際能寫哪裡，要在部署時實機觀察，結果補進這份文件。

## 留給實作者

- 狀態檔的確切欄位格式；節流戳記可以併進狀態檔以外的獨立小檔（kick 寫、子程序不寫）。
- `index_health_log` 的確切欄名與索引。
- 子命令的確切名稱與旗標（`measure collect` / `measure health` 是建議名）。
- 兩個 hook 呼叫 kick 的確切位置（主工作之後、回傳之前）。
- kick 的成本量測方式（benchmark 測試或一次性量測寫進報告）。
