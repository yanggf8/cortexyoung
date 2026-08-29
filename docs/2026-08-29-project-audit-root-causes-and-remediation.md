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
| F-14 | P2 | `rust/tests/fixtures/fake-ast-grep` 是 46 行 Python，被 5 個測試當假 parser 用，抵觸純 Rust 契約 | 已在 `AGENTS.md` 標為「不得擴大的已知例外」；待移植成 Rust fixture bin |
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
