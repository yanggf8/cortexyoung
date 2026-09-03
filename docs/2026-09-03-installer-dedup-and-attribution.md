# 安裝器的第二份實作，以及一份數字說不出自己是哪台機器產的（2026-09-03）

接續 `2026-09-02-hook-wiring-correction.md`。那份文件講的是「hook 接了但不會跑」的十八種形狀；
這一份講的是**同一條規則有兩份實作，而出貨的那份不是有測試的那份**，以及它連帶暴露的兩個歸屬缺口。

## 1. 起點：安裝的 binary 落後到不認得自己的子命令

`./install.sh --check` 六行全部是 `installed cort predates hook-install`。部署的 binary 停在 9-02，
六個 hook 條目（3 harness × 2 event）一個都沒接上。重建部署後六條全 wired。

這件事本身不有趣，有趣的是它讓下一件事現形。

## 2. `--status --format kimi` 回報的是 Claude Code 的檔案

```
$ cort hook-install --status --format kimi
{ "settings": "/home/yanggf/.claude/settings.json", "wired": false }
```

`cmd_hook_install` 先解析路徑、再讀 `--format`：沒給 `--settings` 時一律預設 Claude 的
`settings.json`，然後拿 `--format` 指定的方言去讀它。於是 kimi 明明接著、firing 中，`--status`
說 `wired: false`——**與 §7／§9 同一個假陰性，換一條路徑再發生一次**。

修法是把順序倒過來：格式先決定，路徑由格式決定（`default_settings_path_for`）。

**為什麼沒人發現**：`install.sh` 永遠傳 `--settings`。Rust 那份預設解析從來不在出貨路徑上，
所以它爛掉了也沒有任何東西會失敗。

## 3. 真正的病因：四份重複，bash 那份在跑

舊的 `install.sh:542`，一行裡把 binary 已經知道的四件事全部再講一次：

```bash
"$cort_bin" hook-install --settings "$path" $fmt_arg --event "$ev" \
            --command "$cort_bin $verb --harness $harness"
```

| bash 講的 | Rust 已經擁有的 |
|---|---|
| 三個 harness 的設定路徑（`install.sh:40,47,51`） | `settings{,_toml,_kimi}::default_settings_path()` |
| `--format kimi` ↔ `--harness kimi-code` 的配對 | 無人檢查兩者一致 |
| `ev=post` → `verb=hook-refresh` | `HookEvent::subcommand()` |
| `sed -n 's/.*"change"...'` 重剖自家 JSON | `Outcome::change` |

這正是 CLAUDE.md 對 `judge` 立的規矩（「a second copy of the decision makes hook-probe's
calibration describe something other than what ships」）長在另一個器官上。**解析器可以有很多份，
表只能有一份。**

### 改法

`cort hook-install --all` 擁有 `HOOK_TARGETS`，自己解析六個條目，`--lean` 輸出每列一行 TSV。
`install.sh` 部署、`--check`、uninstall 各一次呼叫；`deploy_one_hook` 與 `check_hook_at` 連同
裡面的 parser 一起刪除。**`install.sh` 裡的 JSON 剖析歸零。** manifest 現在記的是 binary 回報的
路徑——它回歸「發生過什麼的紀錄」，不再是解析規則的第二份拷貝。

### 兩個踩到的坑

**`--command-prefix` 必填，不得預設。** 單一 `--command` 的預設是 `current_exe()`，在安裝後的
佈局裡會解析成 shim 背後的真 binary，而 `--check` 驗的是 `~/.cargo/bin/cort` 這顆 shim。預設下去
每台機器的 `--check` 都會報 `WIRED TO A DIFFERENT BINARY`。只有 installer 知道要指名哪一顆。

