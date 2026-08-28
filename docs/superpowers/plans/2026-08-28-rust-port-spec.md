# cort Rust 移植規格契約（Job A，2026-08-28）

本檔是 `docs/superpowers/plans/2026-08-28-rust-port.md` 的實作契約。後續 Job B / C1 / C2 / C3 / D 必須對此實作，不得改 JS 樹。

來源：凍結的 JS 參照（`bin/cort.js`、`src/*.js`、`src/schema.sql`、`src/pack/**`、`tests/*.test.js`）。有歧義處直接引用原始碼，不猜測。

計數：`tests/*.test.js` 共 20 檔、**176** 條 `test('…')`（node:test）。沒有 `describe`/`it` 巢狀。`tests/install-smoke.sh` 不是 unit test，不列入對照表。

---

## 1. 模組 API

慣例：JS 為 ESM；Rust 應對等匯出。未標「匯出」的常數為模組內部。DB 欄位名逐字沿用。錯誤一律 `CortError`（見 1.1），CLI 把它們渲染成 `{ error, detail }` 並 `process.exit(1)`。

### 1.1 `src/errors.js`

| 匯出 | 簽名 | DB |
|---|---|---|
| `CortError` | `class CortError extends Error { constructor(code, detail = null) }` | 無 |

- `this.name = 'CortError'`
- `this.code = code`
- `this.detail = detail`
- `super(\`${code}: ${JSON.stringify(detail)}\`)`
- `toJSON()` 回傳 **恰好** `{ error: this.code, detail: this.detail }`（鍵名 `error`，不是 `code`）

已知 `code` 字串（從 throw 點蒐集，JS 沒有中央登錄）：

`unknown_format`, `missing_pattern`, `missing_lang`, `missing_query`, `missing_symbol`, `unknown_command`, `empty_query`, `fts_query_failed`, `ast_grep_missing`, `ast_grep_version_unreadable`, `ast_grep_version_mismatch`, `ast_grep_timeout`, `ast_grep_spawn_failed`, `parse_failed`, `ast_grep_run_failed`, `run_aborted_malformed`, `scan_too_broad`, `schema_version_mismatch`, `storage_full`, `storage_busy`, `project_not_indexed`, `missing_file`, `file_not_found`, `path_outside_project`, `not_a_file`, `invalid_line_range`, `invalid_limit`

### 1.2 `bin/cort.js`（dispatch）

匯出：

| 匯出 | 簽名 | DB |
|---|---|---|
| `parseArgs` | `parseArgs(argv) → { _: string[], flags: Record<string, string\|true> }` | 無 |

`parseArgs` 行為（逐字）：

- 不以 `-` 開頭 → 推進 `out._`
- 其餘：`name = a.replace(/^--?/, '')`（一或兩個 dash 都剝掉）
- `next === undefined || next.startsWith('-')` → `flags[name] = true`，否則 `flags[name] = next` 且 `i += 1`
- 重複旗標後寫覆蓋前寫（物件，不是陣列）

未匯出但 CLI 契約必須對等：

- `KNOWN_COMMANDS = ['index', 'status', 'projects', 'delete', 'struct', 'context', 'impact', 'read', 'recall']`
- `main()` 讀 `process.argv.slice(2)`
- `--help` / `-h` / 位置參數 `help`：**在任何指令副作用之前**回傳 usage，exit 0。`cort index --help` 不得建立 cache / 不得 index。
- `USAGE` JSON 形狀見 §4
- `resolveFormat(flags)`：`parseFormat(flags.f ?? flags.format)`；`null` → `CortError('unknown_format', { hint: '--format json|lean' })`
- `emit(value, format = FORMAT.JSON, command = null)`：`process.stdout.write(render(command, format, value))`（**無**結尾以外的 extra newline；`render` 自己加 `\n`）
- `openProject(root)`：`fs.realpathSync(root)` → `projectIdFor` / `dbPathFor` / `openDb` / `ensureSchema`。回傳 `{ real, projectId, db }`
- CortError：`emit(err.toJSON()); process.exit(1)`
- 非 CortError：原樣 rethrow（未捕捉）

Dispatch 表：

| command | ast-grep pin? | `withBusyRetry`? | 開哪個 root | 預設 / 必填 | 輸出 command 名（給 lean） |
|---|---|---|---|---|---|
| help / `--help` / `-h` | 否 | 否 | 不開 DB | — | 無（JSON `USAGE`） |
| `index` | 是 | 是 | `positional[1] ?? cwd` | `--incremental` 為 true 時走 `incrementalIndex`，否則 `fullIndex` | 無（永遠 JSON） |
| `status` | 僅當已 index | 否 | `positional[1] ?? cwd` | 未 index 只 emit `statusOf`；已 index 再 merge `computeStale` | 無 |
| `projects` | 否 | 否 | 不開專案 DB | `listProjects()` | 無 |
| `delete` | 否 | 否 | `realpathSync(positional[1] ?? cwd)` | `deleteProject` | 無 |
| `struct` | 是 | 否 | **永遠 cwd** | `-p`/`--pattern` 必為 string，否則 `missing_pattern`；`--lang` 必為 string，否則 `missing_lang`；`-g` 若為 string 則 `globs=[flags.g]` 否則 `[]`；`budget = Number(flags.budget ?? 1500)` | `'struct'` |
| `context` | 是 | 否 | **永遠 cwd** | `positional[1]` 必為 string，否則 `missing_query`；`budget = Number(flags.budget ?? 1500)`；`includeAmbiguous = flags['include-ambiguous'] === true`；`fullContent = flags.content === 'full'` | `'context'` |
| `impact` | 是 | 否 | **永遠 cwd** | `--symbol` 必為 string，否則 `missing_symbol`；`depth = Number(flags.depth ?? 3)` | `'impact'` |
| `read` | 否 | 是 | **永遠 cwd** | `positional[1]` 當 `filePath`；`startLine: flags.start, endLine: flags.end`（可能是 string 或 undefined） | `'read'` |
| `recall` | 否 | 是 | **永遠 cwd** | `positional[1]` 必為 string，否則 `missing_query`；`limit: flags.limit ?? 5`；`fullContent = flags.content === 'full'` | `'recall'` |
| 其他 | — | — | — | `CortError('unknown_command', { command: command ?? null, known: KNOWN_COMMANDS })` | — |

歧義（引用，不猜測）：

- `-g` 只能留下**最後一個**值（`flags` 是物件）。JS 沒有把重複 `-g` 收成陣列。
- `Number(true) === 1`：`--budget` 後面沒有值時 `flags.budget === true`，budget 變成 1。
- `index` / `status` / `projects` / `delete` / help **忽略** `-f`，永遠走 `emit(value)` 預設 JSON。

環境變數：`CORT_CACHE_DIR`（cache 根；預設見 `cacheDir`）、`CORT_AST_GREP_BIN`（覆寫 binary）。

### 1.3 `src/budget.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `estimateTokens` | `estimateTokens(text)` | `Math.ceil(String(text ?? '').length / 4)`。`''`/`null`/`undefined` → 0；5 chars → 2 | 無 |
| `applyBudget` | `applyBudget(items, budgetTokens, render)` | `render` 是 callback，**不是** `src/render.js`。回傳 `{ kept, truncated }` | 無 |

`applyBudget` 規則：

- 由前往後累加 `cost = estimateTokens(render(item))`
- **第一筆永遠收下**，即使它已超過 budget（`if (kept.length > 0 && used + cost > budgetTokens)`）
- 之後放不下就立刻 `{ kept, truncated: true }`，後面的 item 不再考慮
- 全部放下 → `{ kept, truncated: false }`

### 1.4 `src/pack.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `PACK_DIR` | `fileURLToPath(new URL('./pack', import.meta.url))` | 絕對路徑，指向 `src/pack` | 無 |
| `SGCONFIG` | `path.join(PACK_DIR, 'sgconfig.yml')` | 絕對路徑 | 無 |
| `packFiles` | `packFiles() → string[]` | 遞迴 `PACK_DIR`，收 `p.endsWith('.yml')` 的**檔案**，`out.sort()` 後回傳絕對路徑 | 無 |
| `extractorVersion` | `extractorVersion() → hex-sha256` | 依 `packFiles()` 順序把每個檔案的 **raw bytes** `h.update(...)`；`digest('hex')` | 無 |

契約：**cort 不解析 YAML**。規則由 ast-grep 讀 `SGCONFIG`。`extractor_version` 是 pack 檔案內容的 hash，不是 ast-grep 版本。

現況 `packFiles()` 會包含：`src/pack/sgconfig.yml` + `src/pack/rules/{javascript,python,rust,tsx,typescript}.yml`（至少 5 個 `.yml`，測試 `files.length >= 5`）。

`src/pack/sgconfig.yml` 全文：

```yaml
ruleDirs:
  - rules
```

規則 `message` tag（chunker 靠這些前綴，見 §5）：

| 檔 | rule id | `message` |
|---|---|---|
| javascript.yml | cort-js-chunk-function / class / method / const-function | `chunk:function` / `chunk:class` / `chunk:method` / `chunk:function` |
| javascript.yml | cort-js-edge-imports / calls | `edge:imports` / `edge:calls` |
| typescript.yml | 同上（id 前綴 `cort-ts-`） | 同上 |
| tsx.yml | 同上（id 前綴 `cort-tsx-`） | 同上 |
| python.yml | cort-py-chunk-function / class / const-function | `chunk:function` / `chunk:class` / `chunk:function` |
| python.yml | cort-py-edge-imports / calls | `edge:imports` / `edge:calls` |
| rust.yml | **只有** `cort-rust-chunk-function`（`kind: function_item`） | `chunk:function` |

事實：`rust.yml` **沒有** class / method / imports / calls 規則。Rust `impl` 方法仍是 tree-sitter `function_item`，所以 `work` 會變成 `chunk:function`（C1-10 測的是這個）。Python class 方法是 `function_definition`，pack 測試把 `alpha` 與 `go` 都算 `chunk:function`（沒有 `chunk:method`）。

const-function 規則（JS/TS/TSX）：`variable_declarator` 的 value 是 arrow / function_expression，或 `call_expression` 的 arguments 裡有 arrow（覆蓋 `createHandler("x", async (req) => …)`）。**不**匹配 `.map(n => …)` 賦值、也不匹配 `const gamma = helper` 別名。

### 1.5 `src/fts.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `MAX_OR_TERMS` | `20` | — | 無 |
| `sanitizeFtsQuery` | `sanitizeFtsQuery(raw) → { query, truncated_query }` | 見下 | 無 |
| `keywordSearch` | `keywordSearch(db, projectId, raw, limit) → { rows, truncated_query }` | 見下 | **讀** `chunks_fts` JOIN `chunks` |

`sanitizeFtsQuery`：

1. `String(raw ?? '').trim().split(/\s+/).filter(t => t.length > 0)`
2. 0 個 term → `CortError('empty_query', { raw })`
3. `truncated_query = terms.length > MAX_OR_TERMS`；`kept = terms.slice(0, MAX_OR_TERMS)`
4. 每個 term：`"${t.replaceAll('"', '""')}"`（FTS 雙引號跳脫）
5. `query = quoted.join(' OR ')`

