# Token cost — 台灣中文版

> 範圍說明:這一份只翻譯 `README.md` 的「Token cost」一節,不是整份 README 的翻譯;
> 數字與英文版一致,完整證據與歷史逐 cell 資料請見英文版
> (`README.md` 的 `Eval results`、`Re-analysis (2026-08-28)`、`Demand, re-measured`
> 各節)。

`cort` 在「問的是呼叫端集合」這種問題上才便宜。機制:一次遞迴 SQL 走訪,取代 agent
原本「多跳 `rg` + 開檔閱讀」的迴圈——所以成本幾乎不隨深度成長,而 grep 路線每跳一層
都要把搜到的名字再搜一次、每個 hit 都要開檔才知道誰包住它(`rg` 成本裡閱讀佔
83–87%)。

無模型的成本探針(歷史數字,出自 `evals/relation-cost.mjs`):

| 跳數 | `cort impact -f lean` | `rg` + 開檔讀到同一集合 | 比值 |
|---|---|---|---|
| 1 | 968 tok | 16,584 tok | 14.8x |
| 2 | 1,022 tok | 86,949 tok | 67x |
| 3 | 1,136 tok | 127,531 tok | 62x |

端到端 agent 評測(2026-08-30,5 個需要關聯走訪的任務 × 2 臂 × 2 輪 = 20 cell):
`cort` 臂 **10/10 成功、平均 992 tool-return tokens**,對照臂 **4/10、7,642**——
換算每 cell **$0.28 對 $0.79(2.8x)**,機制是 **7.7x 更小的工具 payload** 加上
**約 4x 更少的回合數**。

兩個數字必須連在一起讀:一是 payload 比值不是帳單(cache_read 主導帳單,所以誠實的
標題是成對的:2.8x 更便宜、10/10 vs 4/10 成功);二是 `cort` 只有在真的需要關聯走訪
時才划算(1,214 筆真實指令裡只有 0.08% 是這種問題,見英文版「Demand, re-measured」)。
請把這些數字讀成「這個工具在它實際運行的 harness 裡至少值多少」的下限,而不是一場
受控的「cort vs rg」比較。完整證據:英文版的「Eval results」、「Re-analysis
(2026-08-28)」、「Demand, re-measured」各節,以及
`docs/2026-08-28-real-session-cost.md` 和 `evals/runs/2026-08-30-graph{,-sample2}/`。

量到最便宜的支援情境是 Rust symbol slice:`cort context <symbol> --content full -f lean`
測得 **27k → 89 tokens**(`docs/2026-08-28-real-session-cost.md` §1.3)。

**這份便宜怎麼長期維持:** 每次呼叫都會在本機 `usage.db` 追加一列(見英文版
「Local usage recording」),每次 hook fire 也都記錄它達到的結果;`cort-evals` 用同一個
`judge` 重播真實 transcript(`hook-probe`)、數「關聯走訪真的被需要幾次」(`demand`)、
並對照檔案文字逐一為每條邊打分(`verify-impact`)。新功能要先贏過 token 數與成功率
才會放行(`evals/README.md`)——每個「變好了」的宣稱都能對一個 commit 重算。
