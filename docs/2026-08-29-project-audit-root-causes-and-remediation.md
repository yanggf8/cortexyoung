# cortexyoung 專案稽核：問題、根因與修正方案

日期：2026-08-29  
範圍：目前 `master`（稽核時 HEAD `ce7ea604`）  
性質：診斷與修正規格；本文件本身不修改產品程式碼

## 1. 執行摘要

`cortexyoung` 建置的是 `cort`：供 coding agent 使用的離線程式碼情報 CLI。它以固定版本的
`ast-grep` 作為唯一 parser，用 SQLite 保存 symbol chunks、全文索引、讀取紀錄與程式關係；不使用
embedding、雲端資料庫或常駐服務。

目前最有實證的價值是：

1. 從大型原始碼檔案只取出指定 function/method，避免整檔進入 agent context。
2. 保存已讀片段，未變更的重讀預設只回傳一行 receipt。
3. 對 TypeScript/JavaScript/Python 建立近似的反向呼叫圖，回答多跳 blast radius。

核心 Rust 測試與安裝測試大致健康，但發現兩項會直接影響交付正確性的 P0 問題：

- `index --incremental` 可能刪除跨檔 incoming edges，最後仍回報 `index_is_stale=false`。
- `install.sh` 在既有 release binary 存在時不重新 build，更新原始碼後可能繼續安裝舊 binary。

此外，CI、Rust graph 支援、評測 harness 與文件仍停留在不同演進階段，造成「測試本身很多，但
release gate 實際失效」以及「README 的部分主張無法由目前 repository 重現」。

## 2. 系統構成與資料流

```text
source files
    │
    ▼
ast-grep 0.45.2 + YAML rule pack
    │
    ├── chunks ───────► SQLite FTS ─────► context / struct
    │
    ├── raw call/import matches
    │        │
    │        ▼
    │   name-based resolution ──────────► relationships ─► impact
    │
    └── file hash/state ────────────────► staleness / incremental index

filesystem read ─► reading_notes + FTS ─► read / recall / receipt
```

主要入口：

- CLI dispatch：[`rust/src/main.rs`](../rust/src/main.rs)
- full index：[`rust/src/indexer.rs`](../rust/src/indexer.rs)
- incremental index：[`rust/src/incremental.rs`](../rust/src/incremental.rs)
- graph resolution：[`rust/src/graph.rs`](../rust/src/graph.rs)
- schema：[`rust/src/schema.sql`](../rust/src/schema.sql)
- extractor pack：[`src/pack/rules/`](../src/pack/rules)
- 安裝：[`install.sh`](../install.sh)

## 3. 稽核方法與現況證據

執行過的唯讀或暫存環境檢查：

```text
cargo test --all-targets                              PASS，213 tests
cargo clippy --all-targets --all-features -- -D warnings
                                                      PASS
cargo fmt --all -- --check                           FAIL，格式差異
bash tests/install-smoke.sh                           PASS，40/40
node --check evals/*.mjs                              PASS
```

以目前專案本身做完整索引：

```text
files=42
chunks=567
relationships=17
unparsed=1
wall≈0.95s
index_is_stale=false
```

以上證明核心功能可 build、可執行且有相當測試覆蓋；它不代表 incremental graph、release update 或
GitHub Actions 正確，這三者必須分別驗證。

## 4. Findings 總表

| ID | 等級 | 問題 | 直接後果 |
|---|---|---|---|
| F-01 | P0 | incremental reindex 刪除 incoming edges | `impact` 靜默漏 caller，卻顯示 fresh |
| F-02 | P0 | installer 以 binary 是否存在判斷是否 build | 更新後可能重新部署舊 executable |
| F-03 | P1 | CI 仍執行已刪除的 npm 專案 | push/PR gate 失效，Rust tests 未進 CI |
| F-04 | P1 | Rust pack 只有 chunks，沒有 call/import edges | Rust `context` 可用，Rust `impact` 幾乎無效 |
| F-05 | P1 | graph 評測 harness、gate 與文件斷線 | 成本與品質主張無法由目前 tree 完整重現 |
| F-06 | P2 | repository 未通過 rustfmt | 無法建立穩定 formatting gate，diff 噪音高 |
| F-07 | P2 | name-based resolution 與 common symbol 消歧不足 | 同名 symbol 產生 ambiguity、錯誤 hub 或截斷 |
| F-08 | P2 | 產品定位文件存在互相衝突的歷史結論 | 使用者可能把實驗性 graph 當成主要可靠能力 |
| F-09 | P3 | 零 rule 命中的檔案也被標成 `unparsed` | `status.unparsed` 與「解析失敗」混淆，誤導排障 |
| F-10 | P1 | 既有測試在平行執行下約 40% 機率失敗（**HEAD 既存**，非本輪引入） | 新 CI 的 `cargo test` gate 會隨機紅，等於沒有 gate |
| F-11 | P1 | `--allowedTools Bash(...)` 在 headless 下不約束 Bash，評測臂的「白名單即實驗組」只是標籤 | 兩臂 A/B 的基線臂實際用了 `grep`/`sed`，比較不成立 |
| F-12 | P1 | 評測器是 6 個 `.mjs`，違反「本 repo 純 Rust」契約，同時也是唯一沒有型別、沒有 lint、沒有 gate 的部分 | 已移植為 `evals/` crate，JS 全部移除 |
| F-18 | P2 | 一批 cell 是**全有或全無**：rows.json 只在所有執行緒成功 join 之後才寫 | 6 格批次只要最後一格被額度擋掉，前 5 格「真的跑過也真的量到」的資料就一起消失；補齊第二輪的批次越大，白燒的風險越大 |
| F-16 | P1 | `install.sh` 把 managed marker 寫成 SKILL.md 的**第一行**，frontmatter 被擠到第二行 | Codex/Claude 載入器判定 `missing YAML frontmatter delimited by ---` 並整個跳過該 skill：§13g 部署的 3 份引導全在盤上、0 份進得了 agent context；smoke 只驗「marker 存在」，53/53 綠燈照常漏 |
| F-17 | P2 | `cort-evals` 不驗證選項，未知選項（含 `--help`）被當成不存在 | `run-agents --help` 以**全套預設值**啟動 5 任務 × 2 臂的真實取樣，並準備寫進 `evals/runs/2026-08-30-graph`；`--out=x` 之類也會靜默退回預設目錄 |
| F-15 | P1 | 被額度拒絕的 cell 仍被寫成 `total_tokens=0 / coverage=0` 的測量值 | 第二輪 5 個任務裡有 3 個（6 格）根本沒跑，卻進入了聚合；已改為丟例外且不落 row |
| F-14 | **已修** | 假 parser fixture 是 46 行 Python，抵觸純 Rust 契約 | 已移植為 Rust bin `fake_ast_grep`（模式語法不變）+ 7 個自身測試 |
| F-13 | **已修** | `cort` 只靠 PATH 找 `ast-grep`，而 agent 的 Bash PATH 會被正規化 | 見 §13e：候選探測 + 版本優先 + probed/hint 錯誤；5 個回歸測試，端到端已證 |

## 5. F-01：incremental index 會產生 fresh-but-wrong graph

### 5.1 重現

建立兩個 TypeScript 檔案：

```ts
// callee.ts
export function target() { return 1; }

// caller.ts
import { target } from './callee';
export function caller() { return target(); }
```

初次 full index：

```text
relationships=1
impact target => caller at hop 1
```

只把 `target` 的 body 從 `return 1` 改為 `return 2`，執行 `index --incremental`：

```text
files_examined=1
files_reindexed=1
relationships=0
status.index_is_stale=false
impact target => dependents=0
```

函式名稱、位置和 caller 都沒有改變，但 incoming edge 消失。

### 5.2 根因

[`reindex_one_file`](../rust/src/incremental.rs) 的順序是：

1. `DELETE FROM chunks WHERE ... file_path = changed_file`。
2. 插入該檔案的新 chunks。
3. 只以該檔案的新 extraction results 重建 relationships。

`relationships.source_chunk_id` 和 `target_chunk_id` 都是 `ON DELETE CASCADE`，見
[`rust/src/schema.sql`](../rust/src/schema.sql)。刪除 callee chunk 時，SQLite 會正確地連帶刪除其他
檔案指向它的 incoming edges；但 caller 沒有列在 changed files，caller 的 raw edge 不會重新 extraction，
所以 incoming edge 永遠不會回來。

更深層的設計問題是：chunks 以「每檔可獨立 commit」管理，但 relationship resolution 是跨檔、跨
project 的 derived state。跨檔 derived state 無法只靠 changed file 的局部資料正確重建。

`foreign_keys` 是在 [`rust/src/db.rs`](../rust/src/db.rs) 顯式開啟的
（`pragma_update(None, "foreign_keys", "ON")`），所以 cascade 必定發生。實測重現後的 DB 裡
`relationships` 是**空表**，不是殘留孤兒列；而 schema 只有 `chunks`/`file_state`/`relationships`/FTS/
`reading_notes`，raw edges 只存在於 `chunker.rs` 的記憶體變數中，沒有持久層。也就是說這條邊被實體刪除後，
資料庫裡沒有任何留存資訊可以就地重建它。

staleness 的定義也要講準：[`rust/src/staleness.rs`](../rust/src/staleness.rs) 比的是「每個檔案的 extraction
hash vs `file_state`」。reindex 後 callee 的 hash 已同步、caller 內容根本沒變，所以在**它自己的定義下**
`index_is_stale=false` 是正確的。真正的缺陷是 staleness 只有 per-file 維度，沒有「derived 跨檔狀態是否
完整」這個概念，於是形成最危險的組合：per-file 新鮮、graph 殘缺、無法就地重建。

### 5.3 立即止血方案

在正確的 incremental graph 設計完成前：

1. 若 `git_candidates` 沒有 changed/deleted files，維持快速 no-op。
2. 只要有任何 changed 或 deleted file，直接呼叫 `full_index`。
3. CLI 回傳 `mode:"full"`，不要假裝完成 incremental。
4. README/skill 暫時不要指示使用者以 `--incremental` 修 freshness，或明確說它會 fallback full。

這會犧牲更新速度，但能恢復「fresh 代表可相信」的基本契約。

觸發邊界已實測確認：零變更、以及 hash 相同的 touched file 所走的 skip 分支**不會**刪除 chunks，edges
完好無損；只有「真的 reindex 了一個檔案」才會觸發遺失。因此止血的判斷條件就是 `changed`/`deleted` 非空，
快速 no-op 路徑可以原封不動保留。

### 5.3b 為何 213 個測試沒擋住

`rust/tests/` 中 `relationship` 只出現在 `db.rs`、`graph.rs`、`context.rs`；`incremental.rs`、`indexer.rs`、
`staleness.rs` 一次都沒有斷言 relationship。最接近的既有測試
`an_edited_file_is_reindexed_and_its_chunks_replaced` 改的正是 callee（`src/helper.ts`），fixture 裡也存在
`alpha -> helper` 這條邊，但它只斷言 `files_reindexed` 與 `helper.ts` 自己的 symbol 清單。觸發條件與測試
樣板都在，唯獨少了「unchanged caller 的 incoming edge 仍在」這條斷言。

### 5.4 長期正解

新增持久化的 unresolved/raw edge layer，例如：

```text
raw_edges(
  project_id,
  file_path,
  source_symbol/source_chunk_key,
  raw_target,
  rel_type,
  callsite_start_line,
  callsite_end_line
)
```

incremental 流程改成：

1. 在 transaction 外 extraction 所有 changed files。
2. 在同一個 project-level transaction 中更新 chunks、file_state、raw_edges。
3. 所有新 chunks 都存在後，以完整 raw edge set 重建 affected relationships；第一版可直接重建整張
   relationship table，證明正確後再最佳化 affected set。
