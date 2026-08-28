# cort Rust 移植：四項效度修正提案（2026-08-28）

本文件把 `2026-08-28-rust-port.md` §5 的四個發現轉成 C3／D 可直接照做的契約。以下模組名與函式名是**語言中立的責任名稱**；Rust 實作者可依 crate 佈局命名，但不得改變 CLI、輸出、驗證順序或淘汰條件。

## 共同決策

- 成功輸出的預設格式仍是 `json`；`-f lean`／`--format lean` 選 lean，`-f json`／`--format json` 選 JSON。格式值大小寫不敏感，其他值回 `unknown_format`。
- 所有 hash 都是檔案原始 bytes 的 SHA-256，小寫十六進位。receipt 的 `content_hash_prefix` 則是「該次要求之精確行範圍內容」UTF-8 bytes 的 SHA-256 前 12 個 hex；它不是安全認證，只是短識別碼。
- stale content 必須 fail-closed：沒有完成本次 hash 驗證，就不能輸出 SQLite 裡的內容，也不能把它算成有效 recall 結果。
- #1～#3 不改 DB schema，也不改 `extractor_version`。#4 不改 DB schema，但一定改 `extractor_version`，因此舊索引必須全量重建。

## 1. Cache hit 改回短 receipt

### CLI 契約

```text
cort read <file> [--start <line>] [--end <line>]
          [--content <auto|receipt|full>] [-f <json|lean>]
```

| flag | 預設 | 語意 |
|---|---:|---|
| `--start <line>` | `1` | 1-based，必須為正整數。 |
| `--end <line>` | EOF | 1-based、含尾端；必須 `>= start`。 |
| `--content auto` | **預設** | `source=filesystem` 回全文；經驗證的 `source=store` 只回 receipt。 |
| `--content receipt` | 非預設 | 無論 miss/hit 都持久化正常內容，但 stdout 不回 body，只回 receipt。 |
| `--content full` | 非預設 | miss/hit 都回 body；它不繞過 hash 驗證。 |
| `-f`／`--format` | `json` | `json` 或 `lean`。 |

`--content` 有出現但沒有值、或值不在三個列舉中，回 `invalid_content_mode`，detail 為 `{provided, allowed:["auto","receipt","full"]}`。既有自動化若要求舊行為，只需加 `--content full`：cache hit 仍標為 `source:"store"`、`read_count` 仍只增加一次，且 body 與初次讀取逐 byte 相同。沒有 flag 的第一次 read 仍可讀到內容；唯一刻意的相容性變更是第二次及後續 verified hit 預設不再重送內容。

### JSON 輸出欄位

receipt 物件逐欄位為：

1. `file_path`
2. `start_line`
3. `end_line`
4. `source`：`"filesystem"` 或 `"store"`
5. `read_count`
6. `content_mode`：`"receipt"`
7. `content_hash_prefix`：固定 12 hex

receipt **不得**帶 `content`（不是 `null`，而是欄位不存在）。full 物件依同一順序／名稱帶：

1. `file_path`
2. `start_line`
3. `end_line`
4. `source`
5. `read_count`
6. `content_mode`：`"full"`
7. `content_hash_prefix`
8. `content`

範例 receipt：

```json
{
  "file_path": "src/main.rs",
  "start_line": 10,
  "end_line": 12,
  "source": "store",
  "read_count": 2,
  "content_mode": "receipt",
  "content_hash_prefix": "82d25b9f72a6"
}
```

### lean 輸出欄位

receipt 恰為一行加換行；欄位依序是 `file range`、`source`、`reads`、`content`、`hash`：

```text
# read src/main.rs:10-12 source=store reads=2 content=receipt hash=82d25b9f72a6
```

full 的 header 使用相同欄位，但 `content=full`，下一行開始才是 body，最後保證一個換行：

```text
# read src/main.rs:10-12 source=store reads=2 content=full hash=82d25b9f72a6
fn work() {
}
```

