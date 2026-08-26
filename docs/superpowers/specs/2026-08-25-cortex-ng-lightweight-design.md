# Cortex NG 輕量版設計 — AST + Graph + FTS（`cort` CLI，離線 SQLite）

- **日期**: 2026-08-25
- **狀態**: Draft v4.2 — v4.1 經 Codex 第七次審查六項全 PASS（內部一致性）；v4.2 為**實機驗證**修正：`parse_failed` 偵測機制與版本釘死改為對 `ast-grep 0.45.2` 實測結果（見 §16）
- **實作**: 已完成（`a2f41625`）。§8 的四個交付階段 `index → struct → context → impact` 全數落地，`npm test` 107/107、`tests/install-smoke.sh` 39/39。實作計畫與四項偏離記錄見 `docs/superpowers/plans/2026-08-25-cortex-ng.md`。**唯一與本設計稿不符處**：§4 DDL 要求的 `tokenize='unicode61 "remove_diacritics 1" "tokenchars ._$"'` 因 bundled SQLite 3.49.2 拒絕任何參數化 `unicode61`，降級為 bare `unicode61`（`src/schema.sql` 有 NOTE，一行可還原）。`cort-v0.1.0` tag 依 §10 待 CI 雙平台綠燈才打。
- **前代**: `v6-final` (5a02c6e3) 歸檔；`de8638fd` 為 xgrep 精簡版
- **決策**: 輕量派 — 保留 AST + Graph + FTS，拔掉 embeddings/Turso/DiskANN；`ast-grep` 子行程為**唯一** parser 權威，`cort` 為 canonical CLI 名稱（內部固定調 `ast-grep`，不接受 `sg` alias）

## 1. 目標與非目標

**目標**：提供 agent 原生的本地離線 code intelligence，讓 `rg`/`xg` 處理重複字串搜尋，`ast-grep` 處理檔內結構，`cort` 補上 `ast-grep` 給不了的跨檔鄰居與爆炸半徑，並以 token-efficient 封包回傳。

- `cort index` / `cort index --incremental` — `ast-grep` 抽 chunk + 建圖 + FTS，離線、可增量
- `cort struct -p '<pattern>'` — `ast-grep run --json=stream` → containment join `chunk_id` → 附 ≤3 條 `EXTRACTED` 鄰居（MVP 階段 2）
- `cort context <symbol|query>` — exact symbol 或 FTS 候選 → depth-1 鄰居，實際輸出算 budget（階段 3，**FTS-only**，不依賴 `struct`）
- `cort impact --symbol <name>` — 反向 CTE 遞移 dependents（depth 3），作為 `rewrite` 的掃描範圍（階段 4）
- `cort rewrite -p '<pattern>' --rewrite '<repl>'` — dry-run JSON 先行，`--interactive` 預設（延後，見 §8）
- `cort modules` — Louvain 檔案級分群（延後）

**非目標**：向量語意搜尋（BGE-small-en-v1.5 / ANN）、雲端同步、全量重建以外的即時 rerank。 embeddings 保留擴充插槽但 v1 不建欄位（見 §4）。

**成功標準（僅限 MVP 命令）**：`cort struct` 的 recall/precision 優於裸 `ast-grep`（因附 graph 鄰居）、`cort context`（FTS-only）在萬檔 repo p95 <800ms 且實際輸出 ≤1500 tokens、`cort impact --symbol` vs 人標 precision ≥0.85、`cort index` 10k 檔 <60s。`search`/`impact --from-diff`/`rewrite`/`modules` 不列入 v1 成功標準。

## 2. 架構總覽

```
agent → cort CLI (Node.js, ESM) → SQLite (better-sqlite3, WAL, busy_timeout 5s, 0600)
                              │    ~/.cache/cortex-ng/<sha256(realpath)>.db
                              ├─→ ast-grep (子行程，唯一 parser) — index: scan --json=stream --config <pack>/sgconfig.yml（每檔一次）
                              │                                    query: run --json=stream
                              │                                    rewrite: run --rewrite --json=stream (dry-run)
                              ├─→ FTS5 (SQLite, unicode61) — 關鍵字召回
                              └─→ Louvain (process 內，檔案級鄰接，僅 Phase 1 greedy)
```

