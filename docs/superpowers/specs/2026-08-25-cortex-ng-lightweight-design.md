# Cortex NG 輕量版設計 — AST + Graph + FTS（`cort` CLI，離線 SQLite）

- **日期**: 2026-08-25
- **狀態**: Draft v2 → 修 Codex 5 項 blocker 後重送審核
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
                              ├─→ ast-grep (子行程，唯一 parser) — index: scan --json=stream
                              │                                    query: run --json=stream
                              │                                    rewrite: run --rewrite --json=stream (dry-run)
                              ├─→ FTS5 (SQLite, unicode61) — 關鍵字召回
                              └─→ Louvain (process 內，檔案級鄰接，僅 Phase 1 greedy)
```

- **唯一 Parser 權威**：索引與查詢皆由 `ast-grep` 子行程提供結構。支援語言 = `ast-grep` 支援的語言（與 V6 的 TS/JS/TSX/Python 為交集，未來隨 ast-grep 擴充）；版本檢查 `ast-grep --version == 0.45.0`，不符則 fail-closed；exit code 非 0 或 stderr 含 `parse error` → 回結構化錯誤，不回 0 筆；`--debug-query` 用於診斷。**不使用 `web-tree-sitter` in-process 解析**，避免雙重真理。
- **Graph 自己擁有、Matcher 去租**：`cort` 擁有 `chunks`/`relationships` 的寫入與交易；`ast-grep` 只提供 matches。index 期用版本化 YAML extractor pack 建圖；query 期 `ast-grep run --json=stream` 再 containment join；rewrite 期先 dry-run。
- **存放**：`better-sqlite3` 唯一 driver（`node:sqlite` 備援僅測試用），WAL + `busy_timeout=5000`，`~/.cache/cortex-ng/` 單庫 per project。不用 JSON sidecar（無 CTE/watch）、不用 Turso/libSQL（向量以外無理由、FTS 實驗性、RTT 成本）。

**ast-grep 子行程合約（精確）**：

| 階段 | 命令 | 輸入 | 輸出 | 失敗 |
|------|------|------|------|------|
| index | `ast-grep scan --json=stream --config <pack>/sgconfig.yml <file>` | 單檔 | JSON lines: `{file, range:{start,end}, kind, text, meta}` | exit≠0 或無 JSON → 標 `chunk_source=unparsed`，僅 FTS |
| struct | `ast-grep run --json=stream -p '<pattern>' --strictness ast <paths>` | pattern (argv, 單引號), globs | 同上 + `pattern` 匹配的 node 範圍 | `--debug-query` 回結構化錯誤 |
| rewrite | `ast-grep run --json=stream -p '<pat>' --rewrite '<repl>' <paths>` | 同上 + rewrite 模板 | diff JSON (range + replacement) | 同上，`--interactive` 由 `cort` 封裝 |

不接受 `sg` binary；`command -v ast-grep` 不存在 → 安裝失敗，不 fallback。

## 3. 元件分解

| 元件 | 職責 | 介面 | 依賴 |
|------|------|------|------|
| `indexer` | 掃檔 → `ast-grep scan` 抽 chunk → 寫 `chunks`/`chunks_fts` → 調 extractor pack 建 `relationships` | `cort index [path]` / `--incremental` | ast-grep (scan), better-sqlite3 |
| `incremental` | `extractor_version` 比對 → `git diff --name-status -M` + `ls-files --others` → `content_hash` 去重 → per-file 序列化重建 | `cort index --incremental` | git, better-sqlite3 |
| `fts` | FTS5 儲存與查詢（unicode61, `remove_diacritics 1`, `tokenchars "._$"`），`sanitizeFtsQuery` 對 `" ( ) - /` 做 `""` 轉義，超長 OR 截斷 20 詞，malformed 大聲失敗 | 內部 `keywordSearch` | SQLite FTS5 |
| `struct` | `ast-grep run --json=stream` → **containment join** (`match.range` ⊆ `chunk.range`) → 回傳 file/symbol + ≤3 `EXTRACTED` edges + confidence | `cort struct -p '<pat>'` | ast-grep |
| `graph` | `relationships` CRUD、confidence 單一常數（`EXTRACTED:1.0 / INFERRED:0.7 / AMBIGUOUS:0.5 × 1/N`）、鄰居查詢 | 內部 `getNeighbors`/`getTransitiveDependents` | better-sqlite3 |
| `context` | exact `symbol_name` 命中或 FTS 候選（**不依賴 `struct`**）→ depth-1 鄰居 → 實際輸出算 budget 修剪 → `index_is_stale`/`truncated` | `cort context <q>` | graph, FTS |
| `impact` | 反向 CTE depth 3 | `cort impact --symbol` | better-sqlite3 CTE |
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
  content_hash TEXT NOT NULL,           -- sha256(content)
  language TEXT,
  chunk_source TEXT NOT NULL CHECK(chunk_source IN ('ast','unparsed')),
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_chunks_project ON chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(project_id, file_path);
CREATE INDEX IF NOT EXISTS idx_chunks_symbol ON chunks(project_id, symbol_name);

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
- `extractor_version = sha256(concat(sorted YAML pack files))`，`ensureSchema` 冪等遷移，`_cortex_meta` 存 `SCHEMA_VERSION`。
- **交易邊界**：全量 `index` 包在單一 SQLite transaction；`--incremental` 每檔一個 transaction（`BEGIN; deleteStaleFileChunks; insert chunks; replace relationships; COMMIT`）；中斷則 rollback，不暴露半套索引。未來可加 generation swap（寫至 `*.db.tmp` 再 `rename`）。

