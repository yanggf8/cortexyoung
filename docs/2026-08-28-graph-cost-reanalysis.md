# `cort` 到底省不省 token？三次 STOP 之後的重新分析

- **日期**：2026-08-28
- **狀態**：定稿。已落地兩項修正（召回、payload），並重新定義 §8 閘門的測量方式
- **一句話**：三次 `STOP` 都是真的，但它們否決的是「`cort` 當 grep 用」，不是「`cort` 當圖用」——而且真正該被拿去判決的那個指標，30 個 cell 裡一個都沒有記錄過

---

## 1. 事件背景

`cort` 的目標寫在 spec 第一行：給 agent 一個離線的 code intelligence CLI，讓 `rg`/`xg` 處理字串、`ast-grep` 處理檔內結構、`cort` 補上跨檔邻居與爆炸半徑，並且以 token _budget_ 回傳。

§8 設了一個很硬的閘門：**若 `cort struct` + `cort context` 無法在 token 數與成功率兩項都勝過 `ast-grep` + 手動 `Read`，就停止加功能。**

2026-08-26 跑了三輪三臂評測（`rg+Read` / `ast-grep+Read` / `cort`，每格一個全新 subagent、嚴格工具政策、人工驗證標籤）。三輪全數 `STOP`。README 隨即在 2026-08-28 上午定調：

> Cort's graph adds correctness nowhere and costs 2-3.6x in total context. **Positioning: cort is an interactive human tool, not an agent token-saver.**

這個結論被寫進 README，也被加進 spec 的狀態欄。它同時凍結了 `rewrite`、`modules`、`--watch`、`impact --from-diff`、`search`、embeddings 六項延期功能。

問題是：**這個結論是從哪個數字讀出來的，那個數字量的又是什麼。**

---

## 2. 三個缺陷

### 缺陷一：決定性指標從未被記錄

harness 的 README 白紙黑字寫著，這個 harness 存在的理由就是補上舊版評測-plan 缺的兩個指標：`tool_return_tokens` 與 `stale_reads`。

實測結果：

| 項目 | 30 個 cell 中的數量 |
|---|---|
| `tool_return_tokens` 為 `null` | **30 / 30** |
| `read_calls` 為 `null` | **30 / 30** |
| `stale_reads` 有值 | 30 / 30 |

也就是說，唯一從頭到尾被記錄、被拿來比較、被拿去判決的數字是 `total_tokens`。而 `token-raw.json`（只有第一輪留了明細，15 cells）顯示：

```
total_context = input + cache_read + output
cache_read / total_context = 69% … 94%（median 88%）
```

`cache_read` 是什麼？是把整段對話重新送進模型的費用。它約等於 **turn 數 × 累積上下文**。所以 `total_tokens` 量的是「這個 agent 講了多少話、繞了多少圈」，不是「這個工具回了多大的封包」。

一個工具只要讓 agent 多繞四圈，它在這個指標下就自動貴四倍——**哪怕它每次回傳的內容只有对手的十分之一**。三輪評測的 turn 數剛好印證這條通道：`cort` arm 的 turn 數是 17、22、20、22、2，`rg` arm 是 14、7、1、14、1。

### 缺陷二：沒有任何一個任務需要圖

這是最關鍵的一個，而且直接可以从 `tasks.json` 看出來。

10 個任務的 `expected_symbols` 全部是 **depth 1 或字面查找**：

| 任務 | 本質 | 需要走圖嗎 |
|---|---|---|
| `who-calls-extractfile`、`who-calls-batchdualai`、`who-calls-createbacktestingstorage` | 「誰直接呼叫 X」= 1 跳 | 1 跳，`rg` 一次搜得到 |
| `blast-radius-parsescanstream`、`blast-radius-getcurrenttimeet` | 標籤寫的是 direct consumers | 自稱爆炸半徑，實際標到 1 跳 |
| `find-all-transactions` | 單一語法形状 | `ast-grep` / `rg` 即可 |
| `where-is-confidence-set`、`where-market-open-decided` | 找一個字串 / 找一個定义 | 完全不需要 |
| `do-bindings-trace` | 讀 config 對類別名 | 完全不需要 |
| `trace-staleness` | 「怎麼算、誰用」 | 定義查找 + 1 跳消費者；十個裡最接近需要鏈路的 |

