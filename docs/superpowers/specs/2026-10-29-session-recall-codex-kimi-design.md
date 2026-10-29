# Codex 與 Kimi 的 session 開場 recall

日期：2026-10-29。分支 `measure`。延續 `cort hook-recall`（Claude Code SessionStart，`774dbfa7`）：Codex 與 Kimi
還沒接，因為 AGENTS.md 的規矩是「沒攔到那個 harness 的 payload 就不接」。這份先記下兩個 harness 的實際行為，
再定攔截與接線的步驟。

## 兩個 harness 的事實（從本機安裝的程式讀出，尚未經實際 payload 證實）

- **Codex 0.164.0**：hook 事件有 `SessionStart`，binary 裡有 `SessionStartHookSpecificOutputWire` 與
  「this event cannot emit additionalContext」的錯誤字串，表示 `SessionStart` 是能回 `additionalContext` 的事件之一，
  輸出格式應與 Claude Code 相同（`hookSpecificOutput.additionalContext`）。新加或改過的 hook 要使用者在 Codex 裡
  審核過才會執行（binary 字串「hooks need review before they can run」），所以攔截一定要使用者在場核准一次。
- **Kimi Code 2.1.1**（`@moonshot-ai/kimi-code`）：`HOOK_EVENT_TYPES` 有 `SessionStart`，但
  `triggerSessionStart` 只 `await this.runner.trigger("SessionStart", …)`，**回傳值丟掉**：SessionStart hook 的輸出
  進不了模型的 context，只能當通知。能進 context 的是 `UserPromptSubmit`：`runPromptSubmitHook` 把非 block 結果的
  `message`（stdout 為 JSON `{"message": …}` 或 `hookSpecificOutput.message`，exit 0）用 `renderUserPromptHookResult`
  附加成一則 user 訊息；exit 2 或 `permissionDecision: "deny"` 會擋下這次 prompt。它**每個 prompt 都觸發**。

## 做法

1. **先攔 payload，不接線。** 兩個 harness 各加一條暫時的攔截 entry（Codex `SessionStart`、Kimi `SessionStart` 與
   `UserPromptSubmit`），只把 stdin 原樣存到 scratchpad、輸出空白、exit 0。Codex 需要使用者在 TUI 核准一次。各開一個
   session 打一句話就關，讀存下的 payload：欄位名（`cwd`、`session_id`／`sessionId`、`source`、`hook_event_name`）、
   `source` 的值（startup／resume／clear／compact 各自是什麼）、`model` 有沒有帶。攔完把 entry 拿掉，config 逐位元組
   還原（與 `hook-install` 的 uninstall 測試同一個標準）。
2. **Codex：與 Claude Code 同一條路。** payload 欄位對得上就把 `SessionStart` → `cort hook-recall --harness codex`
   加進 `settings::events_for` 與 `HOOK_TARGETS`，輸出沿用 `additionalContext`；對不上就在 `hook-recall` 的 payload
   解析加 Codex 方言，不分叉判斷邏輯。
3. **Kimi：`UserPromptSubmit`，每個 session 只注入一次。** SessionStart 那條路不通，所以改在第一個 prompt 注入：
   `hook-recall --harness kimi-code` 在 `UserPromptSubmit` 上以 `{"message": …}` 輸出同一份 ledger 摘要，並在本機記下
   「這個 session 已注入」，之後同一 session 的 prompt 一律空白輸出、exit 0。永遠不 exit 2、不回 deny（那會擋掉使用者的
   prompt）。記號存在 `usage.db` 旁的本機狀態，不寫共享資料庫；session id 取自攔到的 payload。壓縮後重新注入要看 Kimi 的
   `PostCompact` 輸出會不會進 context，攔截時一起確認。
4. 每一步照常：實作 → 我逐行審 → 審查（Muse 回來前由 K3.1 Pro）→ gate → 提交推送 → CI 綠 → 部署 → 實機各開一次
   session 看到 ledger。

## 不做

- 不用 Kimi 外掛的 `sessionStart.skill`：那是靜態 skill 內容、只對 Kimi 有效，會多一個 hook 擁有者。
- 不在攔到 payload 之前改 `HOOK_TARGETS`（AGENTS.md：judge a matcher only from an intercepted payload）。
