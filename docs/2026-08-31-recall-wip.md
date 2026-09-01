# 召回線（receiver 抽取 ＋ call-site 證據）：schema v4 落地紀錄（2026-08-31）

> 狀態（2026-08-31 同日更新）：**schema v4 已落地**（第 2 節改寫為實測結果），第 3 節的殘留仍未動。
> 本檔從「接續規格」改成「這一段做了什麼、量到什麼、還剩什麼」。
> 原狀態：**可接續的工作紀錄**，不是結論。目標句在 `AGENTS.md`（`CLAUDE.md` 是其 symlink）：
> 把 agent 本來就會做、而且常做錯的那次呼叫點枚舉，變便宜**且可查證**。
> 前半（便宜）已有證據；後半（可查證）是本檔要接的事。
>
> 本檔只記「做哪一件、按什麼順序、驗收看什麼」。判讀與數據在：
> `docs/2026-08-31-demand-recheck.md`（需求面）、`docs/2026-08-31-coverage-external-review.md`
> （兩輪外部覆核 ＋ 反事實實驗）、`docs/2026-08-31-rust-qualified-call-resolution.md`（解析根因）。

---

## 1. 這一段已經落地什麼（全部已推 master；⚠ `209fa06f` 那筆**沒過 CI**，見 §3 項目 0）

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
| `a0269cda` | **schema v4**：`raw_edges.call_form` ＋ `relationships.call_site_line`／`call_form`；Rust pack 新增 receiver 規則；解析端「唯一才連 ＋ receiver 必須能綁到該 owner」；`impact` 印 `@<呼叫行> <form>`；`verify-impact` 新增逐行查證 |
| `209fa06f` | 把「上游 receiver 閘」與「本地 module-path 後綴解析（`module_segments`／`expand_use_path`／`split_call_path`／`Candidate`／`resolve_candidates`）＋ Rust `edge:imports`」合併。**這筆沒過倉庫 gate**：rustfmt 4 個 hunk、clippy `dead_code`（`chunks_named` 被重寫後的 `resolve_targets` 遺留）、`ReceiverIndex` 的文件註解在合併時跑到 `ReceiverCandidate` 上面（結構體因此沒有說明了）。 |
| `8d43628c` | 修掉上面三項（含把註解放回 `ReceiverIndex`）、補 `a_std_module_qualifier_that_matches_a_local_module_file_still_attaches` 把 std／本地模組同名的洞**釘住**、並更正 README 對這個洞的講法（它說會回 `AMBIGUOUS`，實測是回 `INFERRED` 的假邊）。合併後的實測：本倉庫同一棵樹 1,386→1,401 條邊（+18 條 module-path 全對、6 條原本 AMBIGUOUS 被 `use` 收窄）、cct 1,839 條與五條鏈逐條不變。 |
| `c689080f` | **coverage-v2**：`unparsed` 不再翻布林值（改 advisory、`why: [unparsed_files]`）、`COVERAGE_METHOD` 升 v2、`reading`／skill／README 三處同時寫明「讀列，不要讀布林值」舆 `false` 能結論到什麼（含 K3 講漏的 >2MB、`dist`/`target` 下來源檔、±2 行容差），lean 多一列 advisory 說明。60 個隨機符號實測：本倉庫 true 從 60/60 變成 5/60。 |
| `ad1caf65` | 宣告行不再被標成 `call`（`DECLARATION_KEYWORDS`；本倉庫 295 行、cct 6,693 行這類宣告不在任何 chunk 起點上）；`blind` 列的 `unread=-` 區分「沒算過」舆 0。 |
| `9d95f8a9` | **lean 印的是它留下的列,不是它找到的列**(2026-09-01)。`no_edge=` 原本是截斷後陣列的長度,本倉庫 `Tally::add` 因此在 61 列上回報 `no_edge=20` 且毫無跡象。改成印 `gap_count`,並在 rows **之前**插一列 `miss	truncated	shown=N	of=M`。這是 `34a2ca10` 的同一個缺陷往上一層:那筆修了排序,沒修「有沒有被切」的揭露,而缺的正是 skill 唯一叫 agent 用的格式。同時把「完整性」從「可查證」的附屬性質升格成 `AGENTS.md` 的獨立目標,並更正目標句裡 7.7x／6.7x 的自相矛盾。測試 `a_truncated_gap_list_says_how_many_rows_it_dropped` |
| `(本提交)` | **coverage L2 納入 imports**(2026-09-01)。實測根因先釘死:Rust `edge:imports` 的 raw 邊**從不成 relationship**——頂層 `use` 不屬於任何函式 chunk(`source_symbol` 是空字串),`chunk_by_symbol.get("")` 落空,解析前就整批丟掉(本倉庫 336 條 import raw 邊、0 條 import relationship);TS 的模組路徑(`./utils`)不是符號名,同樣不匹配。所以 audit 講的「import 邊沒有查證面」拆成兩半:**連錯不可能**(從未成邊),**丟掉不可見**才是真洞——L2 只查 `rel_type='calls'`,而 L1 文字層分不出「pack 沒抽到」和「pack 抽到、解析丟掉」。修法:L2 加查 imports,leaf 用 `expand_use_path` 開 braces(2461d2c8 在 recall-exp 修過的盲點不在這裡重演),抑制用**檔案層級**(該檔已有任一 resolved 邊觸及 seed → 抑制;沒有 → 這個檔案對 seed 的依賴整個缺席於圖,drop 列是它唯一的 pack 認證痕跡)。真實語料:49 列新可見、241 列抑制為重複。刻意**不做**「讓 import 成邊」——那是把檔案級 `use` 硬掛到某個 chunk,是編造答案;目標句說算了 |
| `(本提交)` | `install.sh` 加 `record_deploy` → `~/.local/share/cortexyoung/deploy.log`(`<iso8601>\t<sha>\t<dest>`,一行一次實際變更,移除記 `absent`)。動機是量測本身出現盲點:**skill 的曝光只能靠 session 開場 prompt 裡的 `description` 反推**,而 2026-09-01 那筆只改 body、`description` 逐位元組不變 —— 這個方法在第一次被用到的同一天就瞎了。smoke test 加 5 項(含把該盲點重現) |
| `(本提交)` | `cort-evals recall-exp --venue DIR`：把決定 B／C 的反事實計數從 `/tmp` 的 Python 腳本收成 Rust 子命令（只讀文字、不開 DB、不重刻閘），並附上兩場域現況數字。 |