測試鎖定的字面值：

- `'helper'` → `'"helper"'`
- `'foo(bar)'` → `'"foo(bar)"'`
- `'a - b'` → `'"a" OR "-" OR "b"'`
- `'src/alpha.ts'` → `'"src/alpha.ts"'`
- `'say "hi"'` → `'"say" OR """hi"""'`

`keywordSearch` SQL（欄位名即 rows 形狀）：

```sql
SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type, c.start_line, c.end_line,
       c.content, c.language, c.chunk_source, bm25(chunks_fts) AS score
  FROM chunks_fts
  JOIN chunks c ON c.rowid = chunks_fts.rowid
 WHERE chunks_fts MATCH ? AND c.project_id = ?
 ORDER BY score
 LIMIT ?
```

MATCH 失敗 → `CortError('fts_query_failed', { query, message: String(err && err.message) })`。**沒有** embedding 降級。

### 1.6 `src/staleness.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `computeStale` | `computeStale({ db, bin, root, projectId })` | `{ index_is_stale, deleted_files, changed_files }` | **讀** `projects.path`、`file_state(file_path, file_content_hash)` |

演算法：

1. `base = proj ? proj.path : root`（**永遠** `projects.path`，不是 cwd）
2. `stored = Map(file_path → file_content_hash)` from `file_state`
3. `diskFiles = Set(walkFiles(base))`
4. `deleted = stored keys not in diskFiles`，**sort**
5. `gitCandidates(base)`：
   - `gitAvailable`：candidates = unique(`changed` ∪ `{diskFiles − stored}`)，**sort**
   - 否則 candidates = `[...diskFiles].sort()`
6. 對每個 candidate：檔案不存在就 skip；否則 `extractFile`，若 `stored.get(rel) !== result.file_content_hash` 則推進 `changedFiles`
7. `index_is_stale = deleted.length > 0 || changedFiles.length > 0`

**不寫**任何表。`changed_files` / `deleted_files` 都是相對 posix 路徑。

### 1.7 `src/ast-grep.js`

見 §2。匯出：`AST_GREP_PINNED`, `SUBPROCESS_TIMEOUT_MS`, `resolveAstGrepBin`, `astGrepVersion`, `assertAstGrepVersion`, `execAstGrep`。DB：無。

### 1.8 `src/impact.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `DEFAULT_DEPTH` | `3` | — | 無 |
| `impactCommand` | `impactCommand({ db, bin, root, projectId, symbol, depth = DEFAULT_DEPTH })` | 見 §4.1 | **讀** `chunks`（seed SELECT）、`relationships`（via `getTransitiveDependents`）；**讀** `projects`/`file_state`（via `computeStale`）。**不寫** relationships |

`symbol` 以逗號切：`String(symbol ?? '').split(',').map(trim).filter(Boolean)`。空 names → `seeds = []`。

Seed SQL：

```sql
SELECT chunk_id, file_path, symbol_name, start_line, end_line
  FROM chunks WHERE project_id = ? AND symbol_name IN (/* 一個 ? per name */)
 ORDER BY file_path, start_line
```

對每個 seed 呼叫 `getTransitiveDependents(db, seed.chunk_id, depth)`，以 `chunk_id` merge，留 **最小 hop**。然後剔除 seed 自己的 `chunk_id`。最後 `sort`：`a.hop - b.hop || a.chunk_id.localeCompare(b.chunk_id)`。

Unresolved：對每個 seed **重跑** `extractFile`（讀磁碟），只看 `e.source_symbol === seed.symbol_name` 的邊；`resolveTargets` 長度 0 才 inline。`seenSymbols` 以 `raw_target` 去重。檔案不存在 → skip 該 seed。

回傳欄位見 §4.1。`symbol` 原樣回傳（可能含逗號）。

### 1.9 `src/render.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `FORMAT` | `{ JSON: 'json', LEAN: 'lean' }` | — | 無 |
| `parseFormat` | `parseFormat(raw) → 'json'\|'lean'\|null` | `typeof raw === 'string' ? raw.toLowerCase() : 'json'`；僅接受 `lean`/`json` | 無 |
| `renderImpact` | `renderImpact(payload) → string` | 見 §4.2 | 無 |
| `renderStruct` | `renderStruct(payload) → string` | 見 §4.2 | 無 |
| `renderContext` | `renderContext(payload) → string` | 見 §4.2 | 無 |
| `renderRead` | `renderRead(payload) → string` | 見 §4.2 | 無 |
| `renderRecall` | `renderRecall(payload) → string` | 見 §4.2 | 無 |
| `render` | `render(command, format, payload) → string` | `format !== LEAN` **或** command 不是 impact/struct/context/read/recall → `` `${JSON.stringify(payload, null, 2)}\n` `` | 無 |

`parseFormat(undefined) === 'json'`；`'LEAN' === 'lean'`；`'yaml' === null`。

### 1.10 `src/db.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `SCHEMA_VERSION` | `2` | — | 無 |
| `projectIdFor` | `projectIdFor(realPath) → hex-sha256` | `createHash('sha256').update(realPath).digest('hex')`（64 hex） | 無 |
| `cacheDir` | `cacheDir()` | `process.env.CORT_CACHE_DIR ?? path.join(os.homedir(), '.cache', 'cortex-ng')` | 無 |
| `dbPathFor` | `dbPathFor(realPath)` | `path.join(cacheDir(), \`${projectIdFor(realPath)}.db\`)` | 無 |
| `openDb` | `openDb(dbPath)` | 見下 | 開檔 |
| `ensureSchema` | `ensureSchema(db)` | 見 §3 | **寫** `_cortex_meta` + `exec` schema.sql |
| `getMeta` | `getMeta(db, key) → string\|null` | `SELECT value FROM _cortex_meta WHERE key = ?` | **讀** `_cortex_meta` |
| `setMeta` | `setMeta(db, key, value)` | `INSERT … ON CONFLICT(key) DO UPDATE SET value = excluded.value` | **寫** `_cortex_meta` |
| `listProjects` | `listProjects() → array` | 見下 | **讀** 每個 `*.db` 的 `projects` |
| `deleteProject` | `deleteProject(realPath) → { deleted, db_path }` | 見下 | 刪檔 |
| `withBusyRetry` | `withBusyRetry(fn)` | 見 §7 | 無（包裝器） |

`openDb`：

- `dbPath !== ':memory:'` 時 `mkdirSync(dirname, { recursive: true })`
- `new Database(dbPath)`（better-sqlite3）
- `pragma journal_mode = WAL`
- `pragma busy_timeout = 5000`
- `pragma foreign_keys = ON`
- 非 memory：`chmodSync(dbPath, 0o600)`

`listProjects`：cache dir 不存在 → `[]`。`readdirSync(dir).sort()`，只看 `name.endsWith('.db')`，readonly 打開，`SELECT project_id, name, path, git_head, last_indexed_at FROM projects` 用 `.get()`（一列）。成功則 `{ ...row, db_path }`。任何 throw 吞掉（「not a cort db」）。finally close。

`deleteProject`：檔不存在 → `{ deleted: false, db_path }`。否則對 `''` / `'-wal'` / `'-shm'` 做 `fs.rmSync(..., { force: true })` → `{ deleted: true, db_path }`。

`withBusyRetry`：`attempt = 0..3` 共 **4** 次。`err.code === 'SQLITE_BUSY'` continue。`SQLITE_FULL` 或 `SQLITE_CORRUPT` → `CortError('storage_full', { sqlite_code })`。其他立刻 rethrow。四次都 BUSY → `CortError('storage_busy', { message: String(lastErr) })`。測試字面：`'one attempt plus three retries'` → `always === 4`。

### 1.11 `src/context.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `DEFAULT_BUDGET` | `1500` | — | 無 |
| `NEIGHBORS_PER_SEED` | `8` | — | 無 |
| `CONTENT_HEAD_LINES` | `12` | — | 無 |
| `contextCommand` | `contextCommand({ db, bin, root, projectId, query, budget = DEFAULT_BUDGET, includeAmbiguous = false, fullContent = false })` | 見 §4.1 | **讀** `chunks`（exact symbol）、`chunks_fts`（fallback）、`relationships`（neighbors）、`projects`/`file_state`（stale）。**不寫** relationships |

未匯出：`MAX_SEEDS = 5`。

解析順序：

1. `exactSymbolSeeds`：`SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, content, language FROM chunks WHERE project_id = ? AND symbol_name = ? ORDER BY file_path, start_line LIMIT 5`。命中 → `resolution = 'exact_symbol'`（**不碰 FTS**）。
2. 否則 `keywordSearch(..., MAX_SEEDS)`；`resolution = rows.length > 0 ? 'fts' : 'none'`。
3. 每個 seed：`getNeighbors(..., NEIGHBORS_PER_SEED)`，預設 **丟掉** `confidence === 'AMBIGUOUS'`，除非 `includeAmbiguous`。
4. `unresolvedFor`：與 impact 相同，重跑 `extractFile`，零 target 才 inline。不持久化。
5. content：`fullContent` 為假且行數 `> CONTENT_HEAD_LINES` 時，留下前 12 行再加 `'\n…'`，`content_truncated = true`。否則原 `row.content`，`content_truncated = false`。
6. `applyBudget(seeds, budget, s => JSON.stringify(s))`。
7. 額外：若 `packetTokens(kept) > budget * 1.15`，把每個 seed 的 neighbors 從 `NEIGHBORS_PER_SEED-1` 均勻砍到 0，直到估計值 ≤ `budget * 1.15` 或沒 neighbor。然後 `truncated = true`。

`packetTokens` **不是**對最終 payload 估的。原文：

```js
const packetTokens = (ks) => Math.ceil(JSON.stringify({
  query,
  resolution: ks.length === 0 ? 'none' : resolution,
  seeds: ks,
  seed_count: seeds.length,
  truncated: true,
  truncated_query: truncatedQuery,
  index_is_stale: false,
}).length / 4);
```

注意：估計時 **硬編碼** `truncated: true` 與 `index_is_stale: false`，跟實際回傳值可以不一致。最終 `resolution`：`kept.length === 0 ? 'none' : resolution`。`seed_count` 是 budget **之前**的 `seeds.length`。

`src/context.js` 不得 `import` `struct.js`（C-D 測試鎖這個）。

### 1.12 `src/struct.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `MAX_MALFORMED_RATIO` | `0.10` | — | 無 |
| `MAX_NEIGHBORS` | `3` | — | 無 |
| `UNBOUNDED_SCAN_FILE_LIMIT` | `2000` | — | 無 |
| `preflightPattern` | `preflightPattern({ bin, pattern, lang, paths })` | 成功不回值；失敗 throw `parse_failed` | 無 |
| `runPattern` | `runPattern({ bin, pattern, lang, paths, rewrite, skipPreflight = false })` | `{ matches, malformed, total }` | 無 |
| `containmentJoin` | `containmentJoin(db, projectId, match)` | chunk row 或 `null` | **讀** `chunks` |
| `structCommand` | `structCommand({ db, bin, root, projectId, pattern, lang, globs, budget, fileLimit = UNBOUNDED_SCAN_FILE_LIMIT })` | 見 §4.1 | **讀** `file_state`（count）、`chunks`、`relationships`、`projects`/`file_state`（stale） |

