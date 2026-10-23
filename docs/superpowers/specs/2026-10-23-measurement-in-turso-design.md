# 量測收進 cort：資料統一記在 TursoDB，要看的時候在上面分析

日期：2026-10-23　狀態：設計，待實作（同日第二版：主角改為「壓縮後的代價」，見下一節）

## 第二版：量測要找出能幫 Claude 改進的地方（使用者 2026-10-23）

使用者看過 30 天用量後的方向：cort 一個月只被 Claude 實際呼叫 8 次（`impact`），為「cort 有沒有
用」蓋一套量測不是重點。**量測要持續，但目標是找出能幫 Claude 改進的地方**；claudecat 先封存
（GitHub repo 已於 2026-10-23 封存，本機排程留到 cort 的量測接上才停）；`cort memory` 先在兩台設好
（NUC11i5 已於 2026-10-23 `import-config`，Thinkpade15 原本就有，兩台同專案同帳本）。

使用者指出的那一刻：**壓縮之後要重查、還講錯**。這是本版的主角，下面「主角：壓縮後的代價」一節
定義怎麼量。原本的 index 健康快照與 cort 用量保留，退為配角——後者仍是「cort 在哪裡能幫 Claude」
的證據（hook 看得到每一次搜尋）。兩者共用同一個量測資料庫、同一個 `record` 排程、同一個 `report`。

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

0. **壓縮後的代價進 DB、在 report 裡看得到趨勢**：每台機器每次 Claude Code 壓縮，壓縮後重查了
   多少、有沒有被使用者糾正，跟同一段對話壓縮前的正常水準比。日後任何改進（例如壓縮後先查
   `cort memory`）都用這個數字前後對照，判斷有沒有用。
1. **原始使用事件進 DB**：cort 把本機 `usage.db` 的 `command_log` 同步到共用 TursoDB 的
   `telemetry_events`（取代 `claudecat telemetry sync`），每天自動跑。
2. **每天的快照進 DB**：每台機器每天記一次每個已建 index 專案的 **index 健康**（事件裡沒有的
   資訊）。
3. **`report` 在 DB 上分析**：一個指令印出全部機器最近 N 天的狀況，異常直接標出來，最後有跨機
   器摘要。用量類指標從原始事件算，健康類從快照讀。
4. **沒網路不丟資料**：先寫本機副本，下次推上去。
5. **歷史接得上**：`CORT-AUDIT.md` 的 53 列匯入；claudecat 已上傳的 `telemetry_events` 照用、
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

### 主角：壓縮後的代價

**資料從哪來**：`~/.claude/projects/<project-dir>/<session>.jsonl`。壓縮點是一列
`{"type":"system","subtype":"compact_boundary"}`，帶 `compactMetadata`（`trigger` auto/manual、
`preTokens`、`postTokens`）、`sessionId`、`cwd`、`timestamp`、`uuid`；下一列是摘要（`isCompactSummary`）。
2026-10-23 實查：本機 1,071 份記錄中 14 份有壓縮；審查時全機 653 份主對話記錄裡有 86 個壓縮點、
分佈在 13 份，每個都帶 `compactMetadata`，平均一份 6.6 次。每份記錄裡只有一個 `sessionId`、等於檔名；
`/exit` 後 resume 的同一串對話寫回同一個檔。但 fork／resume 會讓**兩份檔共用一段歷史**（實查兩對，
分別共用 1,638 與 336 則訊息，其中一個壓縮點 uuid 同時出現在兩份）。子代理的記錄在
`<session>/subagents/*.jsonl`，**不算**進主對話（子代理沒有被壓縮的上下文可重查）。只看 Claude
Code；Codex／Kimi 的記錄不在本版。

