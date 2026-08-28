# Job E2：finance-cli 真實場域量測（2026-08-29）

本報告依 `docs/superpowers/plans/2026-08-28-codex-fix-proposal.md` §5 **逐字執行**。量測對象是 Rust port 的 `cargo build --release` binary，場域是 finance-cli 的 `src/main.rs`。只比較 **lean stdout 的完整原始 bytes**；不把 wall time 當 token 改善證據；不把 symbol slice 與 receipt 加總成單次節省。

環境沒有評測模型 tokenizer，因此 `tool_return_tokens` 全為 `null`。主數字是 UTF-8 `bytes` 與 `estimated_tokens = ceil(bytes/4)`。估算值不是模型 token。歷史 `main.rs ≈ 27,431 tokens`（`docs/2026-08-28-real-session-cost.md`，session transcript 估算器）只作參考欄，**不與本輪數字相減**。

---

## 1. 前置與隔離

| 項目 | 值 |
|---|---|
| 量測日 | 2026-08-29 |
| `FINANCE_ROOT` | `/home/yanggf/b/finance-engineering/tools/finance-cli` |
| `MAIN_RS` | `src/main.rs` |
| `CORT_CACHE_DIR` | `/tmp/tmp.nuX1CzJY84`（`mktemp -d`，本輪唯一、全新、空目錄） |
| binary | `/home/yanggf/a/cortexyoung/rust/target/release/cort` |
| 索引 | 在 `FINANCE_ROOT` 執行一次：`<binary> index .` |
| stderr | 每個命令獨立導到自己的 stderr 檔；四次量測與 index／status／recall 的 stderr 皆為 0 bytes |

同一 binary、同一 finance-cli commit、同一 `main.rs` snapshot、同一 cache。中途未改 binary、未改 `main.rs`、未換 cache。

---

## 2. 記錄的 HEAD／hash／profile

| 項目 | 值 |
|---|---|
| finance-cli HEAD (short) | `6e7af7d` |
| finance-cli HEAD (full) | `6e7af7d8fc98c3c8d4d09424e4e104c8a9c8d68c` |
| `src/main.rs` SHA-256 | `62ab2b23cd1dbbe275aca3b3ff63e0b10538a678e5ecb906f015c33dcc4e8861` |
| `src/main.rs` 大小 | 109,100 bytes；POSIX 行數 2,177（檔案以單一 `\n` 結尾） |
| cortexyoung HEAD (short) | `85c9d641` |
| cortexyoung HEAD (full) | `85c9d6418f5f8505b1c78443d98774f76a1abcb5` |
| cortexyoung branch | `master` |
| Rust crate git 狀態 | `rust/` **未追蹤**（`?? rust/`）。binary 來自 working-tree crate，不是已提交的 rust commit |
| `extractor_version`（fresh index 後 `cort status`） | `5ca0faf41626392332c44acb9e9dbdd1c8b44b0945cc5b7486f1a9c787fd7479` |
| `project_id` | `f82d2b2362cd441825ffc15301cb0cbaa1f96be463069fb00517a08b629081d2` |
| index 結果 | `files=69` `chunks=1141` `unparsed=0` `relationships=0` `elapsed_ms=1037` |
| `index_is_stale`（index 直後） | `false`（`changed_files=[]` `deleted_files=[]`） |
| status `git_head` | `6e7af7d8fc98c3c8d4d09424e4e104c8a9c8d68c`（與 finance-cli HEAD 一致） |
| status `readings`（index 直後、量測前） | `0` |
| build 指令 | `cd /home/yanggf/a/cortexyoung/rust && cargo build --release` |
| build profile | Cargo **release**（default：`opt-level=3`）；`Finished release profile [optimized] target(s) in 51.39s` |
| rustc | `1.95.0 (59807616e 2026-04-14)`；host `x86_64-unknown-linux-gnu`；LLVM 22.1.2 |
| cargo | `1.95.0 (f2d3ce0bd 2026-03-21)` |
| binary SHA-256 | `436d9c3fb8719014a4ba455dd08b78f8be12ad882e91b9f2d679356af45e1673` |
| binary 檔案 | ELF 64-bit LSB pie，x86-64，dynamically linked，**not stripped**；4,263,840 bytes；BuildID `922ec2a7f65f0acfb4e98273cffd86be89858b3e` |
| receipt `content_hash_prefix` | `62ab2b23cd1d`（`main.rs` SHA-256 前 12 hex，相符） |

---

## 3. 四個量測 payload

執行順序即提案順序。每次命令的 stdout 存成獨立檔，不刪 header、路徑或換行。

| # | case | command | 角色 |
|---|---|---|---|
| 1 | `whole_file` | `cort read src/main.rs --content full -f lean` | 整檔基線（filesystem／full） |
| 2 | `context_symbol` | `cort context main --content full -f lean` | 符號切片 |
| 3 | `repeat_receipt` | `cort read src/main.rs -f lean` | 預設重讀 receipt |
| 4 | `repeat_full` | `cort read src/main.rs --content full -f lean` | full 相容性控制組 |