未匯出：`ERROR_NODE_MARKER = 'Pattern contains an ERROR node'`。

`preflightPattern` argv 見 §2。`r.code === 2` **或** `r.stderr.includes(ERROR_NODE_MARKER)` → `CortError('parse_failed', { pattern, lang, detail: r.stderr.trim() })`。exit 0 但 stderr 含 ERROR node 也 fail（fake fixture `preflight-bad` 就是這條）。

`runPattern`：

- 預設先 preflight
- argv 見 §2
- 若 `r.code !== 0 && r.stdout.length === 0 && r.stderr.trim().length > 0` → `ast_grep_run_failed`
- `parseScanStream`；`total > 0 && malformed/total > 0.10` → `run_aborted_malformed`（**只 abort 這次 query**，不是 index）
- match 形狀：`{ file: rec.file, text: rec.text, start_line: rec.range.start.line + 1, end_line: rec.range.end.line + 1, replacement: rec.replacement }`
- 零命中（exit 1、兩 stream 空）是乾淨空結果，不是 `parse_failed`

`containmentJoin` SQL：

```sql
SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, language
  FROM chunks
 WHERE project_id = ? AND file_path = ? AND start_line <= ? AND end_line >= ?
 ORDER BY (end_line - start_line) ASC, start_line DESC
 LIMIT 1
```

第三、四個綁定是 `match.start_line`、`match.end_line`。沒列 → `null`。

`structCommand`：

- `globs.length === 0` 且 `SELECT COUNT(*) FROM file_state WHERE project_id = ?` > `fileLimit` → `CortError('scan_too_broad', { indexed_files, limit, hint: "narrow the scan with -g '<glob>', e.g. cort struct -p '<pattern>' --lang ts -g 'src/**/*.ts'" })`
- `paths = globs.length > 0 ? globs : [root]`
- 路徑正規化：`m.file.startsWith(root)` 則 `m.file.slice(root.length + 1).split('\\').join('/')`，否則原樣
- neighbors：`getNeighbors(..., MAX_NEIGHBORS)` 再 filter `confidence === 'EXTRACTED' || confidence === 'INFERRED'`，再 `slice(0, MAX_NEIGHBORS)`
- `applyBudget(enriched, budget, m => JSON.stringify(m))`
- 回傳見 §4.1。`match_count` 是 budget **前**的 `enriched.length`；`malformed_lines` 來自 `runPattern`

### 1.13 `src/graph.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `CONFIDENCE_SCORE` | `{ EXTRACTED: 1.0, INFERRED: 0.7, AMBIGUOUS: 0.5 }` | 測試 `deepEqual` 鎖這三個數字 | 無 |
| `buildImportMap` | `buildImportMap(edges) → Map` | 只收 `rel_type === 'imports'`；`map.set(e.raw_target, e.raw_target)` | 無 |
| `resolveTargets` | `resolveTargets({ db, projectId, filePath, importMap, symbol }) → chunk_id[]` | 見下 | **讀** `chunks` |
| `relationshipRowsForFile` | `relationshipRowsForFile({ db, projectId, filePath, chunks, edges }) → row[]` | 見下 | **讀** `chunks`（via resolveTargets） |
| `unresolvedInline` | `unresolvedInline(symbol)` | `{ confidence: 'AMBIGUOUS', confidence_score: 0.5, confidence_reasoning: \`unresolved: ${symbol}\` }`。**沒有** `target_chunk_id` / `chunk_id` | 無 |
| `getNeighbors` | `getNeighbors(db, chunkId, limit)` | 見下 | **讀** `relationships` JOIN `chunks` |
| `getTransitiveDependents` | `getTransitiveDependents(db, chunkId, depth)` | 見下 | **讀** `relationships` JOIN `chunks` |

`resolveTargets`：

```sql
SELECT chunk_id, file_path FROM chunks
 WHERE project_id = ? AND symbol_name = ? ORDER BY chunk_id
```

優先序：同 `filePath` 全收 → 否則 import map 對得上的（`file_path` 去副檔名後 `=== prefix` 或 `endsWith('/' + prefix)`；相對 import 用 `path.posix.join(dirname(filePath), spec)` 再 normalize）→ 否則全部。0 列 → `[]`。

`relationshipRowsForFile`：

- `e.source_symbol === null` skip（檔級 import 沒有 source chunk）
- source 必須能在 `chunkBySymbol` 找到（**同檔 chunks 後寫覆蓋前寫**；Map）
- targets 去掉 self-edge
- 0 targets：**不寫任何列**
- `n === 1` → `confidence = 'INFERRED'`, score `0.7`, reasoning `` `resolved: ${e.raw_target}` ``
- `n > 1` → `AMBIGUOUS`, score `0.5 * (1/n)`, reasoning `` `ambiguous: ${e.raw_target} (${n} candidates)` ``
- 去重 key：`` `${sourceChunkId} ${target} ${e.rel_type}` ``

**事實：此函式從不寫 `EXTRACTED`。** `CONFIDENCE_SCORE.EXTRACTED` 只存在常數裡。struct 的 neighbor filter 含 EXTRACTED，是死分支，除非人手插入。

`getNeighbors` SQL：outgoing UNION ALL incoming，`ORDER BY confidence_score DESC, chunk_id LIMIT ?`。列：`chunk_id, symbol_name, file_path, start_line, end_line, rel_type, confidence, confidence_score, direction`（`'outgoing'` / `'incoming'`）。

`getTransitiveDependents`：recursive CTE，從 `target_chunk_id = seed` 往 **source**（反向 dependents），`hop < depth` 所以 `depth=3` 得到 hop 1..3。排除 seed 自己。`MIN(d.hop) AS hop`，`ORDER BY hop, c.chunk_id`。列：`chunk_id, symbol_name, file_path, start_line, end_line, hop`。

### 1.14 `src/indexer.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `IGNORE_DIRS` | `Set(['node_modules','dist','build','.git','__pycache__','.venv','venv','target','coverage','.next','.cache'])` | 比對 **目錄名**，不是路徑 | 無 |
| `SOURCE_EXT` | `Set(['.ts','.tsx','.js','.jsx','.mjs','.cjs','.py','.rs'])` | — | 無 |
| `walkFiles` | `walkFiles(root) → posix 相對路徑[]` | 見下 | 無 |
| `gitHeadOf` | `gitHeadOf(root) → string\|null` | `git -C root rev-parse HEAD`；失敗或空 → `null` | 無 |
| `extractAll` | `extractAll({ bin, root, projectId, files })` | `files.map` → `{ rel, result: extractFile(...) }` | 無 |
| `fullIndex` | `fullIndex({ db, bin, root, projectId })` | 見 §4.3 | **寫** `projects`, `chunks`, `file_state`, `relationships`, `_cortex_meta`；DELETE 該 project 的 chunks/file_state |
| `statusOf` | `statusOf({ db, root, projectId })` | 見 §4.3 | **讀** `projects`, `file_state`, `chunks`, `reading_notes`, `relationships` |

`walkFiles`：跳過 symlink；目錄若 `IGNORE_DIRS.has(entry.name)` 不遞迴；檔案需 `SOURCE_EXT.has(extname)`；路徑 `path.relative(root, abs).split(path.sep).join('/')`；readdir 以 `localeCompare` 排，最終 `out.sort()`。

`fullIndex`：

1. `files = walkFiles(root)`；`version = extractorVersion()`；`head = gitHeadOf(root)`
2. **在 transaction 外** `extractAll`（子行程不得佔 write lock）
3. 單一 `db.transaction`：
   - `INSERT INTO projects … ON CONFLICT(project_id) DO UPDATE SET name, path, git_head, last_indexed_at, extractor_version`（`name = path.basename(root)`，`last_indexed_at = Date.now()`）
   - `DELETE FROM chunks WHERE project_id = ?`
   - `DELETE FROM file_state WHERE project_id = ?`
   - 每個檔：insert 所有 chunks；`INSERT INTO file_state … ON CONFLICT(project_id, file_path) DO UPDATE SET file_content_hash, updated_at = datetime('now')`；`unparsed` 只計數
   - 然後第二輪：`relationshipRowsForFile` + `INSERT INTO relationships … ON CONFLICT(source_chunk_id, target_chunk_id, rel_type) DO NOTHING`
   - `setMeta(db, 'extractor_version', version)`
4. 回傳 `{ files, chunks, unparsed, relationships, elapsed_ms }`

`INSERT INTO chunks` 欄位：`chunk_id, project_id, file_path, symbol_name, chunk_type, start_line, end_line, content, content_hash, language, chunk_source`（`created_at`/`updated_at` 走 DEFAULT）。

`statusOf`：沒有 project 列 → `{ project_id, path: root, indexed: false }`。否則見 §4.3。relationships count：`JOIN chunks s ON s.chunk_id = r.source_chunk_id WHERE s.project_id = ?`。**不呼叫 ast-grep**。

### 1.15 `src/chunker.js`

見 §5。匯出：`chunkIdFor`, `parseScanStream`, `edgeString`, `fileContentHash`, `extractFile`。

`extractFile` 讀磁碟由呼叫端做完再傳 `source`；本函式只 spawn ast-grep。DB：無。

### 1.16 `src/incremental.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `gitCandidates` | `gitCandidates(root) → { changed, deleted, gitAvailable }` | 見下 | 無 |
| `removeFile` | `removeFile({ db, projectId, filePath })` | 無回傳 | **刪** `chunks`、`file_state`（該 file）；relationships 靠 FK CASCADE |
| `reindexOneFile` | `reindexOneFile({ db, bin, root, projectId, filePath })` | `{ chunks, unparsed, relationships, skipped, removed }` | **寫** chunks/file_state/relationships（該檔） |
| `incrementalIndex` | `incrementalIndex({ db, bin, root, projectId })` | 見 §4.3 | 視 mode |

`isIndexable(rel)`：副檔名在 `SOURCE_EXT`，且路徑段沒有 `IGNORE_DIRS`。

`gitCandidates`：

- `git -C root diff --name-status -M HEAD`；失敗 → `{ changed: [], deleted: [], gitAvailable: false }`
- `R*`（rename）：舊 path → deleted，新 path → changed
- `D*` → deleted
- 其餘（含 `M`）→ changed
- 再 `git ls-files --others --exclude-standard` 推進 changed
- 兩個陣列都 **sort**

`reindexOneFile`：檔不存在 → `removeFile`，`{ chunks:0, unparsed:0, relationships:0, skipped:false, removed:true }`。hash 與 `file_state` 相同 → `{ …0, skipped:true, removed:false }`（**不寫**）。否則一個 transaction：DELETE 該檔 chunks → insert chunks → upsert file_state → insert rels `ON CONFLICT DO NOTHING`。

**事實：只重建「這個檔當 source」的邊。** 別的檔指向被刪 chunk 的 incoming 邊會被 CASCADE 清掉，**不會**在這次 incremental 重建，直到那些檔自己被 reindex。