## 5. CLI 規格

```
cort index [path] [--incremental]
cort struct -p '<pattern>' [-g '<glob>'] [--json]          # 階段 2
cort context <symbol|query> [--budget 1500] [--json]       # 階段 3，FTS-only
cort impact --symbol <name> [--depth 3] [--json]           # 階段 4
cort search <query> [--json] [--limit N]                   # 薄包裝 FTS，延後
cort impact --from-diff [sha] [--json]                     # 延後
cort rewrite -p '<pat>' --rewrite '<repl>' [--interactive] [--json]  # 延後
cort modules [--min-size N] [--json]                       # 延後（僅 Phase 1 greedy）
cort status / cort projects / cort delete
```

- `cort` 為唯一 canonical binary；內部固定呼叫 `ast-grep`，不存在或版本不符 (`!=0.45.0`) 則 fail-closed，不接受 `sg`。
- Skill 路由（15–25 行）：`rg` 管新鮮/短模式、`xg`（若安裝）管重複大庫字串、`struct` 管 shape、`context`/`impact` 管 who-else/what-breaks；不做 PreToolUse nudge。

## 6. 關鍵流程

**Index（全量）**：掃描（忽略集沿用 `v6-final:cli/src/index.ts:20-22`：`node_modules`, `dist`, `build`, `.git`, `__pycache__` 等）→ per-file `ast-grep scan --json=stream` 抽 chunk → `content_hash` 去重 → 單一 transaction 批次寫 `chunks`/`chunks_fts` → 跑 extractor pack（`ast-grep scan --config <pack>`）→ 按 `rel_type` 寫 `relationships`（單一 confidence 常數）→ 寫 `projects.git_head`/`extractor_version` + `_cortex_meta`。

**Incremental**：先比對 `extractor_version` — **mismatch → 強制全量重建**（graph 與 chunks 皆重建，因 extractor 改變可能影響兩者），並在 stderr 提示 `extractor_version mismatch: <old> -> <new>, full reindex required`；match 才走 `git diff --name-status -M` + `git ls-files --others` + per-file `content_hash` 比對 → per-file 序列化重建（`better-sqlite3` transaction per file，`inFlight` map 僅用於未來的 `--watch`，不套用於 incremental）。