**每次壓縮記一列**，key `(machine_id, boundary_uuid)`——**以壓縮點 uuid 去重**，共用歷史的兩份檔只
算一次（取先出現的那份的 `session_id`）。欄位：時間、`session_id`、`project_id`（用**壓縮點那一列的
`cwd`**，套快照同一套正規化 origin 雜湊；`cwd` 沒有 git origin 時為 NULL，不猜）、`trigger`、
`pre_tokens`、`post_tokens`，以及下面兩個視窗的計數。

**兩個對稱的視窗**（審查後定案，我的判斷）：
- **後窗**：壓縮點之後的前 50 個主對話 tool_use，遇到下一個壓縮點就截斷。不看整段到檔尾：一段可能
  跑好幾個小時或幾天後才 resume，代價會被稀釋。
- **前窗（基準）**：壓縮點之前緊鄰的 50 個 tool_use，遇到上一個壓縮點就截斷。
- 兩窗用**同一套規則**，「已知」都是指**該窗開始之前**這份檔裡出現過的東西：
  - `rereads`：窗內對「窗開始前已知檔案」的 Read，每個路徑只算第一次。已知檔案＝之前出現過的
    Read／Write／Edit／MultiEdit／NotebookEdit 的 `file_path`（2026-10-23 試算的教訓：只看 Read 會漏
    掉 Claude 自己寫出來的檔，那次真實的重讀被算成 0）。窗內第一次碰、之後又讀的不算。
  - `repeated_commands`：窗內跟窗開始前某個 Bash 指令**逐字相同**的 Bash，每個指令只算第一次。
  - `transcript_lookups`：輸入裡含 `.claude/projects/` 且含 `.jsonl` 的 tool_use 數（翻自己的記錄找回
    摘要沒留下的東西，例如 2026-10-23 找子代理 ID）。比對整個輸入字串，不截斷。
  - `human_messages`：窗內使用者親手打的訊息數（判定見下）。
  - `corrections`：其中符合固定糾正詞表的則數。詞表寫死在程式裡並有測試（起始：`不對`、`不是`、
    `錯了`、`講錯`、`搞錯`、`wrong`、`that's not`、`no,`），之後用抽樣結果修。
  - `tool_calls`：窗內實際的 tool_use 數（被截斷時少於 50）。
- 一列存**前窗與後窗各一組**計數（`pre_*`／`post_*`），report 算「後窗減前窗」，不比絕對數——正常
  工作本來就會重讀。前窗長度為 0（壓縮點緊接著上一個）時，report 不拿它比，只列後窗。

**使用者親手打的訊息**（審查後定案）：新版記錄的使用者列帶 `origin`，`origin.kind == "human"` 才算；
`task-notification`、`peer` 等其他 kind 一律不算。沒有 `origin` 的舊列：內容是字串、不是 `isMeta`、
不是摘要（`isCompactSummary`）、不是 tool_result、且第一個字元不是 `<`（排除 `<task-notification>`、
`<system-reminder>`、`<command-name>` 等注入）才算。這兩條規則各有測試。

**還在長的列**：後窗沒滿 50、也還沒遇到下一個壓縮點時，該列是未定；每次 `record` 發現檔案的
`(大小, mtime)` 水位變了就重算並 upsert，**包括 resume 多天後再長的檔**——不設「24 小時結案」，因為
視窗固定，後窗一滿或遇到下一個壓縮點，數字就不會再變。

**歷史回填**：第一次 `record` 掃完本機所有現存記錄（本機最舊是 2026-09-01），之後才增量。改進要有
「之前」的基準，而 Claude Code 可能依它的清理週期刪舊記錄（本機沒設 `cleanupPeriodDays`，用預設），
沒先收進資料庫的歷史會消失；本機明細子命令也只在記錄還在時有用。所以這一步排在實作計畫裡、量測
資料庫一設好就跑。

**不上傳文字**：送進 Turso 的只有上面的計數與 id，不含訊息內容、指令內容、檔名（跟限制「不多不少」
一致：本版新增的就是這些計數）。要看是哪幾則糾正、重讀了哪些檔，用一個**只讀本機**的子命令印出
（給 session id 或壓縮點 uuid），供人工抽樣判斷詞表準不準。