4. 最後才更新 `git_head`、`extractor_version`、`last_indexed_at`。
5. 任一步驟失敗，保留上一個完整 snapshot；不可留下 fresh-but-partial graph。

若仍要保留「每檔 commit、interrupt 可保留進度」，必須加 staging generation 或 `graph_dirty` 狀態，且
所有 graph queries 在 dirty 時 fail closed。以目前產品規模，單一 project transaction 較簡單可靠。

### 5.5 必要驗收測試

- 修改 callee body，unchanged caller 的 incoming edge 仍存在。
- callee rename 後，舊 edge 消失、新名稱正確 resolution。
- 新增同名 target 後，confidence/ambiguity 正確重算。
- 刪除 target 後，不留下 orphan relationship。
- 同一次 incremental 同時修改 caller 與 callee，最終 graph 正確。
- 中斷更新後，要嘛完整保留舊 snapshot，要嘛所有 graph command 明確 fail closed；不可 fresh-but-partial。
- regression 必須走真正 `incremental_index`，不能只單測 helper 或手動 seed DB。

## 6. F-02：installer 可能安裝舊 binary

### 6.1 根因

[`install_cort`](../install.sh) 目前只在 `rust/target/release/cort` 不存在時執行
`cargo build --release`。但 README 建議的更新流程是：

```bash
git pull --no-rebase
./install.sh
```

`git pull` 不會清除 ignored 的 `rust/target/`。因此 source 已更新、舊 release binary 仍存在時，installer
會直接把舊 binary 複製到安裝目錄。shim 又固定回報 `0.1.0`，`--check` 只能看版本字串，無法發現 build
內容落後 source。

### 6.1b 實證與嚴重度界定

在 `/tmp` 的 repo 副本中、以獨立 `HOME` 執行：

1. 首次 install：沒有 build 日誌，部署 payload 的 SHA-256 與既有 `rust/target/release/cort` **完全相同**。
2. 修改 `rust/src/main.rs` 的一個字串常數（插入可字搜尋的 marker）後重跑 install：仍然不 build，部署 hash
   **不變**，`strings` 在部署後的 binary 中**找不到** marker → 實證會部署舊碼。
3. 對照組：移除 prebuilt binary 迫使 installer 走 build 分支 → 產出不同 SHA-256，且 marker **存在**於
   build 與部署的 binary → 證明 build 路徑本身可用，缺陷只在「以檔案是否存在取代 freshness 判斷」。

嚴重度界定（避免誇大）：在本工作副本中，release binary 目前**並未**落後任何 Rust source。唯一比 binary
新的檔案是 `src/pack/rules/typescript.yml`，而 pack 每次 install 都會重拷，所以 F-02 是已被實證的**潛在**
交付缺陷，不是當下正在錯版部署；但它會在任何一次 `git pull` 之後立即變成實際缺陷。

### 6.2 修正方案

每次 `install_cort` 都執行：

```bash
cargo build --release --locked
```

Cargo 自己會判斷 dependency graph 與 source freshness；沒有變更時成本很低，不應由 shell 以「檔案是否
存在」取代 Cargo 的 freshness 判斷。

同時建議：

- `Cargo.toml` 版本與 `CORT_VERSION` 由單一來源產生，避免手動雙寫。
- binary 自己支援 `cort --version`，不要只由 shim 假報版本。
- manifest 記錄 installed binary SHA-256 或 build revision，讓 `--check` 能檢查內容而非只看 `0.1.0`。
- release build 一律使用 `--locked`。

### 6.3 必要驗收測試

- 先 build/install，再修改一個會改變可觀察輸出的 Rust source，第二次 install 必須部署新輸出。
- source 未變時重跑 installer 仍 idempotent。
- `--check` 能區分同版本字串但 binary hash 不同的 stale install。
- pack 更新與 binary 更新都能部署，不互相掩蓋。

## 7. F-03：CI 還在執行已刪除的 Node 專案

### 7.1 根因

Rust cutover 已刪除 `package.json`、`package-lock.json`、JS runtime 與 JS unit tests，但
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) 仍執行：

```text
npm ci
npm test
```

因此 workflow 會在真正的 Rust tests 之前失敗，而且 `cargo test`、Clippy、rustfmt 完全沒有成為 PR gate。
大量本地測試存在，不能補償 remote gate 沒有執行它們。

### 7.2 修正方案

CI 至少拆成以下步驟：

```text
1. checkout
2. 安裝/選定 Rust toolchain，含 rustfmt、clippy
3. 安裝 pinned ast-grep 0.45.2
4. cargo fmt --all -- --check
5. cargo clippy --all-targets --all-features -- -D warnings
6. cargo test --locked --all-targets
7. bash -n install.sh tests/install-smoke.sh
8. shellcheck 自有 shell scripts
9. bash tests/install-smoke.sh
10. node --check evals/*.mjs（若保留 Node 評測工具）
```

Ubuntu/macOS matrix 可以保留。Node 不再是產品 dependency，只是 eval scripts 的 syntax/runtime dependency。
另外，`ludeeus/action-shellcheck@master` 應改成固定 release 或 commit SHA，避免 CI supply-chain 漂移。

### 7.3 驗收條件

- clean clone 在 Ubuntu 與 macOS 都通過。
- 任意製造 Rust compile error、Clippy warning、format diff 或 installer smoke failure，都能讓 CI 變紅。
- workflow 不再引用不存在的 package、test 或 `node_modules`。

## 8. F-04：Rust 能切 symbol，但沒有 dependency edges

### 8.1 現況

[`src/pack/rules/rust.yml`](../src/pack/rules/rust.yml) 只有：

- free function chunk
- impl method chunk
- trait default method chunk

沒有 `edge:calls`、`edge:imports` 或 Rust module/use extraction。這足以讓：

```text
cort context main --content full
cort context Type::method --content full
```

回傳單一 Rust symbol body，但無法建立 Rust caller graph。實測本專案中 `main` 直接呼叫
`context_command`，`impact --symbol context_command` 仍為 `dependents=0`。

直接查本專案自己的索引 DB，數字更明確：

```text
chunks by language:       Rust 545 | JavaScript 21 | NULL 1
relationships by source:  JavaScript calls 17   （Rust = 0）
Rust chunks with any outgoing edge:  0
edges come only from:     evals/*.mjs
```

法則也一致：`src/pack/rules/` 中只有 `typescript.yml`、`tsx.yml`、`javascript.yml`、`python.yml` 定義
`edge:imports` / `edge:calls`；`rust.yml` 只有 3 條 chunk 規則、0 條 edge 規則。所以「Rust 能用 `context`
但 `impact` 無效」不是猜測，而是規則層面的必然結果。

### 8.2 根因

Rust 支援是依「大型 Rust 檔案 symbol slicing」的真實需求加入，沒有完成 graph extraction；但 CLI 與
README 對 `impact` 的描述是通用的，沒有清楚限定語言能力。這是 capability/documentation mismatch，
不是 SQLite walk 本身的錯。

### 8.3 解法選項

選項 A：明確縮小產品契約（建議短期採用）

- Rust：支援 `context/read/recall/struct`；不宣稱 `impact` 完整。
- TypeScript/JavaScript/Python：支援近似、name-based `impact`。
- `status` 增加 indexed capabilities/languages，或 `impact` 對沒有 edge rules 的語言給 warning。

選項 B：完成 Rust graph

- extraction `call_expression`、qualified calls、method calls。
- extraction `use_declaration`、modules 與 aliases。
- 將 `foo::bar`、`Type::method`、`obj.method` 正規化到可 resolution 的 symbol identity。
- 處理 trait dispatch、宏生成、generic/associated functions；無法靜態確定時標成 ambiguous，而非假裝
  exact。
- 補跨 module、alias、同名 free functions、同名 methods、trait/default/impl collision 測試。

Rust 已有 compiler/rust-analyzer 可提供比 name graph 更精確的資訊，因此完成 Rust graph 前要先證明 agent
場景真的需要；不能只因 parser 可抽取就擴大範圍。

## 9. F-05：評測 harness 與目前產品斷線

### 9.1 問題清單

1. `evals/run-agents.mjs`（現為 [`evals/src/arms.rs`](../evals/src/arms.rs)） 已改用 Rust `CORT_BIN`，但 `cort_calls` 仍以
   `command.includes('cort.js')` 計數，因此 Rust arm 的 cort calls 會被記成 0。
2. [`evals/README.md`](../evals/README.md) 還要求執行已刪除的 `bin/cort.js`。
3. README 引用的 `evals/relation-cost.mjs` 已不在 repository，決定性成本數據不能依目前 tree 重跑。
4. 新 `run-agents.mjs` 產生兩臂 rows，但舊 `evals/run-eval.mjs`（現為 [`evals/src/summary.rs`](../evals/src/summary.rs)） 仍是三臂
   `rg+Read / ast-grep+Read / cort` gate，欄位還包含新 rows 沒有的 `stale_reads`；兩者未真正接線。
   實測餵入 runner 形狀的 row 之後，`summarize()` 的 `stale_reads` 變成 `NaN`、經 JSON 序列化成 `null`，
   而且不丟例外。也就是說文件承諾已修掉的「null 指標」失效模式，在 summary 路徑上仍然存在。
5. graph-required end-to-end agent cells 尚未執行；README 已承認這一點。
6. `docs/2026-08-28-end-to-end-eval-wip.md` 是歷史 WIP，仍寫 runner 不存在與 JS binary 路徑，未清楚標示
   已被後續結論取代。

### 9.2 根因

產品在短時間內經過 JS MVP、graph 成本重分析、真實 session 分析、Rust port 與 read-receipt 優化；每一
階段留下自己的 runner、gate 和文件，但 cutover 只移植產品核心，沒有同步建立「哪份評測是 canonical」
及 archive policy。

### 9.3 修正方案

- `cort_calls` 依 `CORT_BIN` 的實際 command prefix 計數，不搜尋已刪除檔名。
- 為 `estimateTokens`、`parseStream`、`gradeAnswer`、`buildPrompt/buildArgs/buildRow` 恢復 Node tests。
- 統一 canonical row schema；所有 gate 欄位寫入前必須為非 null，未提供的舊欄位移除或正式補回。
- runner 完成後直接輸出 `rows.json` 與 canonical `summary.json`，不要依賴另一個不相容的 summarizer。
- 恢復 `relation-cost.mjs` 或撤下 README 中無法重現的命令與精確數字；兩者不可並存。
- 每份歷史文件加 `status: current/superseded/archival` 及 successor link，保留研究歷史但不讓它冒充現況。
- eval artifact 必須記錄 binary hash、pack hash、venue HEAD、index freshness、model、tool whitelist 與
  estimator version。

### 9.4 驗收條件

- synthetic transcript 中一次 Rust cort command 必須得到 `cort_calls=1`。
- missing metric 必須讓 row build 失敗，不能寫 `null`。
- cort arm 越界使用 `rg` 時，permission denial 必須出現在 row 並使該 cell 無效或明確標記。
- README 中每一條評測命令都能從 clean clone 執行，或清楚標示需要的外部 venue/credential。
- summary 的 gate input 與 runner row schema 有單一測試鎖定。

## 10. F-06：rustfmt 未納入完成定義

### 10.1 現況與根因

`cargo fmt --all -- --check` 對多個 source/test files 回報格式差異。Clippy 與 tests 通過，表示這主要是
工程 hygiene，而非 runtime correctness；但它也證明目前沒有有效的 formatting gate。

Rust port 期間以功能與 parity 為主要驗收，CI 又仍停留在 npm，因此 rustfmt 沒有被任何自動流程執行。

