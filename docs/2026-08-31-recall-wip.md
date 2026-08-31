# WIP：召回線（receiver 抽取 ＋ call-site 證據）接續規格（2026-08-31）

> 狀態：**可接續的工作紀錄**，不是結論。目標句在 `AGENTS.md`（`CLAUDE.md` 是其 symlink）：
> 把 agent 本來就會做、而且常做錯的那次呼叫點枚舉，變便宜**且可查證**。
> 前半（便宜）已有證據；後半（可查證）是本檔要接的事。
>
> 本檔只記「做哪一件、按什麼順序、驗收看什麼」。判讀與數據在：
> `docs/2026-08-31-demand-recheck.md`（需求面）、`docs/2026-08-31-coverage-external-review.md`
> （兩輪外部覆核 ＋ 反事實實驗）、`docs/2026-08-31-rust-qualified-call-resolution.md`（解析根因）。

---

## 1. 這一段已經落地什麼（全部已推 master、CI 綠）

| commit | 內容 |
|---|---|
| `8cff134` | `cort-evals demand`：需求面螢幕（剝貼上 + A/B 分類 + 逐筆可稽核）。產物與人工裁決入 `evals/runs/2026-08-31-demand/` |
| `996ec66` | skill 改由**任務動詞**觸發（改名/刪除/遷移/抽取/替換/宣稱沒人用）；評測集加第 6 個「刪除句型」任務；smoke 的載入器探測改為從來源推導 |
| `9be7c16` | 需求重驗文件 ＋ `2026-08-28-real-session-cost.md` 蓋上「證據基礎已被留存策略刪除」校正區塊 |
| `0e64e56` | `impact --coverage`：三層召回診斷（未成邊提及／解析掉落／盲檔），每筆帶**原因**與排序 |
| `78fad50` | 盲檔不得當成安全通行证（外部覆核發現）；`unparsed_example` 給路徑 |
| `9743f65` | 單引號 `quoted` 判因、同行提及去重成 `occurrences`、seed 未入索引時改報 `no_seed_resolved` 而非 exit 1 |
| `3fd9d9b` | **解析根因修復**：`crate::`／`self::`／`super::`／`::` 限定呼叫退回最後一段；外部呼叫（`Vec::new`）刻意不退回、維持可見 |
| `80d92d0` | K3 第二輪覆核紀錄 ＋ B／C 反事實實驗計數 |
| `7ab31af` | `scan_skipped`：>2MB／讀檔失敗納為第三類盲檔並參與翻轉旗標 |

量到的事實（決定優先序用的，不是敘事）：
- 需求面：1,214 筆可用指令 → 問關係 **1 筆（0.08%）**；需呼叫點集合的寫入任務裁決後 **4 嚴格／7 含弱**；**877 筆（41.9%）是貼回來的 agent 報告**。
- 成本面：cort 臂 10/10、平均 992 tool-return tokens；基線臂 4/10、7,642（**比較對象是 agent 的整個 shell，不是 rg**，F-11）。
- 召回實驗（不改 pack，用現況 DB ＋ 盤上文字）：Rust 5,522 個 receiver 呼叫點中**只有 160 個**能在專案內找到唯一符號，96.5% 根本不指向任何專案符號；`crate::` 類殘留在解析修復後＝**0**，剩下 241 次限定呼叫全是 `String::new`／`PathBuf::from`／`fs::write`／`Vec::new`（不該連）→ **B（補 Rust import 邊）建議不做**。

## 2. 下一步：C ＋ 第 3 項，一次 schema v4 做不完就別做一半

**為什麼必須同一次遷移**（這是動手前挖出來的硬前提）：
- 「唯一才連」**不能套在裸名上**。今天多候選是照連並標 `AMBIGUOUS`，cct 的標籤就依賴它
  （`getCurrentTimeET --depth 3` seeds=2 由此而來）。套到裸名等於**降低**現有召回並讓已記錄基線漂移。
- 所以要區分「邊的形式」。規則層只有 `message: edge:calls` 一條通道，而 `chunker.rs` 把 `edge:`
  之後整段當成 `rel_type`，`relationships.rel_type` 的 CHECK 只准 `imports|exports|calls`。
  → 需要欄位，不是靠字串繞。

**v4 兩欄**：
1. `raw_edges.call_form TEXT NOT NULL DEFAULT 'bare'`（`bare|receiver|scoped`）——唯一性閘只作用在 `receiver`。
2. `relationships.call_site_line INTEGER`——資料早在 `raw_edges.start_line`（以及 `Edge.start_line`），
   只在 `relationship_rows_for_symbol_map()` 被丟掉。這是原先延後的那第 3 項，同一個版本跳、同一次重索引。

**執行順序**（逐步可編譯、可測，勿跳）：
1. `rust/src/schema.sql` 加兩欄；`rust/src/db.rs` `SCHEMA_VERSION=4`，升級路徑對**既存表**下
   `ALTER TABLE`（`CREATE TABLE IF NOT EXISTS` 永遠不會補欄位），沿用 `graph_pending` 安全閥。