**report 新增一節「壓縮後的代價」**，放在最前面：每台機器、最近 N 天（預設 30，壓縮次數少）：壓縮
次數、每次壓縮的重讀／重跑／翻記錄／糾正（中位數與總和）、與基準的差；再按週列趨勢，讓改進前後看
得出變化。沒有壓縮的機器印「這段期間沒有壓縮」，不算異常。

**本版不做的**：自動判斷「講錯」的內容是否真的錯（只數糾正）；跨 session 的遺忘（新開 session 重查
上一個 session 知道的事）——同一個問題的另一半，等這一半有數字再說；改進本身（例如壓縮後自動查
`cort memory` 的 hook）是另一份設計，本版只負責讓它量得出效果。

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
- 另外，`cort memory` 本身仍用 `harnessuse`。NUC11i5 原本沒設定，2026-10-23 已用 `cort memory
  import-config --from ~/.local/share/claudecat/turso-memory.json` 補上（`sync_status: synced`，與
  Thinkpade15 同 project id、同 4 筆）。

### 原始事件：沿用 `telemetry_events`

- 表的欄位不動（`source, event_id, machine_id, machine_source, project_id, time_ms, payload`，
  PK `(source, event_id)`，`INSERT OR IGNORE`）。可以加索引：本機一個月約 13 萬筆
  （`hook-refresh` 66,568 + `hook-suggest` 58,947，2026-10-23 的 30 天數字），report 依時間範圍查，
  需要 `(source, time_ms)` 之類的索引。
- **event_id 必須跟 claudecat 算的完全一樣**，否則 claudecat 已上傳的列會被當成新事件再上傳一次
  （反目標）。claudecat 的算法在 `claudecat/src/telemetry.rs:39-170`，以下每一項都會進雜湊，移植時
  逐項照做：
  - `source="cort_command"`；payload＝`SELECT rowid, * FROM command_log` 依欄位順序組成的**緊湊**
    JSON 物件（`telemetry.rs:46`、`:113`）。
  - SQLite 值的轉換：NULL→`null`、整數、浮點、文字以 lossy UTF-8 轉、BLOB→`"blob:<hex>"`。
  - pre-v2 legacy 列：machine id 取自 `_usage_meta`，序列化與雜湊前先移除後來加上的
    `machine_id`/`machine_source` 欄（`telemetry.rs:70`、`:126`、`:152`）。
  - 雜湊輸入是 UTF-8 的 `source\0machine_id\0rowid\0payload`，SHA-256，小寫 hex（`telemetry.rs:39`、
    `:158`）。
- **golden 測試先做，才移植**（審查後補）：claudecat 現有測試（`tests/telemetry_test.rs:146`）只
  斷言兩次掃描的 id 相同，沒有固定的期望值，不能當對照。所以在 claudecat 還在時，用它對一份固定的
  fixture `usage.db`（含 v2 列、pre-v2 legacy 列、NULL、浮點、BLOB、非 ASCII 文字）算出 payload 與
  id，把這些值寫死進 cort 的測試。
- **payload 的 key 順序會進雜湊**：JSON 是照欄位插入順序輸出，因為 serde_json 的 `preserve_order`
  是開著的——但兩邊的 `Cargo.toml` 都只寫 `serde_json = "1"`，這個 feature 是被別的套件順便帶進來
  的（兩邊 `Cargo.lock` 的 serde_json 都依賴 `indexmap`）。所以 cort 要在 `Cargo.toml` **明確**開
  `preserve_order`，golden test 要斷言 key 順序本身；之後改 `usage_schema.sql` 的欄位順序也會改掉
  所有雜湊，要在 schema 旁邊註明。
