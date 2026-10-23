# 量測收進 cort：資料統一記在 TursoDB，要看的時候在上面分析

日期：2026-10-23　狀態：設計，待實作

## 為什麼

使用者看 claudecat 的 `CORT-AUDIT.md` 只為一件事：**看每台機器每天的指標有沒有異常**，例如
`fresh` 變成 `STALE` 那天，再去讀發現區塊找原因。真正的問題是**所有的 audit 從沒放在一起看，
分析永遠只是片段**：

- 每台機器的 `usage.db` 各自獨立，claudecat 09:29 的 LLM 分析只讀得到自己那台
  （i51149R3050 09-28 的發現就寫著「各機 DB 獨立，不可與 NUC11i5 的 12 互比」）。
- 只有 NUC11i5 有分析排程；其他機器的列靠手動合併進檔案。
- 檔案在掉資料：`--track` 重寫表頭兩次刪掉 09-27 的口徑註記，`findings` 的整段取代刪掉別台
  機器的發現區塊。
- 每天的 LLM 分析多半只在確認「沒事」（10 月每天都是「可動作標籤仍從缺」「數據品質全綠」）。

而量測放錯了地方。claudecat 的 `cort-audit` 是**在 cort 外面量 cort**：讀 cort 的 index、讀 cort 的
`usage.db`，自己再算一遍，於是一直在追上游（`feat(cort-audit): catch up with cortexyoung
5f6d5267`、`f8d3d3f4`…；09:29 prompt 還得提醒「census 出現 `unknown/<hook>`＝上游動了詞彙、
claudecat 的常數跟丟」）。cort 自己已經有 `usage.db`、機器戳記、Turso 同步副本（`cort memory`），
量測放回被量的東西裡，這兩個問題都會消失。

claudecat 的其餘功能 30 天內沒人用或已被取代（地圖、`navigate` Codex 15 次、memory 轉接、
`scan`/`explore`/`track`/`update`/`usage`/`findings`/`cort-status` 為 0），所以 claudecat 退場。

## 使用者的決定（2026-10-23）

- **資料統一記在 TursoDB，日後在上面分析。** 原始使用事件與每天的快照都進 DB；分析一律在 DB
  上做，`report` 是第一個分析。
- **不用每天看、不做每日 LLM 分析。** 想看時跑一個指令，在終端機印出**全部機器**的統計。
- **audit 與 telemetry 搬進 cort，claudecat 退場。**
- 不再自動開 hook 規則（claudecat 09:29 分析的第 4 步），report 也不補提示。
- i51149R3050、MacBook-Pro.local 已不使用，它們本機若有未合併的列就放棄。
- 一台機器超過 30 天沒資料就自動算不活躍，不加 retire 指令。
- `STALE/refresh` 連續 2 天以上才算異常。
- 量測另開一個 Turso 資料庫，不跟 `cort memory` 共用 `harnessuse`。
- （另一件工作，不在本文件範圍：cort 拿掉 `context`、`struct`、`read`、`recall`。）

## 目標

1. **原始使用事件進 DB**：cort 把本機 `usage.db` 的 `command_log` 同步到共用 TursoDB 的
   `telemetry_events`（取代 `claudecat telemetry sync`），每天自動跑。
2. **每天的快照進 DB**：每台機器每天記一次每個已建 index 專案的 **index 健康**（事件裡沒有的
   資訊）。
3. **`report` 在 DB 上分析**：一個指令印出全部機器最近 N 天的狀況，異常直接標出來，最後有跨機
   器摘要。用量類指標從原始事件算，健康類從快照讀。
4. **沒網路不丟資料**：先寫本機副本，下次推上去。
5. **歷史接得上**：`CORT-AUDIT.md` 的約 56 列匯入；claudecat 已上傳的 `telemetry_events` 照用、
   不重複。
6. **claudecat 乾淨退場**。

## 不做（non-goals）

- 不做每日 LLM 分析、不寫「發現」。深入判讀時，使用者把 report 輸出（或 `--json`）丟給 agent。
- 不改 hook 記錄什麼：`command_log` 的欄位與寫法照舊。
- 不放進 `cort memory` 帳本：那裡只收從已提交來源審過的主張。只是共用同一個資料庫與設定。
- 不做網頁、dashboard、artifact，只有終端機輸出。
- 不搬舊的發現區塊（留在 claudecat 的 git 歷史）。
- 不救 i51149R3050、MacBook-Pro.local 本機檔裡未合併的列。