符號切片目標：**未替換成 `Type::method`**。`cort context main` 的 JSON／lean 皆為 `resolution=exact_symbol`、`seed_count=1`、`symbol_name=main`、`chunk_type=function`、`src/main.rs:43-51`，與檔案該段逐行相符。`main` 是存在的 free function（`async fn main() -> ExitCode`），不是 impl method。

觀察（不是步驟偏離）：`main` 本體只有 43–51 行（9 行 wrapper，轉呼叫 `run()`）。同檔最大的 free function 是 `async fn run()`（第 83 行起）。本輪依提案指令量測 `main`，因此 symbol 節省反映的是「整檔 vs 這個 9 行 wrapper」，不是 vs `run`。

### 3.1 逐列主表

`estimated_tokens = ceil(bytes/4)`。`tool_return_tokens` 因無評測模型 tokenizer 而為 `null`。`historical_ref_tokens` 只列歷史整檔 Read 的 27,431，不參與加減。

| case | command | source | content_mode | bytes | tool_return_tokens | estimated_tokens | sha256_stdout | historical_ref_tokens |
|---|---|---|---|---:|---|---:|---|---:|
| `whole_file` | `cort read src/main.rs --content full -f lean` | `filesystem` | `full` | 109184 | null | 27296 | `93cacadeab4c268f3b8825c449dc93f7551b5fe11b21f55ab51c345e0c69e697` | 27431 |
| `context_symbol` | `cort context main --content full -f lean` | — | `full` | 354 | null | 89 | `d49adde021eea881cda5cb998071a79b445cd8d7eab2a927f272a167cb90da0e` | — |
| `repeat_receipt` | `cort read src/main.rs -f lean` | `store` | `receipt` | 81 | null | 21 | `75696861644f22be73fe68e3eea99a8a7da6530eb7066b04ef14156c0e1530f1` | — |
| `repeat_full` | `cort read src/main.rs --content full -f lean` | `store` | `full` | 109179 | null | 27295 | `17b88a4ae4af4a761db63a2bfb8ad5e3133ab1e144c30943c9838dfb03d6ebb7` | 27431 |

lean header（原始第一行）：

```text
# read src/main.rs:1-2178 source=filesystem reads=1 content=full hash=62ab2b23cd1d
# context main resolution=exact_symbol seeds=1 truncated=false stale=false
# read src/main.rs:1-2178 source=store reads=2 content=receipt hash=62ab2b23cd1d
# read src/main.rs:1-2178 source=store reads=3 content=full hash=62ab2b23cd1d
```

`repeat_receipt` 完整 stdout（81 bytes，恰一行加換行，無 body）：

```text
# read src/main.rs:1-2178 source=store reads=2 content=receipt hash=62ab2b23cd1d
```

`cort` 報的 `end_line=2178` 比 POSIX `wc -l` 的 2,177 多 1：lean full 的 body 是 `main.rs` raw bytes **再多一個結尾 `\n`**（renderer 保證結尾換行；檔案本身已以 `\n` 結尾）。step 1 與 step 4 的 body 都是這個形狀，彼此仍逐 byte 相同。

### 3.2 五次 tokenization（payload 固定）

對四個凍結的 stdout 檔各做 5 次 SHA-256 與 5 次 `ceil(bytes/4)`。每次結果完全相同（上表那一組數字）。沒有模型 tokenizer 可重跑 5 次 `tool_return_tokens`。

---

## 4. 差異（提案公式，用 `estimated_tokens`）

公式用的 token 欄是本輪的 `estimated_tokens`，**不是**歷史 27,431。

| 指標 | 公式 | 值 |
|---|---|---|
| `symbol_saving` | `whole_file_tokens - context_symbol_tokens` | 27296 − 89 = **27207** |
| `symbol_reduction_pct` | `symbol_saving / whole_file_tokens * 100` | 27207 / 27296 × 100 = **99.6739%** |
| `receipt_saving` | `repeat_full_tokens - repeat_receipt_tokens` | 27295 − 21 = **27274** |
| `receipt_reduction_pct` | `receipt_saving / repeat_full_tokens * 100` | 27274 / 27295 × 100 = **99.9231%** |

兩件事分開：

- **第一次需要內容**：symbol slice 相對整檔 lean full，少 27,207 estimated tokens（99.6739%）。
- **同範圍再次讀取**：receipt 相對 repeat full，少 27,274 estimated tokens（99.9231%）。

兩者不得加總。`repeat_full`（27,295）與 `whole_file`（27,296）只差 header 的 `source`／`reads`（5 bytes），body 相同。

歷史參考欄 27,431 不進入上表加減。本輪整檔 lean full 是 27,296 estimated tokens；兩者 tokenizer 不同，禁止直接相減。

---

## 5. Correctness gate