### 10.2 修正方案與驗收

1. 單獨 commit 執行一次 `cargo fmt --all`，不要與行為修正混在一起。
2. CI 加 `cargo fmt --all -- --check`。
3. CLAUDE/AGENTS 開發指引加入 fmt + clippy + test 順序。

驗收為 clean tree 上 fmt check exit 0，且 formatting-only commit 不改變測試結果。

## 11. F-07：name-based resolution 與消歧限制

### 11.1 現況

[`resolve_targets`](../rust/src/graph.rs) 先按 `symbol_name` 查詢，再依 same-file/imported path prefix 縮小；
無法縮小時會把全 project 所有同名 symbol 都當候選。這在 common helper（例如 `main`、`run`、`logInfo`）
上容易形成 ambiguity 或不相關 hub。

實測 `context main` 在本專案得到兩個 exact-symbol seeds，受預設 token budget 影響只顯示部分內容。Rust
method 已用 `Type::method` 改善，但 free functions 仍沒有 `file::symbol` 查詢契約。

這項限制已部分記錄在 README，屬已知近似，不是像 F-01 那樣的回歸；問題是 agent 很容易把「圖中的
候選」誤讀為「編譯器級的唯一依賴」。

### 11.2 解法

短期：

- CLI/lean header 明確顯示 ambiguous seed count。
- common unqualified name 多 seed 時，預設 fail closed 或要求 `--file`/qualified query，而非靜默 budget
  截斷。
- skill 維持「深層結果是 candidates，需 spot-check」的語意。

長期：

- symbol identity 加入 module/file/owner，不以 display name 作唯一 resolution key。
- import map 保存 imported binding、alias 與來源 module，而不只保存 module path。
- 支援 `file_path::symbol` 或 `--file` 消歧。
- relationship row 保存 call-site line 和原始 callee，讓 agent 可低成本驗邊。

## 12. F-08：產品定位應以已證明價值為主

Repository 內同時存在以下歷史結論：

- 早期三臂評測：graph 沒有贏，應 STOP。
- 成本重分析：原評測量錯 metric、任務不需要 graph，決定性 probe 顯示多跳 graph payload 很小。
- 真實 session 分析：1,565 個真人 prompts 中沒有 code relationship graph 需求；真正成本是找檔與讀大檔。
- Rust 實測：symbol slicing 與 repeat receipt 可把大型檔案 payload 降低約 99% 以上。

這些結論並非完全互斥，但 README 若只強調「relationship tool」，會掩蓋最有真實資料支持的能力。

建議 canonical positioning：

> `cort` 是離線、agent-oriented 的 symbol slicing 與 reusable-reading CLI；另提供適合明確多跳問題的
> 近似 relationship graph。字串搜尋仍使用 `rg`，compiler-supported 語言的精確變更驗證仍使用 compiler/
> language server。

對外能力分級：

```text
Stable:       index, context(symbol), read, recall, receipt, status
Supported:    struct（受 ast-grep pattern/語言限制）
Experimental: impact graph（先限 TS/JS/Python，name-based，需驗邊）
Deferred:     rewrite, modules, watch, diff impact, embeddings
```

## 12b. F-09：零 rule 命中被當成解析失敗

### 現況與根因

本專案索引回報 `unparsed=1`，查證該檔是 [`rust/src/lib.rs`](../rust/src/lib.rs)。它不是語法錯誤：直接跑
`ast-grep scan --config src/pack/sgconfig.yml rust/src/lib.rs` 得到 exit 0、stdout 0 bytes（lib.rs 只有
`pub mod` 宣告，沒有任何 chunk 規則命中）。

但 [`rust/src/chunker.rs`](../rust/src/chunker.rs) 的 `extract_file` 把三種情況都導向同一個
`unparsed_result`：timeout、`r.code != 0`、以及 `parsed.records.is_empty()`。第三種就是本案例。於是：

- `status.unparsed` 把「這個檔合法沒有符號」計入「解析失敗」，數字失去診斷意義。
- 這類檔案只能得到 `chunk_type='unparsed'` 的整檔 chunk，永遠無法成為 relationship 的 source。

### 影響

低。功能上是偏保守的安全行為（整檔仍可 FTS 搜尋），但對使用者與後續量測有誤導性；任何引用 `unparsed`
的文件都必須說明它包含「零 symbol 檔案」。

### 解法

把兩種狀態分開，並保持既有失敗語意不變：

- 保留 `unparsed` 只代表 timeout / 非零 exit / 全 malformed；零命中另計（例如 `status.no_symbol_files`，
  或讓 `chunk_source` 多一個可區分的值）。
- 測試：只有 `pub mod` 的 Rust 檔不得計入 parse-failure；真正不可解析的檔仍計入。

## 13. 已知限制：接受、改善或修復

| 限制 | 分類 | 建議 |
|---|---|---|
| 新 untracked/未保存內容在索引前不可見 | 可接受契約 | header 顯示 stale；fresh edit 優先 `rg` |
| `chunk_id` 隨 symbol 起始行移動 | 可接受但可改善 | 未來以 file + qualified symbol + body identity 做穩定 ID |
| `context` 是 FTS，不是 semantic search | 可接受契約 | 不宣稱 semantic；定義搜尋先用 `rg` |
| FTS bare `unicode61` 會拆 `.`, `_`, `$` | 可接受但需文件化 | SQLite 支援後再改 tokenizer，補 migration/test |
| name-based target resolution | 產品風險 | 見 F-07；結果是 candidates，不是 compiler verdict |
| const-bound wrapper 規則不涵蓋所有 expression | 可接受語言覆蓋限制 | 增加 fixture 前不可擴張 claim |
| Rust 無 call edges | capability gap | 見 F-04；短期明確限定，長期依需求決定是否實作 |
| incremental graph 會遺失 edge | 不可接受 defect | 見 F-01；發布前必修 |

## 12c. F-10：既有的平行測試 flake

### 發現過程

修完 F-01 後整輪 `cargo test` 出現一次 `171 passed / 1 failed`（其餘 binary 因失敗而中止）。
連跑 10 次 `--test staleness` 得到 3 次失敗；為確認不是本輪引入，用 `git archive HEAD` 在 `/tmp`
重建一份**未含任何改動**的基線，跑 20 次得到 **10 次失敗**。也就是說這個 flake 在 HEAD 就存在，
只是過去 CI 跑的是 `npm test`，這批 Rust 測試從未被 gate 執行過（遠端記錄：`ce7ea604`、`1eaad63f`
兩個 master push 都在 12–14 秒失敗）。

### 根因

`a_dirty_but_semantically_identical_file_is_not_stale` 會失敗，是因為同一測試行程裡，
`staleness_is_computed_against_projects_path_not_the_cwd`（C2-22）直接呼叫
`env::set_current_dir`。工作目錄是**行程級**狀態，libtest 預設把同檔的測試丢給多個執行緒共用；
而 `Command::spawn` 繼承呼叫端的 CWD，所以兄弟測試在 CWD 被移走的瞬間 spawn 出的 `ast-grep`
子行程可能解析到不一樣的掃描結果，兩邊 `file_content_hash` 基準不同 ⇒ 隨機判定為 stale。

證據：`--test-threads=1` 時 10/10 通過，`--test-threads=6` 時 4/10 失敗。

### 修正

把 C2-22 拆成獨立 target [`rust/tests/staleness_cwd.rs`](../rust/tests/staleness_cwd.rs)。cargo 是
**串行**執行不同 test binary 的，因此改 CWD 的那個測試不再與任何測試同行程；檔案頭寫明了不得合併回
`staleness.rs`。6 個 staleness 測試一個都沒少（5 + 1）。

修正後：`--test staleness` 連續 20 次 **0 失敗**；`cargo test --all-targets` 連續 3 次 218/218。

### 殘留風險（未處理，記錄之）

`pack.rs`、`db.rs`、`context.rs`、`struct.rs`、`chunker.rs`、`cli.rs` 也會改 `CORT_PACK_DIR` /
`CORT_CACHE_DIR` / `HOME` 等行程級環境。`db.rs` 與 `pack.rs` 自帶 mutex，但這只在**同一 binary 內**的
爭用有效，且依賴每個測試都確實取鎖。後續若再出現罕見 flake，優先懷疑這條路徑；更根本的做法是把這些
環境依賴改為顯式參數注入（較大重構，暫不在此輪）。

## 13f. 端對端 graph 評測：第一批可信數據（2026-08-30，10 cells）

5 個 graph-required 任務 × 2 臂，全部由 `cort-evals run-agents` 跑出，`--strict` 驗證過
`metrics_missing` 為空（這是前三輪做不到的：那時 `tool_return_tokens` 與 `read_calls` 30 格全 null）。
venue 為 cct `86a5ee6`，隔離 `CLAUDE_CONFIG_DIR`，索引由目前 release binary 建立（`stale=false`）。

| 任務 | 臂 | coverage | precision | turns | tool 回傳 | 會話 total | `arm_held` |
|---|---|---|---|---|---|---|---|
| transitive-chain-lastntradingdays | cort | 1.0 | 1.0 | 3 | 336 | 85,510 | true |
| transitive-chain-lastntradingdays | shell | 0.75 | 1.0 | 10 | 2,627 | 307,070 | false |
| route-blast-radius-reportsstatus | cort | 1.0 | 1.0 | 3 | 431 | 86,018 | true |
| route-blast-radius-reportsstatus | shell | 0.75 | 0.2 | 21 | 9,809 | 476,138 | false |
| storage-blast-radius-backtesting | cort | 1.0 | 1.0 | 3 | 576 | 86,563 | true |
| storage-blast-radius-backtesting | shell | 1.0 | 1.0 | 13 | 2,728 | 324,173 | false |
| hub-blast-radius-loginfo | cort | 1.0 | 1.0 | 3 | 1,325 | 91,214 | true |
| hub-blast-radius-loginfo | shell | 0.886 | 0.848 | 20 | 14,783 | 683,286 | false |
| blast-radius-3hop-getcurrenttimeet | cort | 1.0 | 1.0 | 8 | 2,176 | 250,224 | **false** |
| blast-radius-3hop-getcurrenttimeet | shell | 0.957 | 0.88 | 21 | 8,387 | 573,203 | false |

聚合并透過 gate（`cort-evals summarize --strict`）：

```text
cort      runs=5 success=1.0  mean_total=119,905.8  mean_tool_return=968.8   mean_turns=4.0
rg+Read   runs=5 success=0.4  mean_total=472,774.0  mean_tool_return=7,666.8 mean_turns=17.0
verdict: baseline_arm=rg+Read, cort_beats_ast_grep=true → continue to deferred features
```

也就是 spec §8 那個從未被滿足的條件，第一次在**有度量**的前提下成立：cort 臂 5/5 全對、會話成本
約 3.9 倍便宜、工具 payload 約 7.9 倍小；基線臂 2/5 過 gate（三次是漏符號、兩次是混入不相關符號）。

### 這批數據不能說什麼（必須一起讀）

1. **基線不是「rg+Read」，是「agent 的整個 shell」。** 6/10 格 `arm_held=false`；連 cort 臂都有一格
   在 cort 之外又 `grep` 了三次來自我驗證（`arm_held=false`）。所以這是「cort vs 現實中 agent 會做的事」，
   不是「cort vs rg」。想量後者需要能真正束縛工具的 driver。
2. **每格 n=1。** 單一 seed、單一模型（opus）、無重複；前三輪已證明單一格子會被 agent 行為變異主導
   （曾出現 171 turns 的格子）。要拿它當決策依據，至少需要 3 次重複取中位數。