- **上傳前先對帳**：同步指令要有 `--dry-run`：在本機算出 id，跟遠端已有的列比對，不 push。第一次
  正式同步前必須先跑它。理由：`INSERT OR IGNORE` 只擋得住 id 完全相同的列，id 算錯一個欄位就會把
  約 13 萬筆舊事件再上傳一份，而遠端寫進去就收不回來。比對範圍要講清楚（rowid 只在一個 `usage.db`
  內有意義）：
  - 以 `(source, machine_id, payload 裡的 rowid)` 配對本機列與遠端列；machine_id 用跟算 id 時同一套
    歸屬（legacy 列取 `_usage_meta`）。
  - 配得上的列，event_id 必須 100% 相符，不符就拒絕同步，並列出前幾筆不符的 payload 差異。
  - **每台機器的重疊不能是空的**：重疊為 0 時對帳等於沒做（2026-10-23 查到 NUC11i5 從沒跑過
    `claudecat telemetry sync`，`harnessuse` 裡沒有它的事件）。退場步驟因此先讓 claudecat 把每台
    機器的事件送進新資料庫（見退場步驟 4），讓每台都有重疊可比。
  - 只在本機、或只在遠端的列分開計數印出，不算失敗。
- **去重不載入全部遠端 id**（審查後補，我的判斷）：遠端會一直長，每小時把全部 `(source, event_id)`
  讀進記憶體不可行。正式同步以每台機器、每個來源已上傳的最大本機 rowid 當水位，只算水位之後的列；
  `--dry-run` 與水位遺失時才做全範圍比對。
- **遠端事件永久保留**（使用者 2026-10-23 決定）：不刪、不彙總。本機只留 90 天，新資料庫是事件唯一
  的長期存放處；副本每年大約多幾百 MB，這個副本只放量測，不影響記憶。pull 是增量的，去重用下面的
  水位，所以每小時的成本不隨總量成長。
- **本機保留 90 天的邊界**：cort 寫入時會刪掉 90 天前的 `command_log`（`rust/src/usage.rs:11`、
  `:902`）。每次記錄都先把新事件寫進本機副本、再嘗試 push，所以斷網不會丟事件；只有連續 90 天
  完全沒跑記錄，事件才會在上傳前被刪掉。文件與 `--help` 照這句寫明。
- claudecat-only 的來源（`claudecat_navigation`、`claudecat_memory_recall`）：退場時讓 claudecat
  最後一次同步直接寫進新資料庫（見退場步驟 4），之後不再產生。
- `cort-evals` 的成本報告（`~/.local/share/cort-evals/claude-cost/*.json`，claudecat 的 `read_cost`）
  也由 cort 的同步接手，算法同樣逐字移植。
- project alias（`telemetry_project_aliases`）照 claudecat 的做法一起移植。

### 每天的快照：index 健康

另開一張表（命名留給實作者），每台機器、每個已建 index 的專案、每個 UTC 日一列：

- key：`(day, machine_id, root)`。用 `root` 而不是 `project_id`：同一台機器兩個 checkout 指向同一個
  origin 時兩列並存（claudecat `track_table` 的現行語意）。
- **一天內每次排程都 upsert 那天的列，report 判讀當天最後一次觀察**（審查後補，我的判斷）：只寫
  當天第一次會把早上的一個暫時狀態永久記下來，後來修好也看不到，「`STALE/refresh` 連續 2 天」就會
  誤判。列上留 `first_recorded_ms` 與 `recorded_ms`。
- `project_id` 的口徑（審查後更正）：`command_log.project_id` 是本機 realpath 的雜湊
  （`rust/src/db.rs:78`），不是 origin；claudecat 用 `telemetry_project_aliases` 把它對到正規化
  origin 的雜湊（`claudecat/src/telemetry.rs:257`、`claudecat/src/turso_memory.rs:130` 的
  `normalize_remote`）。快照的 `project_id` 用**同一個正規化 origin 雜湊**（逐字移植
  `normalize_remote`），事件靠 alias 接上；沒有 origin 的專案用本機 project id，顯示為
  `path:<root>`，不跨機器合併。