- **唯一 Parser 權威**：索引與查詢皆由 `ast-grep` 子行程提供結構。支援語言 = `ast-grep` 支援的語言（與 V6 的 TS/JS/TSX/Python 為交集，未來隨 ast-grep 擴充）；版本檢查 `ast-grep --version == 0.45.2`，不符則 fail-closed。exit≠0 分兩案：① **index 的 per-file `ast-grep scan` 失敗**（該檔 exit≠0、無 JSON、或該檔 JSON lines 皆 malformed）→ 只把**該檔** chunks 標 `chunk_source=unparsed`（FTS-only），其餘檔繼續，**不因單檔 malformed 中止整個 index**；② **query 的 `ast-grep run` pattern parse 失敗** → **不可**用 exit code 或 stderr 判定：實測 0.45.2，`run --json=stream` 的 zero-match 與 bad-pattern 輸出**逐位元組相同**（皆 exit=1、stdout 0 bytes、stderr 0 bytes，見 §16）。改為**強制 pre-flight**：先跑 `ast-grep run --debug-query=ast --lang <LANG> -p '<pattern>' <paths>`（`--lang` 為**必填**，缺少則 exit=2），其 stderr 含 `Pattern contains an ERROR node` → 回全域結構化錯誤 `{error:"parse_failed", detail}`，**不執行**後續 `run --json=stream`、**不產出**部分 struct 結果、不假裝 0 筆命中。pre-flight 通過後：`run --json=stream` exit=0 → 有命中；exit=1 且 stdout/stderr 皆空 → 真 0 筆；exit≠0 且 stderr 非空 → operational error（如路徑不存在）。malformed JSON line 的 10% abort 適用 `run` 路徑（struct 與 rewrite）的 `ast-grep run --json=stream`，**不**適用 index `scan`。**不使用 `web-tree-sitter` in-process 解析**，避免雙重真理。
- **Graph 自己擁有、Matcher 去租**：`cort` 擁有 `chunks`/`file_state`/`relationships` 的寫入與交易；`ast-grep` 只提供 matches。index 期用版本化 YAML extractor pack（每檔單一 `scan --config <pack>/sgconfig.yml`）建圖；query 期 `ast-grep run --json=stream` 再 containment join；rewrite 期先 dry-run。
- **存放**：`better-sqlite3` **唯一** driver（測試用 in-memory `better-sqlite3`，**不**用 `node:sqlite` 備援），WAL + `busy_timeout=5000`，`~/.cache/cortex-ng/` 單庫 per project。不用 JSON sidecar（無 CTE/watch）、不用 Turso/libSQL（向量以外無理由、FTS 實驗性、RTT 成本）。

**ast-grep 子行程合約（精確）**：

| 階段 | 命令 | 輸入 | 輸出 | 失敗 |
|------|------|------|------|------|
| index | `ast-grep scan --json=stream --config <pack>/sgconfig.yml <file>`（**每檔單一 scan**，同時抽 chunk 與關係；不是「無 config 的 scan + 第二次 scan」） | 單檔 | JSON lines: `{file, range:{start,end}, kind, text, meta}` | 該檔 exit≠0、無 JSON、或無任何可 parse 的 JSON line（含「全部行為 malformed」）→ 只標該檔 `chunk_source=unparsed`（FTS-only）。malformed line 跳過並**按檔計數**，**不 abort 整個 index**。10% abort **不適用** scan |
| struct | `ast-grep run --json=stream -p '<pattern>' --strictness ast <paths>` | pattern (argv, 單引號), globs | 同上 + `pattern` 匹配的 node 範圍 | pre-flight `--debug-query=ast --lang <LANG>` 的 stderr 含 `Pattern contains an ERROR node` → `{error:"parse_failed", detail}`，不執行 `run`、不產出部分 struct 結果（exit code 不可用：zero-match 與 bad-pattern 皆 exit=1 且雙流全空）。malformed JSON line（**此類 `run` 路徑，含 struct 與 rewrite**）→ 跳過並計數，超過該次輸出行 10% 則 abort **該次查詢** |
| rewrite | `ast-grep run --json=stream -p '<pat>' --rewrite '<repl>' <paths>` | 同上 + rewrite 模板 | diff JSON (range + replacement) | 同 struct（含 10% malformed abort）；`--interactive` 由 `cort` 封裝 |

不接受 `sg` binary；`command -v ast-grep` 不存在 → 安裝失敗，不 fallback。

## 3. 元件分解

| 元件 | 職責 | 介面 | 依賴 |
|------|------|------|------|
| `indexer` | 掃檔 → 每檔一次 `ast-grep scan --config <pack>/sgconfig.yml` 抽 chunk+關係 → 寫 `chunks`/`chunks_fts`/`file_state` → target 解析後寫 `relationships` | `cort index [path]` / `--incremental` | ast-grep (scan), better-sqlite3 |
| `incremental` | `extractor_version` 比對 → `git diff --name-status -M` + `ls-files --others` → `file_content_hash` 去重 → per-file 序列化重建（每檔一筆 transaction） | `cort index --incremental` | git, better-sqlite3 |
| `fts` | FTS5 儲存與查詢（unicode61, `remove_diacritics 1`, `tokenchars "._$"`），`sanitizeFtsQuery` 對 `" ( ) - /` 做 `""` 轉義，超長 OR 截斷 20 詞，malformed 大聲失敗 | 內部 `keywordSearch` | SQLite FTS5 |
| `struct` | `ast-grep run --json=stream` → **containment join** (`match.range` ⊆ `chunk.range`) → 回傳 file/symbol + ≤3 `EXTRACTED` edges + confidence | `cort struct -p '<pat>'` | ast-grep |
| `graph` | `relationships` CRUD、target 解析（檔內 import map → 專案 symbol index）、confidence 單一常數（`EXTRACTED:1.0 / INFERRED:0.7 / AMBIGUOUS:0.5 × 1/N`（N≥1，N=已解析 target 數）；0 個 target 不寫列，unresolved 由 `context`/`impact` on-the-fly 內聯回傳、不入庫）、鄰居查詢 | 內部 `getNeighbors`/`getTransitiveDependents` | better-sqlite3 |
| `context` | exact `symbol_name` 命中或 FTS 候選（**不依賴 `struct`**）→ depth-1 鄰居 → 實際輸出算 budget 修剪 → `index_is_stale`/`truncated`；0 target 不讀 relationship 列，on-the-fly 內聯 `unresolved` JSON | `cort context <q>` | graph, FTS |
| `impact` | 反向 CTE depth 3；unresolved 同樣 on-the-fly 內聯，不入庫 | `cort impact --symbol` | better-sqlite3 CTE |
| `modules` | 取 cross-file `relationships` 建無向加權圖 → Louvain Phase 1 greedy（無 aggregation phase） | `cort modules --min-size` | clusterer (移植 V6 `clusterer.ts:109-181`) |
| `rewrite` | `ast-grep --rewrite` 包裝：dry-run JSON → 限縮至 `impact` 檔案清單 → `--interactive` 預設 | `cort rewrite -p … --rewrite …` | ast-grep |