量到的事實（決定優先序用的，不是敘事）：
- 需求面：1,214 筆可用指令 → 問關係 **1 筆（0.08%）**；需呼叫點集合的寫入任務裁決後 **4 嚴格／7 含弱**；**877 筆（41.9%）是貼回來的 agent 報告**。
- 成本面：cort 臂 10/10、平均 992 tool-return tokens；基線臂 4/10、7,642（**比較對象是 agent 的整個 shell，不是 rg**，F-11）。
- 召回實驗（不改 pack，用現況 DB ＋ 盤上文字）：Rust 5,522 個 receiver 呼叫點中**只有 160 個**能在專案內找到唯一符號，96.5% 根本不指向任何專案符號；`crate::` 類殘留在解析修復後＝**0**，剩下 241 次限定呼叫全是 `String::new`／`PathBuf::from`／`fs::write`／`Vec::new`（不該連）→ **B（補 Rust import 邊）建議不做**。

## 2. schema v4：做了什麼、量到什麼（原「下一步」，已實作）

### 實測結果（同一棵樹、同一份 cct HEAD，v3 binary vs v4 binary）

> 下面每一格都能稽核：`evals/runs/2026-08-31-schema-v4/`（兩臂的 `verify-impact` 報告、
> 「只做唯一性閘」的反事實清單 25 條逐條附來源行、以及重現步驟）。

