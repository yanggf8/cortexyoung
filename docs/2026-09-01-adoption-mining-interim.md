# 採用數據早期讀數(2026-09-01,累積窗 ~10 小時)— 中期報告

> **狀態:數據不足以判斷。** 本文件是 [issue #1](https://github.com/yanggf8/cortexyoung/issues/1)
> 挖掘協定的**管道驗證**,不是 09-04 的正式讀數。累積窗只有 2026-09-01 13:12 → 23:06(約 10
> 小時);issue 開放中,09-04 照同一協定重挖,那才是交付物。
>
> 執行指令(可重現):`docs/2026-08-31-recall-wip.md` §6 的配對法,掃
> `~/.claude/projects/*/*.jsonl`(排除 `agent-*`,mtime ≥ 2026-09-01 13:00)。

## 漏斗數字

| 階段 | 值 | 來源 |
|---|---|---|
| 活躍 session | 7(cc-router、cortexyoung、ft×2、hesocial、travel-2026、home) | transcript |
| hook 觸發(PreToolUse:Bash,grep/rg) | **745** | transcript `hook_success` attachments |
| hook-suggest 執行 | **698** | `usage.db`(預設 cache) |
| 呼叫點形狀機會(注入) | **2**(都在稽核 session) | transcript `hook_additional_context` |
| 注入 → `cort impact` tool_use(採納) | 2 中的 **1** | 同上;另 1 筆是刻意實火的觸發測試,非自然採用 |

兩個獨立來源交叉:transcript 745 vs usage.db 698,差 7%(usage 是 7 天窗含 13:00 前的觸發,
transcript 是 13:00 起)——一致。

## 讀數(老實版)

1. **管道端到端可用**:觸發有記錄、注入有結構化 attachment、配對確定性、採納判定可稽核。
   這 10 小時驗證的是量測機制,不是採用率。
2. **真實工作 session 的 745 次 grep/rg 裡,0 次呼叫點形狀機會。** 樣本太小,連「機會率與
   1/200 模型一致」都還不能說——0/745 與模型(期望約 3-4 次)的差異需要更多天才有意義。
   唯一能下的結論:**與「罕見」一致,沒有任何機會被略過的證據。**
3. 唯一的自然採用樣本仍不存在(2 筆注入都在稽核 session,其中 1 筆是刻意的觸發測試)。
4. 附帶發現:impact 的 usage 歸屬要注意 cache——稽核跑的部分落在 scratchpad cache,預設
   cache 只有 14 列;跨 cache 合計才是全貌。

## 09-04 重挖時看什麼

- 機會數(注入)是否隨天數累積到可判讀的量(≥5 筆才有採用率的意義);
- 有注入的 session 裡,採納與否的**裁決**(照 demand screen 的紀律逐筆看);
- `--coverage` 的 gap 列有沒有實際改變過結論(效果,不只是採用);
- stale=true 有無被忽略。