## 4. 資料模型（SQLite，可執行 DDL）

Driver：`better-sqlite3` 唯一。沿用 `v6-final:turso.ts:49-96` 命名，**不建 `embedding` 欄位**（日後 `ALTER TABLE chunks ADD COLUMN embedding BLOB` + `cort embed --backfill`）：

```sql
PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS _cortex_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
); -- SCHEMA_VERSION=1, extractor_version=<sha256(pack)>

CREATE TABLE IF NOT EXISTS projects (
  project_id TEXT PRIMARY KEY,          -- sha256(realpath)
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  git_head TEXT,
  last_indexed_at INTEGER,              -- epoch ms
  extractor_version TEXT NOT NULL,
  created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS chunks (
  chunk_id TEXT PRIMARY KEY,            -- ${project_id}:${file_path}:${start_line}  (V6 公式，見 ast-chunker.ts:494-508)
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  symbol_name TEXT,
  chunk_type TEXT CHECK(chunk_type IN ('function','class','method','config','documentation','unparsed')),
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,           -- sha256(content) per-chunk；檔級 freshness 用 file_state.file_content_hash（chunks + edges）
  language TEXT,
  chunk_source TEXT NOT NULL CHECK(chunk_source IN ('ast','unparsed')),
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_chunks_project ON chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(project_id, file_path);
CREATE INDEX IF NOT EXISTS idx_chunks_symbol ON chunks(project_id, symbol_name);

CREATE TABLE IF NOT EXISTS file_state (
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  file_content_hash TEXT NOT NULL,  -- sha256(concat(sorted chunk contents + sorted relationship edge strings for this file))
  updated_at TEXT DEFAULT (datetime('now')),
  PRIMARY KEY (project_id, file_path)
);

CREATE TABLE IF NOT EXISTS relationships (
  source_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  target_chunk_id TEXT NOT NULL REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  rel_type TEXT NOT NULL CHECK(rel_type IN ('imports','exports','calls')),
  confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence_score REAL NOT NULL CHECK(confidence_score BETWEEN 0 AND 1),
  confidence_reasoning TEXT,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);
-- 不建 unresolved_refs 表。0 個 target 不寫 relationships 列；
-- unresolved reasoning 由 context/impact on-the-fly 內聯於 JSON，不入庫、不帶 FK。

-- FTS5 with unicode61, triggers mirror V6
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
  content, symbol_name, file_path,
  content=chunks, content_rowid=rowid,
  tokenize='unicode61 "remove_diacritics 1" "tokenchars ._$"'
);
CREATE TRIGGER IF NOT EXISTS chunks_fts_insert AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;
CREATE TRIGGER IF NOT EXISTS chunks_fts_delete AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
END;
CREATE TRIGGER IF NOT EXISTS chunks_fts_update AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;
```