| 項目 | v3 (`bc5f887`) | v4 |
|---|---|---|
| 本倉庫 relationships | 1,360 | **1,369**（+9，全部是 receiver 邊） |
| 本倉庫 receiver 呼叫點（raw_edges） | 0（pack 不抽） | 4,833 列，連上 9 條（**這格是 `a0269cda` 那棵樹**；目前樹上是 5,212 列、連上 12 條、12 條全對） |
| `Tally::add --depth 3` | dependents=0 ＋ 兩列 receiver 漏洞 | **dependents=3**，h1 = `evals/src/demand.rs scan 598 @626 receiver` |
| cct（TS 場域）relationships | 1,839 | **1,839**（TS 沒加規則、基線未動） |
| cct `logInfo/getCurrentTimeET/handleReportsStatus/createBacktestingStorage` 依賴者數 | 66 / 23 / 4 / 20 | 66 / 23 / 4 / 20（逐條相同） |
| `verify-impact` 逐行查證（新） | 無此欄位 | cct 5 條鏈 **117/117 通過**（line_precision 1.0）；本倉庫 4 條鏈 **64/64** |
| `lean` 同查詢位元組 | 4,411 / 1,641 / 835 / 1,414 | 5,069 / 1,874 / 873 / 1,619（**+15%**，換掉「確認一條邊要再讀一次檔」） |

三個必须記下來的判讀：

1. **原本預測的 `dependents=2` 是錯的**，實測 `dependents=3`、其中 receiver 只貢獻 1 條。
   原因：`relationships` 的鍵是 (source, target, rel_type)，`scan()` 裡的兩次 `tally.add()`（626、648）
   在同一個 chunk，只會留下一條邊。這不是漏實作，是 chunk 粒度的定義；`call_site_line` 因此是
   「該依賴者**最早**的呼叫點」，文件與註解都照這個說法寫，不宣稱「只有一處呼叫」。
2. **「唯一才連」單獨用是不夠的**：它在本倉庫連出 25 條邊，其中 **12 條是憑空造的**（48% 準確率）——
   `e.kind()` 連到測試替身的 `FailFs::kind`、`status.code()` 連到名為 `code` 的 helper、
   `.chain()` 連到名為 `chain` 的函式。因此加了第二層 `graph::receiver_binds`：
   (a) `x.m()` 只可能連到「有 owner 的方法」（自由函式一律拒），(b) `self.m()` 只連同 `impl` 的 `Owner::m`，
   (c) 其餘要求 receiver 末段與 owner 名稱同形（大小寫／底線正規化後相等，或前後綴、且至少 3 字元）。
   結果：連 9 條、對 9 條；代價是四條真邊被拒（`b.problem()`×3、`err.to_json()`），
   它們照舊以 `extracted_but_unresolved` 出現。**只做名子比對、不做型別檢查**這件事寫進
   `confidence_reasoning`，也寫進 README 限制 #8。
3. **`verify-impact` 的逐行查證比原本的整段查證更準**：`Tally::add` 的 body 檢查只有 0.667
   （檔案裡寫的是 `tally.add`，seed 卻是 `Tally::add`），逐行檢查 1.0。這也暴露一次自己的流程錯誤：
   我第一次跑逐行查證拿到 0.81，原因是**我改完文件沒重新索引**，`stale=true` 早就講了——
   旗標是有用的，被忽略的是我。

### 遷移與相容性

* v3 既存 DB：`ensure_schema` 對兩張表下 `ALTER TABLE ADD COLUMN`（`CREATE TABLE IF NOT EXISTS` 永遠
  不會補欄位），沿用 `graph_pending` 安全閥 → 下一次 `index --incremental` 走全量重建。
  實測 cct 的 v3 cache 直接升級成功（另外也被 `extractor_version` 變化擋了一次，兩條路都通）。
* `ALTER TABLE ... ADD COLUMN ... CHECK(...)` 在內建 SQLite 上**確實生效**（`the_call_form_column_is_checked_on_upgraded_and_fresh_databases` 同時驗既存庫與新庫）。
* 輸出契約：`lean` 依賴者列固定六欄，缺呼叫點時印 `@-` `-`，**不會**縮成四欄（形狀會變=會被讀成不同的宣稱）。
* 規則訊息：`edge:calls` → `edge:calls:bare|scoped|receiver`；`chunker::parse_edge_tag` 讀不懂的
  form／rel_type **丟掉該條邊並計入 `malformed`**，不猜：猜 `bare` 會把不該連的邊連上、
  同時讓 `--coverage` 以為那行已被覆蓋（K3 那一類「把不安全讀成安全」的錯誤）。
* Rust 文法注意：ast-grep 0.45.2 的 Rust grammar **沒有 `method_call_expression`**，
  `t.add()` 是 `call_expression(function: field_expression(field: field_identifier))`。
  原規格寫的節點名會匹配不到東西、而且看起來像「這個檔案沒有方法呼叫」。

