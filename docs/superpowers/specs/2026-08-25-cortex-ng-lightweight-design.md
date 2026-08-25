# Cortex NG 輕量版設計 — AST + Graph + FTS（`cort` CLI，離線 SQLite）

- **日期**: 2026-08-25
- **狀態**: Draft → 送 Codex 審核
- **前代**: `v6-final` (5a02c6e3) 歸檔；`de8638fd` 為 xgrep 精簡版
- **決策**: 輕量派 — 保留 AST + Graph + FTS，拔掉 embeddings/Turso/DiskANN；`ast-grep` 作為結構引擎，`cort` 為 canonical CLI 名稱（內部固定調 `ast-grep`，不依賴 `sg` alias）

## 1. 目標與非目標

**目標**：提供 agent 原生的本地離線 code intelligence，讓 `rg`/`xg` 處理重複字串搜尋，`ast-grep` 處理檔內結構，`cort` 補上 `sg` 給不了的跨檔鄰居與爆炸半徑，並以 token-efficient 封包回傳。

- `cort index` / `cort index --incremental` — AST 抽 chunk + 建圖 + FTS，離線、可增量、可 watch（延後）
- `cort context <symbol|query>` — depth-1 鄰居，token budget 修剪，實際輸出算預算
- `cort impact --symbol <name>` — 反向 CTE 遞移 dependents（depth 3），作為 `rewrite` 的掃描範圍
- `cort struct -p '<pattern>'` — `ast-grep run --json=stream` → join `chunk_id` → 附 ≤3 條 `EXTRACTED` 鄰居
- `cort rewrite -p '<pattern>' --rewrite '<repl>'` — dry-run JSON 先行，`--update-all` 前需確認（MVP 延後，見 §8）
- `cort modules` — Louvain 檔案級分群（延後）

**非目標**：向量語意搜尋（BGE-small-en-v1.5 / ANN）、雲端同步、全量重建以外的即時 rerank。 embeddings 保留擴充插槽但 v1 不建欄位（見 §4）。

**成功標準**：萬檔級 repo `cort search` <1s、`impact --from-diff` 正確標示 blast radius、`rewrite` 結構化改寫安全，agent 總 tokens 與 tool-return tokens 同時優於 `sg` + 手動 `Read`。

## 2. 架構總覽

```
agent → cort CLI (Node.js, ESM) → SQLite (~/.cache/cortex-ng/<sha256(realpath)>.db, WAL, 0600)
                              ├─→ ast-grep (子行程，`ast-grep` canonical) — index 期 extractor pack / query 期 --json=stream / rewrite 期 dry-run
                              ├─→ tree-sitter (AST 抽 chunk，單一真理，無 regex fallback)
                              ├─→ FTS5 (SQLite) — 關鍵字召回
                              └─→ Louvain (process 內，檔案級鄰接)
```

- **單一 Parser 規則**：`ast-grep` 解析失敗的檔案標 `chunk_source=unparsed`，僅 FTS 索引，不入 graph。刪掉 V6 的 regex 作為第二真理，避免雙重來源漂移。
- **Graph 自己擁有、Matcher 去租**：index 期用版本化 YAML extractor pack（`sg` 同語法）建 `relationships`；query 期 `ast-grep run --json=stream` → join；rewrite 期先 dry-run。
- **存放**：`better-sqlite3` 或 `node:sqlite`，WAL + `busy_timeout`，`~/.cache/cortex-ng/` 單庫 per project。不用 JSON sidecar（無 CTE/watch）、不用 Turso/libSQL（向量以外無理由、FTS 實驗性、RTT 成本）。

## 3. 元件分解

| 元件 | 職責 | 介面 | 依賴 |
|------|------|------|------|
| `indexer` | 掃檔 → AST 抽 chunk → 寫 `chunks`/`chunks_fts` → 調 extractor pack 建 `relationships` | `cort index [path]` / `--incremental` | tree-sitter, ast-grep (index pack), SQLite |
| `incremental` | `git diff --name-only` + untracked + `extractor_version` 比對 → per-file `deleteStaleFileChunks`/`replaceFileRelationships` → `content_hash` 去重 | `cort index --incremental` | git, SQLite |
| `fts` | FTS5 儲存與查詢，`""` 轉義、unicode tokenizer、大聲失敗（無 embeddings 時不靜默退化） | 內部 `keywordSearch` | SQLite FTS5 |
| `struct` | `ast-grep run --json=stream` → join `chunk_id` → 回傳 file/symbol + ≤3 `EXTRACTED` edges + confidence | `cort struct -p '<pat>'` | ast-grep |
| `graph` | `relationships` CRUD、confidence 單一常數來源、鄰居查詢 | 內部 `getNeighbors`/`getTransitiveDependents` | SQLite |
| `context` | exact symbol 命中或 `struct`+FTS 候選 → depth-1 鄰居 → 實際輸出算 budget 修剪 → `index_is_stale`/`truncated` | `cort context <q>` | graph, FTS, struct |
| `impact` | 反向 CTE depth 3、`--from-diff` 以 index 期路徑為 seed（`D` 用 `path`、`R` 用 `oldPath`） | `cort impact --symbol` / `--from-diff` | SQLite CTE |
| `modules` | 取 cross-file `relationships` 建無向加權圖 → Louvain Phase 1 | `cort modules --min-size` | clusterer (移植 V6) |
| `rewrite` | `sg --rewrite` 包裝：dry-run JSON → 限縮至 `impact` 檔案清單 → `--interactive` 預設 | `cort rewrite -p … --rewrite …` | ast-grep |