2. `rust/src/chunker.rs`：`edge:` 後綴解析成 `rel_type` ＋ 可选 `:form`；`Edge` 帶 `call_form`；
   `replace_file_raw_edges` / `INSERT_RAW_EDGE` 帶該欄。
3. `src/pack/rules/rust.yml`：新增 `method_call_expression` 規則，`message: edge:calls:receiver`，
   目標取 `field: (field_identifier)` 的裸名。
4. `rust/src/graph.rs`：`receiver` 形式**只在專案內候選恰好 1 時**連邊，否則回空 → 由
   `coverage.extracted_but_unresolved` 顯形（該層已存在，無需新機制）。
5. 輸出：`impact` 的 JSON 每列加 `call_site_line`／`call_form`；lean 依賴者列改成
   `h<跳>	<檔>	<符號>	<定義行>	@<呼叫行>	<form>`。同步 `rust/tests/render.rs` 契約。
6. 覆核與基線：重索引 cct，確認 `getCurrentTimeET --depth 3` 仍 8 個依賴者（TS 不新增規則）；
   跑 `cort-evals verify-impact` 對新連上的 receiver 邊逐條查證；`cort-evals run-agents` 若要拿
   C 的前後對照，需另取樣（花額度，先問）。

**驗收要看到的東西**：本倉庫 `Tally::add` 從 `dependents=0 + 兩列 receiver 漏洞` 變成
`dependents=2`（且帶 `call_site_line`）；`x.get()` 這類多候選**仍然不連**、且仍以
`extracted_but_unresolved`／`receiver` 漏洞形式可見；`Vec::new` 仍留在 `unresolved`。

## 3. 已知殘留（有據，未修）

1. `unparsed` 仍會把**每個** seed 的旗標翻成 true（本倉庫 2 個、cct 4 個這種檔）→ 布林值鑑別力被稀釋。
   修法必須與「**讀列，不要讀布林值**」的 skill／README 措辭**同時**落地，否則只是換一種誤導。
2. trait 內的方法**宣告**（`fn add(&self, x: i32) -> i32;`）被標 `call`，因為定義判定要求精確 `start_line`。
3. `LINE_TOLERANCE = 2` 會把離已抽取呼叫 ≤2 行的提及算成已覆蓋（可能吞掉真漏洞）。
4. 非來源檔（`.sh`／`.txt`／設定檔）與 `IGNORE_DIRS` 下的來源檔，三層全部看不見——**邊界，不是 bug**。
5. `.wrangler/tmp/deploy-*/index.js` 7 份 bundle 在 cct 索引裡（`IGNORE_DIRS` 不含 `.wrangler`）：
   我們引用過的 cct chunk／edge 數都含它們。動 `IGNORE_DIRS` 會移動基線，**未決**。
6. `mod::f()`（靠 `use` 帶進來的模組路徑）仍不解析，因為 Rust 側沒有 import 圖、`pub mod` 也不是
   chunk。就是實驗裡判定「不做」的 B；翻案條件是換一個模塊間呼叫占比更高的 Rust 倉庫重跑計數。

## 4. 接續用的環境事實

- 產品 binary：`./rust/target/release/cort`（不在 PATH；`install.sh` 另裝到 `~/.cargo/bin/cort`）。
- 索引快取：本倉庫 `CORT_CACHE_DIR=/tmp/cort-self-rs`；cct 場域 `CORT_CACHE_DIR=/tmp/cort-exp`
  （cct HEAD `b41e39d`，全量重索引約 2 分鐘）；`/tmp/cort-exp/usage.db` 是使用記錄，別當專案檔。
- 最小重現：`/tmp/rq`（`crate::def::my_func()` 解析）、`/tmp/blindx`（盲檔／scan_skipped）。
- 反事實實驗腳本在 `/tmp/exp/exp.py`（**未進 repo**）。若要常态化，依純 Rust 契約做成
  `cort-evals recall-exp --venue DIR`，別放腳本。
- 外部覆核指令包在 `/tmp/k3-prompt.txt`（同樣未進 repo；要常用就收進來當固定 reviewer brief）。
- 覆核引擎實測：**Kimi K3** `kimi -m kimi-code/k3 -p ...`（`-p` 模式可用 shell；`-p` 與 `-y`／`--auto`
  互斥）；**agy** `agy -p ... --model gemini-3.1-pro-high --dangerously-skip-permissions`；
  **zglmcode** 預設 `glm-5.3` 在 `-p` 路徑被 `unrecognized_model` 拒（`--model glm-5.1` 可用），
  且該帳號已撞 429 週限額（2026-09-05 重置）。
- Gate 慣例（兩 crate 分別跑）：`cargo fmt --all -- --check`、`cargo clippy --all-targets
  --all-features -- -D warnings`、`cargo test --locked --all-targets`、`bash tests/install-smoke.sh`；
  改過 `skills/*/SKILL.md` 要 `bash install.sh` 後比對三處 sha256 一致。