## 反目標（做到了也算失敗）

- **報告把沒有資料的活躍機器藏起來**：缺列要明說「沒有資料」。
- **天天誤報**：不再使用的機器每天都被標「沒有資料」，異常標記就失去意義。
- **重複或遺失事件**：cort 接手同步後，claudecat 已上傳的事件被再上傳一份，或有一段事件兩邊都
  沒傳。
- **離線讀舊副本卻當成最新**。
- **把網路工作塞進 hook**：`hook-suggest`／`hook-refresh` 不碰網路，只有排程與手動指令會。

## 設計

### 量測用自己的 Turso 資料庫（使用者 2026-10-23 決定）

- **為什麼不共用**：Turso 的同步副本是遠端**整個資料庫**的複本。現況是 cort memory 與 claudecat
  telemetry 都指向 `harnessuse`，Thinkpade15 上 cort memory 的副本已經 9.6M，大多是 telemetry 事件；
  事件每月約增 13 萬筆，記憶的 recall 會跟著變慢變大。所以量測另開一個 Turso 資料庫，記憶副本
  不再帶事件。
- 新資料庫與憑證向 `armo` 申請（使用者的全域規則：Turso 憑證問 `armo`）。cort 另有一份量測用的
  設定檔（跟 `turso-memory.json` 分開，命名留給實作者），由一個 `configure` 子命令寫入（比照
  `cort memory configure` / `import-config`：token 走 stdin、拒絕 symlink）。
- 量測用自己的副本檔與鎖。同一個副本的所有量測動作（排程記錄、手動 report、手動 sync）共用那一
  把鎖，一個一個來。
- **舊事件搬家**：`harnessuse` 裡 claudecat 已上傳的 `telemetry_events` 與
  `telemetry_project_aliases`，用一個 cort 子命令原封不動複製到新資料庫（`event_id` 不變，可重跑、
  `INSERT OR IGNORE`）。`harnessuse` 裡的舊表在新資料庫驗證完之後才清——**清之前再問使用者**，
  那是對共用資料庫的破壞性動作。
- 另外，`cort memory` 本身仍用 `harnessuse`：**NUC11i5 的 cort memory 還沒設定**（`cort memory
  status` 回 `Turso memory is not configured`），退場步驟裡用 `cort memory import-config
  ~/.local/share/claudecat/turso-memory.json` 補上。這跟量測無關，但同一次一起處理。

### 原始事件：沿用 `telemetry_events`

- 表的欄位不動（`source, event_id, machine_id, machine_source, project_id, time_ms, payload`，
  PK `(source, event_id)`，`INSERT OR IGNORE`）。可以加索引：本機一個月約 13 萬筆
  （`hook-refresh` 66,568 + `hook-suggest` 58,947，2026-10-23 的 30 天數字），report 依時間範圍查，
  需要 `(source, time_ms)` 之類的索引。
- **event_id 必須跟 claudecat 算的完全一樣**，否則 claudecat 已上傳的列會被當成新事件再上傳一次
  （反目標）。claudecat 的算法在 `claudecat/src/telemetry.rs:100-170`：`source="cort_command"`，
  payload＝`SELECT rowid, * FROM command_log` 整列轉 JSON，`event_id = sha256(source\0machine_id\0
  rowid\0payload)`，加上 pre-v2 legacy 列的特殊處理（移除 `machine_id`/`machine_source` 後保留原
  payload 與 id）。cort 要逐字移植這段，並用 claudecat 的測試案例做對照測試。
- **payload 的 key 順序會進雜湊**：JSON 是照欄位插入順序輸出，因為 serde_json 的 `preserve_order`
  是開著的——但兩邊的 `Cargo.toml` 都只寫 `serde_json = "1"`，這個 feature 是被別的套件順便帶進來
  的（兩邊 `Cargo.lock` 的 serde_json 都依賴 `indexmap`）。所以 cort 要在 `Cargo.toml` **明確**開
  `preserve_order`，golden test 要斷言 key 順序本身；之後改 `usage_schema.sql` 的欄位順序也會改掉
  所有雜湊，要在 schema 旁邊註明。