- 欄位：`project_id`、`host`、`first_recorded_ms`、`recorded_ms`、**`status`（`ok`／`unreadable`／
  `missing_dir`／`no_index`）與 `error` 文字**，以及健康指標：`fresh` 狀態、chunks、relationships、
  未 chunk 檔數、真缺口、FTS 狀態（`synced`／drift 數／讀不到）。數字欄是 NULL 時一律代表「讀不到」，
  原因看 `status`/`error`；0 就是 0。每條異常規則都要能只從存下來的列判斷，測試照這個寫。
- **`fresh` 的判定改用 cort 自己的 staleness**（head 比對＋內容比對，即 `index_is_stale`／repair），
  **不沿用 claudecat 的 7 天年齡門檻**（`claudecat/src/cort.rs:219` `is_fresh`）。快照現在涵蓋每個
  已建 index 的專案，一週沒動的 repo 用 7 天門檻就會天天被標 `STALE`，正是「天天誤報」那條反目標。
  標籤名稱沿用 `fresh`／`STALE`／`STALE/refresh`／`STALE/rebuild`／`STALE/graph`／`fresh?`／
  `no-index`，匯入的歷史列的 `STALE` 含 7 天門檻，歷史表格標明口徑不同。
- `day` 用 UTC（跟 claudecat `today_iso()` 一致，所有機器同一個日曆）。
- **專案清單＝現在列得出來的＋以前記過的**（審查後補）：`list_projects()` 只列出現存的 index
  （`rust/src/db.rs:439`），被刪掉的 index 會直接從清單消失，`no-index` 永遠不會出現。所以每次記錄
  的清單是「`cort projects` 現在列得出來的」聯集「這台機器最近 30 天快照裡出現過的 root」；後者這次
  找不到 index 就記一列 `status=no_index`，目錄不在就記 `missing_dir`（不再算異常、30 天後自然掉出
  清單）。`list_projects` 回 `Unreadable` 時記 `status=unreadable` 加 `error`，不默默略過。

### 機器名稱

`machine_id` 是雜湊，人讀不懂。記錄時順便 upsert 一張 `machine_id → host` 對照（含最後出現時間），
report 用 host 顯示。**report 的分組、活躍判定、缺列檢查一律以 host 為準**：歷史匯入的列只有 host。
同一個 host 底下出現兩個以上的非匯入 `machine_id`（兩台同名），report 印警告，不默默合併。反過來，
同一個 `machine_id` 出現兩個以上的 host（機器改名）也印警告——否則它的歷史會被拆成兩台。

沒有對照的 `machine_id`（審查後補，我的判斷）：`machine_id` 是穩定的，在用的機器（NUC11i5、
Thinkpade15）第一次記錄時就會寫進對照，它們以前上傳的事件也就接得上。永遠不會再記錄的
（i51149R3050、MacBook-Pro.local 的事件、`legacy_unknown`）不手動補對照：用量統計與「最後資料於」
照樣算，在摘要的不活躍那行以 `machine <前 8 碼>` 顯示，不丟掉。

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
  （Thinkpade15）在 09:17 睡著就會整天沒有快照，被標成「沒有資料」。每次執行：upsert 今天（UTC）
  的快照（見「每天的快照」）；然後推送待送的資料、同步新事件。離線的那幾個小時也因此會自己補上。

### 統計：`report`

- 先推待送的、再 pull；失敗就讀本機副本，第一行說明「離線：讀本機副本，最後同步於 …」。
- 預設最近 7 天（`--days N`）。兩種表，因為兩類資料的範圍不同：
  - **健康表**：一個專案一張，列是機器 × 日期，欄是快照的健康指標。
  - **用量表**：一張，列是機器 × 日期。用量是**整台機器**的（claudecat 的算法就是
    `SELECT command, COUNT(*) FROM command_log WHERE ts >= ?` 不分專案，`claudecat/src/cort.rs:858`），
    不放進每個專案的表，免得同一個數字重複出現、被讀成那個專案的用量。