### 為什麼 `raw_target` 存 `t.add` 而不是裸名 `add`（與原規格的唯一差異）

原規格說「目標取 `field_identifier` 的裸名」。實作後改成存呼叫頭（receiver 一起存），三個理由：
1. `receiver_binds` 需要 receiver；丟掉它就只能靠「名字唯一」這一條，也就是上面那 12 條假邊。
2. `raw_edges` 的主鍵不含 form：裸名下 `add()` 與 `t.add()` 同一行會互相吃掉一條；存頭之後兩者 naturally 不同鍵，不再丟資料（`a_bare_and_a_receiver_call_of_the_same_name_on_one_line_are_two_rows`）。
3. `coverage` 的 `%.name` LIKE 與 `bare_name()` 本來就是為帶點目標寫的（`formatter.formatToParts`），JS/TS 側早就是這個形状。

### 動手前的規格（保留備查；標 ※ 的三處已被上面的實作／量測推翻）

※ **v4 其實是三欄，不是兩欄**：`relationships` 也需要 `call_form`。輸出契約要印 form，而 form 存在
`raw_edges`；resolution 之後那一條邊再也回不去 raw 層（同一個 (source,target) 可能由多種形式連來），
所以要嘛在 relationships 存 form、要嘛放弃「這條邊是哪種形状連上的」這個可查證資訊。選存欄。

※ 原第 3 步的節點名與裸名目標都錯了，見上兩節。

**為什麼必須同一次遷移**（這是動手前挖出來的硬前提）：
- 「唯一才連」**不能套在裸名上**。今天多候選是照連並標 `AMBIGUOUS`，cct 的標籤就依賴它
  （`getCurrentTimeET --depth 3` seeds=2 由此而來）。套到裸名等於**降低**現有召回並讓已記錄基線漂移。
- 所以要區分「邊的形式」。規則層只有 `message: edge:calls` 一條通道，而 `chunker.rs` 把 `edge:`
  之後整段當成 `rel_type`，`relationships.rel_type` 的 CHECK 只准 `imports|exports|calls`。
  → 需要欄位，不是靠字串繞。

**v4 兩欄**（※ 三欄）：
1. `raw_edges.call_form TEXT NOT NULL DEFAULT 'bare'`（`bare|receiver|scoped`）——唯一性閘只作用在 `receiver`。
2. `relationships.call_site_line INTEGER`——資料早在 `raw_edges.start_line`（以及 `Edge.start_line`），
   只在 `relationship_rows_for_symbol_map()` 被丟掉。這是原先延後的那第 3 項，同一個版本跳、同一次重索引。

**執行順序**（逐步可編譯、可測，勿跳）：
1. `rust/src/schema.sql` 加兩欄；`rust/src/db.rs` `SCHEMA_VERSION=4`，升級路徑對**既存表**下
   `ALTER TABLE`（`CREATE TABLE IF NOT EXISTS` 永遠不會補欄位），沿用 `graph_pending` 安全閥。
2. `rust/src/chunker.rs`：`edge:` 後綴解析成 `rel_type` ＋ 可选 `:form`；`Edge` 帶 `call_form`；
   `replace_file_raw_edges` / `INSERT_RAW_EDGE` 帶該欄。
3. ※ `src/pack/rules/rust.yml`：新增 `method_call_expression` 規則，`message: edge:calls:receiver`，
   目標取 `field: (field_identifier)` 的裸名。
   → 實作：0.45.2 的 Rust grammar 沒有 `method_call_expression`，用
   `call_expression → function: field_expression → field: field_identifier`；目標存呼叫頭 `t.add`。
4. `rust/src/graph.rs`：`receiver` 形式**只在專案內候選恰好 1 時**連邊，否則回空 → 由
   `coverage.extracted_but_unresolved` 顯形（該層已存在，無需新機制）。
5. 輸出：`impact` 的 JSON 每列加 `call_site_line`／`call_form`；lean 依賴者列改成
   `h<跳>	<檔>	<符號>	<定義行>	@<呼叫行>	<form>`。同步 `rust/tests/render.rs` 契約。