`src/graph.js` 裡那個 `WITH RECURSIVE` 遞移閉包——`getTransitiveDependents`，**整個產品存在的理由**——在三輪評測裡從未被要求跑超過 1 跳。

最能說明問題的是 `where-market-open-decided`（「哪個 export 函數決定美股是否開盤」）：`rg` 1 個 turn、92k tokens 收工；`cort` 20 個 turn、934k tokens。這不是「圖沒有價值」，這是**路由錯誤**：一個字面查找被塞進了圖工具。用這個 cell 去否決圖，等於用「拿扳手釘釘子失敗」去否決扳手。

### 缺陷三：圖正好瞎在場域最重要的符號上

前兩個是方法論問題。這三個是真實 defect，而且它單獨就足以解釋 `cort` arm 為什麼 turn 數爆炸。

評測場域 `/home/yanggf/a/cct` 是 Cloudflare Worker，它的 route handler 全部長這樣：

```ts
export const handlePreMarketBriefing = createHandler('pre-market-briefing',
  async (request: Request, env: CloudflareEnvironment, ctx: any) => { … });
```

而 `src/pack/rules/*.yml` 的 chunk 規則只有三條：`function_declaration`、`class_declaration`、`method_definition`。

一條 `const` 綁定的 arrow function 在三者之中都不存在。於是連鎖失效：

1. `handlePreMarketBriefing` **沒有 chunk**（實測：`SELECT count(*) FROM chunks WHERE symbol_name='handlePreMarketBriefing'` → 0）
2. `relationshipRowsForFile()` 裡 `if (e.source_symbol === null) continue;` — 它內部的呼叫**沒有來源符號**，邊被直接丟棄。cct 實測有 **1,106 / 13,959 = 7.9%** 的邊是 `source_symbol: null`
3. 因此 `cort impact` 在任何 `--depth` 都回不到它

而 round-2 的手動標籤裡，`blast-radius-getcurrenttimeet` 的 `expected_symbols` 恰好就包含 `handlePreMarketBriefing` 與 `handleEndOfDaySummary`。

**結論：評測要求 `cort` 回答的符號，有一半是它在結構上不可能回來的。** `cort` arm 最後 coverage 仍是 1.0，因為 agent 自己跑去 `Read` 檔案把缺口補上——那正是 turn 數 17→22、上下文 656k→974k 的来源。`total_tokens` 懲罰的是「agent 必須自己做圖該做的事」，然後被解讀成「圖沒有用」。

---

## 3. 怎麼在沒有模型變數的情況下計價

要判定「省不省 token」，必須把 agent 的行為變數拿掉。新增 `evals/relation-cost.mjs`：確定性、無模型、對**同一個答案集合**計價。

`rg` 側刻意給到最優條件（這讓結果偏保守，也就是偏袒 `rg`）：

- 每一跳只發**一次** batched alternation grep（`rg -e '\bA\b' -e '\bB\b' …`），不是一次一詞
- 名字來自 oracle：假設 agent 總是完美知道該搜哪些符號，零摸索、零失誤、零多餘 turn
- 加上把命中歸給包圍符號所需的文字量（從索引取該符號起點到命中行的實際位元組）

### 結果（cct，2,713 chunks，6 個自動選出的多跳符號取中位數）

| hops | `cort impact -f lean` | `rg` + 讀檔抵達同一集合 | 倍數 | rg 成本中「讀檔」佔比 | rg 命中精確率 |
|---|---|---|---|---|---|
| 1 | 968 tok | 16,584 tok | **14.8x** | 83% | 0.67 |
| 2 | 1,022 tok | 86,949 tok | **67x** | 86% | 0.42 |
| 3 | 1,136 tok | 127,531 tok | **62x** | 87% | 0.57 |

三個數字講了三個不同的事：