- `chunk_id` 採用 V6 公式 `project_id:file_path:start_line`（audit 已知「同行不動則穩定」的限制，見 `5a02c6e3:docs/2026-08-25-audit-and-repositioning.md:78`；未來三臂 RRF 時仍以此為穩定鍵，移動的行視為新 chunk）。
- **行號基準**：`ast-grep` JSON 的 `range.start.line` / `range.end.line` 為 **0-indexed**（實測 0.45.2）；`chunks.start_line` / `end_line` 與 `chunk_id` 一律存 **1-indexed**，讀入時統一 `+1` 正規化。containment join 與 `file_content_hash` 的排序皆以正規化後的 1-indexed 值為準。
- `extractor_version = sha256(concat(sorted YAML pack files))`，`ensureSchema` 冪等遷移，`_cortex_meta` 存 `SCHEMA_VERSION`。pack YAML 變更走 `extractor_version` mismatch → 全量重建（graph 與 chunks 皆重建）。
- `file_state.file_content_hash = sha256(concat(依 start_line 排序的 chunk contents + 該檔 scan 抽出的 relationship edge strings 詞典序排序))`。edge string 正規式：`${rel_type}\t${source_symbol}\t${raw_target}`（scan 抽出、**target 解析前**的名字，含後來因 0 命中而不寫列的邊）。chunk 內容或關係抽出任一變更都會改 hash；**freshness 訊號分工**：`file_content_hash` 管單檔抽出物（chunks + edges）是否 stale，`extractor_version` 管 pack 規則變更。
- **交易邊界**：全量 `cort index` 包在**單一** SQLite transaction（全部檔的 chunks / file_state / relationships / `projects.git_head` / `_cortex_meta.extractor_version` 一起 commit）。`cort index --incremental` 用 **per-file transaction**（`BEGIN; deleteStaleFileChunks; insert chunks; upsert file_state; replace 該檔 relationships; COMMIT`）；中斷時**已 commit 的檔保留**（視為增量進度，不是「沒有半套索引」），未 commit 的當前檔 rollback。incremental **全部檔跑完後**，另開**最後一筆 transaction** 原子更新 `projects.git_head` 與 `_cortex_meta.extractor_version`。未來可加 generation swap（寫至 `*.db.tmp` 再 `rename`）。

## 5. CLI 規格

```
cort index [path] [--incremental]
cort struct -p '<pattern>' --lang <LANG> [-g '<glob>'] [--json]   # 階段 2；--lang 必填（pre-flight 需要）
cort context <symbol|query> [--budget 1500] [--json]       # 階段 3，FTS-only
cort impact --symbol <name> [--depth 3] [--json]           # 階段 4
cort search <query> [--json] [--limit N]                   # 薄包裝 FTS，延後
cort impact --from-diff [sha] [--json]                     # 延後
cort rewrite -p '<pat>' --rewrite '<repl>' --lang <LANG> [--interactive] [--json]  # 延後
cort modules [--min-size N] [--json]                       # 延後（僅 Phase 1 greedy）
cort status / cort projects / cort delete
```

- `cort` 為唯一 canonical binary；內部固定呼叫 `ast-grep`，不存在或版本不符 (`!=0.45.2`) 則 fail-closed，不接受 `sg`。
- Skill 路由（15–25 行）：`rg` 管新鮮/短模式、`xg`（若安裝）管重複大庫字串、`struct` 管 shape、`context`/`impact` 管 who-else/what-breaks；不做 PreToolUse nudge。

## 6. 關鍵流程

**Index（全量）**：掃描（忽略集沿用 `v6-final:cli/src/index.ts:20-22`：`node_modules`, `dist`, `build`, `.git`, `__pycache__` 等）→ **每檔一次** `ast-grep scan --json=stream --config <pack>/sgconfig.yml <file>`（同一 scan 抽 chunk 與關係，不是無 config 的 scan 再加第二次 scan）→ 該檔 scan 失敗（exit≠0、無 JSON、或無任何可 parse 的 JSON line，含全部行為 malformed）則寫 `chunk_source=unparsed`（FTS-only）；malformed line 跳過並按檔計數，**不 abort 整個 index**。通過的檔依抽出的 chunks + relationship edge strings 算 `file_content_hash` → 單一 transaction 批次寫 `chunks`/`chunks_fts`/`file_state` → 依 target 解析規則寫 `relationships`（單一 confidence 常數；0 target 不寫列）→ 同一 transaction 寫 `projects.git_head`/`extractor_version` + `_cortex_meta`。

**Incremental**：先比對 `extractor_version` — **mismatch → 強制全量重建**（graph 與 chunks 皆重建，因 extractor 改變可能影響兩者），並在 stderr 提示 `extractor_version mismatch: <old> -> <new>, full reindex required`；match 才走 `git diff --name-status -M` + `git ls-files --others` + per-file `file_content_hash` 比對 → per-file 序列化重建（`better-sqlite3` transaction per file；中斷保留已 commit 檔的增量進度；全部完成後最後一筆 transaction 原子更新 `projects.git_head` 與 `_cortex_meta.extractor_version`。`inFlight` map 僅用於未來的 `--watch`，不套用於 incremental）。

**Target 解析**（寫 `relationships.target_chunk_id`）：對每個抽出的符號參照，先查**檔內 import map**，再查**專案 symbol index**（`chunks.symbol_name`）。單一命中 → 寫一列，`confidence=INFERRED`（score 0.7；pack 已直接給出可解析 target 則 `EXTRACTED` / 1.0）；多命中 → 對每個已解析 target 寫一列，`confidence=AMBIGUOUS`、`confidence_score=0.5 × 1/N`（**僅當 N≥1**；N = 已解析 target 數）；零命中（unresolved）→ **不寫 relationship 列**（因 `target_chunk_id` 為 NOT NULL FK），**不建 `unresolved_refs` 表**。`confidence_reasoning="unresolved: <symbol>"` **不入庫**：`context`/`impact` 組 JSON 時**當場**（on-the-fly）對該次查詢的符號跑同一套 target 解析（檔內 import map → 專案 symbol index）；0 個 target 則內聯回傳 `confidence_score=0.5`、`confidence_reasoning="unresolved: <symbol>"`，**不帶 FK、不寫列**。