## 4. 資料模型（SQLite）

沿用 `v6-final:turso.ts` 精簡，**不建 `embedding` 欄位**（日後 `ALTER TABLE chunks ADD COLUMN embedding BLOB` + `cort embed --backfill`）：

```sql
-- _cortex_meta (SCHEMA_VERSION)
-- projects: id, path, git_head, last_indexed_at, extractor_version, created_at
-- chunks: id (project:path:startLine 或 path:content_hash:span), project_id, file_path, start_line, end_line,
--         symbol_name, chunk_type, language, content, content_hash, chunk_source (ast/unparsed), created_at
-- relationships: id, source_chunk_id, target_chunk_id, relation_type (imports/exports/calls),
--                confidence (EXTRACTED/INFERRED/AMBIGUOUS), confidence_score REAL, confidence_reasoning TEXT
-- chunks_fts: FTS5(content, file_path, symbol_name) + triggers on chunks
```

- `chunk_id` 穩定（`project:path:startLine` 或 `path:content_hash:span`）供未來 RRF 融合（`fts`/`struct`/`dense` 三臂）使用。
- `_cortex_meta` 存 `SCHEMA_VERSION`，`ensureSchema` 冪等遷移。
- `extractor_version` 為 YAML pack 的 hash，incremental 必須比對（V6 存了 `grammar_version` 卻未強制執行）。

## 5. CLI 規格

```
cort index [path] [--incremental] [--watch*]   # * watch 為延後
cort search <query> [--json] [--limit N]       # FTS 為主，--structural 轉 struct
cort struct -p '<pattern>' [-g '<glob>'] [--json]
cort context <symbol|query> [--budget 1500] [--json]
cort impact --symbol <name> [--depth 3] [--json]
cort impact --from-diff [sha] [--json]         # 延後
cort rewrite -p '<pat>' --rewrite '<repl>' [--interactive] [--json]  # 延後
cort modules [--min-size N] [--json]           # 延後
cort status / cort projects / cort delete
```

- `cort` 為唯一 canonical binary 名稱；內部結構搜尋固定呼叫 `ast-grep`，僅在驗證 `ast-grep --version` 後才接受 `sg` alias（避開 Linux `sg` 撞名）。
- Skill 路由（15–25 行）：`xg`/`rg` 管重複字串與新鮮檔案、shape 走 `struct`、who-else/what-breaks 走 `context`/`impact`；不做 PreToolUse nudge（V6 實證無效）。

## 6. 關鍵流程

**Index**：掃描（忽略 `node_modules/dist`）→ per-file AST 抽 chunk → `content_hash` 去重 → 批次寫 `chunks`/`chunks_fts` → 跑 extractor pack 建 `relationships` → 寫 `projects.git_head`/`extractor_version`。Streaming per-file upsert，不全量緩衝。

**Incremental**：比對 `extractor_version` → `git diff --name-status -M` + `git ls-files --others` + dirty worktree → per-file 序列化重建（`inFlight` map，沿用 `5a02c6e3` 修復）→ `CORTEX_REPORT.md` 延後（v1 不寫，避免 stale）。

**Struct**：`ast-grep run --json=stream -p '<pat>' <paths>` → 每個 match join `chunks` → 附 ≤3 `EXTRACTED` 鄰居 + confidence → 實際輸出算 budget（目標 ≤1500 tokens），回 `index_is_stale`/`truncated`。

**Context/Impact**：`context` 先 exact symbol 命中，否則 `struct`+FTS 候選 → depth-1 鄰居 → 預設丟 `AMBIGUOUS`（`--include-ambiguous` 才保留）→ 信心分數單一常數來源；`impact` 反向 CTE，`--from-diff` 時 `D` 用 `change.path`、`R` 用 `change.oldPath`（V6 已驗）。

