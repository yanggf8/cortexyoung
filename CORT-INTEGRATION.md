# cortexyoung/cort 整合（2026-09-05）

## 問題（使用者的提問）
「結合使用時，能利用 cortexyoung 的索引或其他資源嗎？」

## 答案
**能。** claudecat 直接唯讀 cort 的 SQLite 索引（不重造、不改寫），
用 cort 做「精準定位層」、claudecat 做「地圖與路線層」。

Markdown 文件現在是 claudecat 的另一個唯讀 sidecar：`navigate` 用 heading path 和行號
定位 README／設計文件／評估報告，再交給 `cort read` 讀取原文。這條路不寫 cort DB、
不產生 `impact` 邊，也不參與 coverage completeness 判定；真正的 cort 命中與使用仍由
cort 自己的索引與 `usage.db` 記錄。

## cort 提供什麼（schema v7）
| 表 | 內容 | claudecat 用途 |
|---|---|---|
| `projects` | project_id / name / path / git_head / last_indexed_at / extractor_version | `cort-status` 新鮮度 |
| `chunks` | file_path / symbol_name / chunk_type / start_line / end_line / content / language | `navigate --cort` 全量符號查詢 |
| `relationships` | source→target 邊（imports/exports/calls/**references**）+ call_site_line + call_form + confidence | 反向依賴路線（`dependents()` 不過濾 rel_type，references 邊自動吃得到） |
| `chunks_fts` | FTS5 external-content（content/symbol/file，tokenize unicode61） | `navigate --cort` 全文 fallback（symbol 未命中時） |
| `_cortex_meta` | key/value：`SCHEMA_VERSION` / `graph_pending` / `extractor_version` | `cort-status` / `cort-audit` 的圖重建狀態（見 2026-09-09 一節） |
| `file_state` | file_path / file_content_hash / `indexed_uncommitted`(v6) / `chunk_count`(v7) | `cort-audit` 的覆蓋缺口三分與未提交漂移警示（見 2026-09-09 下午一節） |

- DB 路徑：`$CORT_CACHE_DIR/<sha256>.db`，`project_id = sha256(real_path)`（0.40 rusqlite 唯讀開啟）
- cort 的 `last_indexed_at` 是 **epoch 毫秒**（13 位數）；freshness 以毫秒比對
  （曾誤當秒導致 7 天關卡失效，2026-09-06 修復，見下方複審）

## 誠實與邊界
- claudecat **唯讀** cort DB（`SQLITE_OPEN_READ_ONLY`）；永不寫入
- 唯讀雙保險：一般唯讀失敗（唯讀檔案系統、缺 -shm/-wal sidecar、寫入端持 EXCLUSIVE lock
  導致 query BUSY）時，自動退回 `immutable=1`（SQLite 完全不碰 sidecar/lock，直接讀主檔；
  代價是 cort 若有未 checkpoint 的 WAL 內容會讀不到——可接受的誠實取捨）
- cort 索引 STALE → `cort-status` 明確提示 `cort index`，不假裝新鮮
- cort DB 不存在 / 未命中 → `navigate` 自動回退 tree-sitter 地圖（並提示）

## 重驗（2026-09-05，換機 + 新版 cort 0.1.0，主要是 reindex）
- 換機後 claudecat 尚未被索引 → `cort index`（全量）：
  **68 files / chunks 551 / relationships 250 / fresh**（git HEAD 385bd7f349 相符，`index_is_stale: false`）
  （舊機數字 chunks=1150 relationships=2034 是新版 extractor 粒度不同，非回歸）
- 新版 schema 相容：claudecat 讀的 `projects / chunks / relationships` 欄位全部仍在；
  新增 `_cortex_meta / file_state / raw_edges / reading_notes / chunks_fts` 不影響唯讀查詢；
  `relationships` 新增 `call_form`、`confidence` 文字欄，claudecat 讀的 `rel_type / call_site_line / confidence_score` 不變
- reindex 行為驗證：`cort index --incremental` 在無基底時 0 examined（需先全量一次）；
  建立基底後 **hook-refresh 自動增量** — 編輯檔案後 chunks 548→551 自動更新、status 持續 `stale:false`
- 唯讀 fallback 修復（受限環境實測）：sandbox 把 `~/.cache` 設唯讀 + WAL DB 無 sidecar 時
  一般唯讀開不起來 → 新增 `immutable=1` fallback 後，`cort-status` / `navigate --cort` 在受限環境照常讀取；
  新測試 `cort_readonly_fallback_when_writer_holds_exclusive_lock` 以 EXCLUSIVE lock
  確定性重現「一般唯讀 query BUSY → fallback 仍讀到」，31+1=32 測試全綠
- 新 probe（舊證據 `rust/tests/staleness_cwd.rs` 已隨 V2 重寫消失）：
  `extractSymbolDefinitions`（legacy/src/core/ast-parser.ts:115）— tree-sitter **0 命中**、
  `navigate --cort` **1 命中** + 完整路線（cort context / impact）✅；反向依賴對新 schema 也通
  （persona-core `setup_conn` → 5 個 calls 依賴者）

## 實測（2026-09-05，cortexyoung 真實索引）
- `cort-status`：chunks=1150 relationships=2034，STALE（索引 HEAD 4042a7a6 ≠ 目前 7c2ad21d）✅ 誠實
- `navigate --cort "staleness"`：找到 tree-sitter top-30 找不到的符號
  （`rust/tests/staleness_cwd.rs:72`），路線含 cort context / impact / 反向依賴 ✅

## 對照（2026-09-06，`cort recall` / `cort context` vs `navigate --cort`）
- `cort context "immutable"` → `resolution=fts seeds=4`：cort 自己的 code 全文也是走 `chunks_fts`，
  且 4 個 seeds 與 `claudecat navigate --cort "immutable"` 的 4 筆 FTS 命中**完全相同** ✅
- `cort recall "immutable"` → `readings=0`：recall 是搜 `reading_notes`（需先有 read/note 進度），
  非 code 索引；**claudecat 的 FTS fallback 對應的是 `cort context`（resolution=fts），不是 recall**（文件已修正）
- 分工不變：`navigate --cort` 給路線 + 壓縮摘要（省 read）+ 反向依賴；
  `cort context` 給全量 content + call graph（icalls/ocalls/unresolved）——到達後的深挖層

## 再確認（2026-09-06，cortexyoung 又改版）
- 當時的索引仍為 v4 schema（`projects/chunks/relationships/chunks_fts` 欄位對 claudecat 零影響），
  `extractor_version` 同前一版；新增 `usage.db`（command_log）與 claudecat 無關
  （**後續**：cort 已於 2026-09-04 `ab1da4f4` 進到 v5，見 2026-09-09 一節）
- `chunks_fts` 確認為 **external-content FTS5**（`content=chunks, content_rowid=rowid`），
  唯讀 MATCH + JOIN chunks 實測可用 → 實作 FTS fallback（見下）

## 驗證迴圈：cort-audit（2026-09-06）
把「有沒有幫到 cort 的目的」變成**可收集、可追蹤的數據**（全部唯讀）：
- `claudecat cort-audit --root X [--window 30] [--json] [--track FILE]`
- 數據：①索引健康（fresh/HEAD 相符/age）②覆蓋缺口（file_state 有、chunks 無的檔案——
  這是 cort 自己 CLAUDE.md 開出的 completeness 缺口之一「a file the screen never read」）
  ③FTS 同步（chunks_fts docs vs chunks）④用量（cort usage.db：命令分佈、hook-suggest/
  refresh 結果、errors、index_stale、saved_bytes）
- 行動：報告尾「解讀 & 行動」依規則給建議；`--track` 每天一列寫 `CORT-AUDIT.md`（同日更新），
  時間序列驗證改善（如：未 chunk 檔數量是否下降、命中率是否上升）
- 首筆實測（2026-09-06，本 repo）：fresh、chunks=576 rels=294、FTS synced、
  **coverage 缺口 5 檔（legacy/test-*.js）**、hook-suggest 命中率 **35/6816（<1%）**、
  context+recall=3 —— 三條行動建議全部是有數據支撐的
- 附帶收穫：驗證過程抓出 `?1` 重複綁定參數的 rusqlite bug（`InvalidParameterCount` 被
  `unwrap_or(0)` 吞掉、假裝「無缺口」）→ 改 qmark 後正確回報 5 檔——正是「收集數據驗證」
  的價值，另有 3 支回歸測試保護

## 複審（2026-09-06 同日：數據正確性審查 → 兩個 P1 修復 → 三個 cortexyoung 議題）
逐項對照真實 DB 重驗首筆實測（多數可重現），審查抓到兩個 bug 並已當場修復（`03c509a`）：
- **P1：`cort-status` 的 fresh 關卡永不觸發** — `freshness()` 把 `last_indexed_at` 當秒，
  實際是毫秒（DB 實測 `1788686742481`）：40 天前的索引 `cort-status` 報 `fresh`、
  `cort-audit` 報 `STALE`，同一欄位兩套口徑。修復：共用 `is_fresh()`（`FRESH_WINDOW_MS`），
  附 40 天→STALE 回歸測試；上節「相容秒/毫秒」的說法一併修正。
- **P1：`--track` 靜默刪除表格後的內容** — `track_table()` 從 section 掃到 EOF 重寫，
  使用者筆記／其他 section 直接消失（實測重現）。修復：section 範圍改為「到下一個
  `#` 標題為止」，範圍外原樣保留；附 2 支回歸測試（含 explore+audit 雙 section 共存檔）。
  修好前 `CORT-AUDIT.md` 的時間序列其實不可信——這是先修它的理由。
- 數據覆核（直接 SQL 查 usage.db / 專案 DB）：
  - hook-suggest 命中 36/7056（0.5%），其中 `no_shape` 5857（83%）——最大槓桿（cortexyoung#3）
  - 報告裡 1032 筆 `unparsed` 全是 2026-09-01→02 的舊格式歷史列（`args_summary` 為字串
    `hook`）；09-02 起即為 `{"hook":…,"v":1}`——量測問題已自解，不需行動
  - `saved_bytes` 只在 `source=store && effective=receipt` 非零（cort `usage.rs:176-186`），
    30 天 11.4k 命令僅 1 筆非零（13 bytes）→「省了多少」目前實際沒被量到（cortexyoung#4）
  - coverage「缺口」5 檔全是 2–3 行、只 import 後呼叫的 driver script（無任何宣告）→
    extractor 沒漏，是指標語意問題（cortexyoung#2）；hint 文案已對齊（`0dc1599`）
- 修復後複驗：40 tests 綠（+3 回歸）、touched files clippy 0 warnings、兩個重現腳本行為翻轉
  為正確；`CORT-AUDIT.md` 同日列更新（11461 命令 / chunks 581）

## 待辦
- `navigate --cort` 命中時帶 cort `content` 摘要進路線（省一次 read）— ✅ 已做：
  路線加「內文摘要（省一次 read）」步驟（壓縮空白、截 220 字），
  `content_summary()` + 命中 route 帶上；實測 `with_readonly` 路線直接含函式簽名
- FTS 全文檢索 fallback（`chunks_fts`，與 `cort context` 的 fts resolution 同源）— ✅ 已做：
  symbol_name 未命中時查 `chunks_fts MATCH <"token1" AND "token2">`（token 加雙引號防
  FTS 運算子注入），JOIN `chunks` 還原 CortHit，路線標示「cort FTS 全文命中」；
  實測 `immutable`（content-only）4 命中、`"immutable" AND "fallback"` 也通
- 附帶修正：cort 命中合併後重算 exact（子字串 token 命中不再壓過精確命中）並重新排序，
  路線優先指向精確符號（`with_readonly` 優先於 `render_with_profile`）
- STALE 提示 `cort index --incremental` — ✅ 已隨新版 cort 解決（`cort status` 提供
  `index_is_stale`；`hook-refresh` 編輯後自動增量，不需 claudecat 再提示）

## 跟上 cortexyoung（2026-09-09；對照 cort `f61ecd00`）

上游自 2026-09-04 起的四項改動，逐條實測後只有一項要動程式：

- **schema v5**（`ab1da4f4`）：只是加寬 `relationships`/`raw_edges` 的 CHECK（新增
  `references` 邊、Rust type 存成 `chunk:class`），claudecat 讀的欄位全在，
  `cort-status` 實跑正常。文件與 `src/cort.rs` 的「v4」字樣一併更新。
- **`graph_pending`（要動程式，P1）**：cort 的 schema 遷移在 `db.rs:322` 設
  `graph_pending=1` 卻**不動** `git_head`/`last_indexed_at`；而增量索引雖然每個檔案都會先設 1，
  卻是在 `incremental.rs` **與時戳更新同一個 transaction** 裡清 0。
  所以外部讀到持續 `1` 只有兩種情況：①遷移後還沒跑過索引（時戳仍新 → claudecat 舊行為會
  **誤報 fresh**，而 relationships 是升級前的舊邊）②增量中斷（時戳沒前進，本來就 STALE）。
  修法（與 09-06 的毫秒事件同一原則：不假裝健康、也不假裝壞掉）：
  `index_info` / `audit_index` 同一口徑讀 `_cortex_meta`，
  `fresh = HEAD+age && graph_pending != Some(true)`；讀不到（舊版 DB／查詢失敗）→ `None`，
  **不翻布林**，另給一條「無法判讀」提示。日表不加欄，改讓既有 `fresh` 格分成
  `fresh` / `fresh?`（圖狀態未知）/ `STALE` / `STALE/graph`。三支回歸測試。
  claudecat **不硬編碼**期望的 SCHEMA_VERSION——那是 cort 自己的常數，寫死必然像文件一樣爛掉。
- **Java / AngularJS 1.x / HTML 索引**（`7227fd96`）：cort 端能力，claudecat 唯讀照吃
  （`navigate --cort` 對 Java 專案反而更有價值）。claudecat 自己的 tree-sitter 不跟進加
  `tree-sitter-java`：那是跟索引搶同一份工作。實際做的兩件小事：①`walk.rs` 第三欄語意
  正名為 `is_code`（只進 LOC/樹/key_files，不代表本地有 grammar；把 java 改成 false 會讓
  Java 專案整個從地圖消失，比「檔案在、符號空」更騙）②outline 對沒有本地 grammar 的
  key file 補一句「無本地 AST → 走 `navigate --cort`」（`symbols::has_grammar`，附測試；
  純 Rust 專案不出現這句）。`html/htm` 不進 `CODE_EXT`：只會灌 total_files/languages，
  進不了 key_files，也拿不到符號。
- **安裝／更新路徑改版**（`26fd3155`…`f61ecd00`，新 `cort-upgrade` 為正式更新路徑）：
  對 claudecat 零影響。`usage.db` 因此多了 `internal-shim` / `internal-ast-grep`，
  只灌 `total_commands`，**不進** hook-suggest 命中率分母（那是
  `max(by_command["hook-suggest"], Σoutcomes)`），`core`/`deep` 時間序列照舊可比 →
  日表分母**不動**（改了 09-06～09-08 的列就不可比，本檔已經有過一次不可比）。

## cortexyoung#3 複量（2026-09-09）：83% no_shape 不是一根槓桿

09-06 把 `no_shape` 5857（83%）記成「最大槓桿」。歸因上線後重量，**這個結論翻轉**——
資料源同前（唯讀 `~/.cache/cortex-ng/usage.db`，機器 `NUC11i5`，對照 cort `f61ecd00`）：

- **歸因這一半上游已做完**：`c290c383` 之後，2026-09-07 00:00Z 起的 `no_shape`
  **100% 帶 decline tag**（09-07 1130、09-08 839，untagged 0）；6447 筆沒 tag 的全部早於 09-07，
  是歷史列不是缺口。**這也是 claudecat 日表 `decline-top` 欄能開始有值的原因。**
- **全量標記窗口（09-07→09-09，hook-suggest 1989 筆／`no_shape` 1975）的拆解**：
  `not_a_search_tool` 1650（84%）、`pattern_not_symbol` 277（14%）——兩者都是設計要的沉默
  （`hook.rs` 的 `judge()` 只在「project source 裡的單一 bare symbol」開火）；
  其餘 `concrete_file_read` 21 / `unindexed_extension` 14 / `non_source_target` 10 /
  `target_not_source` 3 —— **可調表面 48 筆（2.4%）**，不是 83%。
- **命中率沒動**：該窗口 3 命中；30d 39/9849＝0.40%。歸因給出的答案是
  「流量本來就不是 symbol 形狀」，不是「規則太嚴」。
- **誠實限制**：`args_summary` 只記 decline tag、**不記 pattern**，所以能證明「哪條規則擋的」，
  不能證明那 277 筆 `pattern_not_symbol` 每筆都真的不是 symbol 查詢
  （`\bfoo\b`、引號、單項 alternation 都會落進同一格）。要回答得記 pattern 的**形狀類別**
  （不是 pattern 本身）。這是目前唯一可能還藏著靶心的地方。
- **沒人用過的維度**：payload 已是 v3，帶 `harness`——claude-code 1761／codex 217／kimi-code 11，
  **3 個命中全在 claude-code**；`pattern_not_symbol` 佔比 claude-code 14%、codex 11%（形狀問題看來與
  harness 無關，命中卻不是）。claudecat 的 `cort-audit` 已加上這個切面（每 harness：hook-suggest 數／命中／命中率／
  no_shape／top decline／refresh），並把兩件會說謊的事做成明文：①沒有 `harness` 欄的
  v3 前歷史列**另計**（不攤進任一 harness，否則加總悄悄對不上）②`harness_declared` 與實測
  不符的列數要看得見（實測 grok 481 筆宣告成 claude-code——按宣告值分群的歸因會被汙染）。
  30d 實測：claude-code 6604 筆 0.15%、codex 1435 筆 0.63%、grok 218 筆 0.92%、
  **kimi-code 106 筆 3.77%**（差 25 倍）——總命中率 0.4% 看不出這件事。
  切面只進報告、**不進日表**（加欄會斷掉跨日可比性）。

三個 issue 的現況（同日一併複量，已回貼 issue）：#3 見上；
**#4** 30d 17788 筆命令、`saved_bytes>0` 僅 1 筆 13 bytes（比 09-06 多量 6.3k 筆命令，結論不變，
不是短窗抽樣）；**#2** 上游 `rust/src` 未見對應變動（`coverage.rs` 的 `unindexed`/`scan_skipped`
是 recall 側另一張螢幕），claudecat 端仍是同樣 5 檔 `legacy/test-*.js`，5/5 是無宣告的 driver script。

## `unspecified` 是誰、以及上游同日進版（2026-09-09，對照 cort `606449c4`）

**查清 `unspecified`**（harness 切面上線當天就抓到的異常列：19 筆 hook-suggest、47% 命中）：

- 語意：cort 解析 harness 的順序是 **transcript_path（實測）> `--harness` 旗標（宣告）**，
  兩者都認不出來時 fallback 成 `unspecified`（`main.rs:846`/`1000`；`usage.rs:423-438` 把
  「v2 沒帶旗標」與「v1 沒這欄」視為同一種主張：沒人記錄它從哪來）。本機三個接線的 harness
  **全都帶 `--harness`**（`~/.claude/settings.json`／`~/.codex/config.toml`／
  `~/.kimi-code/config.toml`），所以 **agent 流量落不進這一格**。
- 實測 35 筆分兩叢，都是人工：①09-03→09-05 的探針節奏（`00:00:05` 與 `00:31:55` 各 6 筆、
  每叢 3 `hit`+3 `no_evidence`，suggest 的 `project_id` 全為 NULL；同叢 hook-refresh 打在
  project `e27912b0`＝cortexyoung 自己，時間正好是它提交 hook-refresh 修正的 09-05
  10:52/10:55/10:58）②09-09 02:08–02:10，與 `hook-install`/`internal-shim`/`status` 同秒交錯
  ——手動做安裝/升級驗證時打的。
- **影響**：30d 全域 hook-suggest 8976 筆、命中 40 筆，其中 **9 筆是 `unspecified`（22%）**——
  agent 實際命中只有 31 筆。處置：**分母不動**（動了跨日不可比），改在提示裡指名污染並
  給出扣掉後的那組（`cort_audit.rs`，附回歸測試）。
- 附帶解掉 `grok`／`declared=claude-code` 481 筆之謎：`~/.grok/hooks/` 是空的，grok 跑的是
  `~/.claude/settings.json` 那條（旗標寫 claude-code），cort 用 transcript 正確認回 grok
  ——**這是 cort 優先序規則在正常運作的證據，不是 bug**。

**上游同日又進 5 個 commit（`f61ecd00`→`606449c4`），兩個碰到 audit 的詞彙表：**

- `ff66ee59`：unindexed 專案裡「每 session 每目錄的第一筆」shaped search 改記成新 outcome
  **`no_index_hinted`**（並多送一次提示），其後仍照舊記 `no_index`。
  **更正**：本節初稿寫成「去重造成 `no_index` 下降」是錯的——**總量不變，只是拆成兩格**，
  而且只碰 `hook-suggest` 那條路徑。日表裡的大數字是 **`hook-refresh` 的 `no_index`**
  （09-08 415 筆、09-04 1032 筆），與此 commit 無關；`hook-suggest` 的 `no_index`
  本機每天不超過 15 筆。看圖時要用 (command, outcome) 兩個維度，只看 outcome 字串
  會把 hook-refresh 的起伏算到 hook-suggest 的改動頭上。真正的領先指標是
  `no_index_hinted`（提示實際被看到幾次），效果則應該落在 `index` 命令量與之後的 `hit`。
- `606449c4`：上游把 adopt-mine 的 baseline 凍結成 **product-only 1 injection / 0 adoptions**
  （2026-09-01→09-08），並記下 `--exclude` 要用 transcript 目錄名
  （`-home-yanggf-a-cortexyoung`，短名匹配不到、`excluded_sessions: 0` 就是徵兆）。
  **與本節同一類問題**：自測污染指標——他們排除的是產品樹的 session，claudecat 排除不了
  （usage.db 沒有 session 維度），所以走「指名污染、不動分母」這條。
- `09b2be8e`（unknown 命令記下被拒的內容）、`6568965d`（upgrade 的 binary component）、
  `ce6c53d6`（hook 文案改為動詞開頭）對 claudecat 無影響：前者只改 `unknown` 列的
  `args_summary`（claudecat 只解析 hook 列），後兩者不碰 usage.db 或索引 schema。

## cortexyoung#5：索引停在「從未提交、後來被還原」的版本（2026-09-09，循環自己抓到的）

**怎麼發現的**：09-09 的每日列把「未chunk檔」從 5 推到 6，多出來的是 `src/claude_md.rs`
——93 行、滿是宣告的 Rust 檔，不可能是「本來就沒宣告」那一類。這是覆蓋缺口這個指標
第一次抓到真東西（前 5 檔一直都是 driver script 的誤報形狀）。

**根因**：`file_state.file_content_hash` 記的是 `62710829…`（09-08 18:45），磁碟上是
`310163b6…`。那個 hash 屬於一個被改壞、**從未提交**的中間版本——當時 hook-refresh 依設計
就地索引了它（0 chunks）；之後用 `git checkout` 還原，而 `git checkout` 不是編輯工具，
不會再觸發 hook。於是 `cort index --incremental` 回報 `files_examined: 0`：
候選窄化只看 `git diff HEAD` 與 `git diff indexed_head..HEAD`，兩個都空。
`cort status` 全程報 fresh（HEAD 相符、樹乾淨、`graph_pending=0`）。

**確定性重現**（temp repo + `CORT_CACHE_DIR`，已附在 issue）：初次索引 2 chunks →
把檔案寫成非 Rust 內容再增量 → 1 chunk（unparsed）→ `git checkout --` 還原 →
增量 `files_examined: 0`、chunks **仍是 1**（應為 2）。觸發條件毫不特殊：
改檔 → hook 索引 → `git checkout` / `stash` / `reset` 還原，agent 每天都在做。

**處置**：
- 本 repo：跑全量 `cort index` 補回（6 檔 → 5 檔，chunks 627）。增量修不了它。
- 上游：開 [cortexyoung#5](https://github.com/yanggf8/cortexyoung/issues/5)，附重現與機制；
  建議的偵測是「`file_state` 有列、檔案存在、chunks 為 0」——一次查詢、不必 hash 全掃，
  也正是這次抓到它的那個訊號。
- claudecat：覆蓋缺口的提示文案原本只講一種成因（「實測 5/5 是 driver 檔」），
  現在**兩種成因並列**——第二種就是本節，解法是全量索引。指標本身不變。

**這條的意義**：`cort-audit` 的覆蓋缺口欄第一次不是在報噪音。它抓到的不是 extractor 漏抽，
而是**索引與磁碟長期不一致而所有健康訊號都說沒事**——正是這個 repo 一直在防的那種假健康，
只是這次假在 cort 那邊。

## 跟上 cortexyoung（2026-09-09 下午；對照 cort `d7c14bd1`）

上游當天四個 commit，兩個動到 claudecat 讀的 schema。全部逐條實測，不是看 commit 訊息推論。

- **schema v6（`f86a4d78`）`file_state.indexed_uncommitted`**：上一節 #5 的上游修復。
  hook-refresh 就地索引未提交內容時標記，增量會把它留在候選集裡直到內容與 git 一致。
  claudecat 多讀一欄 → `indexed_uncommitted_files`，大於 0 就在覆蓋段印警示。
- **schema v7（`09f55136`）`file_state.chunk_count`**：正是上一節「建議的偵測」，而上游給的版本更好——
  三態而不是布林：`0`＝掃過且無可 chunk 宣告（#2 的正確沉默）、`>0`＝有存下宣告、
  `-1`＝v7 前寫入且從未重寫（**未知，不是掃描結果**）。
  claudecat 新增 `not_chunked_scanned_empty` / `not_chunked_unknown` 與 `real_gap()`
  （三者任一 `None` 就回 `None`——少一個事實就不下結論）。
  實測本 repo：5 檔未 chunk、`chunk_count` 全是 0 → **真缺口 0**；那 5 檔經獨立覆核都是
  2–3 行、零宣告的 driver script，與上游欄位的判定一致。
  三天來那條「5 檔缺口」的曲線，第一次被證明整條都是正確沉默。
- **usage log `shape`（同 `09f55136`，issue #3）**：`no_shape` 列帶 `工具名|排序後的 top-level key 名`，
  不含 payload 內容。claudecat 新增 `no_shape_shapes` 排行（baseline 不混進來，與 `declines` 同口徑）。
- **`saved_bytes` 來源加寬（`3f1d3d96`，issue #4）**：**更正本檔 2026-09-07 那條**
  「只在 `source=store && effective=receipt` 非零」——ranged `read` 省下的檔案位元組現在也計入。
  我們報的數字因此不再等於「快取命中省下的量」；`src/cort.rs` 的 doc comment 已寫明。
  （那條當天為真，保留原文不改；更正寫在這裡。）
- **`d7c14bd1`** 純 README（upgrade note 補 v6/v7），對 claudecat 無影響。

### 這次自己抓到的兩個假數字

- **shape 排行的分母**：第一版用「30 天 actionable no_shape」當分母，但 `shape` 是當天 11:05
  才開始寫的，窗內只有 21 筆帶 shape。`12/6905 = 0.17%` 被 `{:.0}` 印成 **`0%`**，
  排行上每一條都是 0%——等於親手把唯一的行動靶心標成無關緊要。
  改成以「實際帶 shape 的列數」為分母（12/21＝57%），另印一行覆蓋率把 21 ÷ 6905＝0.3% 講明。
  樣本小是事實，該說出來，不是把比例稀釋掉。
- **既有 declines 段同一個病**：`{:.0}` 把 30、30、18、11 筆四個不同量級全印成 0%，
  那一欄不再提供排序資訊。改一位小數後是 3.9% / 0.3% / 0.3% / 0.2% / 0.1%。

### 長期指標表：加一欄，不換欄名

`未chunk檔` 原樣保留（它的 5/5/5/6 才是連續的），右邊新增 `真缺口`。
換欄名會讓 09-06～09-09 四列舊值坐在新語意底下，看起來像「缺口從 6 掉到 0」——
那正是這份文件一路在防的那種悄悄漂移。歷史四列的新欄補 `?`＝當時無法判讀
（舊 schema 沒有 `chunk_count`），不是 0。另加測試釘住 header／對齊列／`row_md()` 三者欄數一致。

### 每日分析腿

`cort-audit-analysis-prompt.md` 的 step 5b（當天上午才加的「逐檔開檔判形狀、比對 sha256」）
改成讀 `chunk_count` 三態。那條人工走法從開出 #5 到被上游一個欄位取代，相隔不到一天——
這是這條循環目前最快的一次往返。step 2 加上 `shape` 當規則需求的排序依據；
step 5 的升級指引改成先 `cort_upgrade --check`（binary 名是 `cort_upgrade`，底線）。

### 環境

`cargo install --path /home/yanggf/a/cortexyoung/rust --force` 一併裝上上游新的 `cort_upgrade`。
實跑 `cort-audit` 時 `indexed_uncommitted` 報 3 檔，就是這次還沒 commit 的
`src/cort.rs` / `src/cort_audit.rs` / `tests/claudecat.rs`——新警示行上線第一次就指著自己，
而且指得對：commit 前若 `git checkout` 掉其中任何一個，索引就會留在一個不存在的版本上。

## 跟上 cortexyoung（2026-09-10；對照 cort `33a1fa43`）

`d7c14bd1` 之後兩個 commit，都在 install/cli 側，**claudecat 讀的介面零變動**
（usage.db schema、cort DB schema、hook payload 欄位全部沒碰），程式不用改：

- **`d143c4c1` retire xgrep**：60 天本機語料 18 次 `xg`（0.03%）且全是開發期 probe——
  路由利基是空的，skill、`--with-xgrep`、pinned digests 全撤。claudecat 內本來就沒有
  xgrep 引用（`rg, ast-grep, cort` 三分法不含它），零對應。
- **`33a1fa43` shim 不再攔截 `--version`**：裝好的 shim 攔下 `--version` 印死字串、
  不 exec，所以 `install.sh --check` 唯一解析的那條命令永遠到不了 payload——
  shim 對 current/stale/missing 三種 payload 回答完全一樣，檢查不可能紅（設計規格
  §69/§267 早已標記此假設，這次修掉）。現在 binary 自己答 `--version`，
  `--check` 終於看得到 stale payload。本機 shim 已是兩行新形狀、`--check` 全 current
  ——修復部署已在這台機器跑過，不需重裝。
- **附帶抓到的操作陷阱（已寫進 `cort-audit-analysis-prompt.md` step 5）**：
  PATH 上 `~/.cargo/bin/cort_upgrade` 的 `repo_root()` 從 `current_exe()` 往上找樹，
  在 `~/.cargo/bin` 下永遠走到 `/home` 就 fatal——診斷/修復必須用樹內
  `rust/target/release/cort_upgrade`（且 target 可能比樹舊，先 `cargo build --release`）。
  這不算上游 bug（工具設計上就得在樹裡跑），是 prompt 照檔案名裸叫會踩的坑。

## 跟上 cortexyoung（2026-09-11；對照 cort `f8d3d3f4`，8 個 commit）

`33a1fa43` 之後 8 個 commit。部署側已就位：`~/.local/share/cortexyoung/cort/cort` 與樹內
release 同 hash，含到 `4b895589`；且 **`34e33a1d` 身分識別變更（CHUNKER_IDENTITY →
chunker-position/3）造成的全量重建已經發生**——本機所有索引的 extractor_version 都與
binary 一致、`cort projects --verdict` 回 compatible，chunker/coverage 三修正
（`1d54400b`/`34e33a1d`/`28a8ff2d`）已落到索引上。CORT-AUDIT 的 chunks/relationships
在重建後跳動（639→645/395→401）是塌縮修復的預期結果，不是數據品質事件。

claudecat 這次的對應（都有測試，67 全綠）：

- **census 窮盡分割（追 `6623113d`+`4b895589` `hook_census`）**：`audit_usage` 原本自己手工
  數 decline，口徑對不上——census 顯示 30 天有 `no_shape/decline_absent=6447` 與
  `unparseable_summary=1032`（2026-09-01 一天的 writer bug，args_summary 是字面 `'hook'`，
  任何 JSON 分析都看不到），手工法這兩塊都不入帳，declines 加總永遠 < no_shape 總數。
  現在 claudecat 自算同一套分割（`status_error` / `unparseable_summary` / `legacy_unsplit` /
  `no_shape/<decline|decline_absent>` / 詞彙內 outcome / `unknown/<hook>`），詞彙常數是
  上游 SUGGEST_OUTCOMES/REFRESH_OUTCOMES 的複製品——會漂，`unknown/` 桶就是跟丟時的訊號。
  報告的閉合檢查跨兩條獨立路徑（GROUP BY vs 逐列掃描）。與上游一個刻意的差異：
  args_summary 為 NULL 時上游整個 census 中止，claudecat 落 `unparseable_summary`——
  分割窮盡優先。舊的 suggest/refresh outcome 清單被 census 取代（`"unparsed"` 假鍵退休）；
  declines / no_shape_shapes / harness 切面不動（那是 claudecat 自己的切面）。
- **repair 三態（追 `f4ad4c7d`）**：從 `cort status` 的 `index_is_stale`/`rebuild_required`/
  `candidates_narrowed` 推導 none/refreshable/rebuild_required（鏡射上游 impact.rs 的
  `forbid_refuses`，純函式有測試）。claudecat 不自算——判定的輸入是 pack hash 比對，
  不在 DB 裡，自算必然是假的；binary 不在 PATH 或輸出不可判讀 → `?`。track 表不加欄
  （09-06 已證明加欄的代價），fresh 格覆寫在前：`STALE/rebuild`、`STALE/refresh`——
  本地 fresh 看不見 extractor/schema 變更，`34e33a1d` 正是那一種（當天索引已重建所以
  沒踩到，但下一個身分識別變更會踩到）。
- **probe declines + `--decline TAG`（追 `f8d3d3f4`）**：09-10 findings 的採樣死結解除——
  evals 的 hook-probe 現在自帶 parsed pattern 與全母體 declines census，不碰 transcript。
  且上游已裁決完 pattern_not_symbol：52,046 搜尋 / 4,418 筆 decline（80.6% 真文字搜尋、
  12.6% 宣告衣服、6.8% 批次 alternation、**0% 偽裝的單 symbol 查詢**），`9e725c76` 的剝皮
  規則把可確定是 symbol 的形狀收回了，剩下的沉默都是故意的。
- **`9e725c76` 本身**：claudecat 09-10 開的保守放行規則被上游收進 hook 本體——
  驗收線（issue #3 命中率 ≥5%）現在真的進入觀察期，kimi 的 3.77% 是第一個觀察點。

## 跟上 cortexyoung（2026-09-13；對照 cort `9820d9f5`，3 個 commit）

`f8d3d3f4` 之後 3 個 commit（09-12～09-13），全在 hook 寫入側與 upgrade 診斷，
**claudecat 讀的介面零變動**：usage.rs（census 口徑）沒動、command_log 沒有 schema 變更
（`project_id` 是既有欄位的賦值，不是新欄），程式不用改。詞彙常數已逐字核對：
SUGGEST_OUTCOMES(9)/REFRESH_OUTCOMES(8) 與 claudecat 的複製品一致，census 無 `unknown/` 漂移訊號。

- **`613b1ec7` probe-paid 列帶 project**：hook-suggest 的
  hit/hit_stale/no_evidence/no_index/no_index_hinted 一律在 verdict 前蓋 `usage.project_id`；
  no_shape 出口刻意不歸因——那些列沒付 probe 的錢，歸因要在 ~95% 的 fires 上多付一次
  canonicalize。evals 的 `adopt-mine` 報告也蓋 machine 章，跨機對帳不再靠記憶。
  對 audit 的意義：「hinted 的專案後來索引了沒」這條 funnel 追問現在 join 得動。
- **`15487664` target 比樹舊＝Drifted**：09-12 現場踩到的坑收進工具——樹前進而 target
  沒重 build 時，stale binary 的 hash 與同版安裝完全一致，`--check` 會把整台機器誤判成
  current。現在 binary 元件先比 build 與 rust/src/**（含 Cargo manifests）的先後：
  樹較新 → Drifted 並附 rebuild 命令；樹讀不到 → 不出訊號；內容比對只在 build 較新時才跑。
  分析 prompt step 5 的「先 `cargo build --release`」指引與此對齊。
- **`9820d9f5` refusal 帶名字與進度**：hook_row v3→v4，probe-paid outcomes 多蓋 `symbol`，
  `no_evidence` 另蓋 `why`（`leaf_in_index`＝bare leaf 其實在 chunks 裡、只是搜尋換了名字；
  `absent`＝任何形狀都沒有：extractor 缺口或本來就不是定義）。09-12 adoption recheck 找到
  3 天 10 筆無法追問的 no_evidence，之後每一筆都能對著檔案、其他專案的 chunks、
  receiver-gate 拒絕去比對。args_summary 的新鍵全是加法——claudecat 的 census 只讀
  hook/decline/shape/harness/harness_declared，v3 舊列沒有這些鍵、照樣落原桶，閉合不變。
  等本機 v4 列累積，no_evidence 的 why 分佈是下一個值得看的切面（目前樣本極小，先等）。

部署側：`rust/target/release/cort_upgrade --check` 全 current（23 個項目，含 `15487664`
新判準的自檢：target 與樹同步）。

日期口徑附記：track 表的日期是 `today_iso()` 的 **UTC** 日（dates.rs 寫明）。09:17 的
cron 在 +0800＝UTC 01:17，UTC 日與本地日一致，歷史列都沒問題；本地 00:00–07:59 之間
手動跑，列會掛前一天（2026-09-13 00:34 補跑的那列因此落在 09-12）。09-12 當天
09:17/09:29 兩個排程都沒有痕跡（機器當時沒開），該列即此補測。

## 跟上 cortexyoung（2026-09-14；對照 cort `5f6d5267`，1 個 commit）

`9820d9f5` 之後 1 個 commit：**query-time self-heal**——`impact`/`context` 回答前先把
index 養新（新 `rust/src/heal.rs` 的 `ensure_fresh`）。「index 是快取不是真相」：修復
不再派工給 agent 或 hook，由最便宜的那個 actor（正在查詢的 foreground 命令）自己做。
上游量測的動機：edit hook 7 天拒了 905 次重建、chain `cort index` 的建議 4 天只轉換
1 次，而全量管線要 1.4–2.3s。

- **heal 機制與 payload 詞彙**：綠路 payload 完全不加 key（byte-identical 契約），
  有話要說才帶 `self_healed`(bool)、`heal_mode`（`incremental`/`full`，僅 healed 時，
  來自 `incremental_index` 的 `stats.mode`）、`heal_ms`（僅 healed 時）、`heal_deferred`
  （僅 deferred 時，伴隨 `self_healed:false`）。deferred 理由六種：`upgrade_in_flight`/
  `heal_failed`/`no_cache_dir`/`background_already_running`/`spawn_failed`/
  `background_spawned`。claudecat 側的 `HEAL_MODES`/`HEAL_DEFERRED_REASONS` 是這份
  詞彙的複製品——會漂，採樣桶按實際字串落鍵，詞彙外照樣顯示自身字串。
- **P2 的 reindex 建議退役**：過去「STALE → 請 agent 自己 chain `cort index`」的
  建議鏈被查詢自癒取代。注意語意邊界：heal 的候選集仍以 git diff 為基底（v6 的
  `indexed_uncommitted` 標記檔會被推進候選集），#5 的真缺口在 `cort status` 眼裡
  是 fresh——「真缺口只能靠全量 `cort index` 修」這條仍然成立，退役的是 staleness
  側的手動 reindex 建議，不是覆蓋缺口的修法。
- **大樹 defer 到背景**：>2000 檔（`DEFAULT_HEAL_MAX_FILES`，`CORT_HEAL_MAX_FILES`
  可覆寫）不 inline 修——foreground 立即回答、`cort index --heal-background`
  單飛重建（子程序自己記 usage 列：`command='index'`、args_summary 帶
  `{"v":1,"heal":"background"}`）。
- **本機已就位**：`~/.local/share/cortexyoung/cort/cort` 17:35 換成 heal 版 binary、
  `~/.claude/skills/ast-grep` 17:39 同步。上游 README 的 `rebuild_required` 語意
  同步改寫：從「只有前景 `cort index` 會修」變「下一個 foreground query 自己修」——
  audit 的 `repair=none` 從此更常見，不得誤讀為沒有 staleness 發生過。
- **claudecat 側採樣對應**（都有測試，69 全綠）：`audit_usage` 同窗多掃
  impact/context 的 heal 欄位——`heal_scanned`（impact+context 分母）、self_healed
  按 mode 分桶、heal_deferred 按理由分桶（輕量 breakdown，不做窮盡分割）、
  `heal_ms` 合計/max、`heal_background`（背景重建次數）、`heal_legacy`（無 heal key
  的 5f6d5267 前歷史列，不混進新桶——「0 次自癒」與「還沒資料」分得開）、
  `heal_unparseable`（NULL／非法 JSON，照 hook census 對 unparseable 的態度入自己
  的桶）。報告在用量節新增「self-heal 採樣」兩行，零樣本也印。heal 當天 17:44
  才上線，首日實測 scanned=495 全 legacy、0 次自癒——零樣本屬預期。