**Struct**：`ast-grep run --json=stream --strictness ast -p '<pat>' <paths>` → 每個 match 依 **containment**（`match.range.start >= chunk.start && match.range.end <= chunk.end`）join 最近的 `chunk_id`（若跨多 chunk 取最小包含者）→ 附 ≤3 `EXTRACTED` 鄰居 + confidence → 實際輸出算 budget（目標 ≤1500 tokens），回 `index_is_stale`/`truncated`。

**Context**：exact `symbol_name` 命中，否則 FTS 候選（**不調 `struct`**，解耦 stage 依賴）→ depth-1 鄰居 → 預設丟 `AMBIGUOUS`（`--include-ambiguous` 才保留）→ 信心分數單一常數；`impact` 反向 CTE 3 層。

## 7. 錯誤處理與邊界

- **$META/引號**：`ast-grep -p "$FOO"` 會被 shell 展開 — 全走 `argv` 陣列，不經 shell 插值；parse 失敗回 `{error:"parse_failed", detail: <debug-query output>}`，不回 0 筆。
- **子行程**：`ast-grep` 不存在 / 版本不符 / 超時 (30s) / 非 0 exit（除無匹配的 0 筆外）→ 結構化錯誤；malformed JSON line → 跳過並計數，超過 10% 失敗則 abort。
- **FTS5**：`sanitizeFtsQuery` 對 `" ( ) - /` 做 `""` 轉義；tokenizer `unicode61` 支援 CJK；查詢詞 >20 個 OR → 截斷並回 `truncated_query:true`；malformed 仍拋錯 → 上層大聲失敗（無 embeddings 可退化，故不靜默）。
- **Confidence**：單一常數來源 `EXTRACTED:1.0 / INFERRED:0.7 / AMBIGUOUS:0.5 × 1/N`（JS 與 SQL 同 `CASE`），`calls` 的多解/無解標 `AMBIGUOUS`（沿用 `5a02c6e3:cli/src/index.ts:1805-1809`），不拿 `AMBIGUOUS` 墊 token 預算。
- **Staleness（防 false-positive）**：回答前比對 `content_hash`，非僅 Git dirty。`git diff --name-status -M` + `ls-files --others` 僅決定候選檔，最終 `index_is_stale = exists(file) && sha256(disk_content) != stored content_hash`；剛索引完但仍 dirty 的檔不會誤判 stale。比對基準為 `projects.path`（非 cwd）。
- **DB/中斷**：`better-sqlite3` `busy_timeout` 5s，`SQLITE_BUSY` 重試 3 次；`SQLITE_FULL`/`SQLITE_CORRUPT` → 回 `{error:"storage_full"}` 並保留舊庫；index 中斷則 rollback 當前 transaction，不污染舊索引。
- **非 Git 根**：無 `.git` 時 `incremental` 退化為全量；`status` 回 `git_head:null`。
- **大庫成本**：`context`/`impact` 限 depth 3、`modules` 取樣上限 5k edges、`struct` 需 `-g` 限縮，未限縮的大庫掃描拒絕並提示加 glob。

## 8. 分階段交付（照順序出貨）

1. `index` / `index --incremental`（含 `extractor_version` 強制全量 + per-file transaction）
2. `struct`（`ast-grep` run + containment join，≤3 `EXTRACTED` edges）
3. `context`（FTS-only depth-1 + 實際輸出 budget）
4. `impact --symbol`（反向 CTE depth 3）
5. 延後：`rewrite`（dry-run → --interactive → --update-all）、`modules`（僅 Phase 1 greedy）、`--watch`、`impact --from-diff`、`search` 作為主動詞、`CORTEX_REPORT.md`

每階段皆可獨立驗收；`struct` + `context` 未在 tokens 與成功率同時贏過 `ast-grep` + 手動 `Read` 就停止疊加功能。

## 9. 測試與量測