### 需要動的責任模組／函式

- CLI：`ReadArgs`／`parse_read_args` 增加三值 `content_mode`，並把 resolved mode 傳給 readings。
- readings：`read_fragment` 在完成來源驗證與 `read_count` transaction 後，決定 effective mode；新增 `fragment_hash_prefix`，回傳 tagged payload（receipt/full），不要讓 renderer 自行猜 `source`。
- render：`render_read_json`／`render_read_lean` 依 payload tag 輸出；receipt 分支在型別上就沒有 `content`。
- error：新增 `invalid_content_mode` 的既有 `{error,detail}` envelope。

### schema／extractor_version

- DB schema：不變；`reading_notes.content` 與 `source_hash` 已足夠。不可為 receipt 新增欄位。
- `extractor_version`：不變；這是 readings／render 契約，不是 AST 抽取語意。
- CLI JSON 是刻意的輸出契約變更，但不是 SQLite `SCHEMA_VERSION` 變更。

### 先紅後綠的 TDD 順序

1. render 單元測試：store receipt 的 JSON 沒有 `content`，lean 只有一行且有五個命名欄位。
2. readings 測試：第一次 `auto` 是 filesystem/full；第二次 `auto` 是 store/receipt，`read_count=2`。
3. readings 測試：第二次 `--content full` 是 store/full，body 與第一次完全相同。
4. readings 測試：`--content receipt` 在第一次 miss 仍寫入 DB、回 filesystem/receipt；下一次 full 可取回 body。
5. CLI 測試：三個合法值、預設 `auto`、missing/unknown value 的結構化錯誤，json/lean 各一組。
6. 回歸測試：whole-file note 服務 subrange 時，hash prefix 必須針對 subrange，不得沿用 whole-file fragment prefix。

## 2. 任何 store 回傳前都做 SHA-256 驗證

### CLI 契約與輸出

不新增 flag。`read` 的 flags、預設值及成功時 JSON／lean 欄位完全依 #1；`recall` 依 #3。使用者不能用任何 flag 關閉驗證，`--content full` 也只是輸出選擇。

### 驗證插入點

`read_fragment` 的順序必須固定為：

1. 正規化並限制 path 在 project root 內。
2. 查詢 covering note，但此時 note 只能是 candidate。
3. 對實檔做 pre-read metadata，開啟並完整讀取 bytes，計算 SHA-256，再做 post-read metadata。
4. 若讀取期間 identity／size／mtime 改變，最多立即重試一次完整步驟；第二次仍變動則回 `validation_error`，保留 note。
5. 只有本次 SHA-256 等於 note 的 `source_hash`，才可增加 `read_count`、從 stored fragment 切 requested range，然後回 `source=store`。
6. hash 不符時，在同一 DB transaction 淘汰該 project/file 的所有 notes；直接使用剛才已驗證完成的 bytes 產生 requested fragment 並重新寫 note，回 `source=filesystem`，不可再讀一次檔案。

`recall_readings` 必須在 FTS 取 candidates 之後、`trim_content`／budget／`results.push` 之前插入同一驗證。每個相對 `file_path` 每次命令只開檔及 hash 一次，以 map 共用結果；驗證未通過的 row 不得進結果。

### mtime／size 的新角色

- `mtime`、`size` 只保留為 observability 與 race detector：記錄「上次成功驗證時看到的 metadata」、比較 pre/post-read 是否在讀取期間改變，並在 hash 相符後更新 DB。
- metadata 與 DB 不同，表示**一定要重驗**；metadata 相同，只表示「沒有便宜的變更訊號」，仍然必須 hash，絕不代表內容相同。
- 不再存在 `stat_matches -> return stored content` 的 fast path。正確性唯一依據是本次完整 bytes 的 SHA-256。

### 需要動的責任模組／函式