1. **`rg` 的錢花在讀檔，不在搜字。** 83–87% 的成本是讀取，因為每一跳都會交回一批新名字必須再搜，而每個命中都得打開才知道它屬於哪個符號。
2. **成本隨深度的走向完全不同。** `rg` 從 16.6k → 127.5k（7.7 倍），`cort` 從 968 → 1,136（1.17 倍）。圖的走訪是一次遞迴 SQL。
3. **`rg` 的精確率崩壞**：六個符號的命中精確率分佈 0.04–0.90。名字越常見越糟——`logInfo`、`createHandler` 這種名字到處都是，而絕大多數的「到處」是無關的。

### 健全性另用獨立來源覆驗

`evals/verify-impact.mjs`：圖負責提出假設，**檔案原文**負責裁決。每個 dependent 的 `[start_line, end_line]` 區間直接从磁碟讀出，正則確認其本體內確實存在前一跳某符號的字邊界引用。

五條鏈、100 個 dependent、**100 個通過**（precision 1.0）。第六條候選 `createSimplifiedEnhancedDAL` 只有 0.965，**從任務集裡移除**——不合格的答案不做標籤。

---

## 4. 兩項修正

### 4.1 召回：`const` 綁定的函數必須是節點

新增 `cort-{ts,tsx,js,py}-chunk-const-function`。收得很緊，因為寬鬆的版本會把整個索引變成垃圾：

| 寫法 | 是符號嗎 | 判定 |
|---|---|---|
| `const f = (x) => { … }` | 是 | ✅ |
| `const f = function () { … }` | 是 | ✅ |
| `const h = createHandler('x', async (r) => { … })` | 是 | ✅ |
| `const rows = xs.map(x => f(x))` | **否**，是資料 | ✗ |
| `const alias = helper` | 否，是別名 | ✗ |

第三列與第四列的差別只有一條規則就能抓住：**被呼叫者必須是裸識別符，不能是 member expression。** 第一版沒這條，211 個 distinct 名字裡塞滿 `articles`、`promises`、`kpiMetrics`（都是 `.map()` / `.filter()` / `.findIndex()` 的回調）；加上這條之後 cct `src/` 從 354 個匹配降到 28 個，全部是真函數，兩個原本對索引不可見的 handler 都回來了。

重索引效果：205 files 不變，2,676 → **2,713** chunks（+1.4%），1,871 → **1,905** relationships（+1.8%）。重點不在量：

```
$ cort impact --symbol getCurrentTimeET --depth 3 -f lean
# 修正前 dependents=5：isMarketHours, getCurrentDateET, getLastNTradingDays, handleReportsStatus, handleReportRoutes
# 修正後 dependents=8：+ handlePreMarketBriefing(h1), + handleEndOfDaySummary(h1), + handleDirectRequest(h2)
```

Round-2 五個手標標籤，**從缺 2 個變成 5/5 全中**。

### 4.2 Payload：`-f lean`

`chunk_id` 的定義是 `${projectId}:${file_path}:${start_line}`，而 `projectId` 是 64 字元的 sha256 hex。`impact` 的每一個 dependent 列都同時帶 `chunk_id` 和 `file_path`——等於**把同一列已經有的資料，用一個 64 字元雜湊再加一個重複路徑再講一遍**。而且 `impact` 是三個命令裡唯一完全沒有 budget 機制的（`struct`、`context` 有），實測一次 depth-3 爆炸半徑吐 10,557 tokens。

`-f lean` 保留同样的答案，一列一筆、tab 分隔、不帶 id：

```
# impact getCurrentTimeET depth=3 seeds=2 dependents=8 stale=false
h1      src/modules/handlers/briefing-handlers.ts     handlePreMarketBriefing       30
h1      src/modules/trading-calendar.ts               isMarketHours                 159
h2      src/routes/report-routes.ts                   handleReportsStatus           1256
```

六個符號實測 JSON/lean 比：**4.5–4.9x**。JSON 是完全的預設路徑；測試從基線 112 支增到 121 支，原本那 112 支零修改全綠。

順帶一提，lean 每行的第二欄就是 `file_path`——所以「**哪些檔要改**」這個 agent 高頻問題，在 `cort` 這裡是一列一個檔案名，不需要额外一趟搜尋。

---

## 5. 修正後的定位

原來的句子：

> Cort's graph adds correctness nowhere and costs 2-3.6x in total context.

改為有條件的結論：