`incrementalIndex`：

1. `stored = getMeta(db, 'extractor_version')`；`stored !== null && stored !== version` → stderr 寫 `extractor_version mismatch: ${stored} -> ${version}, full reindex required\n`，然後 `fullIndex`，回傳 `{ mode: 'full', ...full, elapsed_ms }`
2. `!gitAvailable` → 同樣 full
3. 否則：每個 deleted `removeFile`；每個 changed `reindexOneFile`（**各自一筆 transaction**）
4. **最後另一筆 transaction**：`UPDATE projects SET git_head, last_indexed_at, extractor_version` + `setMeta extractor_version`。中斷時已 commit 的檔留下，`git_head` **不前進**

回傳 incremental 形狀見 §4.3。`files_examined = changed.length + deleted.length`。

### 1.17 `src/readings.js`

| 匯出 | 簽名 | I/O | DB |
|---|---|---|---|
| `DEFAULT_RECALL_LIMIT` | `5` | — | 無 |
| `RECALL_HEAD_LINES` | `12` | — | 無 |
| `readFragment` | `readFragment({ db, root, projectId, filePath, startLine, endLine })` | 見 §4.1 | **讀寫** `reading_notes`（FTS 靠 trigger）；**讀** `projects` |
| `recallReadings` | `recallReadings({ db, root, projectId, query, limit = DEFAULT_RECALL_LIMIT, fullContent = false })` | 見 §4.1 | **讀寫** `reading_notes` / `reading_notes_fts`；**讀** `projects` |

`requireIndexed`：`SELECT 1 FROM projects WHERE project_id = ? AND last_indexed_at IS NOT NULL` 無列 → `project_not_indexed`。

`resolveProjectFile`：空/非 string → `missing_file`。`path.resolve(root, requestedPath)` 再 `realpathSync`；失敗 → `file_not_found`。`path.relative(root, abs)` 若 `''` 或 `'..'` 或 `startsWith('..'+sep)` 或 `path.isAbsolute(relative)` → `path_outside_project`。非檔 → `not_a_file`。回傳 `{ abs, rel }`（rel 為 posix）。

`positiveLine(value, name, fallback)`：`undefined`/`null` → fallback。否則 `Number(value)` 必須是 safe integer 且 `>= 1`，否則 `invalid_line_range: { [name]: value }`。

`readFragment` 流程：

1. `start = positiveLine(startLine, 'start', 1)`；`requestedEnd = positiveLine(endLine, 'end', null)`；`requestedEnd !== null && requestedEnd < start` → `invalid_line_range`
2. 讀 `reading_notes`：`ORDER BY (end_line - start_line), start_line`（最小 span 優先）
3. covering：省略 end 時 `start_line <= start && ends_at_eof === 1`；有 end 時 `start_line <= start && end_line >= requestedEnd`
4. covering 且 `statMatches`（`source_size === stat.size && source_mtime_ms === stat.mtimeMs`）→ increment `read_count`/`last_read_at`，`source: 'store'`，content 用 `sliceStored`
5. 否則讀檔。`start` 或 `end` 超過 `lines.length` → `invalid_line_range`（含 `file_lines`）
6. 既有 notes 的 `source_hash` 不同 → `DELETE FROM reading_notes WHERE project_id AND file_path`（FTS trigger 清）
7. hash 相同但 mtime/size 不同 → `UPDATE … SET source_mtime_ms, source_size`；再找 covering；命中仍 `source: 'store'`
8. 否則 INSERT：

```sql
INSERT INTO reading_notes
  (project_id, file_path, start_line, end_line, ends_at_eof, content, source_hash,
   source_mtime_ms, source_size, read_count, first_read_at, last_read_at)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)
ON CONFLICT(project_id, file_path, start_line, end_line) DO UPDATE SET
  ends_at_eof = excluded.ends_at_eof, content = excluded.content, source_hash = excluded.source_hash,
  source_mtime_ms = excluded.source_mtime_ms, source_size = excluded.source_size,
  read_count = reading_notes.read_count + 1, last_read_at = excluded.last_read_at
```

`ends_at_eof = requestedEnd === null ? 1 : 0`。`source: 'filesystem'`。

事實：partial note（`ends_at_eof=0`）**不能**當 whole-file cache。省略 `--end` 的讀會 cache start→EOF。full re-index **不刪** reading_notes（`fullIndex` 只 DELETE chunks/file_state）。

`recallReadings`：`limit` 必須是 1..100 的 safe integer，否則 `invalid_limit`。FTS：

```sql
SELECT n.*, bm25(reading_notes_fts) AS score
  FROM reading_notes_fts
  JOIN reading_notes n ON n.reading_id = reading_notes_fts.rowid
 WHERE reading_notes_fts MATCH ? AND n.project_id = ?
 ORDER BY score LIMIT ?
```

LIMIT 綁 `parsedLimit * 4`。失敗 → `fts_query_failed`。

對每個 candidate：同檔只 stat 一次。mtime/size 不符就比 content hash；hash 也不符 **或** 檔不存在 → 把該 `file_path` 整檔 notes DELETE，後面同檔 skip。hash 符但 mtime/size 不符 → UPDATE mtime/size 仍收下。`trimContent`：非 `fullContent` 且行數 > 12 → 前 12 行 + `'\n…'`，`content_truncated: true`。停在 `results.length >= parsedLimit`。

---

## 2. ast-grep argv 與 0.45.2 pin

檔案：`src/ast-grep.js`。binary **永遠**是 `ast-grep`（或 `CORT_AST_GREP_BIN`），**從不**呼叫 `sg`。

### 2.1 Pin

```js
export const AST_GREP_PINNED = '0.45.2';
export const SUBPROCESS_TIMEOUT_MS = 30_000;
```

`resolveAstGrepBin()`：

- `candidate = (CORT_AST_GREP_BIN 非空) ? override : 'ast-grep'`
- `spawnSync(candidate, ['--version'], { encoding: 'utf8' })`
- `probe.error || probe.status !== 0` → `CortError('ast_grep_missing', { candidate })`
- 回傳 `candidate` 字串（尚未比對版本）

`astGrepVersion(bin)`：

- `spawnSync(bin, ['--version'], { encoding: 'utf8' })`
- 失敗 → `ast_grep_missing { candidate: bin }`
- `const m = /(\d+\.\d+\.\d+)/.exec(r.stdout)`；沒有 → `ast_grep_version_unreadable { stdout }`
- 回傳 `m[1]`（**只用 stdout**，不用 stderr）

`assertAstGrepVersion(bin)`：

- `found !== AST_GREP_PINNED`（字串全等，不是 semver range）→ `CortError('ast_grep_version_mismatch', { found, expected: AST_GREP_PINNED })`

CLI 在 `index` / 已 index 的 `status` / `struct` / `context` / `impact` 呼叫 `resolveAstGrepBin` + `assertAstGrepVersion`。`read`/`recall`/`projects`/`delete`/`help` 不碰 ast-grep。

### 2.2 `execAstGrep(bin, args, opts = {})`

```js
spawnSync(bin, args, {
  encoding: 'utf8',
  cwd: opts.cwd,           // 可能 undefined
  timeout: opts.timeoutMs ?? 30_000,
  maxBuffer: 256 * 1024 * 1024,
})
```

- `r.error.code === 'ETIMEDOUT'` → `ast_grep_timeout { args, timeoutMs }`
- `r.signal === 'SIGTERM'` → 同樣 `ast_grep_timeout`（node 對 timeout 常給 SIGTERM）
- 其他 `r.error` → `ast_grep_spawn_failed { args, message: r.error.message }`
- **非 0 exit 不 throw**；回 `{ code: r.status ?? 0, stdout: r.stdout ?? '', stderr: r.stderr ?? '' }`

### 2.3 每一種呼叫的確切 argv

| 呼叫點 | argv 陣列（`bin` 之後） | cwd | timeout |
|---|---|---|---|
| `resolveAstGrepBin` / `astGrepVersion` | `['--version']` | 預設 | spawnSync 預設（無 30s pin） |
| `extractFile`（index/stale/impact/context） | `['scan', '--json=stream', '--config', SGCONFIG, absPath]` | 未設 | `opts.timeoutMs ?? 30000` |
| `preflightPattern` | `['run', '--debug-query=ast', '--lang', lang, '-p', pattern, ...paths]` | 未設 | 30000 |
| `runPattern` | `['run', '--json=stream', '--strictness', 'ast', '--lang', lang, '-p', pattern]`；若 `rewrite !== undefined` 再 `['--rewrite', rewrite]`；最後 `...paths` | 未設 | 30000 |

CLI 的 `struct` **從不**傳 `rewrite`。`paths` 在 struct 是 `globs` 或 `[root]`（絕對路徑）。

`SGCONFIG` 是 **絕對路徑**（`src/pack/sgconfig.yml`），不是 `'sgconfig.yml'`。

git 子行程（不是 ast-grep，列在這裡以免漏）：

- `gitHeadOf`：`['-C', root, 'rev-parse', 'HEAD']`
- `gitCandidates`：`['-C', root, 'diff', '--name-status', '-M', 'HEAD']` 與 `['-C', root, 'ls-files', '--others', '--exclude-standard']`

Rust 測試的 fake binary 必須對等 `tests/fixtures/fake-ast-grep.js`：`--version` 印 `ast-grep ${version}`（`FAKE_AG_MODE=version:X`）；`hang` / `streams` / `empty` / `emit:<base64>` / `preflight-bad` / `preflight-ok`。

---

## 3. Schema v2 與 `ensureSchema`

`SCHEMA_VERSION = 2`。`src/schema.sql` 全文（Rust 必須能產生相同表/索引/觸發器/CHECK）：