3. **成本是名目等值。** 10 格 `cost_usd` 合計 $5.209，但憑證是訂閱（claudeAiOauth），實際付出是額度與
   速率限制，不是帳單；且 7 天窗口在量測時已用掉 85%。
4. `stale_reads` 仍無人量測（因此已從 `METRICS` 移除，不再讓 `--strict` 永遠失敗）。
5. 標竿是「標籤 = cort 自家產出後再逐條對原始碼驗證」的圖；它排除了捏造，但**不是**編譯器級真相，
   所以 coverage 1.0 的意義是「與已驗證的圖一致」，不是「不漏任何間接影響」。

資料以 metrics-only 形式保存在 `evals/runs/2026-08-30-graph/`（不含 transcript，避免把 venue 的原始碼
片段帶進本 repo）。

## 13n. 第二輪樣本完成：n=2、20 cells，以及對 §13i 判讀的三點修正

額度窗口重置後補跑，一次呼叫 `--only a,b,c`（F-18 的落盤語意在這裡派上用場：6 格全寫入，
`run-status.json` 回報 `complete: true, planned_cells: 6, written_cells: 6`）。資料在
`evals/runs/2026-08-30-graph-sample2/`（10 cells，metrics-only；transcript 留 `/tmp`，裡面有 venue
原始碼片段）。`cort-evals summarize` 吃兩份 rows.json（20 cells）：

| 臂 | success | mean total_tokens | mean turns | mean tool-return tokens | runs |
|---|---|---|---|---|---|
| cort | 1.0 | 122,142 | 4.2 | 991.8 | 10 |
| rg+Read | 0.4 | 482,859 | 16.8 | 7,642.3 | 10 |

verdict 仍是 `cort_beats_ast_grep=true`。名目成本：第 1 輪 $5.209、第 2 輪 $5.497（cort 側各佔 27%
／27%）。三點必須修正 §13i 當時只看到 4 格所下的判斷：

1. **「基線兩次都漏同一個符號」從 2 個任務變成 5 個任務的現象。** 10 組（task×arm）的 `success`
   兩輪**全部一致**：基線在同樣 3 個任務失敗（hub-loginfo、route-reportsstatus、transitive-chain），
   coverage 逐輪 0.886/0.886、0.75/0.75、0.75/0.75，一格格重覆。n=2 證到的不是「答案會漂」，
   而是「錯得可重覆」——這對「graph 值不值得上」比平均 token 有意義得多。
2. **倍數沒變差，但逐格变异仍在。** 3.95× token、4.0× turns（第一輪是 3.8×／3.9×）。cort 單格变异
   最大的是 route（86,018 → 176,695）與 3hop（250,224 → 183,937）；基線同方向（476,138 → 670,928）。
   所以「便宜多少」這種說法只能用均值，且要連變異量一起講，不能講成一個倍數保證。
3. **`arm_held` 不是正確度的自變數，至少這批資料不是。** 第 2 輪 cort 10 格裡有 2 格 `arm_held=false`
   （route 與 3hop，而且兩輪都 false），它們照樣 `success=true`、coverage 1.0。受控與否目前看到的是
   影響成本（agent 自己 grep 查證），還沒證據說它影響對錯。把 `arm_held` 讀成正確度前提會把之後的
   引導修正帶到錯方向。

邊界照樣講明白：n=2、單一 venue（cct，TypeScript Worker）、單一基線臂、jail 未啟用（F-11）。
n=2 時中位數沒有意義，因此這裡只報均值與逐輪觀測值，不報「中位數」。

## 13l. F-18：一批 cell 不該是一整顆蛋

「把被擋的 3 個任務補回來」現在是一次呼叫（`--only a,b,c`）。但 F-15 之後的 runner 有個相反的性質：
它**刻意**fail closed——任何一格出錯，整個 run 就 `return Err`，而 `rows.json` 是在所有線程 join 成功
之後才寫。6 格一批的含義因此是：**最後一格被擋，前面 5 格白跑**。批次越大，白燒的窗口越大；
這與「不要讓沒跑的格子混進資料」是對的，但用的是同一個開關把量到的格子也一起丟掉。

修法把兩件事拆開：

- **落盤时机**：每格一旦 `build_row` 成功就重寫 `rows.json`（先排序再整個覆寫，因此線程誰先完成
  不影響檔案內容，同一批重跑仍是可 diff 的同一份資料）。
- **批次是否完整**：`run-status.json` 由一個 `Drop` guard 寫，涵蓋包含提前 `return Err` 在內的
  所有出口，內容是 `planned_cells / written_cells / complete`。少了 2 格的批次會自己講：
  「2 of 6 planned cells are missing … Do not read this batch as complete.」

邊界同樣釘住：0 planned 且 0 written 是 `complete: true`（本來就沒東西要跑），
6 planned 只有 0 written 則是 `complete: false`（不是「一組零格的資料」）。
`rows.json` 與 `run-status.json` 在任何一顆 cell 開跑之前就落地（內容 `[]`），
所以「還在等窗口」與「跑過但全被擋」在盤上長得不一樣。

## 13m. F-19：frontmatter 是封閉鍵集，安裝器的帳目不在裡面

F-16 把 marker 從第一行移進 frontmatter 內部，修好了「skill 完全不可見」，但結論本身是錯的，
而錯誤被寫進了 README 與本文件。使用者指出來之前，我們自己沒有再去看第二遍。

實測（`codex debug prompt-input`，每種形狀單獨建一個 skills 目錄，看它會不會出現在 model-visible
prompt 裡）：

| 形狀 | 載入？ |
|---|---|
| fence 在第 1 行，fence 內只有 key | 是 |
| fence 內多一行 `# managed by cortexyoung install.sh`（F-16 形狀） | 是 |
| 註解放在結尾 fence **之後** | 是 |
| frontmatter 多一個文件外的 key | 是 |
| fence **之前**有任何文字（F-15 形狀） | **否** |

兩條結論。第一，F-16 有效：載入器只要求 fence 從 byte 0 開始，「marker 必須留在區塊內」則是把
「不能放上面」誤推成「必須放裡面」，沒有任何一侧的解析器要求這點。第二，官方文件的 frontmatter
鍵集是**封閉**的（`name`/`description`/`license`/`allowed-tools`/`metadata`），註解在 YAML 合法、
在 Codex 也過得了，但它不是這個格式允許存在的內容——倉庫裡其他每一個 skill（含 Codex 內建的
`.system/*`）第二行都是 `name:`，只有我們塞了東西。

真正的根因比「放錯哪一行」更上層一層：**安裝器在改寫一個自己不擁有格式的檔案。** F-15 與 F-16
只是同一件事的兩個位置，所以換位置只會把問題推給下一個形狀。只要帳目還在文件裡，就永遠需要一段
「先把那行刪掉才比得出 hash」的補丁邏輯（`grep -vF` 後 `cmp`），而讀它的人必須先有一個
「記得先刪掉再比」的心智模型才不會誤判。

修法是把帳目整個移出文件：

- `install.sh` 在 SKILL.md **旁邊**寫 `.cortexyoung-managed`，內容是簽名加上**部署當時的 SHA-256**；
- 部署不再插入任何位元組，agent 家目錄看到的與 `skills/<name>/SKILL.md` 逐字節相同（`cmp` 可證）；
- `skill_is_managed()` 兩段式：檔內任何位置出現簽名＝舊安裝器寫的，認領並**升級**；否則用 stamp 的
  hash 認領；
- 所有權自此追蹤內容：**手改已部署的 SKILL.md 會被當作 collision 拒絕**（舊行為是只要 marker 還在
  就無聲蓋掉使用者的修改，那是比較糟的那一種）；`--force` 照舊備份後替換並重新宣稱；
- 七個 uninstall 位點都要連 stamp 一起刪——孤兒 stamp 會宣稱擁有別人之後寫在該目錄裡的 SKILL.md；
- `preflight_skill_at()` 先验**來源**的形狀（fence 在第 1 行、有結尾、內無註解），來源壞掉直接 die，
  不会再出現「安裝回報成功、agent 什麼都沒收到」。

防再次漏掉的閘門是 `rust/tests/skill_format.rs`。F-15/F-16 能一路綠，是因為**沒有任何東西按消費者
的方式解析過這個檔案**；smoke 的 62 項問的都是「檔案在不在」、「marker 在不在」。現在 `cargo test`
會解析 `skills/*/SKILL.md`：fence 必須在 byte 0（BOM 也算失敗）、必須有結尾、block 內只能是文件化的
鍵（註解、空行、重複鍵都拒）、`name` 必須等於目錄名且 ≤64、`description` 非空 ≤1024 且不含未加引號
的 `:`（那會讓 YAML 把它讀成 mapping 而整份解析失敗）、fence 後要有內文、全文不得出現安裝器簽名。
閘門本身也被證明會失敗：`negative_shapes_are_rejected_by_the_same_gates` 在 `rust/tests/.gate-probe/`
臨時建出六種被禁止的形狀（fence 內註解、fence 前有文字、文件外的鍵、`name` 不等於目錄名、未加引號
含 `:` 的 description、沒有內文），逐一跑同一組 gate 函式並要求它們拒絕、且以**對的理由**拒絕
（比對 panic 訊息裡的关键字）。這個機制當場就证明了自己有用：第一版把 probe 目錄取名成
`.gate-probe`，六個案例全被 `name` 不等於目錄名這一條擋下，測試以「rejected for the wrong reason」
失敗，而不是假裝通過。smoke 側對應的是 `assert_frontmatter_keys_only_rejects`。

smoke 62 → 81：部署檔不得含安裝器帳目、fence 內只有鍵、stamp 的 hash 等於檔案 hash、來源與部署檔
`cmp` 逐字節相同、`--force` 後重新宣稱、uninstall 不留孤兒 stamp（manifest 路徑與 codex 路徑各一）、
手改已部署檔案被拒、以及**兩種**舊形狀（marker 在第 1 行／在 fence 內）都被 repaired 並升級成
pristine。F-16 那條真載入器 oracle 保留。

升級路徑由 Test 17 在 temp HOME 內驗證；使用者的真實家目錄（三份 F-16 形狀的部署檔）要等
`./install.sh` 重跑才會變成 pristine＋stamp，這是 `AGENTS.md` 規定不由 agent 自動執行的事。

同一次改動顺手清掉的另一個 node 殘留：Test 14 原本在找不到 ast-grep 時回退到寫死的
另一個開發者機器上的 nvm 安裝目錄（`<home>/.nvm/versions/node/<ver>/bin/ast-grep`）。現在它只從 PATH 解析（避開自己塞的假 binary），
找不到就明確印 `SKIP:` 並讓該測試轉為跳過，另外多驗一條「解析到的必須是 pin 住的 0.45.2」；
fixture 從 `.ts` 換成 `.rs`，這是倉庫裡最後一個 TypeScript 字串。倉庫側現在 `*.js/*.ts/*.py`
與 node 路徑皆為 0。這台機器的 ast-grep 仍是由 npm 交付的（版本剛好等於 pin 值，所以 `install.sh`
的既有檢查直接沿用），要換成 `install.sh` 自己下載的原生資產或 `cargo install`，需要先決定再動家目錄。

### 但誰來讀 completeness？

F-18 的第一版只做到「把狀態寫成檔案」：`run-status.json` 落在 `rows.json` 旁邊，而 `summarize`
只吃 `rows.json`。於是 4/6 的批次被聚合時，輸出跟「本來就只規劃 4 格的實驗」**完全無法區分**。
同一個教訓第五次出現：產生了證據，沒有產生讀證據的人。

