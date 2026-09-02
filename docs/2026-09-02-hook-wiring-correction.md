# 挖掘管道的線是斷的(2026-09-02)——證據、修正、以及 09-04 的窗要重算

> **一句話**:`docs/2026-09-01-adoption-mining-interim.md` 的漏斗數字在**預設 cache 與全域
> settings 上重現不出來**,因為 hook 根本沒有接在 `~/.claude/settings.json` 裡;09-02 09:24 起
> 由 `install.sh` 一併部署後才第一次成立。09-04 重挖的累積窗從 **2026-09-02 09:24** 起算,
> 不是 09-01 13:12。

## 1. 觀察到什麼(2026-09-02 09:00-09:30,本機)

四條各自獨立、都可重跑:

| 檢查 | 結果 |
|---|---|
| `~/.claude/settings.json` 的 `PreToolUse` | 只有 `mos hook`,**沒有** `cort hook-suggest` |
| `$HOME` 下所有 `.claude/settings*.json`(含各專案) | 零個檔案提到 `cort` |
| `~/.cache/cortex-ng/usage.db` | birth `08-31 11:12:43`,**09-01 全天 0 列**(id 6 是 `08-31 14:28`,id 7 是 `09-02 09:03`),`hook-suggest` 0 列 |
| `~/.claude/projects/**/*.jsonl` 搜注入文字 | 只命中本 session 自己的 transcript(因為讀了 `main.rs`) |

`usage.rs` 的 `RETENTION_DAYS = 90`,所以「被 prune 掉」不是解釋。settings.json 的備份只有
`settings.json.bak-20260825`(hook 之前)。

## 2. 這對 09-01 那份讀數的意思

09-01 的中期報告與 `2026-08-31-recall-wip.md` §6 都寫著 hook 已全域生效
(「hesocial 281 次 grep/rg、ft 164…」、「基線:預設 cache `hook-suggest ok=485`」)。那些數字
**不在**這台機器的預設 cache 與全域 settings 上。能確定的只有兩件事:

1. 它們來自**另一份 cache 或另一份設定**——09-01 文件自己就標註過稽核跑落在 scratchpad cache
   (全機現在只找得到一個 `usage.db`,scratchpad 已清);
2. 無論當時是什麼狀態,**09-02 早上線是斷的**,照原協定在 09-04 重挖會挖到 0。

不宣稱 09-01 的數字是錯的——沒有證據支持那個更強的說法。宣稱的是:**它不可重現,所以不能當基線**。

## 3. 為什麼會斷:hook 從來不在安裝路徑上

`install.sh` 裝 binary、裝 skill(Claude + Codex 兩個 home)、寫 manifest,**但從不碰
settings.json**——git log 沒有任何 hook 安裝的 commit。skill 是宣告式、可稽核、部署後不會漂;
hook 是手接的,所以它漂了。一個要靠人記得去接的路由,就是一個沒接上的路由。

這也正是 skill 自己的失效模式在另一個層次上重演:`rust/src/hook.rs` 開頭記著,帶著 skill 的
session 裡 409 次搜尋、`cort` 呼叫 0 次——prospective 的那半靠模型記得,retrospective 的那半
靠人記得,兩半都靠「記得」。

## 4. 改了什麼

**hook 跟 skill 同一次部署**(`install.sh` → `deploy_hook` → `cort hook-install`)。JSON merge
的邏輯在 `rust/src/settings.rs`,不在 bash:保留使用者既有的每一個 hook、跨重裝 idempotent、
binary 搬家時改寫既有 entry 而非新增第二筆、看不懂的 settings.json 拒絕覆蓋——這些是邏輯,
一段沒有測試的 `jq` pipeline 會是第二份實作,而 `jq` 也不是 installer 原本的依賴。
entry 配 `matcher: "Bash"`,因為 `hook-suggest` 只讀 `tool_input.command`。
`--uninstall` 會在刪 binary 前先解線,`--no-hook` 可跳過,`--check` 用唯讀的 `--status` 回報。

**gate 從「db 檔存在」改成「`indexed: true`」**(`rust/src/main.rs` `project_is_indexed`)。
開一個專案就會建 schema,所以一個 0 chunk 的 db 也能通過檔案存在測試,hook 於是在
`impact` 只能回 `no_seed_resolved / stale=true` 的樹上宣稱「cort has an index for this
project」。這是 `cmd_hook_suggest` 自己的 doc comment 早就禁止的失敗,而它在 09-02 是活的。

測試:`rust/tests/settings.rs`(9 條,含「使用者原有的 hook 全數存活」「重裝不重複」「拒絕壞
JSON」)、`rust/tests/cli.rs` 兩條 gate 迴歸(空 index 沉默 / 已 index 才出聲)、
`tests/install-smoke.sh` 四條(與 skill 同一次部署、matcher 是 Bash、manifest 記 `hook_settings`、
uninstall 解線)。全樹 317 + smoke 91,零失敗。

## 5. 09-04 重挖時的實際基線

- **累積窗起點**:2026-09-02 09:24(`hook_settings` 寫入 manifest 的時刻),不是 09-01 13:12。
- **cache**:`~/.cache/cortex-ng/usage.db`,birth 08-31 11:12,`hook-suggest` 第一列
  `09-02 09:08`(手動驗證),第一列真實觸發 `09-02 09:24:26`。
- **index 覆蓋面**:13 個專案已建索引(cc-router、cct、claudecat、cortexyoung、dac、fortuneT、
  ft、mos、zencat、AttentionOS、ainews、gwebcdb、tsheet)。gate 修好之後,只有這 13 個會出聲
  ——這同時是機會母體的定義,挖掘時要一起報。
- **配對法**:`2026-08-31-recall-wip.md` §6 的 `hook_additional_context` +
  `hookName` 以 `PreToolUse` 開頭 + 注入文字帶 `cort impact --symbol '` 這個歸屬標記。配對鍵
  已從 `parentUuid` 鄰接換成 `attachment.toolUseID`——真實紀錄上這個欄位直接指名 hook 掛在哪
  一次 tool_use 上,比鄰接嚴格。
- **先驗證再挖**:`./install.sh --check` 必須印 `hook: ... (wired)`。這一行就是為了讓
  「線斷了」不能再無聲發生。
- **挖掘是指令,不是散文**:`cort-evals adopt-mine --since 2026-09-02T09:24:00+08:00`。
  `--since` 拒絕不帶時區位移的時間——本協定第一次手工執行就是把本地 `09:24` 當 UTC 讀,截止點
  落在未來,漏斗全回 0,錯得像 hook 又斷了。報告的 `usage_db_cross_check` 是第二個獨立來源:
  `hook-suggest` 的 usage 列現在記 outcome(`hit` / `no_index` / `no_shape` / `no_payload`),
  `hit` 少於 `injections` 就代表有第二份 cache 在跑,也就是 09-01 基線遺失的那個方式。
  `legacy_unsplit` 是舊 binary 寫的列,不歸任何一邊;交叉驗證要生效,必須先 `./install.sh`
  部署帶 outcome 的 binary。

## 6. 附帶更正

hook 不需要重開 session 才生效:09-02 09:27 在改設定之後的同一個 session 裡,一次
`grep -rn "canonicalize_root("` 就觸發了注入,usage.db 同步寫入。設定是即時重讀的。