```sql
CREATE TABLE IF NOT EXISTS _cortex_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
  project_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  git_head TEXT,
  last_indexed_at INTEGER,
  extractor_version TEXT NOT NULL,
  created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS chunks (
  chunk_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  symbol_name TEXT,
  chunk_type TEXT CHECK(chunk_type IN ('function','class','method','config','documentation','unparsed')),
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,
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
  file_content_hash TEXT NOT NULL,
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

-- NOTE: spec §4 requires tokenize='unicode61 "remove_diacritics 1" "tokenchars ._$"' but bundled SQLite 3.49.2 (better-sqlite3 11.10.0) rejects any parameterized unicode61 (parse error in tokenize directive) — only bare 'unicode61' passes. Downgraded to bare; revert to spec string when CI SQLite supports it. See task-2-report.md.

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
  content, symbol_name, file_path,
  content=chunks, content_rowid=rowid,
  tokenize='unicode61'
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

CREATE TABLE IF NOT EXISTS reading_notes (
  reading_id INTEGER PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  start_line INTEGER NOT NULL CHECK(start_line >= 1),
  end_line INTEGER NOT NULL CHECK(end_line >= start_line),
  ends_at_eof INTEGER NOT NULL CHECK(ends_at_eof IN (0, 1)),
  content TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  source_mtime_ms REAL NOT NULL,
  source_size INTEGER NOT NULL,
  read_count INTEGER NOT NULL DEFAULT 1,
  first_read_at INTEGER NOT NULL,
  last_read_at INTEGER NOT NULL,
  UNIQUE(project_id, file_path, start_line, end_line)
);
CREATE INDEX IF NOT EXISTS idx_reading_notes_file
  ON reading_notes(project_id, file_path, start_line, end_line);

CREATE VIRTUAL TABLE IF NOT EXISTS reading_notes_fts USING fts5(
  content, file_path,
  content=reading_notes, content_rowid=reading_id,
  tokenize='unicode61'
);
CREATE TRIGGER IF NOT EXISTS reading_notes_fts_insert AFTER INSERT ON reading_notes BEGIN
  INSERT INTO reading_notes_fts(rowid, content, file_path)
    VALUES (NEW.reading_id, NEW.content, NEW.file_path);
END;
CREATE TRIGGER IF NOT EXISTS reading_notes_fts_delete AFTER DELETE ON reading_notes BEGIN
  INSERT INTO reading_notes_fts(reading_notes_fts, rowid, content, file_path)
    VALUES('delete', OLD.reading_id, OLD.content, OLD.file_path);
END;
CREATE TRIGGER IF NOT EXISTS reading_notes_fts_update AFTER UPDATE ON reading_notes BEGIN
  INSERT INTO reading_notes_fts(reading_notes_fts, rowid, content, file_path)
    VALUES('delete', OLD.reading_id, OLD.content, OLD.file_path);
  INSERT INTO reading_notes_fts(rowid, content, file_path)
    VALUES (NEW.reading_id, NEW.content, NEW.file_path);
END;
```

FTS 外部 content table：chunks 用 `content_rowid=rowid`（SQLite 隱含 rowid）；reading_notes 用 `content_rowid=reading_id`。

**沒有** `unresolved_refs` 表、**沒有** `embedding` 欄。`chunk_type` 含 `config`/`documentation` 但現行 pack **不產生**它們。`rel_type` 含 `exports` 但現行 pack **不產生** `edge:exports`。

### `ensureSchema`（`src/db.js` 35–43 行，逐字）

```js
export function ensureSchema(db) {
  db.exec(SCHEMA_SQL);
  const existing = getMeta(db, 'SCHEMA_VERSION');
  if (existing === null) setMeta(db, 'SCHEMA_VERSION', String(SCHEMA_VERSION));
  else if (existing === '1' && SCHEMA_VERSION === 2) setMeta(db, 'SCHEMA_VERSION', String(SCHEMA_VERSION));
  else if (existing !== String(SCHEMA_VERSION)) {
    throw new CortError('schema_version_mismatch', { found: existing, expected: SCHEMA_VERSION });
  }
}
```

遷移語意：

- 全部是 `CREATE … IF NOT EXISTS`。v1→v2 **沒有 ALTER**。
- `existing === null`：新庫，寫入 `'2'`。
- `existing === '1'`：再跑一次 schema.sql（補 `reading_notes` / `reading_notes_fts` / triggers），然後把 meta 改成 `'2'`。
- 其他不等於 `'2'` 的值 → `schema_version_mismatch`。
- 已是 `'2'`：idempotent，不再寫 meta。

歧義：`db.test.js` 的 v1 fixture **只建** `_cortex_meta` + `SCHEMA_VERSION='1'`，沒有舊 chunks 表。若真實 v1 的 `chunks` 欄位與 v2 不同，`CREATE IF NOT EXISTS` **不會**改欄位。JS 沒有測這條；Rust 應複刻「CREATE IF NOT EXISTS + bump 1→2」，不要發明 ALTER。

---

## 4. JSON 與 lean 輸出形狀

`render(command, FORMAT.JSON, payload)` **永遠**是 `` `${JSON.stringify(payload, null, 2)}\n` ``（2-space indent，結尾一個 newline）。lean 各動詞如下；每個 lean 函式 `return lines.join('\n') + '\n'`（結尾 newline）。測試鎖 `lean.endsWith('\n')`。

未知 command 的 lean 仍走 JSON 合同（`render('status', LEAN, payload)` 等於 JSON）。

### 4.1 JSON 欄位（Rust 測試必須 `deepEqual` 這些鍵）

#### `CortError.toJSON` / CLI 失敗

```json
{ "error": "<code>", "detail": <any> }
```

`unknown_command` 的 detail：`{ command, known }` 其中 `known` 是 §1.2 那 9 個字串。

#### help / `--help` / `-h`（`USAGE`）

```js
{
  usage: 'cort <command> [options]',
  commands: {
    index: 'cort index [root] [--incremental]',
    status: 'cort status [root]',
    projects: 'cort projects',
    delete: 'cort delete [root]',
    struct: "cort struct -p '<pattern>' --lang <lang> [-g <glob>] [--budget <n>] [-f json|lean]",
    context: 'cort context <symbol|query> [--budget <n>] [--include-ambiguous] [--content full] [-f json|lean]',
    impact: 'cort impact --symbol <name> [--depth <n>] [-f json|lean]',
    read: 'cort read <file> [--start <line>] [--end <line>] [-f json|lean]',
    recall: 'cort recall <query> [--limit <n>] [--content full] [-f json|lean]',
  },
  env: {
    CORT_CACHE_DIR: 'where indexes live (default ~/.cache/cortex-ng)',
  },
  note: 'Commands read the project at the cwd unless they take a root argument.',
}
```

`Object.keys(usage.commands)` 必須等於 `KNOWN_COMMANDS`（排序後 deepEqual）。

#### `fullIndex`（`cort index`）

```js
{ files, chunks, unparsed, relationships, elapsed_ms }
```

全是 number。`elapsed_ms` 是 wall clock。

#### `incrementalIndex`

full fallback：

```js
{ mode: 'full', files, chunks, unparsed, relationships, elapsed_ms }
```

incremental：

```js
{
  mode: 'incremental',
  files_examined,   // changed.length + deleted.length
  files_reindexed,
  files_skipped,
  files_removed,
  relationships,    // 只計實際寫入的新邊
  elapsed_ms,
}
```

#### `statusOf` / `cort status`

未 index：

```js
{ project_id, path, indexed: false }
```

已 index（`statusOf`）：

```js
{
  project_id, path, indexed: true,
  files, chunks, readings, relationships,
  extractor_version, git_head, last_indexed_at,
}
```

CLI 在已 index 時 merge `computeStale`，所以 stdout 還有：

```js
{ index_is_stale, deleted_files, changed_files }
```

`statusOf` 本身**沒有** stale 欄。`git_head` 在非 git fixture 為 `null`。

#### `listProjects` 的一列

```js
{ project_id, name, path, git_head, last_indexed_at, db_path }
```

（`SELECT` 沒要 `extractor_version` / `created_at`。）空 cache → `[]`。

#### `deleteProject`

```js
{ deleted: true|false, db_path }
```

#### `structCommand`

```js
{
  pattern,              // 輸入原樣
  lang,
  matches: [ {
    file_path, start_line, end_line, text,
    chunk_id,           // null 若 containmentJoin 失敗
    symbol_name,        // null 同上
    chunk_type,         // null 同上
    neighbors: [ {      // getNeighbors 形狀，最多 3，且僅 EXTRACTED|INFERRED
      chunk_id, symbol_name, file_path, start_line, end_line,
      rel_type, confidence, confidence_score, direction
    } ],
  } ],
  match_count,          // budget 前
  malformed_lines,
  truncated,            // boolean
  index_is_stale,       // boolean
}
```

`runPattern` 內部 match 另有 `file`（ast-grep 原路徑）與 `replacement`；**不**出現在 `structCommand` 輸出。

#### `contextCommand`

```js
{
  query,
  resolution,           // 'exact_symbol' | 'fts' | 'none'
  seeds: [ {
    chunk_id, file_path, symbol_name, chunk_type,
    start_line, end_line,
    content,            // 可能被截成 12 行 + '\n…'
    content_truncated,  // boolean
    neighbors: [ { chunk_id, symbol_name, file_path, start_line, end_line,
                   rel_type, confidence, confidence_score, direction } ],
    unresolved: [ { symbol, rel_type, confidence, confidence_score, confidence_reasoning } ],
  } ],
  seed_count,           // budget 前的 seed 數
  truncated,
  truncated_query,      // FTS term 截斷；exact_symbol 路徑為 false
  index_is_stale,
}
```

空查詢：`seeds: []`, `resolution: 'none'`，**不 throw**。

#### `impactCommand`

```js
{
  symbol,               // 輸入原樣，可含逗號
  depth,
  seed_count,
  seeds: [ { chunk_id, file_path, start_line } ],  // 沒有 symbol_name / end_line
  dependents: [ { chunk_id, symbol_name, file_path, start_line, end_line, hop } ],
  dependent_count,
  unresolved: [ { symbol, rel_type, confidence, confidence_score, confidence_reasoning } ],
  index_is_stale,
}
```

未知 symbol：`seed_count: 0`, `dependents: []`, `unresolved: []`，不 throw。無 dependents 的已知 symbol：`dependents: []` 但 `seed_count: 1`。

#### `readFragment`

```js
{
  file_path,    // 專案內 posix 相對路徑
  start_line,
  end_line,
  content,
  source,       // 'store' | 'filesystem'
  read_count,
}
```

沒有 `content_truncated`。

#### `recallReadings`

```js
{
  query,                // 原始 query，不是 FTS 字串
  readings: [ {
    file_path, start_line, end_line,
    content,            // 可能 12 行 + '\n…'
    content_truncated,
    read_count,
    last_read_at,
  } ],
  reading_count,
  truncated_query,
}
```

沒有 `index_is_stale`。

### 4.2 lean 形狀（`src/render.js` 逐字）

內部 `ref(row) = \`${row.file_path}:${row.start_line}\``。

#### impact — `renderImpact`

```
# impact ${payload.symbol} depth=${payload.depth} seeds=${seeds.length} dependents=${payload.dependent_count} stale=${payload.index_is_stale}
seed\t${s.file_path}:${s.start_line}
h${d.hop}\t${d.file_path}\t${d.symbol_name ?? '?'}\t${d.start_line}
unresolved\t${u.symbol}\t${u.rel_type}\t${u.confidence}
```

- header 用 `payload.seeds ?? []` 的 **length**，不是 `seed_count`
- 每個 seed 一行 `seed\t…`
- 每個 dependent：`h` + hop + tab + path + tab + symbol-or-`?` + tab + start_line。**不輸出 chunk_id**（測試：lean 不得出現 64-char project hash）
- 每個 unresolved：`unresolved\t${u.symbol}\t${u.rel_type}\t${u.confidence}`（**沒有** score / reasoning）

測試字面（CHAIN fixture、symbol `d`、depth 3）：

```
# impact d depth=3 seeds=1 dependents=3 stale=false
h1	src/c.ts	c	2
h3	src/a.ts	a	2
```

#### struct — `renderStruct`

```
# struct ${payload.pattern} lang=${payload.lang} matches=${payload.match_count} shown=${(payload.matches ?? []).length} truncated=${payload.truncated} stale=${payload.index_is_stale}
```

