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

## 7. Codex：rollout 的函式名不是 payload 的 tool_name，而我拿前者判了後者

`settings_toml.rs` 寫 `matcher = "Bash"`，而 Codex 0.152.1 的 rollout 裡工具叫 `exec_command`
（今天 323 次）、`exec`、`apply_patch`，binary 裡 `MultiEdit`／`NotebookEdit`／`Grep` 各 0 次。
本機 codex 一列都沒有。看起來鐵證如山，所以我把 matcher 改成 `exec_command|shell` 與 `apply_patch`。

**錯的。** 實地攔截到的 `PreToolUse` payload 是：

```json
{"session_id":"…","turn_id":"…","transcript_path":"…/rollout-2026-09-03T13-29-23-….jsonl",
 "cwd":"/home/yanggf/a/cortexyoung","hook_event_name":"PreToolUse",
 "model":"deepseek-v4-flash-0731","permission_mode":"default",
 "tool_name":"Bash","tool_input":{"command":"ls"},"tool_use_id":"call_…"}
```

**`tool_name` 是 `Bash`。** Codex 在 hook payload 裡把工具名正規化成 Claude Code 的詞彙；
rollout 記的 `exec_command` 是**模型看到的函式名**，兩者是不同的東西，而我從第一輪就混為一談。
`matcher = "Bash"` 從頭到尾都是對的，也就是 §12 說「Codex 的 payload 與 Claude Code 逐字相同」
的真正含意——那句話連 `tool_name` 的**值**都涵蓋，不只欄位名。

### 我製造了自己在追的症狀

| 時間 | matcher | 結果 |
|---|---|---|
| 11:01 之前 | entry 不存在（11:01 那次 install 回報 `installed` 而非 `updated`） | 今天早上那 323 次呼叫沒有 hook 可燒 |
| 12:2x–13:2x | 被我改成 `exec_command\|shell` / `exec_command` | 全部不燒 |
| 回退成 `Bash` 後 | `Bash` | **一次就燒** |

所以「本機 0 次 vs 另一台 417 次」這個謎題，一半是 entry 當天才剛建立，另一半是我親手改壞的。
中間為了解釋它而生的四個假設（matcher 值、trust 過期、command 形狀、工作目錄）全部是在追一個
我自己製造的症狀。

### 一次歸屬錯誤，以及它花掉的兩輪

過程中我把一筆 13:10:26 的 `harness=codex` 列當成「第一筆真實觸發」，據此推出「command 必須是單一
token」，還用它判了另外兩個假設的生死。**那筆是我自己的自測**——建完 probe 腳本後
`echo … | /tmp/codex-hook-probe.sh` 驗證可用，而那條管線下游就是
`cort hook-suggest --harness codex`。五筆 codex 列的毫秒時戳，每一筆都對得上我自己的指令。

最終那次真實觸發之所以可信，是因為 probe 記下了**父行程的命令列**：
`parent=codex -c model_reasoning_effort="medium" …`。攔截手段本身要能證明是誰叫的，否則它產生的
證據跟 §6 那份說不出自己是哪台機器的數字是同一種東西——**只是低一層：那裡分不出機器，這裡分不出
行程。**

### 順帶確認的兩件事

* **Codex 的 payload 帶 `model`**（此例 `deepseek-v4-flash-0731`，因為是透過 `bdcodex` wrapper
  跑的）。這坐實了 §5 的 v3 `model` 欄位在 Codex 這側收得到值——Claude Code 那側收不到。
* **`bdcodex` 是可用的測試載具**：同一顆 codex 二進位、同一份 config、同一組 hooks，只換模型供應商，
  所以不消耗 OpenAI quota。互動模式才驗得準——`codex exec` 那輪的陰性有弱點，本節不採信它。

### 留下來的規矩

**攔截路徑（rollout / transcript）記的名字，不等於 hook payload 送來的名字。** 要判斷 matcher，
只能看實地攔截到的 payload，不能看 rollout。這一節記的是不這樣做的代價：四個假設、兩次錯誤修改、
一次把自己的自測當成證據。


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

## 9. 兩個 staleness 判準都是單向的，而它們往相反方向瞎

這一輪的最後一件事跟安裝器無關，是同步完 `838d17fb..0d51e55b` 之後順手撞到的：`cort status` 說
`index_is_stale: false`，同一份輸出裡 `git_head` 卻是 `70e228f5`——落後 5 個 commit，`git pull`
剛剛重寫了 16 個檔。