現在 `summarize` 對每個 `rows.json` 去找同目錄的 sidecar，把 `batches` 與 `batch_problems` 一併寫進
聚合輸出；`--strict` 之下任何不完整批次 fail closed。實測（拿真資料前 4 格造的 4-of-6 樣本）：

- 非 strict：`2 of 6 planned cells never made it into rows.json (4 measured)`，聚合照給；
- strict：exit 1，同一句話出現在錯誤裡；
- 兩份 F-18 之前的 committed 目錄跑 `--strict`：`problems: []`，20 格照常聚合。

判定順序有意講究：**先**看 `rows.json` 的格數與 sidecar 的 `written_cells` 是否互相矛盾，**再**看
planned/written。被 SIGKILL 的進程不會走到 Drop，sidecar 會停在「running / 0 written」而 `rows.json`
裡已經有 4 格——這時報「掉了 6 格」是錯的，那 4 格就在檔案裡；正確的診斷是「sidecar 來不及更新，
計數不可信」。為了讓這條規則可能觸發，sidecar 現在在**第一顆 cell 之前**就以 `state: "running"`
落地，而不是只在退出時寫。

兩條邊界同樣釘住：沒有 sidecar（F-18 之前的產物）不視為問題——把歷史复現一律標成可疑，只會訓練人
把 `--strict` 拿掉；舊 sidecar 缺 `state` 欄時報 `"unrecorded"`，不印 `null` 讓讀者自己猜。
消費端新增 7 項測試，evals 33 → 42。

## 13k. F-16：部署到 agent 家目錄的 SKILL.md，agent 從來沒看過
**〔本節最後那段「marker 進 frontmatter 內部」的結論已被 F-19（§13m）取代。〕**「兩個載入器都把
fence 錨定在第一行，所以 marker 不能放上面」是對的；「所以 marker 必須放裡面」是錯的推論，
它把「不能放在檔案外」當成「必須放在格式內」。以下原文保留，因為觀察部分是實測得來的。


§13g 的原則是「改動若不會到達 agent 眼前，等於沒改」。這一條被 `install.sh` 自己打破了，而且破了整個 §13g 的成果。

`deploy_skill_at()` 把 managed marker 直接前綴成檔案第一行：

```
# managed by cortexyoung install.sh   ← 第一行
---
name: ast-grep
---
```

兩個載入器的規則不是靠猜，分別從各自 binary 取出：Codex 0.150.1 對不在第一行的 fence 報
`missing YAML frontmatter delimited by ---` 並跳過整個 skill（啟動時只留一行 `⚠ Skipped loading …`）；
Claude Code 2.1.251 的解析正則是 `hR = /^---\s*\n([\s\S]*?)---\s*\n?/`，以 `^` 錨定在第一行，
**沒命中時回 `frontmatter:{}`**——沒有警告、沒有錯誤，只是 name/description 全空，skill 從此不可能被路由到。
換言之 Claude 那一側的失敗模式比 Codex 更安靜。marker 佔掉第一行之後，整個 skill 被載入器判定為
`missing YAML frontmatter delimited by ---` 而**整個跳過**：檔案在硬碟上、字節正確、與來源 hash 對得上，
但 agent 的 context 裡不留痕跡。Codex 只在啟動時列一行 `⚠ Skipped loading 1 skill(s)`，其餘一切照常，
所以這個狀態可以安靜地撐过好幾天。實測證據（修復前）：`~/.claude/skills/ast-grep`、`~/.codex/skills/ast-grep`、
`~/.claude/skills/xgrep` 三份的第一行全是 marker；`codex debug prompt-input` 渲染出來的 model-visible prompt
裡 grep 不到該 skill 描述的任何片段（計數 0）。

為什麼 53 項 smoke 全綠也抓不到：既有斷言清一色是 `assert_contains "$dest" "$MANAGED_MARKER"`，問的是
「marker 在不在」，而失敗模式恰恰是「marker 在，所以 skill 不在」。與 F-15 同類——驗證了存在的條件，
沒驗證生效的條件。

修法：marker 進 frontmatter **內部**（第二行的 YAML 註解）。插入由 `with_managed_marker()` 負責、
形状由 `skill_frontmatter_intact()` 判定。刪掉那唯一一行後與倉庫來源**逐字節相同**，
`deploy_skill_at()` 的 hash 比對語意完全不動；`skill_is_managed()`（`head -n 5`）照樣認得，
既有的 unmanaged-collision／--force／uninstall 路徑一項都不用改。

真正關鍵的是「已部署但形状壞掉」的檔案必須被**修**，不能被 hash 比對放過：舊檔內容與來源相同，
只比內容會一路回報 `skill up to date`，壞形状就永遠留在盤上。所以 up-to-date 條件改成
「內容相同 **且** 第一行是 `---`」，否則進 `repaired skill frontmatter` 分支重寫。
實跑驗證：三份部署檔各被標一次 repaired，`./install.sh` 再跑一次全部 `up to date`（冪等，沒有 churn）。

驗收（smoke 53 → 62，新增 9 項）：

- 第一行必須是 `---`，marker 必須正好落在第二行（claude 與 codex 兩份都驗）；
- `grep -vxF marker` 之後與倉庫來源 `cmp` 逐字節相同——「只多一行 marker」必須是**可證明的**，不是願望；
- 播一個舊安裝器寫出來的 marker-在第一行 檔案，重跑必須出現 `repaired skill frontmatter`，
  且 fence 回到第一行、仍判為 managed；
- 最強的一條直接問載入器：`codex debug prompt-input` 把該 skills 目錄渲染成 model-visible prompt，
  再 grep skill 描述文字，驗的是「在不在模型看得到的地方」。`codex` 不在册時（CI 就是）印 `SKIP:` 而**不計 PASS**。

這條 oracle 在写完的當下就抓到我自己把它接錯：probe 的 `CODEX_HOME` 指向 temp home 本身而不是 `$HOME/.codex`，
於是對一份正確的檔案也宣稱看不到 skill——一次假失敗，但正是這類斷言該有的灵敏度。

## 13j. F-17：`run-agents --help` 曾經是一次真的運算

`main.rs` 用 `at(argv, "--flag", default)` 逐個查選項，未知選項從不報錯。結果是

```
$ cort-evals run-agents --help
```

不印 usage，而是**以全套預設值啟動評測**（`--tasks evals/tasks-graph.json`、`--out evals/runs/2026-08-30-graph`），
也就是朝倉庫工作區寫一批「真實測量」。這次它卡在第一個 cell（沙箱沒網路），被我中止；
`find -newermt` 確認 `evals/runs/` 與 `/tmp/cort-eval-runs` 都沒有新檔落地、`git status` 只剩預期的兩個 bash 檔變更。
同樣一行打在一個有額度、有網路的終端裡，就是一次沒人要求的取樣——而且會蓋掉既有 out 目錄。

同一個洞也吞得下 `--out=dir`：`at()` 找不到 `--out=dir` 這個 key，`--out` 靜默回到預設目錄，
「我明明指定了輸出目錄」變成往別處寫。

修法：每個子命令白名單化自己的選項（`RUN_AGENTS_FLAGS`／`VERIFY_IMPACT_FLAGS`／`SUMMARIZE_FLAGS`），
`guard_options()` 在**任何動作之前**擋下未知選項並連同該子命令的 usage 一起報；`--help`/`-h` 印 usage 後 exit 0；
`--flag=value` 直接拒絕並說明要分開傳值，而不是讓它無聲地退成預設值。新增 6 項測試釘住這些行為
；涵蓋性現在由掃自身原始碼保證（`include_str!` 找每個 `at(argv, "--x")`），不再靠註解呼喚自律。

白名單涵蓋性也從願望變成 gate。原先那條測試只走白名單**本身**（列出來的選項都被接受），
所以「parser 新增一個選項卻忘記登記」這種漂移它照樣綠。現在改成掃 `include_str!("main.rs")` 裡每一個
`at(argv, "--x")` / `has(argv, "--x")`，不在任何白名單者直接失敗；另加一條反向測試，確認這個掃描
**真的匹配到選項**——否則它會因為自己寫錯而永遠綠（我自己犯的正是這類的錯，所以釘兩頭）。

順帶把 §13i「剩下的 3 個任務」真正需要的事放進同一個子命令，不必用 bash 串：

- `--only` 接受逗號清單（3 任務 × 2 臂 = 一次呼叫、一份 rows.json、6 cells）；選錯 id 仍然 fail closed，
  `--only nope` 得到 `no task matched --only nope` 而不是「跑了全部 5 個任務」；
- `--delay-secs N` 讓 runner 自己等額度窗口重置，不需要外層 `sleep`。等待發生在讀 venue HEAD 與任何 cell 之前，
  因此 HEAD 仍是開跑當時那個；只收 0 與正整數，`-5`／`10.5`／空字串一律拒——靜默退回 0 等於把 cell 直接打进還關著的窗口。

### F-17 的邊界：頂層 `--help` 仍然 exit 2（已修）

F-17 的 commit 訊息寫「`--help`/`-h` 印 usage 並 exit 0」，那對**三個子命令**都成立，但
`cort-evals --help`（沒有子命令）走的是 `main()` 的預設分支：印 usage 到 stderr、exit 2。
`wants_help()` 這個判斷式當時就對裸 `--help` 回 true，而且它有單元測試——測試綠，行為錯，因為
`main()` 根本沒呼叫它。這是同一個教訓第四次出現（F-15、F-16、F-19、以及這裡）：**測了條件，沒測生效**。
條件測在純函式上，生效只發生在真正的程序裡。

修法：頂層也走同一條 help 路徑（`cort-evals help` 同義），unknown 選項照舊 exit 2。
驗證方式改成**跑真 binary**：`evals/tests/harness.rs` 用 `CARGO_BIN_EXE_cort-evals` 實際 spawn，
斷言 `--help` 與 `help` 退出 0 且 stdout 三個子命令都提到、`--frobnicate` 仍非零。
evals 32 → 33 tests。

## 13i. 第二輪取樣：額度攔截、F-15，與「正確度穩定 / 成本不穩定」
〔第 3 點的 14 格聚合已被 §13n 的 20 格取代；該節的观察 1、2 仍然成立。〕


### 加了 1 輪重複取樣，結果是兩件事

第二輪（同一份 tasks、同一 venue HEAD、同一 release binary）只完成 2 個任務共 4 格；另外 3 個任務
被 5 小時窗口硬擋（`rate_limit_info.status = "rejected"`）。有效的兩格對照如下：

| 任務 | 臂 | 第 1 輪 | 第 2 輪 |
|---|---|---|---|
| transitive-chain-lastntradingdays | cort | cov 1.0・85,510 tok・336 tool・3 turns | cov 1.0・85,540 tok・336 tool・3 turns |
| transitive-chain-lastntradingdays | shell | cov 0.75・307,070・2,627・10 | cov 0.75・240,683・2,429・8 |
| route-blast-radius-reportsstatus | cort | cov 1.0・86,018・431・3・held | cov 1.0・176,695・1,044・6・**未受控** |
| route-blast-radius-reportsstatus | shell | cov 0.75・476,138・9,809・21 | cov 0.75・670,928・10,436・18 |

讀得出来的結論：

1. **正確度跨輪穩定。** 兩格、兩臂、四次重覆，coverage 完全一致（包括基線臂**兩次都漏同一個符號**
   `reports`）。這不是運氣：漏的是同一個位於 `tests/validation/` 的呼叫端，grep 走法兩次都沒走到那里。
2. **成本不穩定，而且變動來自 agent 是否順手查證。** cort 那一格第 2 輪多花一倍 token、turns 3→6，
   `arm_held` 從 true 變 false——它自己去 grep 驗證圖的答案。所以「cort 比較便宜」的倍數，取決於
   我們讓不让 agent 查證；把查證算進成本時倍數會縮小，不讓查證時資料不可信。這是設計取捨，不是誤差。