## 7. 錯誤處理與邊界

- **$META/引號**：`sg -p "$FOO"` 會被 shell 展開成空 — 全走 `argv`，parse 失敗用 `--debug-query` 回結構化錯誤，不回 0 筆。
- **FTS5**：`sanitizeFtsQuery` 對 `. ( ) - /` 做 `""` 轉義、選 unicode tokenizer、無 embeddings 時大聲失敗（不靜默退化成空）。
- **Confidence 漂移**：JS 與 SQL 的 CASE 權重單一常數來源，`calls` 皆 `INFERRED`，`impact` 不拿 `AMBIGUOUS` 墊預算。
- **Staleness**：回答前疊 `git diff --name-only` + untracked，比對 `projects.path` 存的路徑（非 cwd），cross-project 查對才對。
- **大庫成本**：Louvain/FTS/CTE 限 depth 與 `modules` 取樣，`context`/`impact` 以實際輸出算 budget。

## 8. 分階段交付（照順序出貨）

1. `index` / `index --incremental`（含 `extractor_version` 與 per-file 序列化）
2. `context`（depth-1 + budget）
3. `impact --symbol`（反向 CTE depth 3）
4. `struct`（sg join graph，≤3 `EXTRACTED` edges）
5. 延後：`rewrite`（dry-run → --interactive → --update-all）、`modules`、`--watch`、`impact --from-diff`、`search` 作為主動詞、`CORTEX_REPORT.md`

每階段皆可獨立驗收；`struct` + `context` 未在 tokens 與成功率同時贏過 `sg` + 手動 `Read` 就停止疊加功能。

## 9. 測試與量測

- **Smoke（離線）**：`tests/install-smoke.sh` 擴充 — 各平台 `app-<target>.zip` 對應、SHA 硬失敗、`sg` 誤判、ZIP 內容、cargo 版本不匹配（ast-grep 0.45.x 需 Rust 1.88）、多 skill 碰撞回滾、legacy manifest 遷移（`xg_bin` → `ast_grep_bin`/`skill_ast_grep`）、`--with-xgrep` 可選安裝、冪等與 uninstall 擁有權。
- **Agent Eval**：沿用 `v6-final:docs/agent-eval-plan.md` 協定，同任務同模型完整 trace，arms：`(rg+Read)` vs `(xg+sg+Read)` vs `(xg+cort)`，指標：成功率、總 tokens、tool-return tokens、turns、`Read` 次數、stale-read 事故；micro：index 秒數、`context` p95、實際輸出 tokens（≤1500）、`impact` vs 人標 precision。

## 10. 安裝與遷移

- **版本釘死**：`cort` 與 `ast-grep` 皆釘版（如 `ast-grep 0.45.0`、`cort 0.1.0`），四平台 `app-<target>.zip` 的 SHA-256 入 `install.sh`（repo 維護，上游無 checksum，fail closed）。
- **Manifest v2**：`ast_grep_bin`、`sg_alias`、`skill_ast_grep`、`skill_xgrep`、`legacy_xg_bin`、`profile` 分鍵，預設新安裝只裝 `cort` + `ast-grep`，`xg` 用 `--with-xgrep` 才裝，舊有 manifest 擁有的 `xg` 不主動刪除，pre-existing binary 不認領。
- **Atomic commit**：installer + 兩個 skill + README/CLAUDE/THIRD_PARTY + 煙測 + CI 同一 commit；`v6-final` 釘在 `5a02c6e3` 不動，NG 另起新 tag（如 `cort-v0.1.0`）待 CI 綠燈再打。
- **交易式安裝**：preflight 全部碰撞 → 下載到 tmp → 驗 SHA 與 `--version` → 原子安裝 binary/skill → 最後寫 manifest。

## 11. 未來擴充插槽（不做，僅保留）

- `ALTER TABLE chunks ADD COLUMN embedding BLOB` + `cort embed --backfill`（`sqlite-vec` 或 `libSQL F32_BLOB`），三臂 RRF（`fts`/`struct`/`dense`）融合時 `chunk_id` 保持穩定。
- `CORTEX_REPORT.md`、`--watch`、`impact --from-diff` 待 `struct`+`context` 驗證有效後再補。

## 12. 與現有設計的差異

- V6 的 `vector_top_k` / DiskANN / Turso 雲端 → 刪。
- V6 的 regex fallback → 刪，改單一 Parser（ast-grep）。
- V6 的 `grammar_version` 僅警告 → 改 `extractor_version` 強制 incremental 比對。
- `xg` 從主角降為 `--with-xgrep` 可選加速器，`rg` 保留給新鮮/短模式。