- **Smoke（離線，必過）**：`tests/install-smoke.sh` 擴充 — `app-<target>.zip` 四平台對應（`x86_64-unknown-linux-gnu`/`aarch64-unknown-linux-gnu`/`x86_64-apple-darwin`/`aarch64-apple-darwin`，`app-` 前綴 ZIP）、SHA **硬失敗**（mismatch → exit 1，不 warning）、`ast-grep` 誤判（`sg` binary 存在但非 `ast-grep` 則拒絕）、ZIP 內容校驗（解壓後含 `ast-grep` binary）、cargo 版本不匹配（`ast-grep 0.45.x` 需 Rust 1.88，不符則提示）、多 skill 碰撞回滾（`ast-grep` 與 `xgrep` 分鍵）、**manifest v2 遷移**（見 §10）、冪等與 uninstall 擁有權、DB 交易中斷復原（kill 後舊庫仍可讀）。
- **Agent Eval**：沿用 `v6-final:docs/agent-eval-plan.md` 協定但**補齊**：同任務同模型完整 trace，arms：`(rg+Read)` vs `(ast-grep+Read)` vs `(cort struct+context)`，指標：成功率、總 tokens、**tool-return tokens**、turns、`Read` 次數、**stale-read 事故**；micro：index 秒數、`context` p95、實際輸出 tokens（≤1500）、`impact` vs 人標 precision。原 eval 缺 `tool-return tokens` 與 stale 定義（`5a02c6e3:docs/agent-eval-plan.md:3-27`），本設計補上。

## 10. 安裝與遷移

- **版本釘死**：`cort` 與 `ast-grep` 皆釘版（`ast-grep 0.45.0`、`cort 0.1.0`），`app-<target>.zip` 與 `xg` 的 `xg-<target>.tar.gz` 分流，SHA-256 入 `install.sh`（repo 維護，上游無 checksum，**fail closed**：mismatch → exit 1）。
- **Manifest v2**：
  ```
  manifest_version=2
  ast_grep_bin=/home/.../.cargo/bin/ast-grep   # 新
  skill_ast_grep=/home/.../.claude/skills/ast-grep/SKILL.md
  skill_xgrep=/home/.../.claude/skills/xgrep/SKILL.md
  legacy_xg_bin=/home/.../.cargo/bin/xg        # 舊 xg_bin 遷移至此
  profile=/home/.../.zshrc                     # PATH block 位置
  ```
  遷移：`xg_bin → legacy_xg_bin`、`skill → skill_xgrep`（若存在），新增 `manifest_version=2`；pre-existing binary（無 manifest 記錄）不認領；舊有 `xg` 不主動刪除，僅 `--uninstall` 時按 v2 鍵刪除 owned 項目。預設新安裝只裝 `cort` + `ast-grep`，`xg` 用 `--with-xgrep` 才裝。
- **交易式安裝**：preflight 全部碰撞（兩個 skill 分鍵檢查）→ 下載到 tmp → 驗 SHA（**硬失敗**）與 `ast-grep --version` / `cort --version` → 原子安裝 binary/skill（`install -m 755` + `mv`）→ 最後寫 manifest。多 skill 碰撞時整批 rollback，不留半套。
- **Atomic commit**：installer + 兩個 skill + README/CLAUDE/THIRD_PARTY + 煙測 + CI 同一 commit；`v6-final` 釘在 `5a02c6e3` 不動，NG 另起新 tag（`cort-v0.1.0`）待 CI 綠燈再打。DB 索引原子性由 §4/§6 的 per-file transaction 保障。

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
| `AMBIGUOUS` 退化 | §7 明確多解/無解標 `AMBIGUOUS`，不墊預算 |
| FTS/CJK、failure cases、Louvain、eval 不足 | §3/§6/§7 補 tokenizer/截斷/operational 錯誤、Phase 1 only 註記、§9 補 `tool-return tokens`/`stale-read` |