6. 覆核與基線：重索引 cct，確認 `getCurrentTimeET --depth 3` 仍 8 個依賴者（TS 不新增規則）；
   → 實作：本機 cct 已在 `86a5ee6`，同一棵樹 v3/v4 兩個 binary 都是 **seeds=2 / dependents=23**
   （「8」在這台機器上不可重現，应是旧 HEAD 或旧 depth 的數字）；不變量是「v3 與 v4 逐條相同」，
   已用五條鏈 117 個依賴者驗證。
   跑 `cort-evals verify-impact` 對新連上的 receiver 邊逐條查證；`cort-evals run-agents` 若要拿
   C 的前後對照，需另取樣（花額度，先問）。

**驗收（已跑過，結果見上）**：`Tally::add` 從 `dependents=0 + 兩列 receiver 漏洞` 變成
`dependents=3`、h1 帶 `@626 receiver`；多候選／綁不上 owner 的 receiver **仍然不連**、且以
`extracted_but_unresolved` 形式可見；`Vec::new` 仍留在 `unresolved`；`x.get()` 之類 0 候選的呼叫
不進圖、也不進 `dependencies`。

## 3. 已知殘留（有據，未修）

0. **v4 新增的兩條，是自己量出來、不是別人問出來的**：
   (a) receiver 閘的召回損失是主觀選擇——`b.problem()`／`err.to_json()` 型別的「變數名不像該型別」
   一律拒；要祿回這些邊，下一步是讀 `let b = BatchRead::load(...)` 這種類構造式綁定，不是放寬閘。
   (b) `lean` 依賴者列 +15% 位元組（README「Token cost」有數）。
   (c) 逐行查證能抓到憑空的邊，**抓不到型別錯的邊**：一行寫了 `e.kind()` 就「證實」了 `e.kind`；
   所以 `line_precision=1.0` 不是正確性證明，別引用成那樣。
1. ~~`unparsed` 仍會把每個 seed 的旗標翻成 true~~ **已做（coverage-v2，與措辭同時落地）**：
   `unread_gap = unindexed + scan_skipped` 才會翻布林值；`unparsed` 只進 `blind_files`（含路徑）
   與 `why: [unparsed_files]`（advisory）。同時改的還有：`COVERAGE_METHOD` 升 v2（語意变了就要换名字，
   否則讀到一份 v1 的 `true` 會以為是同一件事）、`reading` 字串寫明「讀列，不要讀布林值」舆
   `false` 能結論到什麼、skill 第一條補上 K3 講漏的三條邊界（>2MB、`dist`/`target`/`node_modules`
   下的來源檔、±2 行容差），lean 多一列 `blind unparsed advisory: …`。
   量測（60 個隨機符號、`--depth 1 --coverage`）：本倉庫 true 從「必定 60/60」變成 **5/60**；
   cct 63 個 seed 裡仍有 62 個 true —— 因為那些是真的有 gap 列，不是稀釋。所以「讀列」仍是主指令。
   回歸測試拆成兩條：`a_blind_file_is_never_a_clean_bill_of_health`（**未讀**檔仍必須翻）舆
   `a_file_with_no_chunks_is_advisory_and_does_not_flip_every_seed`（無 chunk 檔不得翻）。
2. ~~trait 內的方法宣告被標 `call`~~ **已修**：`cause_of` 現在先看名字前面那個 token 是不是宣告關鍵字
   （`fn/struct/enum/trait/union/type/const/static/mod/impl`）→ `definition`。
   類別大小（實測）：本倉庫 **295** 行宣告行不在任何 chunk 的 `start_line` 上、cct **6,693** 行——
   這些行只要點到某個 seed 的名字，在 v1 就会被標成 `call`（rank 1，排在真實漏洞前面）。
   方向是**降級**該列的嚴重度而不是刪掉它，所以仍可在列裡看到，只是不再冒充呼叫點。
   測試：`a_declaration_line_is_never_a_call_even_when_the_declaration_is_not_a_chunk`。
   順手修一個自己造成的渲染缺陷：`blind` 列現在 `unread=-` 表示「沒算過」（no-seed 那條路徑不讀盤），
   而不是空白或 0（空白會被讀成 0，0 會被讀成「沒有跳過任何檔」＝那個假安全宣稱本身）。
