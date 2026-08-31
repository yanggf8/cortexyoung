# WIP：端對端圖評測的前置（2026-08-28）

> ## ⚠️ 2026-08-31 標記：本檔已被取代，不再更新（audit F-05 §9.1(6)）
>
> 本檔是 2026-08-28 的**歷史工作紀錄**。它的處境分兩段，兩段都有文件：
>
> 1. **它的目的先被判作廢。** `docs/2026-08-28-real-session-cost.md` 用真實 session transcript 量需求面，
>    結論是圖對主力 repo 的日常幫助有限。該檔明確寫「本文取代本 WIP 的方向」。
> 2. **但端對端評測後來仍然跑了**，而且不是用本檔 §4 規格裡那支 JS runner。現在它是 `cort-evals
>    run-agents`（Rust，`evals/src/arms.rs`），兩輪 × 5 任務 × 2 臂 = **20 cells，`tool_return_tokens`
>    與 `read_calls` 全部有值**，gate 回 `cort_beats_ast_grep=true`。證據與判讀在
>    `docs/2026-08-29-project-audit-root-causes-and-remediation.md` §13f／§13n，資料在
>    `evals/runs/2026-08-30-graph{,-sample2}/`。
>
> **以下三條已被實測推翻，照做會燒錢或得出錯結論：**
>
> 1. **§3(3)「工具白名單即實驗組」不成立。** 這是本檔最錯的一條。F-11：headless 模式下
>    `--allowedTools Bash(rg:*)` **根本不約束 Bash**（第一格真實 cell 就跑了十次 `grep -rn`/`sed -n`
>    而 `permission_denials: []`）；PATH jail 也擋不住，因為 Claude Code 會正規化 Bash 工具的 `PATH`。
>    因此每格改記 `shells_used` / `arm_held` / `jailed`，`arm_held: false` 的格子不得當成「cort vs rg」
>    來平均——這批資料量到的是「cort vs agent 的整個 shell」。想量前者需要能真正束縛工具的 driver。
> 2. **所有 JS 路徑失效。** 本檔寫的 `bin/cort.js`、`evals/run-agents.mjs`、`relation-cost.mjs` 都已隨
>    cutover 刪除；`AGENTS.md` 現行契約是純 Rust，`CORT_BIN` 是 `rust/target/release/cort`。
> 3. **§2.3 那個索引檔不可複用。** `/tmp/cort-exp/<sha>.db` 是 JS 時代的產物，schema 已升到 v3
>    （新增 `raw_edges` 持久層，修 F-01 的 incremental 漏邊），必須由目前的 release binary 重建。
>
> **仍然有效、而且後來被用上的三條**：§2.1（`--output-format stream-json` 逐格取得 `tool_result`
> 長度與 `permission_denials`——20 格就是靠它才有非 null 指標）、§2.2（不隔離 `CLAUDE_CONFIG_DIR`
> 約 16k tokens 的 hook/plugin 雜訊會進每一格，runner 現在直接拒跑）、§2.3 的 `projectId` 由 `cwd`
> 推得（runner 因此把 `--venue` 列為必要參數）。
>
> ---


> 狀態：**前置全部探通。一個 eval cell 都還沒跑；已花的只有 5 次單字級探測（合計 < $0.40）。**
> 本檔是接續用的工作紀錄，不是結論。結論要等 `evals/run-agents.mjs` 真的跑出兩臂數字之後，
> 寫進 `docs/2026-08-28-graph-cost-reanalysis.md` 的後續版本。
> 這裡記的是「已經用實測買到的事實」與「跑之前必須鎖住的偏差」，兩者都付過代價。

---

## 1. 這是在補什麼債

`docs/2026-08-28-graph-cost-reanalysis.md` §6 列了兩件還沒被證明的事情，本 WIP 對準第一件：

1. **§6.1 — 端對端評測欠兩個東西**：`tool_return_tokens` 必須真的被記錄（三輪 30 格全是 `null`），
   且任務集必須是 `evals/tasks-graph.json`（5 個 `min_hops_required: 3`），不是 `evals/tasks.json`。
2. **§6.2 — 信任成本**：`impact` 不帶邊證據，agent 無從快速查證只能整段重讀。本輪把這條**精確化**了，
   見 §2.4。

先決條件是：要有辦法在同一回合裡拿到 (a) 每次 tool 回傳的 payload 大小，與 (b) 該回合的真實
token 用量。上三輪之所以只能記 `total_tokens`，就是因為只有 (b)。

---

## 2. 已驗證的前置（本輪實測）

### 2.1 headless agent 可取得 per-tool payload

`claude` 2.1.250（`codex-cli` 0.150.1 也在；本輪用 Claude Code，因為上三輪 `token-raw.json` 的
欄位形狀就是它的 `--output-format json` 輸出）。

`claude -p "<task>" --output-format stream-json --verbose` 的串流裡：