| 斷言 | 結果 | 證據 |
|---|---|---|
| context `resolution=exact_symbol` | **PASS** | lean header 與 JSON `resolution` 皆為 `exact_symbol` |
| context `seeds>=1`（提案原文 `seeds=1`） | **PASS** | `seeds=1`／`seed_count=1` |
| seed 為 `main`，起訖行對得上 `main.rs` | **PASS** | JSON：`symbol_name=main` `chunk_type=function` `file_path=src/main.rs` `start_line=43` `end_line=51` `content_truncated=false`。檔案 43–51 即 `async fn main() -> ExitCode { ... }` |
| 未改用 `Type::method` | **記錄** | `main` 是真實 free function，不替換 |
| receipt `source=store content=receipt` 且只有一行 | **PASS** | 81 bytes；恰一個換行且以換行結尾；不含 `mod analysis_cmd` 或任何 source body |
| receipt 不含 source body | **PASS** | lean 無 body；事後 JSON receipt 的 keys 無 `content` 欄（欄位不存在，不是 `null`） |
| repeat full `source=store content=full` | **PASS** | header 如上；`reads=3` |
| repeat full body 與 step 1 相同 | **PASS** | 兩邊 body 皆 109,101 bytes；body SHA-256 皆 `ca5da926556b8d1736686208d29fc58691a4266e09478bb416b0323331a7526c`。stdout 整體不同只因 header（`filesystem`/`reads=1` vs `store`/`reads=3`） |
| 四次命令皆無 `validation_error` | **PASS** | 四份 stdout 不含 `validation_error`；stderr 0 bytes；exit 0 |
| `index_is_stale=false` | **PASS** | index 直後 `cort status`：`false`；context JSON：`index_is_stale=false`；lean header：`stale=false` |

Gate：**全部通過**。

---

## 6. `cort recall`

提案要求另跑 `cort recall <a word that appears in main.rs> -f lean`。查詢詞：`FinanceCliError`（出現在 `src/main.rs` 第 53 行起）。

| 項目 | 值 |
|---|---|
| command | `cort recall FinanceCliError -f lean` |
| exit | 0 |
| stderr | 0 bytes |
| `validation_error` | 無 |
| lean header | `# recall FinanceCliError readings=1 truncated_query=false` |
| 是否找到 stored reading | **是** |
| 命中 | `src/main.rs:1-2178`，`reads=4`（見 §7：量測後多跑了一次 JSON full read，故 recall 看到的 `read_count` 是 4，不是量測 step 4 當下的 3） |
| JSON 核對 | `reading_count=1`；`content_truncated=true`（預設 12 行預覽）；`content` 長度 187 |

stored whole-file note 可被 recall 找回。

---

## 7. 相對提案步驟的偏離

1. **無評測模型 tokenizer。** 依提案 fallback：報 UTF-8 bytes 與 `ceil(bytes/4)`；`tool_return_tokens=null`。五次「tokenization」改為對凍結 payload 做五次 SHA-256 + `ceil(bytes/4)`，結果不變。
2. **量測後的 assertion-only 命令**（stdout **未**進入上表）：
   - step 2 之後：`cort context main --content full -f json`（不碰 readings）。
   - step 4 之後：`cort read src/main.rs --content full -f json`（`read_count` 3→4）。
   - 其後：`cort recall FinanceCliError -f json`、`cort read src/main.rs -f json`（receipt JSON；`read_count` 4→5）。
   - step 1–4 的量測檔本身仍是 `reads=1 / n/a / 2 / 3`，順序未被這些命令插入破壞。
3. **`rust/` 未進 git。** 記錄的 cortexyoung HEAD 是 JS 樹 `85c9d641`；實際跑的是 untracked crate 的 release binary（SHA-256 見 §2）。
4. **`cort status` 只在 index 直後抓一次**，用來記錄 `extractor_version` 與 `index_is_stale`（當時 `readings=0`）。量測命令之間沒有重跑 status。
5. **index `elapsed_ms=1037` 只作建索引紀錄**，不用來主張 token 改善。
6. **未跑 `npm test`、未 commit**（任務禁止）。

沒有改量測順序、沒有事後挑最小函式、沒有把 stderr 混進 payload、沒有把 27,431 直接減進去。

---

## 8. 結論（本輪絕對數字）

- 整檔 lean full： **109,184 bytes／27,296 estimated tokens**（歷史參考 27,431，不同 tokenizer）。
- `context main --content full`： **354 bytes／89 estimated tokens**；`exact_symbol`、1 seed、`main` @ 43–51。
- 預設重讀 receipt： **81 bytes／21 estimated tokens**；一行、無 body、`source=store`。
- 重讀 `--content full`：body 與第一次 **逐 byte 相同**。
- `symbol_saving=27207`（99.6739%）；`receipt_saving=27274`（99.9231%）；兩者不可加總。
- correctness gate 全過；`index_is_stale=false`；無 `validation_error`；`recall FinanceCliError` 找回 stored `src/main.rs:1-2178`。