- **上傳前先對帳**：同步指令要有 `--dry-run`：在本機算出 id，跟遠端已有的 `(source, event_id)` 比
  對，不 push。第一次正式同步前必須先跑它；本機與遠端共有的那段 rowid，id 必須 100% 相符，否則
  拒絕同步。理由：`INSERT OR IGNORE` 只擋得住 id 完全相同的列，id 算錯一個欄位就會把約 13 萬筆
  舊事件再上傳一份，而遠端寫進去就收不回來。
- claudecat-only 的來源（`claudecat_navigation`、`claudecat_memory_recall`）：退場前在每台機器跑
  最後一次 `claudecat telemetry sync` 把它們送完，之後不再產生。
- `cort-evals` 的成本報告（`~/.local/share/cort-evals/claude-cost/*.json`，claudecat 的 `read_cost`）
  也由 cort 的同步接手，算法同樣逐字移植。
- project alias（`telemetry_project_aliases`）照 claudecat 的做法一起移植。

### 每天的快照：index 健康

另開一張表（命名留給實作者），每台機器、每個已建 index 的專案、每個 UTC 日一列：

- key：`(day, machine_id, root)`。用 `root` 而不是 `project_id`：同一台機器兩個 checkout 指向同一個
  origin 時兩列並存（claudecat `track_table` 的現行語意）。同一天重跑就 upsert 覆蓋。
- 欄位：`project_id`（git origin，沒有 origin 時 `path:<root>`）、`host`、`recorded_ms`，以及健康
  指標：`fresh` 狀態（`fresh`／`STALE`／`STALE/refresh`／`STALE/rebuild`／`STALE/graph`／`fresh?`／
  `no-index`，判定與 claudecat `row_md` 相同，改用 cort 內部的 status／repair 直接算）、chunks、
  relationships、未 chunk 檔數、真缺口、FTS drift。
- `day` 用 UTC（跟 claudecat `today_iso()` 一致，所有機器同一個日曆）。
- 「每個已建 index 的專案」＝`cort projects` 列得出來的專案；不存在的目錄照 `cort_upgrade` 現行
  慣例跳過。

### 機器名稱

`machine_id` 是雜湊，人讀不懂。記錄時順便 upsert 一張 `machine_id → host` 對照（含最後出現時間），
report 用 host 顯示。**report 的分組、活躍判定、缺列檢查一律以 host 為準**：歷史匯入的列只有 host。
同一個 host 底下出現兩個以上的非匯入 `machine_id`（兩台同名），report 印警告，不默默合併。反過來，
同一個 `machine_id` 出現兩個以上的 host（機器改名）也印警告——否則它的歷史會被拆成兩台。

### 記錄：每天的排程

- 一個 cort 指令（建議 `cort usage record`）做兩件事：寫快照、同步原始事件。
- 順序：**先在本機副本提交快照，再** push、pull、上傳事件。不能照抄 claudecat `sync_telemetry`
  的順序（`claudecat/src/turso_memory.rs:185`，寫入前先 push/pull，任何一步失敗就返回），否則斷網
  那天連本機的快照都不會落地。
- push 失敗：本機資料保留，下次任何一次記錄或 report 先推待送的；指令印 `saved locally, push
  failed: …` 並**以非零碼結束**，讓 cron log 看得到。Turso 沒設定：非零碼結束並說明。
- 副本第一次開啟要連得到遠端（`turso::sync::Builder` 的 `bootstrap_if_empty`，現有程式都沒明確設，
  靠預設值）；cort 要明確設定它，並用測試固定 turso 0.9.0-pre.1 的行為。安裝排程時先開一次。
- 排程條目由 cort 的一個子命令擁有（比照 `hook-install`：合併、去重、整行比對、不認得的
  crontab 不動、有測試），由安裝與 `cort_upgrade` 呼叫。整行合併的邏輯從 claudecat
  `doctor::merge_whole_line`（`c768644`）移植。`crontab` 一律用 stdin（`crontab -`）寫入。