3. 以 14 格（cort 7、基線 7）聚合：cort success 1.0、mean 123,109 tok、mean tool 889、mean turns 4.1；
   基線 success 0.286、mean 467,925 tok、mean tool 7,314、mean turns 15.9。verdict 仍是 cort 勝。
   n 仍只有 2（且 3 個任務 n=1），中位數在 n=2 沒有意義，所以這裡報兩次觀測值而非「中位數」。

### F-15：被拒的 cell 曾經被當成測量值

第二輪那 3 個任務的 row 長這樣：`total_tokens: 0, tool_return_tokens: 0, coverage: 0, success: false`。
但 transcript 顯示 `terminal_reason: api_error`、`is_error: true`、結果文字是
`You've hit your session limit · resets 2:40am`。也就是**根本沒有跑的格子被寫進聚合，把平均分數拉成
「兩臂都很便宜又都不對」**——正是這輪一直在剷除的靜默污染類別，這次長在我自己的 runner 裡。

修法（`evals/src/stream.rs`）：`parse_stream` 現在先掃描 `rate_limit_event`，`status == "rejected"` 即
丟例外；此外 `is_error == true` 且 `terminal_reason != "completed"` 也丟例外。例外訊息保留原始拒絕文字，
方便判斷是額度還是上游錯誤。runner 因此以 exit 1 失敗、**不寫任何 row**（實測在仍被擋的狀態下跑一次：
只有錯誤訊息，無檔案落地）。

要分辨的邊界：**turn-cap 仍是一個有效測量**。`subtype == "error_max_turns"` 的格子會照常寫入並標
`hit_turn_cap: true`——那是「跑了但沒跑完」，跟「沒跑」不同。3 個新測試把這三種情況釘住。

### 剩下的 3 個任務怎麼補

第二輪的 6 格需要等 5 小時窗口重置（本地 02:40）之後再跑。F-17 之後這是一次呼叫，不是三次外層串接：

```bash
CORT_BIN=$PWD/rust/target/release/cort ./evals/target/release/cort-evals run-agents \
  --tasks evals/tasks-graph.json \
  --only blast-radius-3hop-getcurrenttimeet,hub-blast-radius-loginfo,storage-blast-radius-backtesting \
  --arms rg+Read,cort --max-turns 40 --config-dir /tmp/cc-eval --cache-dir /tmp/cort-exp \
  --out /tmp/cort-eval-runs/round2/refused3 --delay-secs <到 02:43 的秒數>
```

現在若又被擋，會直接失敗而不會再污染資料。

**那 6 格舊記錄已經隔離，不是刪掉。** F-15 修的是「往後不再寫入污染 cell」，但 blocked 那次**之前**
寫進 `/tmp/cort-eval-runs/round2/` 的檔案還在原地：`blast-radius-3hop` 兩格 `cov 0.0`（27,933／27,826 tok，
跑到一半被擋），`hub-blast-radius-loginfo` 與 `storage-blast-radius-backtesting` 各兩格 `total_tokens: 0`。
只要有人對整個 `round2/` 目錄跑 `summarize`，F-15 剷除的那個「兩臂既免費又都不對」就會原地复活。
已移到 `/tmp/cort-eval-runs/pre-f15-quarantine/`（附 README 說明这是證據不是資料），
`round2/` 現在只剩兩個有效任務加這次的新 out 目錄。

## 13h. F-14：ast-grep 測試替身移植為 Rust

原本的 `rust/tests/fixtures/fake-ast-grep` 是 46 行 Python，被 4 個測試檔、10 個呼叫點當成假 parser
使用（`hang` / `streams` / `empty` / `emit:<base64>` / `preflight-*` / `version:<x>`）。它是倉庫裡最後
一個非 Rust 的**可執行邏輯**（`*.py` 掃不到，因為檔名沒有副檔名）。

移植成 `rust/src/bin/fake_ast_grep.rs`（`[[bin]] fake_ast_grep`），**模式語法完全不變**，所以既有測試
只需把 `fake_ag()` 從「倉庫裡的路徑」改成 `env!("CARGO_BIN_EXE_fake_ast_grep")`，10 個呼叫點的斷言一行
都沒動。選擇「維持語法」而不是「順手簡化 base64」，是為了讓這次移植可被證明等價：移植前後 4 個測試檔
的 60 個測試結果完全相同。

新增 `rust/tests/fixture.rs`（7 項）測這個替身自己：預設回報 pin 版本、`streams` 同時寫兩條 pipe 且
exit 1、`empty` 與真實零命中**逐位元組不可區分**（这正是設計要求 pre-flight 的理由）、`emit:` 含換行的
位元組級還原、`preflight-*` 能区分壞 pattern 與好 pattern、未知模式安靜 exit 0。

過程中該測試立刻抓到實作太寬鬆：`emit:YQ===`（壞 padding）會被解成 `"a"`，讓「malformed stream」測試
可能因為「什麼都沒輸出」而假性通過。已讓解碼器嚴格檢查長度與 padding 並回 exit 2。

它仍是 build 產物之一，所以 smoke 新增一條：安裝後的 payload 目錄 (`~/.local/share/cortexyoung/cort`)
不得出現 `fake_ast_grep`——開發用雙重身份不得進入部署。`AGENTS.md` 裡的「已知例外」段落同時移除。

## 13g. Agent 引導的連動（skill / AGENTS.md / Codex）

改動若不會到達 agent 眼前，等於沒改。本輪把三條路徑接起來：

- `skills/ast-grep/SKILL.md` 改寫：開頭同時引用**需求面**（1,565 prompt 內 0 次問圖）與**成本面**
  （§13f 的 5 任務 2 臂：圖臂 5/5、~120k tokens；shell 臂 17 turns、~473k、3/5 錯），第 4 條從
  「只在明確問題時才用、不要因為有圖就用」改成「明確關係問題時**一次** call，不要 grep 逐跳走」，
  並給出便宜的正确查證方式（`cort context <dependent>` 讀一個函式，而不是重讀整檔）。另加
  `ast_grep_missing` 的自救說明（F-13 之後只需設 `CORT_AST_GREP_BIN` 或重跑 install）。
  長度 955 → 1,079 est tokens（+13%）：新增的都是行為改變，其餘段落同步壓縮過。
- `install.sh` 現在把同一份 skill 部署到 `~/.claude/skills/` 與 `~/.codex/skills/`（尊重
  `CODEX_HOME`），兩份 byte-identical，各自有 managed marker、manifest key（`skill_ast_grep` /
  `skill_ast_grep_codex`）、preflight 拒絕未管理檔、`--force` 採用、uninstall 只刪自己擁有的。
  smoke 新增 Test 16（10 項斷言，總數 42 → 52）。
- 新增 `AGENTS.md` 作為唯一來源，`CLAUDE.md` 改為它的符號連結（兩邊讀到同一份 bytes，不可能漂移），
  內容含純 Rust 契約、兩 crate 的提交前檢查、目錄邊界，以及「引導文字是受度量的人工产物」這條規則。
  同時把 Python fixture 登记為 F-14（不得擴大的已知例外）。

注意：倉庫改變不會自己進入你的 agent 設定。要让 Codex 與 Claude 都用到新的引導，需要執行一次
`./install.sh`（依 `AGENTS.md` 的規則，我不會自動安裝）。

## 13e. F-13：agent 的 PATH 被正規化後，cort 找不到它唯一的 parser

### 實測

jail 那次跑出的 cort 臂 20 turns 全毀，工具輸出是：

```json
{ "error": "ast_grep_missing", "detail": { "candidate": "ast-grep" } }
```

`rust/src/ast_grep.rs` 的 `candidate_bin()` 只有兩種可能：`CORT_AST_GREP_BIN`，或裸名 `ast-grep`（靠
PATH 查）。而 Claude Code 給 Bash 工具的 PATH 會被正規化成
`/usr/local/bin:/usr/bin:/bin:$HOME/.local/bin`。本機 ast-grep 位在 nvm 的 bin 目錄；`install.sh` 預設
則把 ast-grep 裝進 `$CARGO_HOME/bin`（`~/.cargo/bin`）。兩者都不在那個正規化清單裡。

所以這不是評測器的邊角問題，而是：**用 `install.sh` 正常安裝的 cort，在 agent 會話裡執行任何需要解析器的
命令都會失敗**，而且錯誤訊息只說 `candidate: "ast-grep"`，不告訴使用者该設定什麼。

### 已實作的解法（本輪）

1. `resolve_ast_grep_bin()` 改為依序探測候選清單（純函式 `ast_grep_candidates()` 可測）：
   PATH 上的裸名 → cort executable 同目錄與其 `../bin` → `$HOME/.local/bin` → `$CARGO_HOME/bin` →
   `$HOME/.cargo/bin` → `/usr/local/bin` → `/opt/homebrew/bin`，並**優先選版本正好等於 pin 的那個**
   （PATH 上一個 0.44.x 不得遮掉裝在旁边的 0.45.2）。
2. `ast_grep_missing` 的 detail 帶 `probed`（查過哪些位置）與 `hint`（含 `CORT_AST_GREP_BIN`），
   agent 能自我修復；找到可用但版本不對時回 `ast_grep_version_mismatch` 並附 found/expected，
   仍是 fail-closed，不會退化成 in-process parser。
3. `CORT_AST_GREP_BIN` 設定时**絕不 fallback**（「明確指定 = 就要這一個」），否則 fail-closed 的
   測試會因為探測到真實 ast-grep 而僥倖通過。
4. 沒有改 `install.sh` 的安裝位置：探測清單已經消除對 caller PATH 的相依，動 BIN_DIR 會牽動
   smoke suite 的 42 項斷言與 manifest 所有權，代價大於收益。
5. 5 個回歸測試（`rust/tests/ast_grep.rs`）：候選清單順序與內容；PATH 清空但 `~/.cargo/bin` 有
   pinned 假裝者時解析成功；可達但版本不對時回 `ast_grep_version_mismatch`；全數缺席時回
   `ast_grep_missing` 且帶 probed/hint；明確覆寫失效時不 fallback（detail 帶 `source`）。
6. 端到端實證：`env -i PATH=/usr/local/bin:/usr/bin:/bin HOME=<sim>`（ast-grep 只出現在
   `<sim>/.cargo/bin`，也就是 `install.sh` 的預設落點）下，release cort 成功 `index`（2,583 chunks）
   並回報 `dependents=23`。同一命令在修復前就是那次 cell 裡的 20 turns `ast_grep_missing`。

在此之前，評測器由 `build_env` 把父行程解析到的路徑注入 `CORT_AST_GREP_BIN`，讓被測的 cort 與使用者
跑的確實是同一個工具——這是量測效度的要求，不是對產品缺陷的掩蓋。

## 13d. F-12：評測器移植為 Rust，repo 恢復單一語言

規則（已寫進 `CLAUDE.md`）：**本 repo 是純 Rust**，JS/TS/Python 不得以可執行碼存在，工具與測試也不例外。
Bash 只准留在 `install.sh` 與 `tests/install-smoke.sh`（平台需求），邏輯不得落在 shell 裡。

`evals/` 現在是獨立 crate（`cort-evals`，dev-only，`install.sh` 永不建置或安裝它），依賴只有產品端
已 vendor 的 `serde_json`（加 dev 用 `tempfile`），因此離線可建：