**Struct**：`ast-grep run --json=stream --strictness ast -p '<pat>' <paths>` → 每個 match 依 **containment**（`match.range.start >= chunk.start && match.range.end <= chunk.end`）join 最近的 `chunk_id`（若跨多 chunk 取最小包含者）→ 附 ≤3 `EXTRACTED` 鄰居 + confidence → 實際輸出算 budget（目標 ≤1500 tokens），回 `index_is_stale`/`truncated`。

**Context**：exact `symbol_name` 命中，否則 FTS 候選（**不調 `struct`**，解耦 stage 依賴）→ depth-1 鄰居（僅已寫入的 `relationships` 列）→ 對該次查詢符號 on-the-fly 做 target 解析，0 命中則內聯 `confidence_reasoning="unresolved: <symbol>"`（無 FK）→ 預設丟 `AMBIGUOUS`（`--include-ambiguous` 才保留）→ 信心分數單一常數；`impact` 反向 CTE 3 層（同樣 on-the-fly 處理 unresolved，不讀不存在的 relationship 列）。

## 7. 錯誤處理與邊界

- **$META/引號**：`ast-grep -p "$FOO"` 會被 shell 展開 — 全走 `argv` 陣列，不經 shell 插值。`ast-grep run` pattern parse 失敗由 **pre-flight** `ast-grep run --debug-query=ast --lang <LANG> -p '<pattern>' <paths>` 判定（stderr 含 `Pattern contains an ERROR node`）→ 回 `{error:"parse_failed", detail}`，**不執行後續 `run`、不產出部分 struct 結果、不回 0 筆命中**。
- **子行程**：`ast-grep` 不存在 / 版本不符 / 超時 (30s) → 結構化錯誤。`ast-grep scan`（index）單檔 exit≠0、無 JSON、或無任何可 parse 的 JSON line（含全部行為 malformed）→ 該檔 `chunk_source=unparsed`（FTS-only），不是全域錯誤；malformed line 跳過並**按檔計數**，**不 abort 整個 index**（10% abort **不適用 scan**）。`ast-grep run`（struct/rewrite）的 parse error 由 pre-flight `--debug-query=ast --lang <LANG>` 判定，**不看 exit code**。pre-flight 通過後：exit=0 → 有命中；exit=1 且 stdout/stderr 皆空 → 真 0 筆；exit≠0 且 stderr 非空 → operational error。malformed JSON line 的 10% abort 適用 `run` 路徑（struct 與 rewrite）的 `ast-grep run --json=stream`：跳過該行並計數，超過該次輸出行 10% 則 abort **該次查詢**。
- **FTS5**：`sanitizeFtsQuery` 對 `" ( ) - /` 做 `""` 轉義；tokenizer `unicode61` 支援 CJK；查詢詞 >20 個 OR → 截斷並回 `truncated_query:true`；malformed 仍拋錯 → 上層大聲失敗（無 embeddings 可退化，故不靜默）。
- **Confidence**：單一常數來源 `EXTRACTED:1.0 / INFERRED:0.7 / AMBIGUOUS:0.5 × 1/N`（JS 與 SQL 同 `CASE`）。`× 1/N` **僅當 N≥1** 套用；N = **已解析 target 數**。多解（N≥2）寫 N 列 `AMBIGUOUS`；**零命中不寫 relationship 列**（避免 NOT NULL FK），也**不建 `unresolved_refs` 表**。unresolved 由 `context`/`impact` **on-the-fly** 計算：當場跑同一套 target 解析，0 個 target 則在回傳 JSON 內聯 `confidence_score=0.5`、`confidence_reasoning="unresolved: <symbol>"`（無 FK、不寫列）。不拿 `AMBIGUOUS`/unresolved 墊 token 預算。
- **Staleness（防 false-positive）**：回答前比對**檔級** `file_content_hash`（`sha256(concat(依 start_line 排序的 chunk contents + 該檔 scan 抽出的 relationship edge strings 詞典序排序))`，存於 `file_state`），非僅 Git dirty、也不是拿 chunk 的 `content_hash` 去比 raw disk bytes。`git diff --name-status -M` + `ls-files --others` 僅決定候選檔。刪除規則：`deleted_files = db_files − disk_files`（`db_files` = `file_state.file_path`，`disk_files` = `projects.path` 下現存檔）；`deleted_files` 非空 → stale。最終 `index_is_stale = deleted_files.nonempty OR file_content_hash != sha256(disk 檔抽出的 chunks + relationship edge strings)`。剛索引完但仍 dirty 的檔，只要抽出 chunks **與** relationship edge strings 的 concat hash 不變，不會誤判 stale。比對基準為 `projects.path`（非 cwd）。**freshness 訊號分工**：`file_content_hash` 覆蓋單檔 chunk 與關係抽出物；pack 規則變更由 `extractor_version` 覆蓋（mismatch → 全量重建）。
- **DB/中斷**：`better-sqlite3` `busy_timeout` 5s，`SQLITE_BUSY` 重試 3 次；`SQLITE_FULL`/`SQLITE_CORRUPT` → 回 `{error:"storage_full"}` 並保留舊庫。全量 `cort index` 中斷 → rollback **整筆** transaction，舊庫不變。`--incremental` 中斷 → rollback 當前檔，**已 commit 的檔保留為增量進度**。測試用 in-memory `better-sqlite3`（不用 `node:sqlite`）。
- **非 Git 根**：無 `.git` 時 `incremental` 退化為全量；`status` 回 `git_head:null`。
- **大庫成本**：`context`/`impact` 限 depth 3、`modules` 取樣上限 5k edges、`struct` 需 `-g` 限縮，未限縮的大庫掃描拒絕並提示加 glob。