- **每小時跑一次**（第 17 分，避開整點），不是每天 09:17 一次：cron 不會補跑錯過的時間，筆電
  （Thinkpade15）在 09:17 睡著就會整天沒有快照，被標成「沒有資料」。每次執行：今天（UTC）還沒有
  快照才寫快照，已經有就跳過；然後推送待送的資料、同步新事件。離線的那幾個小時也因此會自己補上。

### 統計：`report`

- 先推待送的、再 pull；失敗就讀本機副本，第一行說明「離線：讀本機副本，最後同步於 …」。
- 預設最近 7 天（`--days N`）。兩種表，因為兩類資料的範圍不同：
  - **健康表**：一個專案一張，列是機器 × 日期，欄是快照的健康指標。
  - **用量表**：一張，列是機器 × 日期。用量是**整台機器**的（claudecat 的算法就是
    `SELECT command, COUNT(*) FROM command_log WHERE ts >= ?` 不分專案，`claudecat/src/cort.rs:858`），
    不放進每個專案的表，免得同一個數字重複出現、被讀成那個專案的用量。
- 用量欄：命令數/30d、命令數/7d、`impact`/30d、decline-top，從 `telemetry_events` 的 `cort_command`
  事件算。decline-top 要逐字移植 claudecat 的 census（`claudecat/src/cort.rs:877` 起解析
  `args_summary`，每列恰落一桶、加總等於 fires，排除 `not_a_search_tool`）。
- **core／deep 改定義**：claudecat 的 core＝context+impact+recall+struct+read、deep＝context+recall
  （`claudecat/src/cort_audit.rs:10-20`）。另一件工作拿掉這四個指令後，core 只剩 impact、deep 恆為 0，
  所以新 report 的欄位是 `impact`/30d，不再有 deep 欄。匯入的歷史列保留它們原本的 core/deep 值，在
  歷史表格裡照原樣顯示，並標明口徑不同。
- **活躍機器**＝最近 30 天有任何資料（快照或事件）的 host。只有活躍機器會被標「沒有資料」；其他
  機器不進表格，摘要裡**每次都**列一行「不活躍，最後資料於 YYYY-MM-DD」。
- **異常標記**（固定、可測）：
  1. index 不新鮮：`STALE`、`STALE/rebuild`、`STALE/graph`、`fresh?`、`no-index` 當天就標；
     **`STALE/refresh` 要連續 2 天以上才標**（使用者 2026-10-23 決定）——單日的 `STALE/refresh`
     只代表有未提交的編輯、下次查詢會自己修好，在表上照樣顯示但不標紅；連續 2 天代表自癒沒生效。
  2. FTS 不是 `synced`。
  3. 真缺口 > 0，或讀不到。
  4. 活躍機器那天沒有快照。今天（UTC）不檢查：今天的第一次排程還沒跑時，本來就還沒有。
- 原本草稿的「命令數掉超過一半」不列入：那是我猜的，使用者沒提過；它拿 7 天總數跟每日值的中位數
  比，單位不一致，放假就會觸發。用量表照樣顯示數字，由人看。
- 摘要：每台機器這段期間幾天異常、哪幾種；每台最後一次有資料是哪天；同名 host 警告。
- `--json`：同樣內容給 agent 讀。

### 歷史匯入

- 一次性指令，解析 claudecat `CORT-AUDIT.md` 長期指標表，寫進快照表。舊列的用量欄也一起存（那些
  日子不一定有對應事件），report 顯示那幾天時用匯入值。
- 舊列只有 host：`machine_id` 用 `imported:<host>`、來源記 `cort-audit-md`；2026-09-06 那列沒有 host，
  記 `unknown`，**不參與活躍判定與缺列檢查**，只出現在歷史表格。
- 可重跑（upsert）。`project_id` 用 claudecat repo 的 origin（現有列全是 claudecat）。
- 09-27 口徑註記不搬（report 預設 7 天，不會跨到 09-27；註記留在 git 歷史）。

### claudecat 退場步驟

順序很重要：先讓 cort 接得住，才拆 claudecat。

1. cort 的同步、快照、report、匯入、搬家、排程子命令完成並測過。
2. 向 `armo` 申請量測用的新資料庫與憑證；每台機器用 cort 的量測 `configure` 寫入設定。NUC11i5 另外
   `cort memory import-config ~/.local/share/claudecat/turso-memory.json`。