根因在 `rust/src/incremental.rs` 的 `git_candidates`：候選集是
`git diff --name-status -M HEAD`，拿工作樹跟 **HEAD 現在指的地方** 比。所以任何「不弄髒工作樹就
搬動 HEAD」的動作——`pull`、`checkout`、`rebase`、`reset`、同事或另一個 agent 的 commit——都產生
空 diff，而那棵樹索引從沒看過。存下來的 `projects.git_head` 從頭到尾沒有人拿來比。

**同一個編輯，commit 之前抓得到，commit 之後就隱形。** C2-19（`a_changed_chunk_body_makes_the_index_stale`）
測的正是這個編輯：dirty 時它在 `git diff HEAD` 裡，是候選，hash 對不上，判 stale。把它 commit 掉，
diff 變空，它不再是候選，於是 `index_is_stale: false`。測試覆蓋率沒有掉，因為 384 條測試裡沒有
一條讓 HEAD 動過。這跟 §8 的結論是同一個形狀，再往下一層：不是「邏輯沒測到」，是**「輸入從哪來」
沒測到**——這次連 path 都不是,是 HEAD。

### 第二個後果才是真的難看

`incremental_index` 共用同一組候選集，所以 pull 之後它重抽 0 個檔，**然後照樣把新 head 蓋上去**
（`UPDATE projects SET git_head = …`）。而那個戳章正是 `hook-suggest` 拿來比對的東西
（`main.rs` 的 `IndexState::BehindHead`）。

所以 PostToolUse 的 refresh hook 不是「修不動 pull 過的樹」而已——**下一次編輯,它會把「需要修」
這個唯一訊號抹掉**。README 說 `--incremental` 追檔案內容所以關上了 head 比較的窗;那句話只對了
一半:它追的是 *git 願意列出來的那些檔* 的內容。兩個判準各瞎一邊,而修復機制站在會抹掉證據的那邊。

### 改法

`git_candidates` 收下索引當初的 head（新增 `db::indexed_head`），多做一次
`git diff -M <indexed-head> HEAD`。git 不肯回答時——沒有 repo，或存下的 head 解析不了
（force-push、shallow clone、db 從別台搬來，正好接上 §6 那條）——候選集**拒絕收窄**，呼叫端去看
全部。`git_available` 改名 `narrowed` 就是為了讓這個語義說得出口：false 不是「沒有 git」，是
「git 不告訴我」。**一組收窄不了的候選集必須擴張到全部，絕不能安靜地縮到零。**

### 證據

先寫三個失敗測試（`a_commit_that_moves_head_without_dirtying_the_tree_is_stale`、
`an_unreachable_stored_head_falls_back_to_hashing_every_file`、
`a_head_that_moved_without_dirtying_the_tree_is_reindexed_not_just_restamped`），再改。

端到端在一份 clone 上重現 `70e228f5..0d51e55b`：

| | 修前 | 修後 |
|---|---|---|
| `index_is_stale` | `false` | `true`，並列出 17 個檔 |
| `index --incremental` | `files_reindexed: 0`，只蓋新 head | `files_reindexed: 17` |
| 結果 | 5 commit 舊的圖 | 1206 chunks / 2146 relationships |

最後那組數字與同一 head 上跑完整 `cort index` 相同，所以 incremental 這條路是真的補齊，不是補一半。

順帶一提，找 `git_candidates` 的三個呼叫點是 hook 攔下 `grep` 之後用
`cort impact --symbol git_candidates --depth 1 --coverage -f lean` 做的，一次到齊，兩筆 `miss`
是 import 不是呼叫。這是本文件裡唯一一次產品用在自己身上並且真的省了事。

## 10. 一個測試把我未提交的編輯 `git checkout` 掉了

寫 §9 的時候改了 `skills/ast-grep/SKILL.md`,存檔、確認,然後跑閘門,再回頭看——改動不見了,
`git status` 乾淨,檔案的 mtime 卻是幾分鐘前。改了第二次才發現是誰。

`tests/install-smoke.sh` 的 Test 18b 要驗「只動 body、不動 frontmatter description 的編輯,
deploy log 也要記一筆」。install.sh 是從它自己所在的 repo 讀 skill 的,所以這條斷言確實得弄髒
開發者工作樹裡一個被追蹤的檔。它附加一行,然後這樣收拾:

```sh
git -C "$REPO_ROOT" checkout -- skills/ast-grep/SKILL.md 2>/dev/null || true
```

`git checkout --` 丟掉的不是它自己附加的那一行,是**那個檔案裡所有未提交的東西**。而
`2>/dev/null || true` 保證它安靜地成功。這不是清理,是還原到 HEAD,兩者只有在開發者剛好沒在
改那個檔的時候才等價——而 §9 正好在改。中途 Ctrl-C 的話下場相反:附加的那行留在 repo 裡。