**TSV 不得有空欄位。** tab 是 IFS *whitespace*，bash 的 `read` 會把連續 tab 摺成一個並丟掉中間的
空欄位：`a\t\tb` 讀成兩欄不是三欄。`detail` 為空時（claude-code、kimi）`command` 被讀進 `detail`，
六條錯四條、綠兩條——**只在某些列出錯**是最難發現的形狀。規則是「永不輸出空欄位」（空的送 `-`），
不是「在 bash 裡小心一點」。smoke 抓到的。

### 外部審查

用 grok CLI（`grok -p`，`grok-bridge.mjs show <run-id>` 取回）審這個計畫，判決是 **wrong as
stated**：擁有那張表是對的，但做成會預設 command、且只回傳一個 exit code 的 `--all` 是錯的形狀。
它列了六條必要修正，實作獨立命中五條（`--bin` 必填、安裝盡力而為、remove 以檔為單位、輸出旗標
不重用 `--format`、`--check` 逐列且 `trusted:false` 只報不擋）；第六條「`ok=0` 要傳得出去」本實作
用 here-doc 而非 pipe 所以沒中它警告的 subshell 陷阱，但**smoke 從來沒斷言過 `--check` 的離開碼**
——訊息對不等於契約對。已補兩條斷言。

它另外主張根本不該做 `--all`，改成 `--harness X --event Y --bin Z` 的六次呼叫保留行程隔離。
這個反對意見沒有被推翻，只是被權衡；記在這裡，不要假裝它不存在。

## 4. `hook-refresh` 丟掉了它收到的每一個參數

```rust
fn cmd_hook_refresh(_args: &[String], ...)   // 底線
```

`install.sh` 有傳 `--harness kimi-code` 進去，函式整包丟掉，然後寫一行
`outcome=refreshed reindexed=2`——**連 JSON 都不是**，與 `hook_args` 文件寫的「one parser reads
the whole column」直接矛盾。三個 harness 一接上，每次 reindex 都是匿名列，「Kimi 的 post-hook
到底有沒有在燒」在資料裡沒有答案。這是 `f3cb567f`（Grok 被記成 Claude Code）修好了 pre 事件、
從沒帶過來 post 事件。

已改成與 pre 同一套：`v` 版本列、記 harness、`transcript_path` 蓋過 flag、counts 收進物件裡。
讀取端加 `outcomes_of_hook_at(path, command, …)`，`hook_outcomes_at` 成為它的 `hook-suggest`
特化——兩個事件永遠不相加，但共用同一份 harness 歸屬規則。

## 5. router：harness 是對的，所以沒有東西能分辨

`kimicode` → `~/.claude-code-router/target/release/cc_claude` 啟動的是**真正的 Claude Code**，
只是把端點導向別的模型。同一個 `settings.json`、同一條 hook、同一個 `~/.claude/projects`，
所以 `harness_of` 回答 `claude-code` 是**正確的**——正因為它正確，這件事無法被察覺。

本機 `~/.claude/projects` 的 assistant 訊息裡約 2,167 / 7,290（30%）不是 Anthropic 模型
（`muse-spark-1.2-contributor` 773、`stealth/ox-alpha` 476、四種 `glm-5.*` 共 860、
`deepseek-v4-flash` 27、`k3` 16、`qwen3.5:4b` 15）。

分清楚受影響的範圍：**需求面的宣稱不受影響**（使用者問了什麼不因誰回答而改變）；
**行為面的宣稱被污染**（「409 次搜尋、0 次 cort 呼叫」「6/10 格多跳答錯」「重放 4,436 次搜尋」
都是「agent 怎麼行動」的宣稱）。

`hook_row` 加了 `model` 欄位（payload 沒帶就留空不猜）。**但要更正一個先前的說法**：
docs §12 列出 `model` 是在 **Codex** 的 payload 上觀察到的，Claude Code 的 payload 並不帶它——
161 筆真實 claude-code 列的 `model` 全是空。**所以 `model` 欄位不能當成 router 分辨機制**，
要分辨得從 transcript（`~/.claude/projects/*.jsonl`）走，那是 evals 的事。