| JS（813 行，已全部刪除） | Rust |
|---|---|
| `agent-stream.mjs` | `src/stream.rs`（`estimate_tokens`、`parse_stream`，指標非數字即 `Err`） |
| `grade.mjs` | `src/grade.rs`（contract、GATE、`grade_answer`、tasks 載入） |
| `run-eval.mjs` | `src/summary.rs`（`summarize(rows, strict)`，未量測指標計數而非平均成 NaN） |
| `run-agents.mjs` | `src/arms.rs`（arm 定義、PATH jail、`arm_held`、`build_row`）+ `src/main.rs`（`run-agents`） |
| `verify-impact.mjs` | `src/verify.rs`（不引入 regex crate，自行實作 `name`） |
| `harness.test.mjs`（14） | `tests/harness.rs`（16） |

### 移植時刻意做的三處收緊

1. **絕對路徑不再算受管制**：JS 的 `armHeld` 比 basename，因此 `/opt/cort`、`/usr/local/bin/rg` 會被放過。
   Rust 版規定：帶 `/` 的 token 必須「正好等於」該臂設定的 binary，裸名才認（裸名只能透過 jail 的 PATH 取得）。
2. **驗證器的限制寫明**：`contains_word` 是文字比對，註解裡提到某個識別字也算「確認」，所以
   `verify-impact` 是防捏造的 soundness screen，不是呼叫關係的證明；精度 1.0 時要記住這點。
3. **轉錄解析不再靜默**：JS 用 `JSON.parse` 逐行，壞行會整個 throw；Rust 版把「非 JSON 行」變成帶行信息的
   `Err`，並保留 `no result event` / `no usage` / `not a number` 三道拒絕。

### 等價性與驗證（不花模型額度的部分）

- `cort-evals summarize evals/runs/2026-08-26/rows.json` 重現 JS 的三份平均：
  `rg+Read 185,523.8` / `ast-grep+Read 199,744.0` / `cort 387,855.2`，verdict 仍是 STOP ✓
- 未量測指標現在是 `null` + `metrics_missing` 計數（JS 是 NaN 靜默變 null），`--strict` 直接拒絕並退出 1 ✓
- `cort-evals verify-impact` 在 cct 上重跑 3 條鏈：4/4、4/4、20/20 確認，precision 1.0 ✓（與文件宣稱一致）
- `cargo test --manifest-path evals/Cargo.toml --all-targets`：16 passed / 0 failed；
  `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings` 全清 ✓
- CI 改為 `rust` + `evals` 兩 crate 的 matrix（各跑 fmt/clippy/test），`actions/setup-node` 與所有
  `node` 步驟移除。

## 13c. F-11：評測臂的白名單從來沒有生效（第一個真實 cell 暴露）

推送後第一次跑 smoke cell（1 任務 × 2 臂）時，基線臂出現自相矛盾的 row：11 turns、12,846 bytes
工具回傳，卻 `rg_calls=0`、`read_calls=0`、`permission_denials=0`。翻查 transcript 發現它 10 次 Bash
呼叫全是 `grep -rn ...` 與 `sed -n ...`——`rg` 與 `Read` 一次都沒用，而且**沒有被拒**。

单独探测确认（`--allowedTools "Bash(rg:*)"` 下要求它跑 `grep -c '' package.json`）：命令執行成功、
`permission_denials: []`。也就是說 `Bash(prefix:*)` 在 headless `-p` 模式下並不約束 shell 內容。

影響：評測設計寫著「工具白名單即實驗組」，但那是個標籤不是管制。cell 出來的 `rg+Read` 其实是
「agent 拿到的整個 shell」。這不代表數據無用——它恰好是「cort vs agent 自然行為」的答案，而且我們的
真實 session 分析本來就說過 agent 是 grep-native——但它**不能**被解讀成「cort vs rg」。

修正（`evals/run-agents.mjs`）：

- `ARM_BINARIES` + `makeJail()`：每個臂改用只含其獲准 binary 的 PATH jail（純 JS 解析 PATH，
  不 spawn `which`，否則沙箱下會 EPERM）。預設開啟，`--no-jail` 可跑「真實 shell」對照組。
- row 新增 `shells_used`、`arm_held`、`jailed`，列為 REQUIRED_FIELDS。`arm_held=false` 的 cell
  不能當作比較來讀，且這個值現在由資料自己說出來，不需要事後翻 transcript 才發現。
- 4 個新測試（不需要模型）：jail 後 `rg` 可用而 `grep`/`sed` 不存在；`armHeld` 精確標出上述
  那次外洩；cort 臂偷偷用 `rg` 也要被抓到。`node evals/harness.test.mjs` 14/14。

同時保留的誠實記錄：第一輪 smoke 的 cort 臂數據是可信的（`cort_calls=1`、114 tokens 工具回傳、
2 turns、coverage/precision/hop 全 1），外洩的只有基線臂的身分。

## 13b. 實作狀態（2026-08-29 開工後）

本節記錄已落地內容、驗收證據，以及仍未處理的項目。

| ID | 狀態 | 落地方式 | 證據 |
|---|---|---|---|
| F-01 | **已修（正解，非降級）** | schema v3 新增 `raw_edges` 持久層；`relationships` 改為「所有檔案落地後，由 raw edges 一次性全域重建」；`graph_pending` 作為 derived-state 安全閥，並讓 `compute_stale` 在它為真時回報 stale | 5 個新測試（含 3 跳鏈、新呼叫者、pending→stale、pending→full 自愈）；`cargo test` 218/218；最小重現與 2,037-chunk 場域端到端皆保持 `dependents` 不變 |
| F-02 | **已修** | `install_cort` 改為每次 `cargo build --release --locked`（把 build 提到 `rm -rf $CORT_HOME` 之前，失敗時保留舊安裝） | 新 smoke Test 15（用可記錄呼叫的假 cargo 驗證「prebuilt 存在時仍必須 build」）；`tests/install-smoke.sh` 42/42；實測 no-op build 0.04s、有變更 6.7s |
| F-03 | **已修** | CI 改為 Rust gate：`rustfmt`、`clippy -D warnings`、`cargo test --locked --all-targets`、release build、`bash -n`、直接安裝 shellcheck 執行 lint、install smoke、評測 `node --check` + `node --test` | YAML 結構可解析；`package.json`/`npm` 相依全部移除 |
| F-04 | **已文件化（未實作 Rust edges）** | README 新增「各語言實際能力表」；skill 明確指示 Rust 不要走 `impact`，改用 `rg`/`cargo check` | 能力表 + 限制 #8；實測本專案 Rust 545 chunks / 0 outgoing edges |
| F-12 | **已修** | `evals/` crate 取代 6 個 `.mjs`；`CLAUDE.md` 寫下純 Rust 契約；CI 改跑兩 crate 且移除 Node | JS 813 行刪除；Rust 16 tests、fmt/clippy 全清；summarize 與 verify-impact 對同一份歷史資料等價；CI 無 node 引用 |
| F-05 | **部分已修** | `cort_calls` 改用 `isCortCommand()`（比對 Rust `CORT_BIN`，也接受 `cort`/`…/bin/cort`）；`summarize()` 改為計數未量測指標、`strict` 模式直接丟例外、指標缺失時 verdict fail closed；新增 `evals/harness.test.mjs`（10 tests，純 Node，無需 npm）；`evals/README.md` 的 `bin/cort.js`／已刪測試路徑全部校正 | `node --test evals/harness.test.mjs` 10/10 |
| F-06 | **已修** | 執行 `cargo fmt --all` 並把 `cargo fmt --all -- --check` 放進 CI | 98 處 diff → 0；Clippy 同時通過 |
| F-07 | **未修（維持既有契約）** | 僅在文件層面保留「candidates 需查證」的語意 | 需 CLI 契約變更（多 seed 時 fail closed 或 `--file` 消歧），影響現有輸出格式，另行提案 |
| F-08 | **部分已修** | README 能力表把 `context/read/recall` 列為主要能力、`impact` 標明語言邊界 | 完整 Stable/Experimental/Deferred 分級仍待一次文件整理 |
| F-09 | **僅文件化** | README 限制 #9 說明 `status.unparsed` 也包含「合法零符號檔案」 | 程式層面需區分 `no_symbols` 與 parse failure，屬 P3 |
| F-18 | **已修** | 每格量到當下就重寫 `rows.json`（排序後整個覆寫，線程完成順序不影響檔內容）；`RunStatus` guard 在**包括提前 return 在內**的每一條出口寫 `run-status.json`，記 planned/written/complete；**消費端一併補上**：`summarize` 讀同目錄 sidecar、輸出 `batches`/`batch_problems`、`--strict` 對不完整批次 fail closed，且 sidecar 在第一顆 cell 之前就落地 | 2 項新測試（部分批次必須自報 incomplete、rows 排序與完成順序無關）＋ 7 項消費端測試（4-of-6、SIGKILL 計數矛盾、無 sidecar 不疑、缺 `state` 報 unrecorded），此節合計 9 項；實際取樣中可在跑完前直接看到 rows.json/run-status.json 長大 |
| F-19 | **已修** | 安裝器帳目整個移出 SKILL.md：改寫同目錄的 `.cortexyoung-managed`（簽名＋部署當時的 SHA-256），部署檔與來源逐字節相同；`preflight` 先驗來源 frontmatter 形狀；新增 `rust/tests/skill_format.rs` 以消費者角度解析 `skills/*/SKILL.md` | 負向驗證內建在 `negative_shapes_are_rejected_by_the_same_gates`（六種被禁止的形狀）；smoke 62 → 81（含兩種舊形狀升級、孤兒 stamp、手改被拒）；Test 14 移除寫死的 nvm 路徑與最後一個 `.ts` fixture |
| F-16 | **已修** | marker 移到 frontmatter 內部第二行（`with_managed_marker`）；up-to-date 的條件從「內容相同」改成「內容相同**且**第一行是 `---`」，舊形状走 `repaired skill frontmatter` 分支重寫 | smoke 53 → 62：fence 在第一行、marker 正好在第二行、`grep -vxF marker` 後與來源 `cmp` 逐字節相同、legacy 形状必須被 repair、外加**真載入器 oracle**（`codex debug prompt-input`，缺 codex 時明確 SKIP 不冒充 PASS）；實測 3 份部署檔 repaired 後可見 |〔結論已被 **F-19** 取代：marker 不進文件，改寫旁邊的 stamp〕
| F-17 | **已修** | 每個子命令白名單化選項，`guard_options()` 在任何動作前擋下未知選項；`--help`/`-h` 印 usage 並 exit 0（頂層漏了一層，見下）；`--flag=value` 直接拒絕 | 新增 6 項測試（help 不再等於執行、未知選項附 usage、`=` 形式被拒、`--jail`/`--jailed` 可區分、位置參數不误殺）；whitelist 涵蓋性後來改為掃自身原始碼保證（2 項，含「掃描必须真的匹配到」的反向測試） |
| F-10 | **已修** | C2-22（會改行程 CWD 的測試）拆到獨立 target `rust/tests/staleness_cwd.rs` | 修正前 HEAD 基線 10/20 失敗；修正後 `--test staleness` 20/20 通過、整輪 218/218 連續 3 次 |

### 過程中發現並一併處理的兩件事

1. **全域重建不可無條件執行。** 第一版實作在每次 `index --incremental` 結束時都重建圖，結果乾淨工作區也要 1.05–1.11s（cct 規模 1,837 條邊／16,624 條 raw edges）。改為「只有真的有 chunk 或 raw edge 變動才重建」後，髒檔 0.73s、乾淨工作區 0.06–0.11s，比修法之前還快。
2. **schema 升級必須帶安全閥。** 舊 v2 DB 有 chunks 但 `raw_edges` 是空的，若直接走「由 raw edges 重建」會把整張圖擦成 0 條邊。因此 v2→v3 升級會設定 `graph_pending=1`：`status` 立刻報 stale，下一次 `index --incremental` 自動退回 full 重建後才清標記。實測本專案 567-chunk 舊索引與 cct 舊索引都正確自愈。