- readings：刪除／封死 `stat_matches` 的 cache-hit 決策用途；新增 `validate_source(file, expected_hash)`，由 `read_fragment` 和 `recall_readings` 共用。
- filesystem adapter：提供 open/read/pre-post metadata 與 stable file identity，讓 race retry 與錯誤分類可測。
- DB：新增一個「hash match 後更新 metadata + read_count」transaction，以及「hash mismatch 後按 project/file 淘汰」transaction；不得在驗證前更新計數。
- render：無額外工作；只接收已驗證的 payload。

### schema／extractor_version

- DB schema：不變，繼續使用 `source_hash`、`source_mtime_ms`、`source_size`。`source_hash` 的定義釘死為整檔 raw bytes SHA-256。
- `extractor_version`：不變，readings cache 不屬 AST pack。

### 先紅後綠的 TDD 順序

1. 核心回歸：第一次 cache 後做**等長修改並恢復原 mtime**；第二次不得回舊 body 或 receipt，必須回 filesystem/new body。
2. 只改內容、size/mtime 都相同的 recall：舊 note 被淘汰且 `reading_count=0`。
3. metadata 完全相同且 hash 相同：仍呼叫一次 hasher，然後才准 store hit（用計數 filesystem seam 證明）。
4. whole-file note 服務 subrange：先驗整檔 source hash，再切 stored fragment；不能只 hash subrange。
5. pre/post metadata 在讀取期間改變：第一次 retry；連續兩次改變則保留 note 並回 #3 的 `validation_error`。
6. hash mismatch 後重建：只使用那次已讀 bytes，沒有第二次 filesystem read，且新 note 的 metadata/hash/content 是同一 snapshot。

## 3. recall 只在兩種確證下淘汰

### CLI 契約

```text
cort recall <query> [--limit <n>] [--content full] [-f <json|lean>]
```

| flag | 預設 | 語意 |
|---|---:|---|
| `--limit <n>` | `5` | 正整數 `1..=100`。 |
| `--content full` | 關閉 | 關閉時每筆最多 12 行並以 `content_truncated` 標示；開啟時回完整 stored fragment。 |
| `-f`／`--format` | `json` | `json` 或 `lean`。 |

不新增 `--ignore-validation-errors` 或 `--prune`。只要任一候選檔無法確證，整次 recall fail-closed、exit code `1`、不輸出部分 readings；note 保留。如此呼叫端不會把「暫時讀不到」誤判成「沒有記憶」。

### 唯二淘汰條件與 errno 分類

| 驗證結果 | note 動作 | 命令結果 |
|---|---|---|
| 任一 metadata/open/read 階段得到 POSIX `ENOENT`（或平台明確等價的 target-not-found） | 淘汰該 project/file 全部 notes | 繼續處理其他 candidates，不回 validation error |
| 完整 read 成功且 SHA-256 與 `source_hash` 不同 | 淘汰該 project/file 全部 notes | 繼續，不回 stale content |
| `EMFILE`, `ENFILE`, `EIO`, `EINTR`, `EAGAIN`/`EWOULDBLOCK`, `EBUSY`, `ETIMEDOUT`, `ESTALE` | **保留** | `validation_error`；`retryable=true` |
| `EACCES`, `EPERM`, `ELOOP`, `ENAMETOOLONG`, `ENOTDIR`, `EINVAL` | **保留** | `validation_error`；`retryable=false` |
| 非 regular file、讀取期間持續變動、無法取得 raw OS code、或任何未列 errno | **保留** | `validation_error`；只有已知 transient 類別才 `retryable=true`，其餘 `false` |

特別禁止把語言 runtime 的寬泛 `NotFound` 類別直接視為可淘汰；POSIX 上必須確認 raw errno 是 `ENOENT`。`ENOTDIR` 看似也是路徑不存在，但不在獲准的兩種理由內，所以必須保留。

### validation_error 的 JSON／lean 形狀