- 用量欄：命令數/30d、命令數/7d、`impact`/30d、decline-top，從 `telemetry_events` 的 `cort_command`
  事件算。日期 D 那列的視窗＝截至 UTC 日 D 結束的 30（7）天（claudecat 是從本機 09:17 往回算，
  所以跟匯入值會有小差，不需對齊）。
- decline-top（審查後更正，我的判斷）：沿用 claudecat `row_md` 實際用的 **`declines` 計數**
  （`claudecat/src/cort.rs:877` 起的掃描裡 `u.declines`，`cort.rs:906`；挑最大值在
  `claudecat/src/cort_audit.rs:619`），以 `hook/decline` 為 key、排除 `not_a_search_tool`。它跟
  census（每列恰落一桶）是兩個不同的數，匯入的歷史值是 `declines` 算的，用同一個才對得起來。
- **匯入值與事件算出的值重疊時**：某台機器某天只要有事件，用量表就用事件算的；只有那天完全沒有
  事件時，才顯示匯入值並標「匯入」。同一個 host、同一天、同一個 root 同時有 `imported:<host>` 與
  真 `machine_id` 的快照時，用真的那列。
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
  4. 活躍機器那天沒有快照。今天（UTC）不檢查：今天的第一次排程還沒跑時，本來就還沒有。以
     **host 為單位**判斷：那天這台一列快照都沒有才標「沒有資料」。專案表裡，一台機器只會出現在它
     的專案清單（見「每天的快照」）有這個 root 的那幾張表；從沒建過 index 的專案不會出現這台機器。
- 原本草稿的「命令數掉超過一半」不列入：那是我猜的，使用者沒提過；它拿 7 天總數跟每日值的中位數
  比，單位不一致，放假就會觸發。用量表照樣顯示數字，由人看。
- 摘要：每台機器這段期間幾天異常、哪幾種；每台最後一次有資料是哪天；同名 host 警告。
- `--json`：同樣內容給 agent 讀。

### 歷史匯入

- 一次性指令，解析 claudecat `CORT-AUDIT.md` 長期指標表（53 列）。健康欄寫進快照表；**用量欄
  另存一張 host × 日的匯入用量表**（審查後補）：那些是整台機器的 30／7 天滾動值，放進以專案為 key
  的快照表會在每個專案重複一次。core/deep 只存在這張表。
- 舊列只有 host：`machine_id` 用 `imported:<host>`、來源記 `cort-audit-md`；2026-09-06 那列沒有 host、
  欄數也比較少，照它自己的欄數解析，記 `unknown`，**不參與活躍判定與缺列檢查**，只出現在歷史表格。
- 匯入值與新資料重疊時的優先順序見「統計：`report`」。
- 可重跑（upsert）。`project_id` 用 claudecat repo 的 origin（現有列全是 claudecat）。
- 09-27 口徑註記不搬（report 預設 7 天，不會跨到 09-27；註記留在 git 歷史）。

### claudecat 退場步驟

順序很重要：先讓 cort 接得住，才拆 claudecat。

1. cort 的同步、快照、report、匯入、搬家、排程子命令完成並測過。
2. 向 `armo` 申請量測用的新資料庫與憑證；每台機器用 cort 的量測 `configure` 寫入設定。
3. 移除 claudecat 的 09:17 與 09:29 條目（先停，之後才不會有新事件沒人送）。
4. 每台在用的機器（NUC11i5、Thinkpade15）用 `claudecat telemetry configure` 把 claudecat 指向
   **新資料庫**，跑最後一次 `claudecat telemetry sync`（審查後改）。這一步把每台本機 90 天內的事件
   以 claudecat 自己的算法寫進新資料庫，claudecat-only 來源也一起送完；NUC11i5 從沒同步過，這是它
   第一次有可對帳的遠端列。