每 match 一行：

```js
[
  `${m.file_path}:${m.start_line}`,
  m.symbol_name ?? '?',
  neighbors,   // `${n.direction[0]}${n.rel_type}:${n.symbol_name ?? n.file_path}` 以逗號接合
  String(m.text ?? '').replaceAll('\n', ' ').slice(0, 120),
].filter(Boolean).join('\t')
```

`direction[0]` 是 `'o'` 或 `'i'`。`.filter(Boolean)`：**空的 neighbors 字串會被丟掉**，後面的 text 會往前移一欄。這是 JS 行為，Rust 必須複刻。

測試字面：`# struct d() lang=ts matches=1 shown=1 truncated=false stale=false`

#### context — `renderContext`

```
# context ${payload.query} resolution=${payload.resolution} seeds=${payload.seed_count} truncated=${payload.truncated} stale=${payload.index_is_stale}
${file_path}:${start_line}\t${symbol_name ?? '?'}\t${chunk_type}
  ${n.direction[0]}${n.rel_type}\t${n.file_path}:${n.start_line}\t${n.symbol_name ?? '?'}\t${n.confidence[0]}
  unresolved\t${u.rel_type}\t${u.symbol}\t${u.confidence}
  {
  ${each content line prefixed with two spaces}
  }
```

`confidence[0]` → `I` / `A` / `E`。content 區塊只在 `typeof s.content === 'string' && s.content.length > 0` 時出現。content 行前綴是兩個空白 + 原行（含空行也加前綴）。

測試：header 以 `# context c resolution=exact_symbol` 開頭；seed 行 `src/c.ts:2	c	function`。

#### read — `renderRead`

```
# read ${payload.file_path}:${payload.start_line}-${payload.end_line} source=${payload.source} reads=${payload.read_count}
${payload.content}

```

測試字面：`# read src/main.rs:10-12 source=store reads=2` 然後本文 `fn work() {`。

#### recall — `renderRecall`

```
# recall ${payload.query} readings=${payload.reading_count} truncated_query=${payload.truncated_query}
${reading.file_path}:${reading.start_line}-${reading.end_line}\treads=${reading.read_count}
${reading.content}   // 只在 truthy 時輸出
```

測試字面：`# recall work readings=1 truncated_query=false` 與 `src/main.rs:10-12	reads=2`。

---

## 5. Chunk 模型、tag、關係、budget / containment

### 5.1 什麼是 chunk

`extractFile` 跑 `ast-grep scan --json=stream --config SGCONFIG <absPath>`，把 stdout 當 NDJSON。

`parseScanStream`：非空行；`JSON.parse` 失敗計 `malformed`，成功推進 `records`。`total = 非空行數`。

每筆 record：

- `tag = rec.message ?? ''`
- 行號 **0-based → 1-based**：`start_line = rec.range.start.line + 1`，`end_line = rec.range.end.line + 1`

`tag.startsWith('chunk:')` → chunk：

```js
{
  chunk_id: `${projectId}:${filePath}:${startLine}`,  // chunkIdFor
  project_id, file_path,
  symbol_name: rec.metaVariables?.single?.NAME?.text ?? null,
  chunk_type: tag.slice('chunk:'.length),   // function | class | method | …
  start_line, end_line,
  content: rec.text,
  content_hash: sha256(rec.text) hex,
  language: rec.language ?? null,           // ast-grep 的 language 字串，如 'TypeScript' / 'Rust'
  chunk_source: 'ast',
}
```

然後：

1. `chunks.sort((a,b) => a.start_line - b.start_line || a.end_line - b.end_line)`
2. 以 `chunk_id` Map 去重，**先到的留下**（同一 start_line 的碰撞是確定性的）

`tag.startsWith('edge:')` → raw edge：

- `rel_type = tag.slice('edge:'.length)`
- `raw_target = unquote(single.SRC?.text ?? single.CALLEE?.text)`；兩者都 null 則丟掉
- `unquote`：若首尾是同一 `'`, `"`, 或 `` ` `` 則剝掉一層

edge 歸屬（innermost containment）：

```js
containing = deduped
  .filter(c => c.start_line <= e.start_line && e.start_line <= c.end_line)
  .sort((a,b) => (a.end_line - a.start_line) - (b.end_line - b.start_line))[0]