改法:在附加之前拷一份位元組備份,從備份還原,並掛 `trap … EXIT`,這樣中斷也收得乾淨。判準是
**一個測試只能還原它自己造成的改動,不能還原到 HEAD**;git 不是收拾自己垃圾的工具,它是丟掉別人
東西的工具。

驗證:改動存在 → 跑 smoke(119 passed, 0 failed)→ md5 不變。

這條跟 §9 的教訓是同一句話的兩種說法。§9 是**一組收窄不了的候選集必須擴張到全部,不能安靜地縮到
零**;這裡是**一個收拾不了的還原必須只碰自己碰過的,不能安靜地擴張到整個檔**。兩次都是 `|| true`
和空 diff 讓「什麼都沒發生」跟「發生了但我不說」長得一模一樣。

## 11. CI 紅了三次推送,而本機每次都回報「全部閘門綠」

`0eabf76b` 推上去之後才去看 CI:今天的三次推送——`--all` 重構、matcher 回退、Codex 結論——
**全部 failure**,而且死在同一關,`shellcheck`。

```
SC2034 HOOK_SETTINGS appears unused
SC2034 CODEX_HOOK_SETTINGS appears unused
SC2034 KIMI_HOOK_SETTINGS appears unused
SC2034 line appears unused
```

前三個正是 §3 那個重構的殘骸。表搬進 `HOOK_TARGETS` 之後,設定檔路徑改由 `hook-install --all`
的回覆帶回來,這三個 bash 變數就沒人讀了——存在了一整天,唯一注意到的是 linter。第四個是
`local line` 忘了拿掉。

兩件事值得記下來:

**一,`shellcheck` 排在 `offline install smoke test` 前面,所以那一關這三次一次都沒跑。** 這正是
`838d17fb`(「a broken gate costs you the gates behind it」)那條教訓,在下一個閘門上原樣重演。
上一次是 rustfmt 擋住 clippy 和 test,這一次是 shellcheck 擋住 smoke。**修好一個閘門的收穫不是
它自己,是它後面那些。**

**二,「本機全綠」跟「CI 全綠」是兩個不同的宣稱,而它們的差集就是本機沒裝的東西。** 那三次的
session 都據實跑了 fmt / clippy / test / smoke 並回報綠,只是本機沒有 shellcheck,那一關在本機
根本不存在,於是「跑過的都綠」被讀成了「全部綠」。這一輪的收尾是抓一份 static binary 到暫存目錄
跑一次真正的 CI 指令,`exit=0` 才算數——順帶抓到我自己寫的註解有一行以 `# shellcheck` 開頭,
被當成 directive 解析(SC1073)。**一個本機跑不到的閘門,回報時要說它沒跑,不能算在綠裡面。**

## 12. `--status` 報的是狀態存在,不是狀態新鮮 —— 第三次同一個形狀

`7be1cab1` 之後這台重裝,post 三條全部 `updated`,installer 也照規矩印了「Codex 要重新信任」。
然後 `hook-install --all --status --lean` 說:

```
codex pre  wired trusted=true
codex post wired trusted=true
```

這不是矛盾,是 `settings_toml.rs:416-420` 已經寫明的限制:`trusted_at` 只檢查那個 `gi:hi` 位置
**有沒有** `trusted_hash`,不比對它是否對應現在的 entry。所以一條剛被改寫的 entry,在這裡必然
還是 `true`。

值得記下來的是這是**第三次同一個形狀**:`--status` 兩次把正在觸發的 hook 報成 `wired: false`
(§7/§9,錨在行尾的後綴比對),這次把需要重新信任的 entry 報成 `trusted=true`。三次都不是這個
欄位算錯,是**它回答的問題跟讀的人以為的不同**——它報的是「某個東西在那裡」,讀的人要的是
「那個東西還對嗎」。

**規矩:改寫當下 installer 印的那行是權威訊號;`--status` 的 `trusted` 不能拿來驗收重新信任。**
真正能結案的只有一次真實的 `codex` 執行。

當天稍後那次執行結案了,而且比規矩更強:**Codex 真的跳出了 review**。這次改寫**只動 matcher,
command 字串一個字沒變**,所以那次 prompt 證明 `trusted_hash` 涵蓋的不只是 command——整條 entry
的形狀都在裡面。於是 installer 那行提示從「保守起見」升級成有證據的必要提醒,而 `--status` 在同
一時刻回報的 `trusted=true` 是實測誤導,不是理論上可能誤導。

## 13. Codex 的 post matcher 是一次沒有證據的加寬

`EDIT_MATCHER` 從 `Edit|Write|MultiEdit|NotebookEdit` 加上 `Bash` 之後,三個方言的 post 都拿到
`Bash|Edit|Write|MultiEdit|NotebookEdit`。Claude Code 和 Kimi 沒問題——兩者的 matcher 都確認是
regex(Kimi 是 `new RegExp()`,見 `settings_kimi.rs`)。**Codex 這一格沒有這個確認。**