JSON 沿用全域 error envelope，stdout 只輸出下列欄位，然後 exit `1`：

```json
{
  "error": "validation_error",
  "detail": {
    "command": "recall",
    "file_path": "src/main.rs",
    "operation": "read",
    "errno": "EIO",
    "os_code": 5,
    "retryable": true,
    "note_action": "retained"
  }
}
```

`detail` 逐欄位固定為 `command`、`file_path`（project-relative）、`operation`（`metadata|open|read|source_changed_during_validation|not_regular_file`）、`errno`（symbolic name；無 raw errno 時為 `null`）、`os_code`（整數；無時為 `null`）、`retryable`、`note_action`（此 error 永遠是 `"retained"`）。不得放絕對路徑或完整 OS message。

lean 恰為一行加換行，逐欄位固定如下；null errno/os code 用 `-`：

```text
! validation_error command=recall file=src/main.rs operation=read errno=EIO os_code=5 retryable=true note=retained
```

成功 recall 的 JSON 欄位維持：top-level `query`、`readings`、`reading_count`、`truncated_query`；每筆 reading 為 `file_path`、`start_line`、`end_line`、`content`、`content_truncated`、`read_count`、`last_read_at`。成功 lean 的 header 欄位為 `query`、`readings`、`truncated_query`；每筆先輸出 `file_path:start_line-end_line`、`reads`，下一行才是 body。ENOENT/hash mismatch 淘汰後若無結果，仍是成功的空 recall。

### 需要動的責任模組／函式

- readings：`recall_readings` 使用 #2 的 `validate_source`；新增純函式 `classify_validation_failure(raw_os_error, operation)` 與 `prune_notes_for_file`。
- error：`ValidationErrorDetail` 使用固定欄位，不把 runtime message 直接序列化。
- CLI/render：保存已解析的 format 直到 error handler；新增 `render_validation_error_json`／`render_validation_error_lean`，確保 `-f lean` 的 error 不偷回 JSON。
- DB/FTS：淘汰必須走同一 transaction，依既有 trigger 同步清掉 FTS row；保留路徑零寫入。

### schema／extractor_version

- DB schema：不變；錯誤不持久化，不加 validation table。
- `extractor_version`：不變。

### 先紅後綠的 TDD 順序

1. `classify_validation_failure` table test：逐一釘死上表 errno；尤其 `ENOENT=prune`、`ENOTDIR=retain`、unknown=retain。
2. recall + `ENOENT` integration：刪 note/FTS row、命令成功、結果為空。
3. recall + successful hash mismatch integration：刪 note/FTS row，不回 stale content。
4. recall + `EIO`：note/FTS row 數不變、JSON error 欄位逐欄相等、exit `1`。
5. recall + `EMFILE` 與 `EACCES`：都保留，但 `retryable` 分別為 true/false。
6. recall + `ENOTDIR`、non-regular、unknown OS error：全部保留；null 欄位按契約輸出。
7. 多 candidate：前面已有 valid result、後面遇到 EIO，stdout 仍只回 error，不洩漏部分 readings；所有未確證 notes 保留。
8. `-f lean` 同一錯誤：只有指定的一行，無 JSON、無絕對路徑。

## 4. Rust method 以 owner 消歧

### `rust.yml` 的確切 rule 形狀

自由函式、impl item、trait default method 必須是三個互斥 rule。以下形狀已對 ast-grep `0.45.2` 的 Rust grammar 驗證 `$OWNER`／`$NAME` 可出現在 scan JSON：