`hook_models_at` 是**第二個鏡頭，不是把總計拆開**：「Claude Code 這個 harness 攔了幾次」是一個
有單一答案的真問題，不因為底下跑了幾種模型而消失。`a_model_breakdown_never_splits_the_harness_total`
釘住這件事。

## 6. 一份數字說不出自己是哪台機器產的

一份 hook 歸屬表顯示 417 次 Codex 觸發，被拿來與本機的 2 次並排比較。兩邊是不同的電腦，
**兩份報告都沒有說**。對帳花掉的時間就是這個缺口的帳單。

本機全歷史 633 列（08-31 .. 09-03）：claude-code 196、v1 無欄位 116、非 JSON 112、grok 3、
codex 2、kimi-code 1。與那份 3,653 列的表不是同一個母體。

改法：

* `usage.db` 的 `_usage_meta` 記 `MACHINE_ID` / `MACHINE_ID_SOURCE`。id 是**推導**而非亂數
  （`/etc/machine-id` → dbus → `HOSTNAME`，可用 `CORT_MACHINE_ID` 覆寫），所以刪掉 cache 再建
  答案一樣；亂數會讓同一台機器變成兩台，正好製造這件事要消滅的混亂。**雜湊而非主機名**，
  與 `project_id` 同一套隱私紀律。`source` 一起記，因為來源可信度不同。
* **stamp 寫一次，永不覆寫。** 資料庫被同步或還原到第二台時仍指名建立它的那台，於是兩個 id
  對不上，報告才說得出 `mixed`。覆寫會讓它看起來像最後開啟那台的原生資料——同一個問題的靜音版。
* `cort usage` 的 lean 標頭（也就是會被貼進文件的那一行）帶 `machine=` 與 `source=`；混到時
  警告在**標頭**不在註腳。
* `cort-evals` 的每一份報告經 `print_report` 戳章。這一條是必要的：被拿來比較的 417 vs 2 就是
  從 hook 統計來的，而那條路原本完全沒有機器標記。

措辭刻意保守：**列本身不帶機器**，兩台寫進同一個檔之後就再也分不開了。報告能做的只有拒絕沉默。

## 7. Codex：四個假設連續被推翻，而我還把自己的自測當成了證據

`settings_toml.rs` 寫的是 `matcher = "Bash"`，整組從 JSON 那側抄來。Codex 0.152.1 的工具是
`exec_command` / `exec` / `apply_patch`，binary 裡 `MultiEdit`、`NotebookEdit`、`Grep` 各 0 次。
本機 codex 全歷史只有 2 列且都是手動餵的，今天 323 次 `exec_command` 呼叫一列都沒產生——
`cort hook-suggest` 只要被執行就會寫一列（連 `no_payload` 都會），所以零列代表**行程根本沒啟動**。

看起來很清楚，所以我把 matcher 改成 `exec_command|shell` 與 `apply_patch`。**那是錯的。**

### 依序被推翻的四個假設

| 假設 | 怎麼死的 |
|---|---|
| matcher 值配不到 | `matcher = "exec_command"` 精確吻合實際工具名，仍然不燒 |
| trust 過期（`--status` 只查 hash 存不存在，不重算） | `--dangerously-bypass-hook-trust` 開新 session，仍然不燒 |
| command 含空格、Codex 不做 shell 切詞 | 另一台的 command 就是含空格的那一串，燒了 417 次 |
| 工作目錄不在專案裡 | 在 repo 內以 `bdcodex exec` 跑（同一顆 codex、同一份 config、只換模型供應商所以不吃 quota），仍然不燒 |

一併排除的還有：feature flag（`doctor --all` 顯示 `hooks` 在 47 個啟用旗標裡）、entry 沒載入
（Codex 自報 `PreToolUse Installed 1 / Active 1`）、session 讀到舊設定（重開過）。

### 兩台的並排

