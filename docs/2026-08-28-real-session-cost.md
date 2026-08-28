# 日常 session 的真實成本,以及圖問題的結束(2026-08-28)

> 狀態:**結論,不是 WIP。** 這份量測結束了「cort impact 對日常 Claude Code 有沒有幫助」這條線。
> 資料來源是真實的 session transcript,不是編出來的任務集。本文取代
> `2026-08-28-end-to-end-eval-wip.md` 的方向;該份筆記的前置量測仍有參考價值,但其目的未執行即作廢。

---

## 1. 結論

**「找關連 graph」對這兩個主力 repo 的日常沒有幫助,四個理由,每一個都獨立成立:**

1. **沒有需求。** 兩個最重的 repo(finance-engineering、claw-skills)合計 1,565 筆真人輸入,
   撈「影響/誰呼叫/相依/重構/改名/移除」等關鍵詞得到 27 筆,逐筆檢視:finance 的 24 筆全是金融語境
   的「影響」(上市對市場、利率對泡沫),claw 的 3 筆裡唯一真的在問消費者的是 **npm 套件**
   (express/node-fetch),不是 Rust 符號。**真正在問程式碼呼叫關係的:0 筆。**
2. **真的發生時,波及範圍是 1 個檔案。** claw-skills 的 transcript 裡有 38 個 Rust 編譯錯誤,
   其中 29 個(76%)是 blast-radius 形狀(E0061 簽章/E0425 改名/E0063 欄位/E0599 方法),
   但歸納成 7 次事件,**每一次都集中在單一檔案**(多為測試檔),最大一次 7 個錯誤全在
   `crates/cct2/tests/merge_render.rs`。沒有一次是跨 crate、多跳的連鎖。1 跳、1 檔,
   是 `rg` 綽綽有餘、3 跳圖完全用不上的場景。
3. **Rust 已有更準的工具。** `cargo check` 直接給 `file:line` 和缺少的參數,是精確答案;
   cort 的圖是近似的,lean 輸出還帶 `unresolved ... AMBIGUOUS`。在 Rust 上 cort 不會比編譯器好。
4. **cort 根本讀不了 Rust。** `src/indexer.js:14` 的 `SOURCE_EXT` 只有
   `.ts .tsx .js .jsx .mjs .cjs .py`。而主力 repo 的組成:finance-engineering rs=72/ts=0、
   claw-skills rs=162/ts=0、ft rs=63/ts=8。**唯一 TS 重的 repo 是 cct(181 個 TS 檔)——
   評測場域選它是因為別無選擇,不是因為那是工作的地方**(cct 沒有自己的 session 目錄,
   但常被 claw-skills 的 session 從外面操作,271 次)。

## 2. 方法

- 資料:`~/.claude/projects/` 底下兩個最大的 repo transcript。
  finance-engineering 245 MB / 764 檔;claw-skills 47 MB / 57 檔。
- 真人輸入:`promptSource ∈ {typed, queued}` 且 `message.content` 為字串的事件
  (排除 tool_result、system 注入、sdk)。
- 成本:每個 tool_result 的內容字元數(ASCII/4 + 非 ASCII×1 估 token,與
  `evals/agent-stream.mjs` 同一個估算器),時間用 `tool_use` 與 `tool_result` 的
  timestamp 差(上限 30 分鐘,排除跨 session 斷點)。
- 分類:非 Bash 工具按工具名;Bash 按 command 內容歸類(delegation=觸及 codex/grok/kimi/agy
  runtime,cargo,python heredoc,rg/grep,ls/cat/find,git,其他)。

## 3. 真實成本(finance-engineering + claw-skills 合計)

**14,731 次工具呼叫、約 4.27M tokens 的工具輸出、34.8 小時的工具時間。**

| 類別 | ~tokens | % tok | 時間 | % time | tok/次 |
|---|---:|---:|---:|---:|---:|
| Read | 1,417k | **33.2%** | 3 分 | 0.2% | 1,328 |
| rg/grep | 690k | 16.1% | 3.1 hr | 8.8% | 262 |
| ls/cat/find | 521k | 12.2% | 4.7 hr | 13.5% | 227 |
| bash other | 407k | 9.5% | 1.6 hr | 4.6% | 342 |
| delegation | 308k | 7.2% | 4.0 hr | 11.4% | 294 |
| cargo | 224k | 5.3% | **6.5 hr** | **18.7%** | 168 |
| python heredoc | 204k | 4.8% | 1.4 hr | 4.0% | 175 |
| WebSearch | 105k | 2.5% | 0.5 hr | 1.5% | 718 |
| git | 92k | 2.1% | 0.4 hr | 1.3% | 237 |
| AskUserQuestion | 13k | 0.3% | **7.9 hr** | **22.7%** | 76 |