```yaml
id: cort-rust-chunk-free-function
language: Rust
severity: hint
message: chunk:function
rule:
  kind: function_item
  all:
    - has: { field: name, pattern: $NAME }
    - not:
        inside:
          stopBy: end
          any:
            - { kind: impl_item }
            - { kind: trait_item }
---
id: cort-rust-chunk-impl-method
language: Rust
severity: hint
message: chunk:method
rule:
  kind: function_item
  all:
    - has: { field: name, pattern: $NAME }
    - inside:
        kind: impl_item
        stopBy: end
        has: { field: type, pattern: $OWNER }
---
id: cort-rust-chunk-trait-default-method
language: Rust
severity: hint
message: chunk:method
rule:
  kind: function_item
  all:
    - has: { field: name, pattern: $NAME }
    - inside:
        kind: trait_item
        stopBy: end
        has: { field: name, pattern: $OWNER }
```

沒有 body 的 trait declaration 是 `function_signature_item`，本修正不把它偽裝成可取全文的 function chunk；有 body 的 trait default method 才由第三條捕捉。`impl Trait for Type` 的 `field:type` 是 `Type`，因此顯示 owner 是實作型別，不是 trait。

### `Type::method` 命名契約

- free function：`symbol_name = NAME`，例如 `main`。
- impl/trait method：`symbol_name = canonical_owner(OWNER) + "::" + NAME`，例如 `Ledger::new`、`Worker::run`。
- `canonical_owner` 對 type path 保留路徑、移除每個 path segment 的 generic arguments，並移除 token 間非必要空白：`crate::ledger::Ledger<T>` → `crate::ledger::Ledger`。非 type-path owner 保留語法、只正規化空白。
- trait default method 的 owner 是 trait 名；trait impl method 的 owner 是 impl target type。若同一 Type 的多個 trait impl 合法地產生相同 `Type::method`，兩個 chunk 都保留，由 `file_path,start_line` 穩定排序，不能任意丟一個。
- `chunk_id` 仍由 project/file/start line 決定；只改 `symbol_name` 與 `chunk_type`，不把 owner 另存新欄。

### context 查詢語法與 flags

```text
cort context <symbol|query> [--budget <n>] [--include-ambiguous]
             [--content full] [-f <json|lean>]
```

| flag | 預設 | 語意 |
|---|---:|---|
| `--budget <n>` | `1500` estimated tokens | 既有 packet budget。 |
| `--include-ambiguous` | `false` | 只控制 ambiguous neighbours，不控制同名 seed 是否存在。 |
| `--content full` | 關閉 | 關閉時 seed body 最多 12 行；開啟時完整 function/method body。 |
| `-f`／`--format` | `json` | `json` 或 `lean`。 |

不新增 public `--limit`；seed hard limit 保持 `5`。qualified symbol 語法是 `<owner>::<member>`，以最後一個不在 generic arguments 內的 `::` 分隔，兩側皆不得為空。CLI 例：

```text
cort context 'Ledger::new' --content full -f lean
cort context 'crate::ledger::Ledger::run' --content full -f json
```

qualified query 先套與 index 相同的 owner canonicalization，再做 `symbol_name` 完全相等查詢；完全相等為零時直接 `resolution=none`，**不得**退化成對 `new`／`run` 的 FTS，否則 owner 消歧會失效。沒有 `::` 的 query 維持既有流程：free symbol exact match，否則 FTS。

### 先消歧、後套 limit 的演算法

1. parse query，得到 `Qualified(owner,member)` 或 `Unqualified(text)`。
2. Qualified：先 canonicalize owner，從完整 project symbol index 取 `symbol_name == "owner::member"` 的候選；SQL／iterator 此階段不得有 `LIMIT 5`。
3. 對候選做完整 equality、project scope 與 deterministic dedupe（`chunk_id`）後，依 `file_path,start_line,chunk_id` 排序。
4. **此時才**取前 5 個 seeds，再套 packet budget。`seed_count` 是消歧後、limit 前的總數；`truncated=true` 若 hard limit 或 budget 任一截斷。
5. Unqualified：完整 exact free-symbol 集合先走同樣的「收集／排序／limit」流程；若為零才呼叫 FTS，FTS 自己產生候選後也先 deterministic dedupe，再套 5。