- `type:"assistant"` 事件帶 `message.content[]`，其中 `tool_use` 有 `name` 與 `input`
  → `read_calls`、每次 `impact` 的參數，全部可數。
- `type:"user"` 事件帶 `tool_result`，**內容是純文字且可量長度** → `tool_return_tokens` 可算。
  這是最關鍵的一條：上三輪的 `null` 不是「拿不到」，是沒人寫這支解析。
- `type:"result"` 事件帶 `usage.{input_tokens,cache_creation_input_tokens,cache_read_input_tokens,
  output_tokens}`、`num_turns`、`total_cost_usd`、`session_id`、**`permission_denials`**。

`permission_denials` 是白名單外洩的檢漏器：評測中途 cort 臂如果試圖改打 `rg`，會被拒，而且會留下
紀錄，而不是被默默忽略。

### 2.2 不隔離環境會先淹死實驗（實測數字）

使用者的 `~/.claude/settings.json` 掛了 `mos hook` 於 `SessionStart` / `UserPromptSubmit` /
`PreToolUse` / `PostToolUse` / `SubagentStart`，且 `enabledPlugins` 會帶進 superpowers 那套
「只要有 1% 機率該用 skill 就必須用」的注入文案。這跟 `cort` 無關，但會進到每一格。

同一 cwd（`/tmp/probe`）、同一單字提示、`--strict-mcp-config`，只換設定目錄：

| 設定 | `cache_creation` | `cache_read` | 每請求 in-context |
|---|---|---|---|
| 預設 `~/.claude` | 19,067 | 11,165 | **30,232** |
| 隔離 `/tmp/cc-eval` | 6,044 | 8,121 | **14,165** |

差 **16,067 tokens（約 53%）**，全部是環境雜訊。一個 40 turns 的 cell 裡這筆開銷會被重複讀取數十次，
足以壓過 `cort impact -f lean` 整個 payload（~1.1k tokens）——那就又變成在量 agent 講了多少話。

隔離做法（憑證用 symlink，**不複製任何 secret**）：

```bash
mkdir -m 700 /tmp/cc-eval
ln -s $HOME/.claude/.credentials.json /tmp/cc-eval/.credentials.json
printf '{ "model": "opus", "hooks": {}, "enabledPlugins": {} }' > /tmp/cc-eval/settings.json
# 執行時：CLAUDE_CONFIG_DIR=/tmp/cc-eval ... --strict-mcp-config
```

### 2.3 場域與索引狀態（接續時先確認這幾個）

- cct 場域：`/home/yanggf/a/cct`，HEAD `b41e39d`，工作區乾淨（僅既有 untracked `.codex`）。
- 索引：`/tmp/cort-exp/e3b7de5dae9bc8db….db`（21 MB，2,713 chunks）。實測
  `impact --symbol getCurrentTimeET --depth 3 -f lean` → `seeds=2 dependents=8 stale=false`。
- **`projectId` 由 cwd 推得**：在 `cortexyoung` 底下對 cct 的符號跑 `impact` 會得到 `seeds=0` 且
  `stale=true`（實測踩過）。評測時 cort 臂的 `cwd` 必須是 venue，否則測到的是「找不到符號」而不是工具。
- `cort` 不在 PATH，一律 `node /home/yanggf/a/cortexyoung/bin/cort.js`。

### 2.4 lean 已經帶了半邊證據（把 §6.2 精確化）

實測 `-f lean` 的依賴者列是 `h<depth>	<file>	<symbol>	<line>`，另有
`unresolved <callee>	calls	AMBIGUOUS`。所以 §6.2 講的「不帶邊證據」要分成兩半來看：

- **已有**：hop 距離、依賴者的檔案與**定義行**、以及 unresolved 列的 `calls` 型別。這足以讓 agent
  自己決定先開哪一個，也足以回答「幾跳、誰」。
- **仍缺**：**呼叫點那一側**的行號（是誰、在哪一行呼叫了父節點），以及依賴者那條邊的 `rel_type`。
  沒有它，agent 要驗證一條邊只能把整個符號重讀一次——§6.2 的判斷成立。

因此要補的仍然是：`relationships` 表新增呼叫點欄位（schema 變更）。**這也正是評測要先量的東西**：
如果 cort 臂在帶 hop+定義行但無呼叫點行號的情況下仍然不再回頭重讀，那筆「信任稅」就沒有 §6.2 講的
那麼重，schema 變更的優先序就該往後放。先評測、後改 schema，順序不能反。

---

## 3. 跑之前必須鎖住的偏差（逐條都付過代價）

1. **cct 的 `CLAUDE.md` 有 19,380 bytes、另有 `AGENTS.md`**，兩臂都會載入。對稱，但要在結果裡寫明
   每格的固定開銷，否則跨場域比較會誤讀。
2. **transcript 不能寫進 venue**。實測過：探測時 `rg` 把評測自己寫的 `stream.jsonl` 當成命中結果。
   輸出一律 `/tmp`。