## 8. 分階段交付（照順序出貨）

1. `index` / `index --incremental`（含 `extractor_version` 強制全量；全量單一 transaction、incremental per-file transaction）
2. `struct`（`ast-grep` run + containment join，≤3 `EXTRACTED` edges）
3. `context`（FTS-only depth-1 + 實際輸出 budget）
4. `impact --symbol`（反向 CTE depth 3）
5. 延後：`rewrite`（dry-run → --interactive → --update-all）、`modules`（僅 Phase 1 greedy）、`--watch`、`impact --from-diff`、`search` 作為主動詞、`CORTEX_REPORT.md`

每階段皆可獨立驗收；`struct` + `context` 未在 tokens 與成功率同時贏過 `ast-grep` + 手動 `Read` 就停止疊加功能。

## 9. 測試與量測

- **Smoke（離線，必過）**：`tests/install-smoke.sh` 擴充 — `app-<target>.zip` 四平台對應（`x86_64-unknown-linux-gnu`/`aarch64-unknown-linux-gnu`/`x86_64-apple-darwin`/`aarch64-apple-darwin`，`app-` 前綴 ZIP）、SHA **硬失敗**（mismatch → exit 1，不 warning）、`ast-grep` 誤判（`sg` binary 存在但非 `ast-grep` 則拒絕）、ZIP 內容校驗（解壓後含 `ast-grep` binary）、cargo 版本不匹配（`ast-grep 0.45.x` 需 Rust 1.88，不符則提示）、多 skill 碰撞回滾（`ast-grep` 與 `xgrep` 分鍵）、**manifest v2 遷移**（見 §10）、冪等與 uninstall 擁有權、DB 交易中斷復原（全量 kill 後舊庫仍可讀；incremental kill 後已 commit 檔保留）。單元測試用 in-memory `better-sqlite3`（不用 `node:sqlite`）。
- **Agent Eval**：沿用 `v6-final:docs/agent-eval-plan.md` 協定但**補齊**：同任務同模型完整 trace，arms：`(rg+Read)` vs `(ast-grep+Read)` vs `(cort struct+context)`，指標：成功率、總 tokens、**tool-return tokens**、turns、`Read` 次數、**stale-read 事故**；micro：index 秒數、`context` p95、實際輸出 tokens（≤1500）、`impact` vs 人標 precision。原 eval 缺 `tool-return tokens` 與 stale 定義（`5a02c6e3:docs/agent-eval-plan.md:3-27`），本設計補上。

## 10. 安裝與遷移

- **版本釘死**：`cort` 與 `ast-grep` 皆釘版（`ast-grep 0.45.2`、`cort 0.1.0`），`app-<target>.zip` 與 `xg` 的 `xg-<target>.tar.gz` 分流，SHA-256 入 `install.sh`（repo 維護，上游無 checksum，**fail closed**：mismatch → exit 1）。
- **Manifest v2**：
  ```
  manifest_version=2
  cort_bin=/home/.../.local/bin/cort           # cort binary 擁有權
  ast_grep_bin=/home/.../.cargo/bin/ast-grep   # 新
  skill_ast_grep=/home/.../.claude/skills/ast-grep/SKILL.md
  skill_xgrep=/home/.../.claude/skills/xgrep/SKILL.md
  legacy_xg_bin=/home/.../.cargo/bin/xg        # 舊 xg_bin 遷移至此
  profile=/home/.../.zshrc                     # PATH block 位置
  ```
  遷移：`xg_bin → legacy_xg_bin`、`skill → skill_xgrep`（若存在），新增 `manifest_version=2` 與 `cort_bin`（cort binary 路徑）；pre-existing binary（無 manifest 記錄）不認領；舊有 `xg` 不主動刪除，僅 `--uninstall` 時按 v2 鍵刪除 owned 項目（含 `cort_bin`）。預設新安裝只裝 `cort` + `ast-grep`，`xg` 用 `--with-xgrep` 才裝。