因此即使專案有 `A::run`～`F::run` 六個以上，查 `F::run` 仍先縮到 F owner，不能先拿裸 `run` 的前五筆再過濾。

### JSON／lean 輸出欄位

不新增輸出欄，改的是 `symbol_name` 的值與 seed 選取順序。JSON top-level 逐欄位為：

1. `query`
2. `resolution`：`exact_symbol|fts|none`
3. `seeds`
4. `seed_count`
5. `truncated`
6. `truncated_query`
7. `index_is_stale`

每個 seed 逐欄位為 `chunk_id`、`file_path`、`symbol_name`（qualified，例如 `Ledger::run`）、`chunk_type`（impl/trait default 為 `method`）、`start_line`、`end_line`、`content`、`content_truncated`、`neighbors`、`unresolved`。neighbor/unresolved 的既有逐欄位契約不變。

lean header 欄位依序是 `query`、`resolution`、`seeds`、`truncated`、`stale`；每個 seed row 依序是 `file_path:start_line`、`symbol_name`、`chunk_type`，所以 method 範例為：

```text
# context Ledger::run resolution=exact_symbol seeds=1 truncated=false stale=false
src/ledger.rs:42	Ledger::run	method
```

neighbour、unresolved、content block 的既有 lean 行形狀不變。

### 需要動的責任模組／函式

- pack：`rust.yml` 拆成上述三條互斥 rules。
- chunk extraction：`parse_scan_record` 讀 `OWNER`；新增 `canonical_owner`／`compose_symbol_name`；method record 沒有 OWNER 時 fail-closed 成 malformed extraction，不得退回裸 NAME。
- graph/source attribution：innermost chunk 必須使用已組好的 qualified `symbol_name` 作 `source_symbol`，chunk-by-symbol map 也用同一名稱。
- context：新增 `parse_symbol_query`／`normalize_qualified_symbol`；`exact_symbol_seeds` 改成無 limit 的 qualified-first 收集；新增 `sort_dedupe_then_limit`。
- render：不新增欄，但 parity tests 要接受 `Type::method` 原樣輸出，不得在 `::` 切字。

### schema／extractor_version

- DB schema：不變；`chunks.symbol_name` 已可存 qualified string，現有 index 也可做 equality lookup。
- `rust.yml` 在 pack hash 範圍內，所以 `extractor_version` 必須改變；full/incremental index 看到 mismatch 都必須依既有契約要求全量重建，不能混用裸名與 qualified name。
- 這次 YAML 與 record composition 必須同一交付；驗收要斷言新 `extractor_version !=` 舊值。

### 先紅後綠的 TDD 順序

1. ast-grep bridge fixture：free `main` 只有 NAME；inherent impl method 同時有 OWNER/NAME；trait default 同時有 OWNER/NAME；三條不重複命中。
2. `canonical_owner` table test：simple、qualified、generic 與 whitespace case；`compose_symbol_name` 產生 `Type::method`。
3. extractor test：六個以上不同 impl 都叫 `run`，DB 內是 `A::run`…`F::run`，不存在裸 `run` method。
4. context 核心回歸：seed hard limit 為 5 時查第六個 `F::run` 仍精確回 F，證明 limit 在 owner filter 之後。
5. trait default：`Worker::run --content full` 只回 default method body，`chunk_type=method`。
6. trait impl collision：兩個相同 `Type::run` 都以 stable order 保留，`seed_count=2`，不任意選一個。
7. end-to-end CLI：`Type::method` 的 json/lean 欄位逐欄驗證；不存在的 qualified query 是 `none`，不 FTS fallback。
8. pack/version：改動 `rust.yml` 後 version hash 改變，既有 DB 觸發 full reindex。

## 5. finance-cli `main.rs` 的前後 token 驗證流程

### 前置與隔離

使用同一個 release/debug binary、同一 commit、同一 `main.rs` snapshot 與一個全新的 cache。從 Rust port repo 建好 binary 後，令：