3. `LINE_TOLERANCE = 2` 會把離已抽取呼叫 ≤2 行的提及算成已覆蓋（可能吞掉真漏洞）。
4. 非來源檔（`.sh`／`.txt`／設定檔）與 `IGNORE_DIRS` 下的來源檔，三層全部看不見——**邊界，不是 bug**。
5. `.wrangler/tmp/deploy-*/index.js` 7 份 bundle 在 cct 索引裡（`IGNORE_DIRS` 不含 `.wrangler`）：
   我們引用過的 cct chunk／edge 數都含它們。動 `IGNORE_DIRS` 會移動基線，**未決**。
6. ~~`mod::f()` 仍不解析~~ **已被 `209fa06f` 翻案**：Rust 現在有 `edge:imports`（`use_declaration`）與
   module-path 後綴解析，本倉庫因此 +18 條真邊、並把 6 條 AMBIGUOUS 收對。
   當初「B 不做」的結論之所以相反，是因為當時想像的做法是「把 `fs::write` 退回末段 `write` 再全域比名字」；
   實作的版本比的是**模組路徑**（`src/fs.rs` 的 module 結尾是 `fs`），所以 std／依賴呼叫照樣落空、
   留在 `--coverage` 的列裡。教訓：判定「不做」时要寫清楚是**哪種機制**不做。
   仍未做：`pub mod def;` 不是 chunk（mod 聲明還是沒有圖層身份）。
7. **std／本地模組同名會造假邊**（`use std::fs;` + `fs::write()` 遇到專案裡的 `src/fs.rs::write`
   → 直接連成 INFERRED，不會 AMBIGUOUS，因為外部 crate 根本沒入索引、沒有第二個候選可否決它）。
   要修得靠「本 crate 名字」或 `mod` 聲明，也就是上面那條；目前用測試釘住行為，README 限制 #8 明寫。

## 4. 接續用的環境事實

- 產品 binary：`./rust/target/release/cort`（不在 PATH；`install.sh` 另裝到 `~/.cargo/bin/cort`）。
- 索引快取：本倉庫 `CORT_CACHE_DIR=/tmp/cort-self-rs`；cct 場域 `CORT_CACHE_DIR=/tmp/cort-exp`
  （cct HEAD `b41e39d`，全量重索引約 2 分鐘）；`/tmp/cort-exp/usage.db` 是使用記錄，別當專案檔。
- 最小重現：`/tmp/rq`（`crate::def::my_func()` 解析）、`/tmp/blindx`（盲檔／scan_skipped）。
- ~~反事實實驗腳本在 `/tmp/exp/exp.py`（未進 repo）~~ **已收進來**：
  `cort-evals recall-exp --venue DIR [--top N]`（`evals/src/recall.rs`）。它只讀來源文字、不開
  cort 的 DB、也不重刻 `receiver_binds`——所以它的 `unique` 是**文字面上界**，
  與本檔前面那個「160 個唯一可連」（用 cort chunks 算的）不是同一個量，引用時要分開標。
  目前兩場域（**要連树一起引用**，數字會跟著工作樹動）：本倉庫 receiver 約 6,500 點（約 80% 無同名符號）、
  `::` 路徑進本地模組 48 點、被依賴同名遮蔽 **0** 點；cct 13,297 點、遮蔽 0 點。
  這個 0 值經過四次「往安心方向錯」的修正才可信：把內部路徑當風險、只讀根目錄 Cargo.toml、
  `cort-evals` 與 `cort_evals` 當成兩個 crate、以及**只看 `{` 前面的路徑**（`use std::{fs, io};`
  這種最常見的寫法整個看不見 → 有真實遮蔽却报 0）。最後一條是先寫出 `/tmp/k3chk` 那個反例才被抓出來的。
  腳本本身不再需要。