- **交易式安裝**：preflight 全部碰撞（兩個 skill 分鍵檢查）→ 下載到 tmp → 驗 SHA（**硬失敗**）與 `ast-grep --version` / `cort --version` → 原子安裝 binary/skill（`install -m 755` + `mv`）→ 最後寫 manifest。多 skill 碰撞時整批 rollback，不留半套。
- **Atomic commit**：installer + 兩個 skill + README/CLAUDE/THIRD_PARTY + 煙測 + CI 同一 commit；`v6-final` 釘在 `5a02c6e3` 不動，NG 另起新 tag（`cort-v0.1.0`）待 CI 綠燈再打。DB 索引原子性見 §4/§6：全量單一 transaction；incremental per-file + 完成後原子更新 `git_head`/`extractor_version`。

## 11. 未來擴充插槽（不做，僅保留）

- `ALTER TABLE chunks ADD COLUMN embedding BLOB` + `cort embed --backfill`（`sqlite-vec` 或 `libSQL F32_BLOB`），三臂 RRF（`fts`/`struct`/`dense`）融合時 `chunk_id` 保持穩定。
- `CORTEX_REPORT.md`、`--watch`（`inFlight` 序列化，沿用 `5a02c6e3` 的 watch 修復但不套用於 incremental）、`impact --from-diff` 待 `struct`+`context` 驗證有效後再補。

## 12. 與現有設計的差異

- V6 的 `vector_top_k` / DiskANN / Turso 雲端 → 刪。
- V6 的 `web-tree-sitter` in-process 解析 → 刪，改唯一 `ast-grep` 子行程。
- V6 的 `grammar_version` 僅警告 → 改 `extractor_version` **強制全量重建**。
- `xg` 從主角降為 `--with-xgrep` 可選加速器，`rg` 保留給新鮮/短模式。
- `sg` alias 完全移除，統一為 `ast-grep`。

## 13. 修訂對照（v1 → v2）

| Codex 指摘 | 修法 |
|------------|------|
| Parser 雙權威 | §2 明確唯一 `ast-grep` 子行程，刪 `web-tree-sitter`，定義三階段精確合約 |
| Graph extractor 不可實作 | §2/§3/§6 補 `scan --config <pack>`、YAML pack hash、containment join、單一 confidence 常數 |
| DDL 草圖 | §4 改為可執行 DDL（`better-sqlite3` 唯一、`project_id`/`rel_type`/複合主鍵、`chunk_id` 單一公式、unicode61 FTS、WAL/transaction） |
| MVP 依賴倒置 + 成功標準含延後項 | §1/§8 拆 `struct`(2)→`context` FTS-only(3)→`impact`(4)，成功標準僅限 MVP |
| `sg` 殘留 | 全文刪 `sg`，統一 `ast-grep` |
| Staleness 誤判 | §7 改 `content_hash` 比對，非僅 Git dirty |
| `extractor_version` 無動作 | §6 定義 mismatch → 全量重建 |
| Manifest 錯映射 | §10 明確 `xg_bin→legacy_xg_bin`/`skill→skill_xgrep` + `manifest_version=2` |
| SHA warning 通過 | §10 定 `fail closed`，`§2` 同步 |
| `chunk_id` 穩定性謊言 | §4 承認 V6 限制，維持 `project:file:line` 作為穩定鍵的取捨說明 |
| `AMBIGUOUS` 退化 | §7 明確多解標 `AMBIGUOUS`，無解不寫列（on-the-fly unresolved），不墊預算 |
| FTS/CJK、failure cases、Louvain、eval 不足 | §3/§6/§7 補 tokenizer/截斷/operational 錯誤、Phase 1 only 註記、§9 補 `tool-return tokens`/`stale-read` |

## 14. 修訂對照（v2 → v3，Codex 二次審查 PARTIALLY FIXED）

| Codex 指摘 | 修法 |
|------------|------|
| Parser exit≠0 雙語義衝突；index 寫成兩次 scan | §2/§6/§7 拆 scan 單檔 `unparsed` vs run `{error:"parse_failed"}`；index 每檔單一 `scan --config <pack>/sgconfig.yml` |
| 交易 vs「無半套索引」矛盾；`node:sqlite` 備援 | §2/§4/§6/§7：全量單一 tx；incremental per-file 中斷保留增量進度；完成後原子寫 `git_head`/`extractor_version`；測試用 in-memory `better-sqlite3` |
| file hash 層級錯、刪檔不 stale、AMBIGUOUS 0 解無 FK、`× 1/N` 未定義 | §4 加 `file_state.file_content_hash`；§7 stale 公式與 `deleted_files = db_files − disk_files`；§6 target 解析（import map → symbol index）；unresolved 不寫列 |
| Manifest 缺 `cort_bin` | §10 加入 `cort_bin`，uninstall 按 v2 鍵刪 owned 項目（含 cort） |

## 15. 修訂對照（v3 → v4，Codex 三次審查 STILL BROKEN）