2b. 把 `harnessuse` 的 `telemetry_events` 與 alias 複製到新資料庫，對一次列數。
3. 移除 claudecat 的 09:17 與 09:29 條目（先停，之後才不會有新事件沒人送）。
4. 每台在用的機器（NUC11i5、Thinkpade15）跑最後一次 `claudecat telemetry sync`。
5. 每台機器跑 cort 同步的 `--dry-run`，確認與遠端共有的 rowid 段 id 100% 相符。不相符就停在這裡，
   修 cort 的算法；claudecat 都還在，什麼都沒壞。
6. 在 NUC11i5 匯入 `CORT-AUDIT.md`，對一次列數。
7. 裝 cort 的排程。
8. **隔天檢查**：遠端每台機器都有昨天的快照、事件持續增加且沒有重複，才往下走。
9. 移除 `~/.cargo/bin/claudecat`。在這之前，回退方法就是把 claudecat 的排程裝回去。GitHub 上的
   claudecat repo 怎麼處理（封存或保留），**退場時再問使用者**——那是對外的動作。
9b. 問使用者後，才清掉 `harnessuse` 裡的舊 `telemetry_events` 與 alias（之後記憶副本會變小）。
10. 文件：cortexyoung `AGENTS.md` 的「claudecat (separate repo)」一節與 `claudecat telemetry sync`
    那句改成描述 cort 自己的量測；記憶與 README 裡指向 claudecat 的地方一併改。改的時候寫明：
    這是 claudecat **退場**取代了 2026-09-23 的「不要併進來」規則（那條防的是兩份並存），不是違反它。

## 限制

- 全部 Rust，不加腳本。Turso 同步副本只能用 `turso` crate 開，不能用 rusqlite 或 Turso CLI。
- hook 不碰網路；`hook-suggest` 照舊只寫本機 `usage.db`。
- `usage.db` 的 schema 不為這件事改。
- 本機 NUC11i5 記憶體緊（9.7Gi / 6 核）：驗證時 cargo 一次只跑一個。
- 寫進 Turso 的內容：`command_log` 整列（含 `args_summary`）與健康數字，跟 claudecat 現在上傳的
  一樣，不多不少。

## 留給實作者

- 指令與子命令命名、表名、JSON 欄位名。
- report 的終端機排版與顏色（`NO_COLOR`／非 TTY 時關掉）。
- 儲存介面的確切形狀（見下一節的測試）。
- 同步副本與鎖的模組放哪，跟 `memory_store.rs` 共用多少開檔程式碼。

## 我的判斷（使用者沒有逐條決定，讀的時候請檢查）

- 快照只存 index 健康；用量類指標由 report 從原始事件算（「資料記在 DB、在上面分析」）。
- 健康表按專案分、用量表按機器分；core/deep 改成 `impact`/30d，歷史列保留舊口徑並標明。
- （已由使用者決定改為另開資料庫，見「量測用自己的 Turso 資料庫」。）舊事件用 cort 子命令搬家，
  不用 Turso CLI 或腳本。
- 快照涵蓋每台機器所有已建 index 的專案，不只 claudecat 那一個。
- key `(day, machine_id, root)`；`project_id` 只用來分表；report 以 host 為準、同名印警告。
- 排程由一個 cort 子命令擁有（比照 `hook-install`），由安裝與 `cort_upgrade` 呼叫。
- push 失敗時非零碼結束，資料留在本機。
- 活躍＝30 天、預設 7 天、今天不檢查、日期用 UTC；拿掉「命令數掉一半」規則。
- 排程每小時跑、每天只寫一次快照（筆電睡著也補得上）。
- 上傳前 `--dry-run` 對帳；`preserve_order` 明確寫進 `Cargo.toml`。
- 退場順序：先停 claudecat 排程、再最後同步、再對帳、隔天檢查後才移除 binary。
- 測試：記錄與查詢走一層儲存介面；測試用本機假儲存（暫存目錄裡的 rusqlite，不是同步副本檔，
  不違反限制）自動測 upsert、異常規則、活躍判定、同名警告、push 失敗保留資料、離線第一行；
  event_id 用 claudecat 的現有案例做對照測試。真正的 Turso 同步在實機上驗。