5. 把 `harnessuse` 的 `telemetry_events` 與 alias 複製到新資料庫（`INSERT OR IGNORE`，可重跑），
   補上本機已刪掉、只存在 `harnessuse` 的舊事件。放在步驟 4 之後，claudecat 最後送的東西不會漏；
   之後按來源 × machine_id 對一次列數。
6. 每台機器跑 cort 同步的 `--dry-run`：重疊非空、配得上的 id 100% 相符。不相符就停在這裡，修 cort
   的算法；claudecat 都還在，什麼都沒壞。
6b. 在 NUC11i5 匯入 `CORT-AUDIT.md`，對一次列數。
7. 裝 cort 的排程。
8. **隔天檢查**：遠端每台機器都有昨天的快照、事件持續增加且沒有重複，才往下走。
9. 移除 `~/.cargo/bin/claudecat`。在這之前，回退方法就是把 claudecat 的排程裝回去。GitHub 上的
   claudecat repo 已於 2026-10-23 依使用者指示封存；封存後推不上去，之後 `CORT-AUDIT.md` 的新列只在
   本機，由步驟 6b 的匯入收進資料庫。
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
  一樣；第二版只再加壓縮列的計數與 id。不多不少：對話文字、指令內容、檔名都不上傳。

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
- 第二版（壓縮後的代價）的判斷：
  - 只看 Claude Code 主對話，子代理與 Codex／Kimi 不算。
  - 「已知檔案」含 Read／Write／Edit 類；重跑只算逐字相同；糾正用寫死的詞表，起始詞表如上。
  - 前後各 50 個 tool_use 的對稱視窗，一列存兩組計數，report 算後窗減前窗。
  - 以壓縮點 uuid 去重；`project_id` 用壓縮點那列的 `cwd`。
  - 使用者訊息：有 `origin` 看 `kind == human`，沒有的用舊列規則。
  - 水位一變就重算，不設結案期限；第一次 `record` 回填全部現存記錄。
  - 只上傳計數與 id，不上傳文字與檔名；明細只在本機看。
  - report 把這一節放最前面，預設 30 天、按週列趨勢。
  - 記錄解析放在 cort（排程跑的是 cort，`cort-evals` 不會安裝到每台）；`evals/` 現有的記錄解析
    （`evals/src/stream.rs` 等）能不能改成共用 cort 的，留給實作者。
- 以下是 2026-10-23 他審（實作者角度＋Codex 對照原始碼）後補的判斷：
  - `fresh` 改用 cort 自己的 staleness、拿掉 7 天門檻（依「天天誤報」反目標）。
  - 快照一天內每次排程都 upsert，判讀當天最後一次觀察。
  - 快照 `project_id` 用正規化 origin 雜湊，事件靠 alias 接上；加 `status`/`error` 欄；專案清單含
    過去 30 天記過的 root。
  - decline-top 用 `declines` 計數（跟匯入歷史同口徑），不是 census。
  - 匯入用量另存 host × 日的表；同一天有事件就用事件算的。
  - 缺快照以 host 為單位判斷。
  - 沒有對照的 machine_id 以前 8 碼顯示，不手動補。
  - 去重用每台、每來源的 rowid 水位；`--dry-run` 要求每台重疊非空。
  - 退場時 claudecat 最後一次同步直接寫進新資料庫，之後才從 `harnessuse` 複製舊事件。
- 測試：記錄與查詢走一層儲存介面；測試用本機假儲存（暫存目錄裡的 rusqlite，不是同步副本檔，
  不違反限制）自動測 upsert、異常規則、活躍判定、同名警告、push 失敗保留資料、離線第一行；
  event_id 用 claudecat 的現有案例做對照測試。真正的 Turso 同步在實機上驗。