### CI 實際執行結果（F-03 的最終證明）

本地全綠不等於 gate 有效，因此以遠端 Actions 為準。同一份 workflow 的三個 run：

| run | commit | 結果 |
|---|---|---|
| 舊 workflow（`npm ci`） | `ce7ea604` | failure，12 秒失敗 |
| 舊 workflow | `1eaad63f` | failure，14 秒失敗 |
| 新 workflow 第一次 | `9d4b69e0` | ubuntu failure — 唯一失敗在 `shellcheck` 步驟；10 個 SC2002 (style) + 1 個 SC2015 (info)，全部是本輪之前就存在的發現 |
| 新 workflow 修正後 | `7057f1e1` | **success**，ubuntu-latest 與 macos-latest 各 20 步全過 |

第一次的失敗是我替換第三方 action 時漏了它原本的 `severity: warning` 門檻，等於把 gate 無意中收緊。
已用 `--severity=warning` 還原原門檻（不是放寬），並從日誌確認該步驟沒有任何 warning/error 級發現，
所以這個修法沒有掩蓋問題。前兩列同時佐證 F-03：CI 在主幹上已經連續紅了兩個 commit，而本地的
218 個 Rust 測試從未被 gate 執行過。

### 尚未做（刻意留在後續）

- F-07 的消歧契約變更、`chunk_id` 穩定 ID、FTS `tokenchars`。
- Rust `edge:calls`/`edge:imports` 規則（需要 module/`use` 解析與 trait dispatch 決策，且要先有真實需求）。
- `call-site line` + `rel_type` 進 lean 輸出（原 §6.2 信任成本）。
- ~~端對端 graph 評測的實際執行~~ → 已完成 10 格，見 §13f；仍缺**重複取樣**（每格目前 n=1）與
  一個能真正束縛工具的 driver（若要回答「cort vs rg」而非「cort vs agent 的 shell」）。
- `stale_reads` 目前無人量測（要實作需比對「回答當時磁碟內容」）。
- shellcheck 未在本機執行（此主機沒有 shellcheck），僅在 CI 中安裝執行；新增 shell 已照 `-e SC2143 -e SC1091` 規則手動對齊。

## 14. 建議執行順序

### Phase 0：恢復可信性

1. 新增 F-01 regression test，確認測試先失敗。
2. changed/deleted incremental 暫時 fallback full index。
3. 修 installer 每次 `cargo build --release --locked`。
4. 新增 installer stale-binary regression。

### Phase 1：恢復 release gate

1. 單獨執行並提交 rustfmt。
2. CI 改成 fmt、Clippy、Rust tests、shell smoke、eval syntax check。
3. 確認 Ubuntu/macOS clean clone 都綠。

### Phase 2：校正產品契約

1. README 與 skill 標示 Rust graph 能力範圍。
2. Stable/experimental/deferred 分級。
3. 修 eval runner 的 `cort_calls`、row schema、summary 及測試。
4. 移除或恢復無法重現的 eval commands。

### Phase 3：正確的 incremental graph

1. 加 raw edge/call-site schema。
2. project-level atomic graph rebuild。
3. 完成 rename/delete/ambiguity/interrupt 測試。
4. 用真實大型 venue 量 correctness、index time、DB size，再決定是否最佳化 affected-set rebuild。

### Phase 4：有需求才擴張

1. 先量 graph-required end-to-end agent cells。
2. 若真實使用仍以 Rust symbol reading 為主，優先改善消歧、fragment selection 與 harness receipt。
3. 只有在有真實 Rust multi-hop 需求時，才投入 Rust graph；否則交給 cargo/rust-analyzer。

## 15. 完成定義

此輪問題不能以「所有既有 tests 都綠」作結，因為 F-01 正是在既有 tests 全綠時存在。完成必須同時滿足：

- incremental 最小重現不再產生 fresh-but-wrong graph。
- installer 從更新後 source 部署的 binary 可觀察地是新 build。
- GitHub Actions 真正執行 Rust fmt、Clippy、tests 與 install smoke。
- Rust `impact` 的支援範圍與文件一致。
- eval runner 的 command accounting、row schema、gate 都有自動測試。
- README 中的現行命令可執行，歷史/WIP 文件清楚標為 archival 或 superseded。
- `cargo fmt --check`、Clippy、213+ Rust tests、40+ install smoke 全部通過。

在以上條件完成前，`context/read/recall` 可繼續作為主要能力使用；`impact` 應視為需人工驗證的候選產生器，
而 `index --incremental` 不應被用來恢復 graph freshness，除非已採用 full-index fallback。

## 16. 復核記錄（2026-08-29 第二輪獨立查證）

本節把上文每個主張對應到「可重跑的命令 + 實際觀測」，並記錄查證後對根因描述的修正。

| ID | 主張 | 查證方式 | 實際觀測 | 判定 |
|---|---|---|---|---|
| F-01 | incremental 刪除跨檔 edge | 雙檔最小重現，只改 callee body | `relationships` 1→0；`impact` `dependents` 1→0 | 確認 |
| F-01 | 機制是 FK cascade | 讀 `db.rs:56`；直接查重現後的 DB | `foreign_keys=ON`；`relationships` 是**空表**（非孤兒列） | 確認並精修 |
| F-01 | fresh 判斷看不見此問題 | 通讀 `staleness.rs` | 比 per-file extraction hash vs `file_state`，沒有 derived-graph 維度 | 確認並改準描述 |
| F-01 | 測試層破口 | `rg -ni relationship rust/tests/` | 僅 `db.rs`/`graph.rs`/`context.rs` 命中；incremental/indexer/staleness 為 0 | 確認 |
| F-01 | 止血邊界 | 髒檔 vs 乾淨檔的 incremental | 髒檔 edges 歸零；乾淨檔走 skip 分支，edges 保持 1 | 確認 |
| F-01 | 可恢復性 | 事後跑 full `index .` | `relationships` 回到 1、`impact` 恢復 caller ⇒ 必須重新抽取才能重建 | 確認 |
| F-02 | installer 可能部署舊 binary | scratch repo + 獨立 HOME + 可字搜尋 marker | 改 source 後不 build；部署 hash 不變；marker 不在 binary | 確認 |
| F-02 | 解方有效 | 移除 prebuilt 迫使走 build 分支 | 新 hash `3764d640…`（不同於舊 `0293a0d4…`）且 marker 存在 | 確認 |
| F-02 | 當下是否已錯版 | `find -newer target/release/cort` | 僅 `src/pack/rules/typescript.yml`（pack 每次 install 重拷） | 屬潛在缺陷，非現行錯版 |
| F-03 | CI 必然失敗 | `git ls-files`、`ls package.json bin` | 無 `package.json`/`bin/`/tracked JS；workflow 仍跑 `npm ci`、`npm test` | 確認 |
| F-04 | Rust 無 call edges | 查規則檔與索引 DB 統計 | `rust.yml` 0 條 edge 規則；Rust 545 chunks、0 outgoing edges；17 條邊全來自 `evals/*.mjs` | 確認（規則＋資料雙重證據） |
| F-05 | `cort_calls` 永遠 0 | 讀 `run-agents.mjs:107` | 仍比對 `includes('cort.js')`，產品已是 Rust binary | 確認 |
| F-05 | 評測命令不可重現 | `test -f evals/relation-cost.mjs` | MISSING，而 README:148 引用它產出 headline 成本表 | 確認 |
| F-05 | summarizer 靜默產 null | 以 runner row 形狀呼叫 `summarize()` | `stale_reads: null`（NaN 序列化），未丟例外 | 新增細節 |
| F-06 | 格式 gate 缺失 | `cargo fmt --all -- --check` | 98 處 diff，橫跨 15 個檔案（src + tests） | 確認並量化 |
| F-07 | 未消歧會多 seed | `cort context main -f lean` | `seeds=2 truncated=true`：`main` 同時命中 Rust 與 `.mjs` | 確認 |
| F-08 | 結論文件互相衝突 | 讀 README 與兩份分析文件 | STOP／撤銷 STOP／取代 WIP 三代並存且未標 status | 確認 |
| F-09 | `unparsed` 語意混用 | ast-grep 對 lib.rs：exit 0、0 bytes；讀 `extract_file` | 零 records 也導向 `unparsed_result`；本專案 `unparsed=1` 即 lib.rs | 新增 |
| — | 測試基線 | `cargo test --all-targets`、`bash tests/install-smoke.sh` | 213 passed / 0 failed；smoke 40/40 | 確認 |
| — | Clippy | `cargo clippy --all-targets --all-features -- -D warnings` | clean | 確認 |

### 查證後對原始描述的修正

1. F-01 原先寫「status 不驗證 graph completeness」，改為：staleness 在**自身 per-file 定義下是正確的**，
   缺的是 derived 跨檔層的新鮮度概念。這決定修法不能只靠「再多算一個 hash」。
2. F-02 從「會部署舊 binary」收緊為「已被實證的潛在交付缺陷」，並標明目前工作副本尚未錯版，避免誇大。
3. F-04 由單點觀察（`dependents=0`）升級為規則層 + DB 統計的雙重證據。
4. F-05 補上 `summarize()` 靜默 null 的具體失效路徑。
5. 新增 F-09，並記錄「零變更 skip 分支安全」這條止血邊界。

### 查證用的可重跑命令

```bash
# F-01 最小重現（獨立 temp repo + 獨立 CORT_CACHE_DIR，不觸碰本專案索引）
cd /tmp && rm -rf repro && mkdir -p repro/src && cd repro && git init -q
printf 'export function target() {\n  return 1;\n}\n' > src/callee.ts
printf "import { target } from './callee';\n\nexport function caller() {\n  return target();\n}\n" > src/caller.ts
git add -A && git -c user.name=a -c user.email=a@b commit -qm init
B=/path/to/cortexyoung/rust/target/debug/cort
CORT_CACHE_DIR=/tmp/repro-cache $B index .
CORT_CACHE_DIR=/tmp/repro-cache $B impact --symbol target -f lean   # dependents=1
printf 'export function target() {\n  return 2;\n}\n' > src/callee.ts
CORT_CACHE_DIR=/tmp/repro-cache $B index --incremental .            # relationships=0
CORT_CACHE_DIR=/tmp/repro-cache $B status . | grep -E 'relationships|index_is_stale'
CORT_CACHE_DIR=/tmp/repro-cache $B impact --symbol target -f lean   # dependents=0

# F-04 規則與 DB 統計
rg -n "^id: .*edge" src/pack/rules/*.yml
python3 -B -c "import sqlite3; c=sqlite3.connect('/tmp/<cache>/<project_id>.db'); \
print(c.execute('select language,count(*) from chunks group by 1').fetchall()); \
print(c.execute('select c.language,r.rel_type,count(*) from relationships r join chunks c on c.chunk_id=r.source_chunk_id group by 1,2').fetchall())"

# F-05 / F-06
grep -n "cort.js" evals/run-agents.mjs
test -f evals/relation-cost.mjs || echo "relation-cost MISSING"
cd rust && cargo fmt --all -- --check | grep -c '^Diff in'
```

F-02 的 scratch 量測需要獨立 `HOME`；若同時要驗證 build 分支可用，必須把 `RUSTUP_HOME`/`CARGO_HOME` 指向
真實 toolchain，否則 rustup 在新 HOME 下沒有 default toolchain，會得到與產品無關的環境錯誤（本輪第一次
嘗試即因此失敗，改用顯式 `RUSTUP_HOME`/`CARGO_HOME` 後成功）。