- **找字串：`rg`（或重複搜尋用 `xg`）永遠是對的答案。** 這不需要圖，而模型本來就是 grep-native。
- **找關係：圖是唯一在深度上是平的選項。** 在 cct 上 1 跳 14.8x、2–3 跳 62–67x；且 `rg` 側已經是給到 oracle 名字、一次 batched call 的理想條件。
- **一個 3 跳爆炸半徑只要同時是「誰必須一起改」的問題，`cort` 就沒有對手**，因為文字工具要拿到同一個集合，必須自己把圖走出來，而那個動作的價格隨深度成長。

**「graph adds correctness nowhere」這句撤銷。** 它從未被在圖能適用的場景上測試過。

分工現在寫死在 `skills/ast-grep/SKILL.md`：字串 → `rg` / `xg`，關係 → `cort`，並且一律帶 `-f lean`；skill 長度維持在約 610 tokens，沒有變胖。

---

## 6. 還沒被證明的（重要，別把話說過頭）

1. **以上全部是 payload 層級的證據，不是 end-to-end。** 一輪合格的 agent 評測還欠兩個东西：`tool_return_tokens` 必須真的被記錄，以及任務集必須是 `evals/tasks-graph.json`（5 個 `min_hops_required: 3`）。原 `tasks.json` 測的是 `cort` 不該打的仗。
2. **信任成本尚未解決。** 三輪評測裡最貴的一格（171 turns、5.2M tokens）是一個 `cort` cell 不斷回頭驗證自己的答案。`index_is_stale` 與 `AMBIGUOUS` 語意會推著 agent 去查證。圖正確不等於 agent 會相信圖——這是下一個要打的點，而 `impact` 目前**不帶邊證據**（沒有呼叫點行號、沒有 `rel_type`），所以 agent 無從快速查證，只能整段重讀。relationships 表也沒有存呼叫點行號，要補就是 schema 變更。
3. **單一符號的數字受 agent 行為變異主導。** 本文引用的確定性測量不受此影響，但三輪評測的 cell 級數字受。
4. **一個既有 flake（非本次引入）**：本機負載下 `ast-grep` 子行程可能觸及逾時閾值，依 `d3cbff3` 的設計該檔案降為 `unparsed`——於是在 15 個測試檔全速並行時，`chunker.test.js` 的 innermost-containment 測試偶爾會紅。重跑 3 次 121/121 全綠。新增的測試多帶 3 次 spawn，會略微提高這個機率。

---

## 7. 給同類 tool 作者的教訓

這次最有價值的產出其實不是 `cort` 變好了，而是四條可搬運的規則：

1. **先確認你的 gate 指標量的就是你的假設。** 如果成本指标裡 88% 是 `cache_read`，你量的其實是行為，不是工具。
2. **每個指標都要在記錄之前寫一支斷言檢查它不為 null。** `tool_return_tokens: null` 連續三轮没人为之惊讶，是因為 summarize() 對 `undefined`/`null` 很宽容。
3. **任務集必須在結構上無法由 baseline 達成，否則你測的是路由，不是能力。** 「誰直接呼叫 X」是 grep 一行的事；要測圖就必須多跳。
4. **否決之前，先確認 baseline 與被測方看到的標籤是同一套東西。** 我們差一點就因為評測場域的 **索引器看不見的程式風格** 而砍掉整個圖功能。順帶一提：const 綁定函數在 Worker / Express / Lambda 生態是**常態而不是例外**，這個盲區的適用範圍比 cct 大得多。

---

## 8. 重現

```bash
CORT_CACHE_DIR=/tmp/cort node bin/cort.js index /path/to/repo
node evals/relation-cost.mjs --repo /path/to/repo --depth 3 --pick 6
node evals/verify-impact.mjs --repo /path/to/repo --symbols getCurrentTimeET,logInfo --depth 3
node --test tests/render.test.js tests/chunker.test.js
```

- 任務集（需走圖）：`evals/tasks-graph.json`
- 計價工具：`evals/relation-cost.mjs`
- 標籤覆驗：`evals/verify-impact.mjs`
- 原評測證據：`evals/runs/2026-08-26{,-cct,-cct-r3}/`
- 背景決策：`docs/superpowers/specs/2026-08-25-cortex-ng-lightweight-design.md` §8
