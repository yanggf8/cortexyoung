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

## 攔截結果（2026-10-29 15:05–15:26 台灣時間，NUC11i5）

兩個 config 先備份（SHA-256 記下），各加暫時 entry，攔完以備份覆蓋，雜湊與攔截前相同。Codex 那段期間的其他寫入
沒有逐行比對：還原前看到的差異只有檔尾 entry 與核准寫入的 `[hooks.state."…:session_start:0:0"]`。

- **Kimi Code 2.1.1**（`kimi -p`、`kimi -r <id> -p`）：
  - SessionStart：`{"hook_event_name":"SessionStart","session_id":"session_…","cwd":…,"client_type":"kimi_code_cli","source":"startup"|"resume","model":"kimi-code/k3.1-pro","profile":"agent"}`。
  - UserPromptSubmit：`{"hook_event_name":"UserPromptSubmit","session_id":…,"cwd":…,"client_type":"kimi_code_cli","prompt":[{"type":"text","text":…}],"is_steer":false}`。
  - resume 沿用同一個 `session_id`。
  - **注入實測**：UserPromptSubmit hook 印 `{"message":"cort probe: the code word is PAPAYA-7"}`、exit 0，問模型 hook 給了什麼
    暗號，答 `PAPAYA-7`。`PostCompact` 這次沒有觸發（沒有發生壓縮），還沒攔到。
- **Codex 0.164.0**（`codex exec`、`codex exec resume <id>`）：
  - SessionStart：`{"session_id":…,"transcript_path":…,"cwd":…,"hook_event_name":"SessionStart","model":"gpt-6.1-terra","permission_mode":"default","source":"startup"|"resume"}`，
    欄位與 Claude Code 相同。呼叫者命令列是那次 `codex exec …`。
  - 改了 `command` 的 entry 在重新核准前不執行（payload 檔數不變、模型答 NONE）：信任綁 entry 內容，所以正式接線時
    使用者要在 Codex 裡核准一次。
  - **注入實測**：hook 印 `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"…MANGO-3"}}`，模型答
    `MANGO-3`；把 payload 原樣交給現有的 `cort hook-recall --harness codex`，在 tpm repo 問模型開場拿到哪些 cort memory
    key，答出該專案 ledger 最新的 5 個 key。現有 hook-recall 不用改就能服務 Codex。
  - 一次在 tpm repo 的 `codex exec` 沒有觸發 hook 也沒有輸出，原因沒查到；同樣的指令加上 `< /dev/null` 重跑就正常。
    正式接線後若再看到「沒注入」，先查這個。

## 進度

- **B1 Codex（`53d65f9f`、`d058ac00`，2026-10-29 16:2x 台灣時間部署 NUC11i5、Thinkpade15）**：`settings::events_for`
  給 Codex `Recall`，TOML 方言寫不帶 matcher 的 `[[hooks.SessionStart]]`。使用者 17:1x 在 Codex 核准後，
  `--check` 由 `NOT TRUSTED` 轉 `wired`；tpm repo 實測 hook 有跑，但那次讀共用資料庫逾時，模型只拿到逾時通知。
  Thinkpade15 沒裝 codex。
- **B2 Kimi（`db551c93`，同日部署兩台）**：Kimi 的 `Recall` 寫成 `event = "UserPromptSubmit"`、無 matcher；
  `hook-recall` 依 `session_id` 的 SHA-256 在記憶資料目錄 `recall-sessions/` 下 create-new 一個標記檔，第一個
  prompt 注入 `{"message"}`，之後的 prompt 不讀設定也不連網路（`already_injected`）。tpm repo 實測：新 session 列出
  11 個 ledger key，`--continue` 的第二個 prompt 沒有再注入。
- **逾時（`36524521`，同日部署兩台）**：閒置後第一次讀取在三個 harness 的 session 開始各逾時一次，熱讀約
  300–400 ms，30 天統計 53 次裡 5 次 `remote_timeout`。總期限 2500 → 4500 ms（`remote::RECALL_DEADLINE`），
  網路逾時預設 1500 → 4000 ms、上限 2400 → 4400 ms，watchdog 改在期限後 200 ms（寫出時再 300 ms），總長仍在
  harness 的 5 s 內。harness 的 5 s 從程序啟動算、watchdog 從 `main()` 算，啟動本身卡住（編譯中見過一次 5 s）
  不在程式能管的範圍。