本機 `usage.db` 裡 `harness=codex` 的列有 **417 筆,全部是 `hook-suggest`,全部 `no_shape`**,
最後一筆 09-03 12:49。(順帶把 §6 那筆對帳結清:417 在 `machine=09a907a06cdea300`,另一台是
`2eb02d46ec5c4319` 的 2 筆。)這 417 筆證明的是一件很窄的事:**字面字串 `"Bash"` 會燒**。它沒有
證明 matcher 是 regex,而 §7 唯一試過的另一個值(`exec_command|shell`)本來就不會中,所以那次
不燒也不能反證。

於是有兩個未決風險,而且方向相反:

1. **若 Codex 是對 `tool_name` 做相等比對**,`"Bash|Edit|Write|MultiEdit|NotebookEdit"` 永遠不等於
   `"Bash"`,Codex 的 post hook **靜默失效**。§7 的結論原話是「changing this constant was a change
   with no evidence behind it: reverted」——這次是同一個動作,只是換到 post 那格。
2. **就算 regex 成立**,`Edit|Write|MultiEdit|NotebookEdit` 是 Claude Code 的工具名。Codex 的編輯
   工具是 `apply_patch`;它確實會把 shell 工具正規化成 `Bash`(§7 攔到的 payload),但**沒有任何
   證據說它把 `apply_patch` 正規化成 `Edit`**。所以 Codex 的 refresh 只會在 shell 指令後燒,永遠
   不會在 Codex 自己的檔案編輯後燒——而那正是這個 hook 存在的主要理由。

支持這份疑慮的現況:`harness=codex` 的 `hook-refresh` 列,**至今 0 筆**。

兩個都由同一次實驗結案,而且必須是真實的 `codex` 執行,不是自餵(§7 的教訓):在專案目錄裡跑
一次 `codex`,叫它 (a) 跑一個 shell 指令、(b) 改一個檔,然後查

```sh
sqlite3 ~/.cache/cortex-ng/usage.db \
  "SELECT ts,command,args_summary FROM command_log
    WHERE args_summary LIKE '%\"codex\"%' AND command='hook-refresh' ORDER BY ts DESC LIMIT 5"
```

(a) 沒有列 → 風險 1 成立,Codex 的 matcher 不是 regex,post 那格要退回 `Bash`。
(a) 有、(b) 沒有 → 風險 2 成立,`apply_patch` 需要自己的 matcher。
兩者都有 → 加寬是對的,把這件事寫成 `MATCHER` 旁邊的證據行。

### 結果:兩個風險都被推翻

2026-09-03 16:44:20 釘基準線(`codex hook-suggest = 417`、`codex hook-refresh = 0`),在 repo 目錄
內跑一次真實 `codex`(`model=gpt-5.6-sol`),之後新增的 codex 列只有三筆:

```
16:48:20  hook-suggest  no_shape         ← (a) shell 指令
16:48:20  hook-refresh  already_current  ← (a) 同一次呼叫
16:50:16  hook-refresh  already_current  ← (b) 檔案編輯
```

**風險 1 推翻**:16:48:20 那筆 `hook-refresh` 是 matcher 為 `Bash|Edit|Write|MultiEdit|NotebookEdit`
的 entry 對一次 `tool_name = "Bash"` 的呼叫燒出來的。Codex 會編譯這個交替式,它的 matcher 是
regex。

**風險 2 也推翻,而判別它的是 pre 那條的窄 matcher。** 16:50:16 只有 `hook-refresh`,**沒有**
`hook-suggest`——而 pre 的 matcher 是純 `Bash`。所以那次觸發的工具不是 Bash,是編輯工具;而
`Bash|Edit|Write|MultiEdit|NotebookEdit` 裡沒有任何一段是 `apply_patch` 的子字串,交替式又是唯一
的閘門。結論只能是:**Codex 把自己的編輯工具也正規化成了 Claude Code 的詞彙**,和 §7 的
`exec_command → Bash` 是同一條規律,不是特例。

這裡有個方法論上的意外收穫:**pre 那條窄 matcher 在這次實驗裡是對照組。** 我們沒有攔 payload,
卻靠「哪一條沒燒」反推出了觸發的工具類別。兩條 matcher 寬窄不同,本來只是歷史,這次變成了儀器。

**仍未知(而且不打算為它再跑一次):** 四個名字裡實際命中的是哪一個。要知道得攔 payload 讀
`tool_name`。不知道也不影響結論——閘門是整個交替式,不是其中某一段。
