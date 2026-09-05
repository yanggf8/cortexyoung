# PostToolUse 送來的到底是什麼(2026-09-05)

`hook-refresh` 接在 `PostToolUse` 上一年多,而**沒有人攔截過它收到的 payload**。這個庫唯一記錄過的
payload 是 `PreToolUse`/Bash 的那一份(`2026-09-03-installer-dedup-and-attribution.md` §12),而
Codex 會對檔案編輯觸發 `PostToolUse` 這件事,是靠**哪一個 hook 保持沉默**推斷出來的,不是讀出來的。

CLAUDE.md 對此有明文:**只能從攔截到的 payload 判斷 matcher,絕不能從 transcript 判斷**。同一條規則
適用於欄位名。所以在任何程式碼讀 `tool_input.file_path` 之前,先把它攔下來。

## 攔截方法

`cmd_hook_refresh` 在讀完 stdin 之後、做任何事之前,檢查 `$CORT_CACHE_DIR/hook-capture` 這個標記檔;
存在就把 payload 逐行追加到 `hook-payloads.jsonl`。**臨時措施,同一個 task 內移除。**

計畫原本寫的是環境變數。執行時才發現行不通:**harness 的 hook 繼承的是它自己啟動時的環境**,session
內改不了。而改寫已接線的命令去傳旗標,在 Codex 上要付一次真人重新審核的代價
(`2026-09-03-…` §12-13:`trusted_at` 只問那個位置有沒有雜湊,問不出雜湊是否對應現在那條命令)。標記檔
兩者都避開。

## Claude Code

三個工具,同一個 session,識別碼已抹除。

**頂層欄位(三者相同):**

```
cwd, duration_ms, effort, hook_event_name, permission_mode, prompt_id,
scratchpad_dir, session_id, tool_input, tool_name, tool_response,
tool_use_id, transcript_path
```

`hook_event_name = "PostToolUse"`,`cwd = /home/yanggf/a/cortexyoung`。

| `tool_name` | `tool_input` 的鍵 | 路徑欄位 |
|---|---|---|
| `Write` | `content`, `file_path` | **`file_path`,絕對路徑** |
| `Edit` | `file_path`, `new_string`, `old_string`, `replace_all` | **`file_path`,絕對路徑** |
| `Bash` | `command`, `description`, `timeout` | **無** |

`MultiEdit` 與 `NotebookEdit` 這個 session 沒有觸發到,**未攔截**。matcher 涵蓋它們
(`rust/src/settings.rs:82`、`:106`),所以實作必須容忍它們的欄位名不同,而不是假設。

`Write` 的 `tool_response` 另外帶 `filePath`(駝峰)、`originalFile`、`structuredPatch`、`type`、
`userModified`。**不要讀它** —— `tool_input` 是請求,`tool_response` 是結果,兩者在失敗的編輯上會分岔。

## 兩件計畫沒預料到的事

**一 · 路徑是絕對的。** 計畫花了篇幅處理相對路徑要對誰解析,並為此寫了一個測試。對 Claude Code 而言
那個問題不存在。測試仍然值得留著 —— 另外兩個 harness 未攔截,而一個送相對路徑的 harness 會在
production 靜默失敗、測試全綠。

**二 · payload 自己帶 `cwd`。** 計畫的 fallback 是問 process 的 `cwd()`,但 harness 已經在 payload
裡說了。兩者在正常情況下相同;不同的情況是 hook 被從別處呼叫。**優先讀 payload 的 `cwd`**,因為那是
harness 對「這次工具呼叫發生在哪」的宣告,而 process cwd 只是它碰巧繼承到的東西。

## 未攔截的

- **Codex**、**Kimi** 的任何工具
- `MultiEdit`、`NotebookEdit` 的任何 harness

實作對這三類必須降級成「沒有路徑」而走 cwd,不能猜欄位名。等有人在那些 harness 裡工作時,把標記檔
打開再收一次。