| Codex 指摘 | 修法 |
|------------|------|
| Parser：scan「無 JSON → unparsed」vs 未限定的「malformed JSON line >10% abort」同一條件衝突 | §2 合約表 / §6 Index / §7 子行程：10% abort 適用 `run` 路徑（struct 與 rewrite）的 `ast-grep run --json=stream`；index `scan` 的 malformed line 按檔計為該檔 `unparsed` fallback（FTS-only），**不 abort 整個 index** |
| `file_content_hash` 只涵蓋 chunk contents，scan 同時抽關係，關係抽出變更未覆蓋 | §4 DDL 註解 + §4 公式 + §7 stale：`file_content_hash = sha256(concat(sorted chunk contents + sorted relationship edge strings))`；`extractor_version` 仍管 pack YAML |
| 零命中承諾 `confidence_reasoning="unresolved: <symbol>"` 但 DDL 無 `unresolved_refs` 表 | §4/§6/§7：不建表；0 target 不寫列；reasoning 由 `context`/`impact` on-the-fly 內聯 JSON、不帶 FK |
| §13「多解/無解標 AMBIGUOUS」vs §7「零命中不寫列」 | §13 改為「多解標 `AMBIGUOUS`，無解不寫列（on-the-fly unresolved）」 |

## 16. 修訂對照（v4.1 → v4.2，實機驗證修正）

v4.1 通過 Codex 內部一致性審查後，對本機 `ast-grep 0.45.2` 做子行程合約實測，發現兩項與實機不符，於本版修正。實測命令與結果：

| 實測 | 命令 | 結果 |
|------|------|------|
| A | `ast-grep run --json=stream -p 'alpha($A)' probe.ts`（有命中） | exit=0，stdout 一行 JSON |
| B | `ast-grep run --json=stream -p 'zzzNoSuchFn($A)' probe.ts`（合法 pattern、0 命中） | **exit=1，stdout 0 bytes，stderr 0 bytes** |
| C | `ast-grep run --json=stream -p 'function (' probe.ts`（**壞 pattern**） | **exit=1，stdout 0 bytes，stderr 0 bytes** — 與 B **逐位元組相同** |
| D | `ast-grep run --debug-query=ast --lang ts -p 'function (' probe.ts` | exit=0，stderr 含 `Debug AST:` 內有 `ERROR (0,0)-(0,10)` 與 `Warning: Pattern contains an ERROR node and may cause unexpected results.` |
| E | `ast-grep run --debug-query=ast -p 'alpha($A)' probe.ts`（缺 `--lang`） | exit=2，`error: the following required arguments were not provided: --lang <LANG>` |
| F | `ast-grep scan --json=stream --config pack/sgconfig.yml plain.ts`（0 findings） | exit=0 |
| G | `ast-grep scan --json=stream --config pack/sgconfig.yml broken.ts`（語法壞檔） | exit=0，**無任何 JSON line** → 落 `chunk_source=unparsed` 分支，與 v4.1 §2① 相符 |
| H | `ast-grep run --json=stream -p 'alpha($A)' nope.ts`（路徑不存在） | exit=1，stderr `ERROR: nope.ts: No such file or directory (os error 2)` |
| I | scan/run JSON 的 `range.start.line` | **0-indexed**（`probe.ts` 第 1 行的 import 回報 `line:0`） |

| v4.1 的敘述 | 實機反證 | v4.2 修法 |
|-------------|----------|-----------|
| `run` 的 parse 失敗＝「exit≠0 且 stderr 含 `parse error` / `--debug-query`」 | 實測 C：壞 pattern 的 stderr 是 **0 bytes**，且 exit code 與實測 B 的合法 0 命中**完全相同**。此規則不可實作 | §2 bullet／§2 合約表／§7：改為**強制 pre-flight** `ast-grep run --debug-query=ast --lang <LANG> -p '<pattern>' <paths>`，stderr 含 `Pattern contains an ERROR node` → `{error:"parse_failed", detail}` 且不執行後續 `run` |
| 「無匹配且 exit=0 才是 0 筆」 | 實測 A/B：有命中才 exit=0，**0 命中是 exit=1** | §7：pre-flight 通過後 exit=0→有命中；exit=1 且雙流全空→真 0 筆；exit≠0 且 stderr 非空→operational error（實測 H） |
| `cort struct` / `cort rewrite` 無 `--lang` | 實測 E：`--debug-query` 缺 `--lang` 直接 exit=2，pre-flight 無法執行 | §5：`--lang` 改為 `struct` / `rewrite` 的**必填**參數 |
| 釘 `ast-grep 0.45.0` | 上述全部合約僅在 **0.45.2** 上驗證過，0.45.0 未經驗證 | §2／§5／§10 釘版改為 `0.45.2`（釘在已驗證的版本，而非未驗證的版本） |
| 未說明行號基準 | 實測 I：JSON 行號 0-indexed | §4 新增「行號基準」條：讀入一律 `+1` 正規化為 1-indexed 後才組 `chunk_id`、排序與 containment join |

未受影響（v4.1 原判定經實測確認正確）：index `scan` 的 exit≠0／無 JSON → 該檔 `chunk_source=unparsed`（實測 G），以及 `scan` 0 findings 為 exit=0（實測 F）。