| | codex-cli | 工具名 | matcher | command | 結果 |
|---|---|---|---|---|---|
| 另一台 | 0.152.1 | `exec_command` 711 / `exec` 143 | `Bash` | 含空格 | **417 次** |
| 本機 | 0.152.1 | `exec_command` 323 | `Bash` | 含空格 | **0 次** |

**同版本、同工具詞彙、逐字相同的設定，一台燒一台不燒。** 所以 `Bash` 並不是按工具名做等值比對
——它在另一台照樣配到 `exec_command`。差別不在設定檔裡，而我連續四輪都在設定檔裡找。

### 一次歸屬錯誤，以及它花掉的兩輪

過程中我把一筆 13:10:26 的 `harness=codex` 列當成「第一筆真實觸發」，並據此推出「command 必須是
單一 token」，還用它判了另外兩個假設的生死。**那筆是我自己的自測**——建立 probe 腳本後用
`echo … | /tmp/codex-hook-probe.sh` 驗證它可用，而那條管線的下游就是
`cort hook-suggest --harness codex`。查全部五筆 codex 列的毫秒時戳，每一筆都對得上我自己的指令。

**這台機器至今沒有一次真實的 Codex 觸發。** 一個沒有立足點的東西被當成立足點，代價是兩輪。
教訓與 §6 同源：**一筆數字說不出它是誰產生的，就不能拿來當證據**——那裡是機器，這裡是行程。

### 這一節目前的結論

四個假設全滅，而且沒有任何一次真實觸發可以當立足點。已知的只有：本機的 Codex 把 hook 報成
`Active`，卻在試過的每一種設定下都沒有執行過它。

減損一併記下：最後一輪用的 `codex exec` 是非互動模式，**它本身可能就不跑 hook**，所以那一輪的
陰性證據力弱於互動模式。另一台的 config 也無法確認與貼出來的那份是同一份（該機 session resume
失敗），所以 417 這個數字與「`matcher = "Bash"` + 含空格 command」的綁定，比本文前一版寫得要鬆。

matcher 已回退為 `Bash`：沒有證據支持改動它。

**在拿到一次真實 payload 之前，不要再對 Codex 的接線做任何「看起來很明顯」的修改。** 這一節記錄的
就是連續四次這樣做、外加一次歸屬錯誤的結果。

### 順帶修掉的一個真 bug

`is_canonical` 只看 hook entry（command / type / timeout），而 **matcher 掛在上一層的 group 上**，
所以過期的 matcher 每次重裝都被當成 `already_present`。這是 skill 部署早就有的規矩
（smoke Test 17「a hash match must not excuse the shape」），這個模組從沒學到。修法：group 只有
我們這一條時才校正 matcher；共用的 group 不動——別人的 matcher 也是別人的路由。這個修正與上面
那三個死掉的假設無關，獨立成立，留著。

## 8. 這一輪關於測試的結論

不是「測試不夠」。384 條 rust 測試裡只有 75 條（20%）真的跑二進位檔，`main.rs` 是全 crate 最大的
檔（1,567 行）而**沒有專屬測試檔**，最近 7 個 `fix` 有 5 個落在它裡面。

逃逸的 bug 有同一個形狀：**不是「邏輯沒測到」，是「參數怎麼決定」沒測到。** 模組測試永遠是
`install_hook(&path, …)`——path 是測試自己餵的，而決定 path 的那段沒有任何測試驅動。同樣的形狀
往回數都對得上：`|| true` 那次是測試餵自己寫的 command 字串；Grok 被記成 Claude Code 是測試餵
`--harness`、真實世界餵一個跟 flag 打架的 `transcript_path`。

`install.sh` 這一側更直接：出貨六個 hook 條目，smoke 的斷言全部指向 `~/.claude/settings.json`，
**codex 與 kimi 的四條、以及全部三條 post 事件，共五條，install 層 0 個斷言**。現已 121 條斷言，
六條全覆蓋。
