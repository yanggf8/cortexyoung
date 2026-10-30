# Codex 的 `~/.codex/hooks.json`：external-agent import-sync 副本（2026-09-29）

## 事實

在本機（macOS，Codex 桌面版 153.0.8010.53，2026-09-23 build）觀察到：

1. **`~/.codex/hooks.json` 不是死檔。** `docs/2026-09-02-hook-wiring-correction.md` §12 記的
   「兩個位置都試過、靜默不載入」對 codex-cli 0.152.1 成立，對這個桌面 build 不成立：Codex
   為 hooks.json 的條目寫了 `[hooks.state]` trust rows（連 `session_start`／`user_prompt_submit`
   這些只在 hooks.json 出現的事件都有），寫 trust row 的前提是它載入了那些 hook 並彈過審核。
   二進位的 string table 也有 `codex.external_agent_config.import`、
   "failed to serialize hooks.json"。
2. **這個檔案是 Codex 自己生成的。** 桌面版的 external-agent import-sync（config.toml 的
   `[desktop] external-agent-import-sync-enabled = true`）把 `~/.claude/settings.json` 的整個
   hooks 表照抄過來：本機實測 10 條（2 條 cort，帶 `--harness claude-code` 的錯 flag；
   8 組 iterm2 cc-status），每一條都能在 settings.json 裡找到逐字相同的原件。
3. **後果是雙重觸發。** `config.toml` 管理的條目（`--harness codex`）加上 hooks.json 的副本
   （`--harness claude-code`），每次 Bash 觸發兩次 `hook-suggest`、每次編輯兩次
   `hook-refresh`，第二份在 usage.db 裡歸因到錯的 harness。

## 順帶發現的 `--status` 假信任

`settings_toml::trusted_at` 原本只比 `:<event>:<gi>:<hi>` 尾綴，而
`hooks.json:pre_tool_use:0:0` 與 `config.toml:pre_tool_use:0:0` 尾綴相同 —— 一份 Codex 給
**另一個檔案**條目的信任，會替 `config.toml` 的條目背書。2026-09-29 起尾綴錨定到設定檔自己的
basename（`config.toml:pre_tool_use:0:0`），basename 之前的路徑拼寫差異仍然不比較（§7/§9 的
原則不變）。回歸測試：`a_hooks_json_trust_row_no_longer_vouches_for_config_toml`。

## 改法

`cort hook-install --all`（`rust/src/settings_sidecar.rs`）在接線之後清 sidecar：

- **ours 一律移除**（`is_ours_for` token rule —— 這也是 sidecar 裡那些 `--harness claude-code`
  條目會被認出的原因）。
- **foreign 只在整份檔案可證明是副本時移除**：每個 foreign 條目都要能在現行的
  `~/.claude/settings.json` hooks 表裡（同 event、同 matcher、結構相等）找到對應；任一條比
  不上，整批留下、報告 `foreign_all_copies: false`。「相同」是 `serde_json::Value` 相等而非
  byte 相等 —— Codex 抄寫時會重新序列化。
- **`[desktop].external-agent-import-sync-enabled` 翻成 false**（使用者核可的範圍）；不然
  清掉的副本會被同步塞回來。若使用者在 Codex UI 重開 sync，下次 install 的 sweep 會再清
  （只要簽章規則仍成立，這是自癒的）。
- **trust 孤兒清理**：sidecar 被寫過之後，它的 `[hooks.state]` rows 全數移除 —— 條目移走後
  索引位移，留下的 row 不是孤兒就是替「滑進同一個位置的其他條目」背書。沒被動過的檔案
  （config.toml 自己、plugin）的 rows 不碰。
- **Codex 桌面程式在跑時整個 sweep 跳過**（回報 `codex_running`）：它離開時會整檔重寫
  config.toml，sync 也能在掃描當下把副本塞回來。`CORT_CODEX_RUNNING=0|1` 是測試用的覆寫。
  偵測用 `pgrep -f` 只匹配桌面程式的 helper（`Codex Framework.framework`、`Codex \(Service\)`、
  ChatGPT.app 主程式）；bare `codex` CLI 不算 —— agent 常駐跑 `codex exec`，它不重寫
  `[desktop]`。

`--remove` 對 sidecar 只清 ours、不動 sync；`--status` 唯讀回報 sidecar 狀態；`--lean` 的
六欄 TSV 一個位元組都不變（`install.sh:check_all_hooks` 逐欄消費），sweep 資訊只走 JSON。

## 沒有做的事

- **沒有刪 sidecar 檔案本身。** 清空的檔案留 `{}`：檔案是 Codex 的，刪掉一個 sync 想重建的
  檔案，比留一個空檔更難收拾。
- **沒有改寫 §12 的歷史文件。** 更正記在 `settings_toml.rs` 的模組文件與本文，原文件保持
  當時為真的樣子。
- **沒有動 Kimi 與 Claude Code 側。** 本機檢查過 `settings.local.json` 與專案層設定都沒有
  副本；這個 import-sync 機制目前只在 Codex 側看到。