兩個黑洞是**不同的東西**,排序也不相關:

- **token 的黑洞是「找檔案 + 讀檔案」:Read + rg/grep + ls/cat/find = 61.5%(約 263 萬 tokens)。**
- **時間的黑洞是等待與建置**:等使用者回答 7.9 hr、cargo 編譯 6.5 hr、翻檔案 4.7 hr。

按次數排序會嚴重誤導:delegation 的呼叫次數排前面(1,046 次),但按成本只排第五
(7.2% tok / 11.4% 時間)——`ps | grep` 幾乎不回傳東西,`Read main.rs` 一次就是 27k tokens。

另外:Glob 原生工具總共只被呼叫 6 次、Grep 79 次,同樣的事經由 Bash 做了 4,919 次。

## 4. 可回收的部分

**1. 讀「一段」而不是讀「一個檔」— 最大的單一機會(33.2% 的工具 token)。**

Read 有 36%(379/1,067)是重讀 session 內讀過的檔案;單次最大的讀取:

```
27,431 tok  finance-engineering/tools/finance-cli/src/main.rs
14,272 tok  gwebcdb/crates/bridge-core/src/login.rs
14,166 tok  finance-cli/src/chart_cmd.rs   ← 同一檔至少整讀 3 次
```

`cort context --budget 1500` 做的正是按符號取片段,而且**不需要 graph**——但它不支援 Rust。
把「按符號/行範圍取片段」帶到 Rust,是資料支持的第一優先。

**2. 讀過就別再讀 — 純浪費 124k tokens。**

重讀拆開:改過之後重讀 161 次(合理);**沒改過就重讀 218 次 ≈ 124k tokens(純浪費)**。
這是 harness 層級的問題,不需要新工具。

**3. delegation 生命週期 — 第三順位。**

每 1 次派工配 7.5 次照顧(status/log/ps/cancel),`Task ... is still running` 硬阻塞 241 次
(global CLAUDE.md 那段警告的存在本身就是證據)。成本是真實的但排第三:7.2% tok、11.4% 時間。

## 5. 對既有紀錄的更正

- WIP 筆記 §2.3 寫「`getCurrentTimeET` → 8 dependents」是錯的:8 是 `expected_symbols` 的長度,
  不是工具輸出。實測:新索引(cct `86a5ee6`)→ 23,舊索引(8/26)→ 20。
  本輪已用 `verify-impact.mjs`(precision 1.0)重新生成 `evals/tasks-graph.json` 的標籤,
  並釘在 cct `86a5ee6`;同時發現並修掉舊 `by_hop` 未去重的問題(66 列 vs 44 符號)。
- WIP 筆記與 `30a14d87` 講的「probes 合計 < $0.40」是 **API 名目等值**,不是實際支出——
  憑證是 `claudeAiOauth`(Pro 訂閱),沒有 API key。評測的代價是訂閱額度與速率限制,不是帳單。

## 6. 留下來的東西

- `evals/agent-stream.mjs` — 補齊三輪 null 的 stream-json 解析器,含「指標非數字即丟例外」。
  之後任何評測都用得上。
- `evals/run-agents.mjs` + `grade.mjs` — 實作了 WIP 筆記 §4 的兩臂規格,**一格都沒跑過**。
  其「rg vs cort 打擂台」的框架已被本文取代;程式留著,因為 runner 的骨架(隔離設定、
  白名單即實驗組、cwd=venue、非 null 斷言)對任何下一輪評測仍然正確。
- `evals/tasks-graph.json` — 標籤已修正並綁 venue HEAD,角色從「答案卷」變成
  「檢查表」(查改動漏了哪個呼叫端)。

## 7. 接下來(按資料排序,不是按興奮程度)

1. **Rust 的按符號取片段**(ast-grep 支援 Rust;先驗證在 finance-cli 的 main.rs 這種
   27k-token 檔案上能省多少)。
2. **消除無謂重讀**(harness 層)。
3. delegation 等結果原語(第三順位)。
4. 圖/schema(§6.2 call-site line)——**擱置**,直到上面 1–3 做完後重新量測還有剩餘需求。