```text
FINANCE_ROOT=/home/yanggf/b/finance-engineering/tools/finance-cli
MAIN_RS=src/main.rs
CORT_CACHE_DIR=<mktemp -d 的空目錄>
```

在 `FINANCE_ROOT` 執行 `cort index .`。記錄 finance-cli commit、`main.rs` SHA-256、cort commit、`extractor_version`、binary build profile；中途任一值改變就整輪重跑。量測命令本身不得把 stderr 混進 payload。

### 四個要保存的原始 payload

1. **整檔基線**：`cort read src/main.rs --content full -f lean`。這是 filesystem/full，代表原本整檔 Read 的 tool-return payload。
2. **符號切片**：`cort context main --content full -f lean`。先斷言 `resolution=exact_symbol`、`seeds=1`、seed 為 `main`，且 body 的起訖行與 `main.rs` 對得上；若真正目標函式不是 `main`，把同一輪明確記錄的 `Type::method` 代入，不得事後挑最小函式。
3. **預設重讀 receipt**：在第 1 步已 seed whole-file note 後，執行 `cort read src/main.rs -f lean`；斷言 `source=store content=receipt` 且 payload 只有一行。
4. **full 相容性控制組**：緊接著執行 `cort read src/main.rs --content full -f lean`；斷言 `source=store content=full`，body 與第 1 步相同。

每個命令獨立保存 stdout 原始 bytes，不人工刪 header、路徑或換行。另跑 JSON 作 correctness assertions，但 token 主比較固定使用 lean，避免格式成為混雜因子。

### 計量與報表

- 主要指標：用實際評測模型的同一 tokenizer 計算每個完整 stdout 的 `tool_return_tokens`。若環境沒有該 tokenizer，先報 UTF-8 bytes，並以 `ceil(bytes/4)` 另列為 `estimated_tokens`；估算值不得冒充模型 token。
- 報表逐列欄位：`case`、`command`、`source`、`content_mode`、`bytes`、`tool_return_tokens`（可 null）、`estimated_tokens`、`sha256_stdout`。
- 差異欄位：
  - `symbol_saving = whole_file_tokens - context_symbol_tokens`
  - `symbol_reduction_pct = symbol_saving / whole_file_tokens * 100`
  - `receipt_saving = repeat_full_tokens - repeat_receipt_tokens`
  - `receipt_reduction_pct = receipt_saving / repeat_full_tokens * 100`
- correctness gate：`context` 必須恰為所選 function/method；receipt 不含任何 source body；repeat full 與首次 full 的 body hash 相同；四次命令都沒有 `validation_error`；`index_is_stale=false`。
- 至少跑 5 次 tokenization（payload 固定時結果應完全相同）；時間可另量，但不得把 wall time 當 token 改善證據。最終同時報絕對 token 差與百分比，並與既有 `main.rs ≈27,431 tokens` 的歷史值分欄，避免不同 tokenizer 的數字直接相減。

此流程分開回答兩件事：symbol slice 節省「第一次需要內容」的 payload，receipt 節省「同範圍再次讀取」的 payload；不得把兩者加總成單次節省。

## 6. 交付邊界

- C3 交付 #1～#3 的 readings、error 與核心 tests；D 接 CLI/render parity，並接 #4 的 context/render。
- pack rule 與 chunk composition 必須由同一個 D 變更整合，避免產生有 OWNER capture 卻仍寫裸 NAME 的中間狀態。
- E 先跑全部 Rust tests，再依 §5 做 finance-cli 實測；任何 stale content、暫時 I/O 導致 note 被刪、或 qualified query 受前五個裸名候選影響，都直接判定失敗。

ast-grep rule 語法依官方 [Rule Object Reference](https://ast-grep.github.io/reference/rule) 的 relational `inside`／`has` 契約；實作仍以計畫釘死的 `ast-grep 0.45.2` fixture 結果為準。