source_symbol = containing?.symbol_name ?? null
```

檔級 import 的 `source_symbol` 是 `null`（不在任何函式內）。

### 5.2 `file_content_hash`

```js
edgeString(edge) = `${rel_type}\t${source_symbol ?? ''}\t${raw_target}`
// 例：'calls\tgo\talpha' 與 'imports\t\t./helper'
```

hash：chunks 依 `start_line` 排序後依序 `update(content)`，然後 `edges.map(edgeString).sort()` 依序 `update`。**chunk 陣列順序不影響**；edge 字串或 chunk 本文變了就變。這是 staleness / incremental skip 的唯一判準（不是 git dirty、也不是原始檔 bytes）。

### 5.3 unparsed 降級（單一檔，永不 abort 整個 index）

`unparsedResult`：一個 chunk，`start_line=1`，`end_line = max(1, source.split('\n').length)`，`symbol_name=null`，`chunk_type='unparsed'`，`chunk_source='unparsed'`，`language=null`，`content=source`，`content_hash=sha256(source)`，`edges=[]`，`unparsed=true`。

觸發：

| 條件 | 行為 |
|---|---|
| `execAstGrep` throw `ast_grep_timeout` | 降級 unparsed，`malformed: 0` |
| 其他 throw（含 `ast_grep_spawn_failed`） | **再 throw**，index abort |
| `r.code !== 0` | 降級，`malformed: 0` |
| `records.length === 0`（含全 malformed） | 降級，保留 `malformed` 計數 |
| 部分 malformed、仍有 records | **繼續 index 存活 record**。scan **沒有** 10% abort（那是 `runPattern` 才有） |

### 5.4 關係列

見 §1.13。Indexer 第二輪才跑，因為 `resolveTargets` 需要**全專案** chunks 已插入。`ON CONFLICT DO NOTHING`。零 target 永不落表。self-edge 濾掉。

### 5.5 containment join（struct）

見 `containmentJoin`：最小 span（`end-start ASC`），同分 `start_line DESC`。測試：`src/alpha.ts` 第 4 行在 class `Beta` 與 method `go` 內，必須得到 `go`。

### 5.6 budget join

- struct：`applyBudget` 以 `JSON.stringify(match)` 計價，budget 來自 `--budget` 預設 1500
- context：先對 seed 做 `applyBudget`，再以 `budget * 1.15` 上限砍 neighbors（§1.11）
- impact：**沒有** budget
- 第一筆永遠保留（§1.3）

---

## 6. 行為對照表（176 列）

編號規則：依 Job 分組，組內依檔名再依檔內出現順序。Job D 先排 cort 本體（context/struct/impact/render/cli），再排 `evals/` harness（**不是 rust/cort crate**；D 不必移植 evals，E 驗收 rust parity 時可跳過 D-48..D-88，除非協調者擴範圍）。

摘要：B 29、C1 29、C2 22、C3 8、D 88（其中 cort 本體 47 + eval 41）= **176**。

### B — 地基（errors / ast-grep / db / fts / budget）

| id | file | test name | module |
|---|---|---|---|
| B-1 | ast-grep.test.js | resolves the real ast-grep and it matches the pin | ast-grep |
| B-2 | ast-grep.test.js | missing binary is fail-closed | ast-grep |
| B-3 | ast-grep.test.js | wrong version is fail-closed with found/expected detail | ast-grep |
| B-4 | ast-grep.test.js | a hung subprocess raises ast_grep_timeout | ast-grep |
| B-5 | ast-grep.test.js | execAstGrep returns code, stdout and stderr separately | ast-grep |
| B-6 | budget.test.js | token estimate is four characters per token, rounded up | budget |
| B-7 | budget.test.js | applyBudget keeps items while the cumulative rendered size fits | budget |
| B-8 | budget.test.js | applyBudget reports no truncation when everything fits | budget |
| B-9 | budget.test.js | applyBudget always keeps at least one item so the answer is never empty | budget |
| B-10 | db.test.js | project id is a stable sha256 of the real path | db |
| B-11 | db.test.js | db path lands under the cortex-ng cache keyed by project id | db |
| B-12 | db.test.js | ensureSchema is idempotent and records the schema version | db |
| B-13 | db.test.js | ensureSchema upgrades a v1 database with the reading-notes FTS layer | db |
| B-14 | db.test.js | schema uses the V6 column names required by the spec | db |
| B-15 | db.test.js | relationships primary key is the composite triple | db |
| B-16 | db.test.js | fts triggers mirror chunk writes | db |
| B-17 | db.test.js | zero-target relationships are impossible: target_chunk_id is NOT NULL | db |
| B-18 | db.test.js | listProjects enumerates every indexed project in the cache dir | db |
| B-19 | db.test.js | deleteProject removes only that project db and reports what it did | db |
| B-20 | db.test.js | withBusyRetry retries SQLITE_BUSY and gives up after three retries | db |
| B-21 | db.test.js | withBusyRetry converts a full or corrupt db into storage_full | db |
| B-22 | fts.test.js | each term is quoted so FTS operators cannot leak through | fts |
| B-23 | fts.test.js | more than MAX_OR_TERMS terms truncates and reports it | fts |
| B-24 | fts.test.js | an empty query is rejected loudly | fts |
| B-25 | fts.test.js | keywordSearch finds a symbol by name | fts |
| B-26 | fts.test.js | keywordSearch survives punctuation that would otherwise be FTS syntax | fts |
| B-27 | fts.test.js | unicode61 tokenizing lets CJK identifiers through | fts |
| B-28 | fts.test.js | results are scoped to the project | fts |
| B-29 | fts.test.js | the limit is honoured | fts |

`CortError` 沒有獨立測試檔；B-3 鎖 `toJSON()` 形狀 `{ error, detail }`。

### C1 — pack / chunker / graph

| id | file | test name | module |
|---|---|---|---|
| C1-1 | pack.test.js | pack files are enumerated in sorted order and hash deterministically | pack |
| C1-2 | pack.test.js | extractor_version changes when any pack file changes | pack |
| C1-3 | pack.test.js | the pack extracts chunks and edges from TypeScript with the expected tags | pack |
| C1-4 | pack.test.js | the pack extracts chunks and edges from Python | pack |
| C1-5 | pack.test.js | PACK_DIR points at a real directory containing sgconfig.yml | pack |
| C1-6 | chunker.test.js | malformed lines are skipped and counted, valid ones survive | chunker |
| C1-7 | chunker.test.js | edge strings use the tab-separated pre-resolution form | chunker |
| C1-8 | chunker.test.js | file_content_hash covers both chunk contents and edge strings | chunker |
| C1-9 | chunker.test.js | extractFile produces 1-indexed lines and V6-shaped chunk ids | chunker |
| C1-10 | chunker.test.js | Rust functions and impl methods are symbol-scoped AST chunks | chunker |
| C1-11 | chunker.test.js | edges are attributed to the innermost containing chunk | chunker |
| C1-12 | chunker.test.js | a file ast-grep cannot parse becomes a single unparsed FTS-only chunk | chunker |
| C1-13 | chunker.test.js | an all-malformed scan stream degrades that file to unparsed and never throws | chunker |
| C1-14 | chunker.test.js | a 90%-malformed scan stream still indexes the surviving record — scan never aborts | chunker |
| C1-15 | chunker.test.js | a scan that times out degrades that file to unparsed instead of aborting | chunker |
| C1-16 | chunker.test.js | a spawn failure still propagates — only timeout degrades to unparsed | chunker |
| C1-17 | chunker.test.js | const-bound arrow and function expressions become function chunks | chunker |
| C1-18 | chunker.test.js | collection transforms and bare aliases do not become chunks | chunker |
| C1-19 | chunker.test.js | calls inside a const-bound handler get the handler as their source symbol | chunker |
| C1-20 | graph.test.js | confidence constants match the spec exactly | graph |
| C1-21 | graph.test.js | a single-hit call resolves to one INFERRED row | graph |
| C1-22 | graph.test.js | an ambiguous call writes one row per target with score 0.5/N | graph |
| C1-23 | graph.test.js | a call with no resolvable target writes no row at all | graph |
| C1-24 | graph.test.js | unresolvedInline is the on-the-fly shape and carries no chunk id | graph |
| C1-25 | graph.test.js | a symbol never calls itself | graph |
| C1-26 | graph.test.js | getNeighbors returns depth-1 edges in both directions, capped | graph |
| C1-27 | graph.test.js | getTransitiveDependents walks the reverse edge up to depth | graph |
| C1-28 | graph.test.js | buildImportMap keys only the module specifiers of import edges | graph |
| C1-29 | graph.test.js | resolveTargets prefers files reachable through the import map | graph |

C1-3 鎖 TS tags 恰好 `['chunk:class','chunk:function','chunk:method','edge:calls','edge:calls','edge:imports']`，且 import 的 `$SRC` 仍含引號 `"'./helper'"`（unquote 在 chunker，不在 pack）。

### C2 — indexer / incremental / staleness

| id | file | test name | module |
|---|---|---|---|
| C2-1 | indexer.test.js | walkFiles skips ignored dirs and non-source extensions | indexer |
| C2-2 | indexer.test.js | walkFiles includes Rust sources and fullIndex stores function fragments | indexer |
| C2-3 | indexer.test.js | a full index writes chunks, fts rows, file_state and meta | indexer |
| C2-4 | indexer.test.js | re-indexing is idempotent — no duplicate chunks, no orphan fts rows | indexer |
| C2-5 | indexer.test.js | an unparsable file is indexed as unparsed without failing the run | indexer |
| C2-6 | indexer.test.js | the whole index is one transaction — a mid-run failure leaves the db untouched | indexer |
| C2-7 | indexer.test.js | statusOf reports the indexed project without touching ast-grep | indexer |
| C2-8 | incremental.test.js | an extractor_version mismatch forces a full rebuild | incremental |
| C2-9 | incremental.test.js | no changes means nothing is reindexed | incremental |
| C2-10 | incremental.test.js | an edited file is reindexed and its chunks replaced | incremental |
| C2-11 | incremental.test.js | a touched-but-identical file is skipped without a write | incremental |
| C2-12 | incremental.test.js | a new untracked file is picked up via git ls-files --others | incremental |
| C2-13 | incremental.test.js | a deleted file drops its chunks, fts rows and file_state | incremental |
| C2-14 | incremental.test.js | an interrupt keeps already-committed files and does NOT advance git_head | incremental |
| C2-15 | incremental.test.js | a non-git directory degrades to a full index | incremental |
| C2-16 | incremental.test.js | removeFile and reindexOneFile each run in their own transaction | incremental |
| C2-17 | staleness.test.js | a freshly indexed clean tree is not stale | staleness |
| C2-18 | staleness.test.js | a dirty-but-semantically-identical file is NOT stale | staleness |
| C2-19 | staleness.test.js | a changed chunk body makes the index stale | staleness |
| C2-20 | staleness.test.js | an edge-only change makes the index stale | staleness |
| C2-21 | staleness.test.js | a deleted file makes the index stale and is reported | staleness |
| C2-22 | staleness.test.js | staleness is computed against projects.path, not the cwd | staleness |

### C3 — readings

| id | file | test name | module |
|---|---|---|---|
| C3-1 | readings.test.js | reading notes require an indexed project | readings |
| C3-2 | readings.test.js | a first fragment read is persisted and an unchanged repeat comes from the store | readings |
| C3-3 | readings.test.js | a stored whole-file reading serves later subranges without another filesystem payload | readings |
| C3-4 | readings.test.js | a partial note never masquerades as a whole-file cache entry | readings |
| C3-5 | readings.test.js | an omitted end line caches the requested start through EOF | readings |
| C3-6 | readings.test.js | unchanged reading notes survive a full re-index | readings |
| C3-7 | readings.test.js | FTS recall returns stored readings and drops them after the source changes | readings |
| C3-8 | readings.test.js | reading rejects paths outside the indexed project and invalid ranges | readings |

### D — context / struct / impact / render / CLI（+ eval harness 列在末）

| id | file | test name | module |
|---|---|---|---|
| D-1 | context.test.js | the default budget is 1500 tokens | context |
| D-2 | context.test.js | an exact symbol name resolves without touching FTS | context |
| D-3 | context.test.js | a non-symbol query falls back to FTS | context |
| D-4 | context.test.js | seeds carry depth-1 neighbours | context |
| D-5 | context.test.js | AMBIGUOUS neighbours are dropped unless explicitly requested | context |
| D-6 | context.test.js | an unresolvable reference is inlined on the fly and never persisted | context |
| D-7 | context.test.js | the emitted JSON actually fits the budget and reports truncation | context |
| D-8 | context.test.js | an unknown query returns an empty packet rather than throwing | context |
| D-9 | context.test.js | context never invokes struct | context |
| D-10 | context.test.js | seed content is truncated by default and restorable with fullContent | context |
| D-11 | context.test.js | short content is untouched and not flagged | context |
| D-12 | context.test.js | a Rust symbol returns only its function body, not the rest of a large file | context |
| D-13 | struct.test.js | constants match the spec | struct |
| D-14 | struct.test.js | a malformed pattern is caught by the pre-flight, not by the exit code | struct |
| D-15 | struct.test.js | a valid pattern passes the pre-flight | struct |
| D-16 | struct.test.js | zero matches is a clean empty result, never parse_failed | struct |
| D-17 | struct.test.js | matches are returned with 1-indexed lines | struct |
| D-18 | struct.test.js | a few malformed JSON lines are skipped and counted | struct |
| D-19 | struct.test.js | more than 10% malformed aborts THIS query only | struct |
| D-20 | struct.test.js | containmentJoin picks the smallest chunk that contains the match | struct |
| D-21 | struct.test.js | containmentJoin returns null when no chunk contains the match | struct |
| D-22 | struct.test.js | structCommand attaches at most MAX_NEIGHBORS neighbours and reports staleness | struct |
| D-23 | struct.test.js | structCommand surfaces parse_failed as a structured error and runs nothing | struct |
| D-24 | struct.test.js | an unglobbed scan of a large project is refused with actionable advice | struct |
| D-25 | struct.test.js | the same scan succeeds once a glob narrows it | struct |
| D-26 | impact.test.js | the default depth is 3 | impact |
| D-27 | impact.test.js | dependents are returned with their hop distance | impact |
| D-28 | impact.test.js | depth is respected | impact |
| D-29 | impact.test.js | a symbol with no dependents returns an empty list, not an error | impact |
| D-30 | impact.test.js | an unknown symbol reports zero seeds without throwing | impact |
| D-31 | impact.test.js | an ambiguous symbol seeds from every matching chunk | impact |
| D-32 | impact.test.js | unresolved references are inlined on the fly and nothing is persisted | impact |
| D-33 | impact.test.js | the packet reports index staleness | impact |
| D-34 | impact.test.js | symbol accepts a comma-separated batch and merges dependents at min hop | impact |
| D-35 | render.test.js | parseFormat accepts json and lean case-insensitively and rejects anything else | render |
| D-36 | render.test.js | lean impact output lists every dependent with its hop and drops the stored chunk_id | render |
| D-37 | render.test.js | lean is smaller than json for the same payload on all three verbs | render |
| D-38 | render.test.js | lean context keeps neighbours and unresolved refs one per line | render |
| D-39 | render.test.js | lean struct emits one row per match with the enclosing symbol | render |
| D-40 | render.test.js | unknown commands and json format fall through to the JSON contract | render |
| D-41 | render.test.js | lean reading output identifies cache provenance and keeps stored content | render |
| D-42 | cli.test.js | asking a command for help explains it instead of running it | cli |
| D-43 | cli.test.js | every spelling of help reaches the same usage, and none of them is an error | cli |
| D-44 | cli.test.js | usage documents every command the dispatcher actually knows | cli |
| D-45 | cli.test.js | an unknown command is still a failure, not usage | cli |
| D-46 | cli.test.js | index without --help still indexes, so the guard did not swallow the command | cli |
| D-47 | cli.test.js | read persists a fragment and recall finds it through FTS | cli |
| D-48 | eval-harness.test.js | the three arms are exactly the ones the spec names | evals |
| D-49 | eval-harness.test.js | the metric set includes what the V6 eval plan was missing | evals |
| D-50 | eval-harness.test.js | every task declares a verifiable expected answer | evals |
| D-51 | eval-harness.test.js | summarize computes per-arm aggregates and the stop/go verdict | evals |
| D-52 | eval-harness.test.js | summarize returns a stop verdict when cort loses on tokens | evals |
| D-53 | eval-harness.test.js | every graph task labels each symbol exactly once, at one distance | evals |
| D-54 | grade.test.js | the answer contract is one text, so both arms are asked for the same shape | evals |
| D-55 | grade.test.js | a complete answer scores one on both axes | evals |
| D-56 | grade.test.js | a missing symbol costs coverage but not precision | evals |
| D-57 | grade.test.js | an invented symbol costs precision but not coverage | evals |
| D-58 | grade.test.js | naming a symbol twice neither helps coverage nor hurts precision | evals |
| D-59 | grade.test.js | the wrong distance is recorded without being confused with a wrong symbol | evals |
| D-60 | grade.test.js | an answer with no block at all is a failed cell, not a null metric | evals |
| D-61 | grade.test.js | only the last block counts, so a quoted example cannot pad the answer | evals |
| D-62 | grade.test.js | spacing and stray bullets in the block do not change the answer | evals |
| D-63 | grade.test.js | a line without a distance still names a symbol, and is marked as unplaced | evals |
| D-64 | grade.test.js | the gate is the one the plan fixed in advance, not one tuned to a result | evals |
| D-65 | grade.test.js | a cell that hit the turn cap can still be graded on what it answered | evals |
| D-66 | agent-stream.test.js | an ASCII payload costs about a token every four characters | evals |
| D-67 | agent-stream.test.js | a CJK character costs a whole token, so cct comments are not under-counted | evals |
| D-68 | agent-stream.test.js | the empty payload is zero tokens, not a fraction of one | evals |
| D-69 | agent-stream.test.js | tool results are measured, which is the metric three rounds recorded as null | evals |
| D-70 | agent-stream.test.js | Read calls are counted apart from the other tools | evals |
| D-71 | agent-stream.test.js | every cort invocation is kept so the arm can be proved to have used its own tool | evals |
| D-72 | agent-stream.test.js | usage is summed the way the earlier rounds summed it | evals |
| D-73 | agent-stream.test.js | a denied tool call is surfaced, because a leaking whitelist invalidates the cell | evals |
| D-74 | agent-stream.test.js | a stream that never produced a result throws instead of reporting nulls | evals |
| D-75 | agent-stream.test.js | a result without usage throws rather than writing a null metric | evals |
| D-76 | agent-stream.test.js | blank lines in the stream are skipped, not treated as corruption | evals |
| D-77 | agent-stream.test.js | the turn cap is reported as a fact about the cell, not as failure | evals |
| D-78 | agent-stream.test.js | tool_result content arrives as blocks as well as a bare string | evals |
| D-79 | run-agents.test.js | the two arms are the experiment: identical prompt, different tools | evals |
| D-80 | run-agents.test.js | neither arm is handed the tool that defines the other arm | evals |
| D-81 | run-agents.test.js | the command the prompt tells the cort arm to run is one the whitelist accepts | evals |
| D-82 | run-agents.test.js | the cell runs in the venue, because projectId is derived from the cwd | evals |
| D-83 | run-agents.test.js | the transcript flags the earlier rounds lacked are all requested | evals |
| D-84 | run-agents.test.js | every tool the arm may use is passed, and nothing else | evals |
| D-85 | run-agents.test.js | the environment is the isolated one, not the user configuration | evals |
| D-86 | run-agents.test.js | a row carries every metric the gate reads, none of them null | evals |
| D-87 | run-agents.test.js | a row refuses to be built from a transcript missing a metric | evals |
| D-88 | run-agents.test.js | a denied tool call is carried onto the row, where a reader will see it | evals |

D-48..D-88 測的是 `evals/*.mjs`（JS eval harness），**不在** rust crate 範圍。列出來只為 176 條編號完整，讓 E 能寫「skip / out of crate」。

---

## 7. 天真 port 會漏的邊角

### 7.1 Busy retry

兩層：

1. `openDb`：`PRAGMA busy_timeout = 5000`（SQLite 內部等 5s）
2. `withBusyRetry`：外層 4 次（1 次 + 3 retry）。只包 CLI 的 `index` / `read` / `recall`。`struct`/`context`/`impact`/`status` **沒包**。

`SQLITE_FULL` 與 `SQLITE_CORRUPT` 都映射 `storage_full`（測試只打了 `SQLITE_FULL`）。其他錯誤不重試。

### 7.2 Unparsed 降級（index 路徑）

見 §5.3。重點對照：

- **scan/index**：malformed 比例再高只要還有一筆 JSON，就 index 那筆（C1-14，95% malformed 仍 `unparsed=false`）
- **struct `runPattern`**：`malformed/total > 0.10` abort **這次 query**（D-19），error `run_aborted_malformed`
- 語法壞掉的檔（`function (((`）→ 單一無符號 unparsed chunk，run 繼續（C2-5, C1-12）
- `ast-grep` 非 0 且不是 timeout → 該檔 unparsed，不是 throw

### 7.3 Timeout 降級

只有 `err.code === 'ast_grep_timeout'` 在 `extractFile` 被吃掉。`ETIMEDOUT` 與 `SIGTERM` 都算 timeout。`timeoutMs` 可覆寫（測試用 150–200ms）。環境級 spawn 失敗必須響（C1-16）。

`execAstGrep` 的 `--version` probe **沒設** 30s timeout。

### 7.4 Glob 拒絕（scan_too_broad）

`globs.length === 0` **且** `file_state` 列數 `> fileLimit`（預設 2000，測試把 `fileLimit` 調成 10）。hint 必須含 `-g`。有 glob 就跳過這關，即使專案很大。CLI 只把單一 `-g` 字串放進陣列。計數來源是 **indexed `file_state`**，不是 `walkFiles`。

### 7.5 Stale reads

- `computeStale` 比的是 **extraction hash**，不是 git dirty、也不是檔案 bytes。trailing comment 若沒進任何 chunk/edge，**不是 stale**（C2-18）
- 只改 call target（chunk 本文長度類似但 edge 變）**是 stale**（C2-20）
- 基準目錄是 `projects.path`，不是 cwd（C2-22）
- git 可用時，candidates = git dirty ∪（磁碟上但 `file_state` 沒有的新檔）。**已 commit 但尚未 reindex 的變更，若 working tree 相對 HEAD 是乾淨的，JS 不會把它當 stale。** 這是現有行為，不是漏寫；Rust 應複刻，不要改成「掃全部檔」
- git 不可用：掃全部 `walkFiles`
- `context`/`struct`/`impact`/`status`（已 index）都把 `index_is_stale` 放進 JSON。`read`/`recall` **沒有**這個欄位
- readings 的「stale」是另一套：mtime/size + content hash，失敗就 DELETE 該檔所有 notes（C3-7）

### 7.6 ON CONFLICT 路徑

| SQL | 衝突鍵 | 動作 |
|---|---|---|
| `_cortex_meta` | `key` | `DO UPDATE SET value = excluded.value` |
| `projects` | `project_id` | `DO UPDATE SET name, path, git_head, last_indexed_at, extractor_version` |
| `file_state` | `(project_id, file_path)` | `DO UPDATE SET file_content_hash, updated_at = datetime('now')` |
| `relationships` | `(source_chunk_id, target_chunk_id, rel_type)` | **`DO NOTHING`** |
| `reading_notes` | `(project_id, file_path, start_line, end_line)` | `DO UPDATE` content/hash/mtime/size/`ends_at_eof`，且 `read_count = reading_notes.read_count + 1` |

`chunks.chunk_id` 是 PRIMARY KEY；fullIndex 先 DELETE 再 INSERT，沒有 OR REPLACE。重複 insert 會丟 SQLITE 錯並 rollback 整個 fullIndex transaction（C2-6）。

### 7.7 FTS sanitization

見 §1.5。每一個空白切開的 token 都用雙引號包住；內嵌 `"` 變成 `""`。`AND`/`OR`/`-`/`*` 因此不會當運算子。超過 20 個 term 截斷並設 `truncated_query`。空字串（或純空白）→ `empty_query`。CJK 靠 `tokenize='unicode61'`（**bare**，不是註解裡那條 parameterized 字串）。`keywordSearch` 必須再 filter `c.project_id`。

reading_notes FTS 用同一套 `sanitizeFtsQuery`。

FTS5 外部 content 的 DELETE trigger 必須是 `INSERT INTO chunks_fts(chunks_fts, rowid, …) VALUES('delete', …)` 這種特殊語法；天真 `DELETE FROM chunks_fts` 不對。

### 7.8 Path traversal guards

`readFragment` → `resolveProjectFile`：

- `realpathSync`（解析 symlink；不存在 → `file_not_found`）
- `path.relative` 不得是 `''`（root 自己）、`'..'`、`..${sep}…`、或絕對路徑 → `path_outside_project`
- 必須是檔

C3-8 對 `'../outside'` 接受 `file_not_found` **或** `path_outside_project`（視該路徑是否存在）。`start > end` → `invalid_line_range`。

`walkFiles` 跳過 symlink，因此 index 不會跟著 link 走出專案。struct 的 glob 則是直接交給 ast-grep；JS **沒有**再驗證 glob 是否在 root 內。

`openProject` 用 `realpathSync`，所以 project id 是真實路徑的 sha256。

### 7.9 其他 fail-closed / 降級

- 版本 pin 全等 `'0.45.2'`；stdout 抽不到 x.y.z → `ast_grep_version_unreadable`
- missing binary：把 `CORT_AST_GREP_BIN` 指到不存在路徑，或 PATH 空
- preflight：exit 2 **或** stderr 含 `Pattern contains an ERROR node`（即使 exit 0）
- extractor_version mismatch → incremental **強制 full**，並寫 stderr 一行
- 非 git 目錄 → incremental full
- fullIndex 單 transaction：中途 boom 必須回到舊 index（C2-6）。extraction 在 transaction 外，所以失敗發生在寫入階段才 rollback
- incremental 每檔一 transaction：中斷保留已寫入的檔，但 **不** 更新 `git_head`（C2-14）
- `db chmod 0o600`；WAL + shm 隨 `deleteProject` 刪
- `listProjects` 對壞掉的 `.db` 吞例外
- context 的 `packetTokens` 硬編碼 `truncated: true`、`index_is_stale: false`（§1.11）— 不要「修正」成對最終 payload 估，否則 D-7 的 1.15 容差會漂
- ast-grep `function_declaration` 的 `rec.text` **不含** `export` 關鍵字（D-10 註解）。Rust port 不得自己把 `export` 補回去
- `applyBudget` 第一筆永遠保留
- struct neighbor 的空字串被 `.filter(Boolean)` 丟掉（§4.2）
- `parseArgs` 把 `--include-ambiguous` 存成旗標名含 dash 的 key
- `--content full` 是字串相等，不是 boolean；`--content` 單獨出現會變成 `true`，`fullContent` 仍為 false
- readings `source_mtime_ms` 是 REAL，比對用 `===` 對 `stat.mtimeMs`（浮點）。Rust 若改存整數可能誤判 stale
- `recall` FTS 取 `limit*4` 再過濾，因為中間可能丢掉 stale 檔
- pack 的 `extractorVersion` hash **含** `sgconfig.yml` 的 bytes
- Job 依賴白名單：不引入 YAML parser

### 7.10 測試 fixture 契約

- `tests/helpers/tmp-project.js`：`makeProject(files)` 在 `os.tmpdir()/cort-proj-*` 寫檔並 `realpathSync`。`SAMPLE` 含 `src/helper.ts`、`src/alpha.ts`（import+class+method）、`node_modules/pkg/index.ts`（必須被 ignore）、`README.md`（非 source ext）
- `tests/fixtures/fake-ast-grep.js`：見 §2.3。Rust 測試需要同等物，且靠環境變數切 mode（JS 用 `FAKE_AG_MODE`）

---

## 附錄：模組 → Job 對照（實作時）

| Job | 寫入範圍 | 對照列 |
|---|---|---|
| B | errors, ast-grep bridge, db+schema v2+遷移, fts, budget | B-1..B-29 |
| C1 | pack 枚舉+hash（不 parse YAML）, chunker, graph | C1-1..C1-29 |
| C2 | indexer full, incremental, staleness, walk/ignore | C2-1..C2-22 |
| C3 | readings read/recall, hash/mtime 失效, FTS 淘汰 | C3-1..C3-8 |
| D | context, struct, impact, render json/lean, CLI clap（`--help` 無副作用） | D-1..D-47（D-48..D-88 eval-only） |