- 外部覆核指令包在 `/tmp/k3-prompt.txt`（同樣未進 repo；要常用就收進來當固定 reviewer brief）。
- 覆核引擎實測（2026-09-01 更正，先前這行寫錯、害下一次白等 45 分鐘）：**Kimi K3**
  `kimi -m kimi-code/k3 -p ...`。`-p` 與 `-y`／`--auto` **互斥，是 CLI 直接拒絕的**（`error: Cannot combine
  --prompt with --yolo.`），所以 `-p` 底下任何需要核准的工具呼叫（shell、建 repo）會**永久掛起**，
  而且看起來像在跑。可行做法：提示詞裡明確「唯讀、不準叫用工具」，並把要審的原始碼**內嵌**進去
  （讓它自己 Read 會把時間燒在探索上）；`--output-format stream-json` 可看到它卡在哪個工具。
  第一次送 8 項 + 要它自建 repo：45 分鐘零輸出；第二次禁 shell：它改自己讀檔，15 分鐘被 timeout 砍；
  第三次內嵌 coverage.rs + 單一大哉問：交貨（見覆核紀錄第三輪）。
  **agy** `agy -p ... --model gemini-3.1-pro-high --dangerously-skip-permissions`（這個有真正的跳核准旗標）；
  **zglmcode** 預設 `glm-5.3` 在 `-p` 路徑被 `unrecognized_model` 拒（`--model glm-5.1` 可用），
  且該帳號已撞 429 週限額（2026-09-05 重置）。
- Gate 慣例（兩 crate 分別跑）：`cargo fmt --all -- --check`、`cargo clippy --all-targets
  --all-features -- -D warnings`、`cargo test --locked --all-targets`、`bash tests/install-smoke.sh`；
  改過 `skills/*/SKILL.md` 要 `bash install.sh` 後比對三處 sha256 一致。

## 5. 採用率與「這功能到底有沒有用」(2026-09-01 加)

這一節記的是**量測方法本身的失敗**,不是功能。

**曝光只能靠 `description` 反推,而這個方法已經瞎了。** skill 的 catalog description 在 session
啟動時被快照進開場 prompt;body 是延遲載入。所以「某個 session 有沒有看到新 trigger」只能 grep 它
的前幾行。實測(`996ec66e` 於 08-31 11:25 推):

| session 啟動 | catalog 裡的 description |
|---|---|
| 08-31 12:10 → 18:07(7 個) | 舊 |
| 08-31 20:10 → 09-01 09:23(7 個) | 新 |

source 改了到部署生效差**約 8.75 小時**(要等 `install.sh` 跑)。而這個推論法只在 description 本身
變了時才有效——`9d95f8a9` 改的是 body,`description` 逐位元組相同。所以才有了 `deploy.log`。

**曝光後的實測:14 個 session,執行過的 `cort impact` = 0**(用 `"command"` 欄位數,不是文字提及;
唯一有執行的是做這次稽核的那個 session)。**但分母未知**——沒人數過那 14 個 session 裡有幾次是
真正該叫用的機會。所以現在是「0 / 未知」,還不能斷定路由失敗。

**因此撤回一個原本要做的儀表。** 原計畫在 `command_log` 加 `skill_sha`,把每次叫用綁到 skill 版本。
不做了:用量是**落後指標**,分子被需求(0.33-0.58%)結構性壓在接近零,再精緻的刻度也長不出分子。
改成量**效果**——被用到時答案有沒有變對——那不依賴採用率。

**效果的前半已經量到了,零成本。** `CortError::to_json` 的真實呼叫者全部被 receiver 閘拒絕:

```
臂 1  cort impact --symbol 'CortError::to_json' --depth 1 -f lean
      # impact CortError::to_json depth=1 seeds=1 dependents=0 stale=false
      → 一個乾淨、自信、錯誤的清白

臂 2  同上 + --coverage
      seed  CortError::to_json  mentions=12  no_edge=10  dropped=1  incomplete=true
      miss  receiver  rust/src/render.rs:397    pretty(&err.to_json())
      miss  receiver  rust/tests/errors.rs:13   err.to_json(),
      miss  receiver  rust/tests/render.rs:403  assert_eq!(parsed, err.to_json());
      → rank-0 的列逐條點名每個真實呼叫者,帶檔案與行號
```

**資訊在**,而且把假清白換成了可行動的正確答案。**未證**的是 agent 會不會照做——它可能看到 10 列
`receiver` 仍然說「可以刪」。那需要兩臂 agent eval,判分標準是**有沒有做出假清白宣稱**,不是答對幾個。
`CortError::to_json` 可直接當 fixture,不必發明案例。