3. **工具白名單即實驗組**。rg 臂：`Bash(rg:*)` + `Read`。cort 臂：`Read` +
   `Bash(node /home/yanggf/a/cortexyoung/bin/cort.js:*)`。兩臂拿同一份任務提示，只換這一段。
4. **`CORT_CACHE_DIR` 必須由父行程注入**。`allowedTools` 是**字面前綴比對**，agent 自己寫
   `CORT_CACHE_DIR=/tmp/cort-exp node …` 會變成另一種命令前綴而被拒——然後它就會退化成去 `Read`，
   整格資料報銷。提示裡要給出可直接照抄的命令形狀。
5. **SKILL.md 的 `-f lean` 指引要不要餵給 cort 臂？——要。** 因為 `164fdee` 的交付物本來就包含重寫
   `skills/ast-grep/SKILL.md`（字串→`rg`/`xg`、關係→`cort`、一律 `-f lean`）。這是被測產品的一部分，
   不是偷加分；但結果裡必須標注「cort 臂收到了與 cort 同行的使用指引」，讓讀者自己判斷。
6. **`--max-turns` 要有上限，且把 `hit_turn_cap` 老實記下來**，不要讓它表現為 `success: false`。
   上三輪有 171 turns 的格子；`hub-blast-radius-loginfo` 有 44 個 expected symbols，rg 臂很可能重演。
7. **token 估算器要對兩臂一致且寫進文件**。不新增依賴，所以：ASCII 字元 / 4 + 非 ASCII 字元 × 1
   （cct 原始碼有繁體中文註解；一律除以 4 會**低估** rg 臂的 payload，反而偏袒 rg）。同時記 bytes，
   並與 `usage` 的精確總量交叉檢查（估算值必須 ≤ 實際 in-context 增量）。
8. **每個指標在寫入前 assert 非 null**。這是 §7 教訓 2 的根治：`summarize()` 對 `undefined`/`null`
   太寬容，所以 null 連續三輪沒人吃驚。runner 要寧可丟例外也不要寫 null。

---

## 4. Runner 規格（尚未實作）

`evals/run-agents.mjs`：

- 輸入 `evals/tasks-graph.json`，`--arms rg+Read,cort`、`--only <task-id>`、`--concurrency 2`
  （round 2 以 15 併發跑會 thrash，round 3 已是 2 波 × 3）。
- 每格：`claude -p`（`stream-json`、`cwd = venue`、`CLAUDE_CONFIG_DIR` 隔離、`--strict-mcp-config`、
  臂別 `--allowedTools`、`--max-turns`）→ 解析串流 → 覆寫 `run-eval.mjs` 的 row 形狀。
- 必記欄位：`success`、`coverage`、`precision`、`answered_symbols`、`total_tokens`、
  **`tool_return_tokens`（非 null，否則丟例外）**、**`read_calls`（同上）**、`turns`、
  `hit_turn_cap`、`permission_denials`、估算器版本、`resolved_model`、venue HEAD。
- 成敗定義寫死在檔案頭，不要事後調：`coverage ≥ 0.9` 且 `precision ≥ 0.7`。
- 每格一個 JSON 落在 `evals/runs/2026-08-28-graph/<arm>/<task>.json`，再加 `rows.json` /
  `summary.json`（沿用 `run-eval.mjs` 的 `summarize()`，但 gate 比較對象改成 `rg+Read` vs `cort`，
  因為 §7 教訓 3：任務集要在結構上無法由 baseline 達成）。

## 5. 執行計畫

1. **Smoke：1 任務 × 2 臂**，選 `transitive-chain-lastntradingdays`（3 個 expected symbols，仍是 3-hop，
   兩臂都便宜）。驗則：`tool_return_tokens` 與 `read_calls` 兩格都非 null、`permission_denials` 沒有
   越界、cort 臂確實只呼叫過 `cort`。
2. 再補 `blast-radius-3hop-getcurrenttimeet`（8 個符號；這一格直接對上修正後的 8 個 dependents）。
3. 最後才跑 `storage-blast-radius-backtesting`（20）與 `hub-blast-radius-loginfo`（44）——先看 smoke 的
   每格成本再決定要不要跑滿，這兩格是預算黑洞。
4. 結果若支持圖主張，才回頭動 §6.2 的 schema（call-site line + `rel_type`）；不先動，因為評測本身就是
   為了量「不信任 cort 要付多少」，先改就量不到那筆稅。

---

## 6. 本輪**沒有**做的事

- 沒寫任何 repo 內的評測程式碼（`evals/run-agents.mjs` 不存在）。
- 沒跑任何 eval cell（5 次探測都是單字提示，不屬評測資料）；沒有新的 `evals/runs/` 目錄。
- 沒碰 `relationships` schema，也沒碰 `impact` 輸出。
- 規格 §8 的凍結清單（`rewrite`、`modules`、`--watch`、`impact --from-diff`、`cort search`、
  embedding/RRF）照常未動。
