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

**gate 從「db 檔存在」改成「`indexed: true`」**(`rust/src/main.rs` `index_state`;本文件原先寫的 `project_is_indexed` 這個名字不存在)。
開一個專案就會建 schema,所以一個 0 chunk 的 db 也能通過檔案存在測試,hook 於是在
`impact` 只能回 `no_seed_resolved / stale=true` 的樹上宣稱「cort has an index for this
project」。這是 `cmd_hook_suggest` 自己的 doc comment 早就禁止的失敗,而它在 09-02 是活的。

測試:`rust/tests/settings.rs`(9 條,含「使用者原有的 hook 全數存活」「重裝不重複」「拒絕壞
JSON」)、`rust/tests/cli.rs` 兩條 gate 迴歸(空 index 沉默 / 已 index 才出聲)、
`tests/install-smoke.sh` 四條(與 skill 同一次部署、matcher 是 Bash、manifest 記 `hook_settings`、
uninstall 解線)。全樹 317 + smoke 91,零失敗。

## 5. 09-04 重挖時的實際基線

> **本節在 2026-09-02 11:00 被 §7 取代。**下面 09:24 那組數字是同一天稍早寫的,而當天稍晚的
> 檢查證明 hook 從來沒有真的由 installer 接上過。實際要用的起點在 §7。

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

## 7. 第二次線斷,以及它為什麼是同一個 bug(2026-09-02 11:00)

§5 寫完之後,`./install.sh --check` 在同一台機器上回 `check: ISSUES FOUND`。四條各自獨立:

| 檢查 | 結果 |
|---|---|
| `~/.cargo/bin/cort` | 不認得 `hook-install`(`unknown_command`)→ **早於 `ccf90c07`** |
| manifest | **沒有 `hook_settings`** —— §5 說它在 09:24 寫入,這台機器上沒有 |
| `~/.claude/settings.json` | 有 cort hook,但是**手接的兩筆重複 entry**,各帶自己的 `if` |
| `cort usage` | `hook-suggest ok=988`,全部 `legacy_unsplit`(舊 binary 不寫 outcome) |

也就是說 §5 的三個前提一個都沒成立:`--check` 印不出 `(wired)`、窗起點沒有 manifest 憑證、
cross-check 沒有 outcome 可比。**§5 那句「09-02 09:24 由 installer 部署後才第一次成立」是錯的**
—— installer 從來沒跑過那一版,那兩筆 entry 是手接的。

### 根因:`is_ours` 把辨識錨在字串結尾

```
$HOME/.cargo/bin/cort hook-suggest 2>/dev/null || true
```

`is_ours` 當時要求指令 `ends_with(" hook-suggest")`。上面這行結尾是 `|| true`,所以**不是我們的**。
一個 hook command 是 shell,而人寫 shell 就會加重導向和 `|| true`。後果是三個,全部同源:

1. `--status` 在 hook 每天觸發數百次的機器上回 `wired: false`;
2. 每次重新部署**再加一筆**,而不是更新既有那筆(實測 2 → 3);
3. `--remove` 解不掉手接的那些,uninstall 會留下會觸發的殘骸。

第二個獨立 bug:`installed_command` 用 `?` 讀 group 的 `hooks` 陣列,所以 `PreToolUse` 裡任何一個
沒有 `hooks` 的 group 會**終止整個掃描**,讓排在它後面的我方 entry 報成沒接上。

修法:`is_ours` 改成把 `hook-suggest` 當 **token** 認(且前一個 token 必須是 `cort` 或以 `/cort`
結尾,所以 `echo hook-suggest` 不會誤判);`install_hook` 收斂重複(保留第一筆、丟掉其餘)並把
存活那筆**整個 entry 正規化**——留著手打的 `if: Bash(grep:*)` 會讓實際覆蓋面小於 installer 宣稱的
「wired for Bash」,那正是本文件要消滅的那類落差;`installed_command` 的 `?` 改成 `continue`。
另外 `install.sh --check` 的失敗訊息本來一律印 `could not read <settings.json>`,但 `--status`
會把讀不到/解不開的檔案吞成 `wired: false` 而不是失敗,所以那個分支**在每一條可達路徑上都不是
檔案的錯**;現在會指名是 binary 太舊。

測試:`rust/tests/settings.rs` 新增 7 條(帶重導向後綴仍認得、重複收斂成一筆、存活 entry 不留
`if`、收斂後 idempotent、`--remove` 清得掉手接的、壞 group 不遮蔽後面的 entry、只是提到字眼的別
人的 hook 不會被誤刪),`tests/install-smoke.sh` 新增 Test 19(用舊 binary 替身確認 `--check`
指名 binary 而非 settings.json)。全樹 326 + smoke 93,零失敗。

### 實際部署後的狀態(2026-09-02 10:57:05 +0800)

`./install.sh` → `hook: updated PreToolUse command`;`--check` → `check: OK` / `hook: ... (wired)`;
settings.json 的 PreToolUse 收斂成**一筆** canonical entry,其餘 15 個 top-level key 未動;
manifest 有 `hook_settings`。手動觸發一次 `grep` 立刻注入,且注入文字是 `ce1b56c7` 的誠實版本
(索引 stale 就說 stale)。

### 09-04 真正要用的基線

- **累積窗起點:`2026-09-02T10:58:00+08:00`**(三個部署寫入都在 10:57:05 之前完成;取 10:58 保證
  窗內沒有 `legacy_unsplit` 列)。**不是** 09-01 13:12,也不是 09-02 09:24。
- 指令:`cort-evals adopt-mine --since 2026-09-02T10:58:00+08:00`。
- 已驗證此起點下 `usage_db_cross_check.comparable_to_injections` 為 `true`;往前取到 10:50 就會因
  23 列 `legacy_unsplit` 而正確地拒絕比較。
- **機會母體是 9 個專案**(cct、claw-skills、cortexyoung、dac、finance-cli、finance-engineering、
  ft、gwebcdb、persona-core),不是 §5 寫的 13 個,也不是清理前 `cort projects` 顯示的 11 個
  ——多出來的兩列是 install smoke test 留在 cache 裡、指向已刪除 `/tmp` 目錄的殘骸。挖掘時要連
  母體一起報。
- `hit_stale` 是一個獨立的 outcome,代表 hook 出聲了但索引落後,讀漏斗時不能和 `hit` 混為一談。

## 8. 全機索引重建(2026-09-02 13:44)

`cort index --incremental` 在本 repo 印 `extractor_version mismatch ... full reindex required`。
那不是單一專案的事:全機共存**三種** extractor_version,只有本 repo 是當前版。重建前後:

| 專案 | files | rels 之前 | rels 之後 |
|---|---:|---:|---:|
| finance-cli | 69 | **0** | 1,644 |
| ft | 76 | **1** | 617 |
| finance-engineering | 76 | **4** | 1,720 |
| persona-core | 64 | 25 | 2,111 |
| claw-skills | 165 | **0** | 2,308 |
| gwebcdb | 228 | 1,481 | 4,777 |
| dac | 231 | 731 | 816 |
| cct | 183 | 1,792 | 1,839 |
| cortexyoung | 67 | 1,725 | 1,725 |

`relationships = 0` 不是「語言不支援」——claw-skills 是 184 個 Rust 檔、finance-cli 是 88 個。舊
extractor 對 Rust 沒抽出邊,而 hook 的 gate 只看「`projects` 有沒有那一列」,所以它照樣在那些樹上
出聲說 `cort has an index for this project`,`impact` 卻只能回 `seeds=0 dependents=0`。唯一守住
誠實的是 `--coverage`,它印 `no_seed_resolved / not a clean answer: nothing was looked at`。
全機重建約 15 秒。**任何跨越 2026-09-02 13:44 的漏斗數字都不可比**:同一個 hook、同一個 `impact`,
底下的邊數差了兩到三個數量級。

### 這一輪順帶修掉的兩個誤診

1. **`cort status` 在舊 schema 上回 `storage_busy`。** 真因是 `no such table: reading_notes`
   ——資料庫早於加入該表的 schema。`status` 以唯讀開啟,所以無法順路遷移,於是**唯一以稽核索引為
   職責的指令,正好是那個在舊索引上會壞掉的**,而且它把原因說成競爭(重試就會好),實際上永遠不會
   好。`impact` 開讀寫、順手遷移,所以跑一次 `impact` 之後同一個 `status` 就正常了——這正是
   `dac` 與 `persona-core` 在本次稽核中先失敗、後成功的原因。現在回 `schema_outdated`,並在
   `hint` 指名 `cort index`。分類器 `cort::db::classify_sqlite` 是單一來源,原本 17 處各自把任何
   sqlite 錯誤都寫成 `storage_busy`;真正的 busy(重試迴圈用盡)那一處保留不動。
2. **`cort delete` 刪不掉目錄已消失的那一列。** 它先 canonicalize 路徑再去雜湊出 db 檔名,而
   **最該被刪的那一列,正是路徑無法 canonicalize 的那一列**。registry 本身就是掃 cache 目錄得來
   的,每列都帶自己的 `db_path`,所以現在 canonicalize 失敗時改用 registry 反查;查不到才報錯,
   免得把打錯的路徑變成靜默成功。

### 仍然開著的兩件事

- cache 裡有兩個 db(`8275c303…` 09-01 17:08、`8a5edab2…` 08-29 02:40)有 schema 但 `projects`
  表是空的。它們不是任何專案,`cort projects` 不會列出,也就無法用 `cort delete` 依路徑清掉。
  這正是 `indexed: true` gate 當初要防的那種「開過但沒索引」的 db。
- 測試會寫進**真實**的 `~/.cache/cortex-ng`:本次新增測試的第一版就用 `db_path_for` 在真 cache
  建了兩個 0 byte 的檔(已刪)。`db_path_for` 讀的是呼叫端行程的 `CORT_CACHE_DIR`,測試行程沒設,
  只有它 spawn 出來的 `cort` 有。測試已改成掃 sandbox cache;但這個陷阱對其他測試同樣成立。

## 9. 複核發現的四條(2026-09-02 傍晚)

§7 的修正被複核,四條全部成立,前三條逐條實測重現過。它們有一個共同點:**§7 為了讓辨識更寬而
改動的 predicate,同時把「不該認領的」也一起認領了**——修一個方向的漏,開了另一個方向的洞。

### 1. 別人的 hook 會被吃掉(最重)

`is_ours` 接受任何 `ends_with("/hook-suggest")` 的 token,而 `i == 0` 時前面沒有 token 可檢查,
於是走 `None => true`。實測:

```
before          {"command": "/opt/vendor/bin/hook-suggest --daemon", "timeout": 30}
install →       {"command": "/x/cort hook-suggest", "timeout": 5}     ← 整筆被覆寫
remove  →       hooks 整個 key 消失                                    ← 整份被刪光
```

而且 §7 的正規化讓它更糟:舊行為只改 `command` 字串,新行為換掉整個 entry,所以 vendor 的
`--daemon` 和 `timeout: 30` 一起沒了。這正好穿過 `a_command_merely_mentioning_the_word_is_not_ours`
守的縫——那條守的是 `echo hook-suggest`,不是「路徑結尾剛好是它」。

`ends_with("/hook-suggest")` 沒有正當用途:`cort hook-install` 產生的 command **永遠**是
`<exe> hook-suggest` 兩個 token。已移除該分支,`None` 只接受**恰好等於** `hook-suggest` 的裸 token。

### 2. 看不懂的結構被無聲覆蓋

`pre_tool_use` 在 `hooks` 或 `PreToolUse` 型別不對時直接 `*hooks = Value::Object(Map::new())`。
實測 `{"hooks": "oops-user-data", ...}` → 字串消失、回傳 `Ok(Installed)`。這與
`SettingsError::Unparsable` 自己的 doc 直接矛盾(*overwriting a settings file we failed to
understand is the one unrecoverable thing this module could do*)。Claude Code 的 schema 下不會
產生那個形狀,而那正是沒人會去找的原因。現在改成回 `Unparsable`,檔案一個 byte 都不動。

### 3. 註解宣稱的保護不存在

`emptied_a_group` 是**全域**旗標,一旦為真,`list.retain` 掃掉**所有**空 group。實測使用者原有的
`{"matcher":"Read","hooks":[]}` 在收斂重複時一併被刪——而該處註解寫的正是「不會被無關的 install
掃到」。`remove_hook` 更早就有同一問題且**連旗標都沒有**,無條件掃:實測它刪掉使用者的 Read group
之後,`PreToolUse` 變空,連整個 `hooks` key 都被移除。

改成記錄「**這一次呼叫**清空了哪幾個 group」的 index,只刪那幾個。功能影響本來很小(空 group 不做
事),但一個宣稱保護、實際沒有保護的註解,比沒有註解更糟。

### 4. `--check` 查的 binary 和實際觸發的 hook 可以是兩個(未實測,讀出來的)

`install.sh` 的 `--check` 用 `command -v cort`(PATH),而 `deploy_hook` 寫進 settings.json 的是
絕對路徑。機器上有兩份 cort 時(PATH 上新、被接上的舊),`--check` 會回 OK,實際觸發的卻是舊
binary、不寫 outcome——正好是 `--check` 被造出來要抓的東西。

改成問 manifest 記錄的 `cort_bin`(這份安裝真正擁有的那個),並額外比對 `--status` 回報的
`command` 是否指向同一個 binary,不同就印 `WIRED TO A DIFFERENT BINARY` 並讓 `--check` 失敗。
smoke Test 19 原本測的是相反方向(舊 binary 在 PATH 上),已改成把替身放在 **managed** 路徑,
另加兩條:PATH 上的另一個 cort 不會被採信、以及接到別的 binary 時會被抓出來。

測試:`rust/tests/settings.rs` 再加 5 條(vendor binary 存活於 install 與 remove、`hooks` 型別錯
被拒、`PreToolUse` 型別錯被拒、使用者空 group 在 collapse 與 remove 兩邊都存活)。
全樹 336 + evals 92 + smoke 95,零失敗。

### 附帶:磁碟,以及那個 99% 是怎麼回事

修這批時 build 撞到 `No space left on device`,連 harness 用來接指令輸出的 tmpfs 都滿了(`df`
本身跑不出來)。當下根目錄 251G 用掉 236G(**99%**),清掉 `evals/target` 才有空間繼續。

**但那個 99% 不是真的被檔案佔著。**幾分鐘後同一個 `df` 回報 134G used / 105G free(57%)——中間
沒有人刪掉 99G。可信的解釋只有一個:有行程抓著**已刪除但未關閉**的檔案描述子,空間要等行程結束
才回收。這值得記下來,因為它會讓「磁碟滿了」這個結論在事後完全重現不出來——和本文件 §2 記的
09-01 基線是同一種病:**一個當下為真、事後無法重現的觀察,不能當基線,但也不代表它沒發生過。**

清理分兩輪。第一輪只動本 repo 的建置產物:`cargo clean` 兩個 crate,回收 12.5G。第二輪經使用者
指示擴大到全機:

| 項目 | 回收 | 判準 |
|---|---:|---|
| cargo target × 12 個專案 | ~40 GB | 建置產物,一律可重建 |
| `~/.grok/sessions` 的 Grok 4.5 session(121 個) | 10.75 GB | 依版本保留 4.6 |
| 本 repo 的兩個 target | 12.5 GB | 同上 |
| `~/.cache/modelscope`(SenseVoiceSmall) | 0.9 GB | 不是 whisper |

`134G used` → **`76G used` / 163G free(32%)**。

值得記的是**分辨的方法**,不是總量。Grok session 沒有版本欄位,但每個 session 的
`system_prompt.txt` 開頭寫著 `You are Grok 4.6 released by xAI`,所以版本可以逐一判定而不是靠
mtime 猜:4.6 有 144 個共 11.75G(保留),4.5 有 121 個共 10.75G(刪除)。另有 53 個沒有
`system_prompt.txt`、合計僅 0.12G——**無法判定版本的就不刪**,省下的空間不值得用一次猜測換。

同樣的原則讓 `~/.cache/huggingface` 留了下來:它整個只有 `Systran/faster-whisper-small` 與
`faster-whisper-tiny`,而 faster-whisper **就是** whisper,指示是留 whisper。按目錄名字清會刪掉
它;按內容清不會。`~/.cache/modelscope` 裝的 SenseVoiceSmall 不是 whisper,所以刪。

深度也有陷阱:第一次用 `find -maxdepth 3` 盤點 cargo target 得到 29G,`-maxdepth 5` 才看到
`finance-cli/target` 的 15G——**盤點的深度限制會直接變成清理的漏網**,而報告不會顯示它漏了什麼。

部署不受影響:`install.sh` 把 binary 裝在 `~/.local/share/cortexyoung/cort`、`~/.cargo/bin/cort`
只是 shim,不在 `target/` 裡;清完 `cort --version` 與 `./install.sh --check` 仍是 `check: OK`。
測試結果停在 `a91ef5d2` 的 336 / 92 / 95(此後未改任何程式碼);重跑會重建約 10G,那正是剛清掉的。

## 10. 多 harness:usage 列現在說得出自己是誰寫的(2026-09-02 16:24)

起因是一個看似無關的問題:能不能把 cort 裝進 Grok / Kimi。答案牽出一個會**靜默毀掉 §7 基線**
的問題,所以先修它。

### 問題

所有 harness 都呼叫同一個 `cort hook-suggest`、寫進同一個 `~/.cache/cortex-ng/usage.db`,而那個
usage 列**沒有任何欄位記錄是哪個 harness 觸發的**。`adopt-mine` 的 cross-check 是拿
**`--claude-dir` 的 transcript injections** 比對 **usage 的 `hit` 列**。多接一個 harness,`hit`
就混進一批在 Claude transcript 裡不存在對應的列,而 `comparable_to_injections` 仍會回 `true`
——它當時只擋 `legacy_unsplit`。**一個因為沒被問對問題而通過的守衛**,和 §7 那些是同一類。

### 改了什麼

`hook_args` 升到 `v: 2`,多記 `harness`。**由 installer 明示傳入**
(`cort hook-suggest --harness claude-code`),不從環境變數嗅探——只有 installer 知道自己在接哪個
harness,而在這裡猜錯等於靜默污染量測。`hook_outcomes_at` 多收一個 `want_harness`,把列分成:

| key | 意思 |
|---|---|
| 正常 outcome | harness 相符 |
| `other_harness` | 別的 harness 寫的,這個窗的 transcript 側沒有對應 |
| `unspecified` | v2 但沒帶 `--harness`,**或** v1 沒有該欄位 |
| `legacy_unsplit` | 早於 outcome 記錄,和 `unspecified` 分開保留 |

`unspecified` 刻意不歸給「當時唯一接上的那個 harness」——今天為真,而它變成假的那天不會有人發現。
`legacy_unsplit` 也刻意不併入 `unspecified`:兩者都不可歸屬,但在等不同的升級。

### 代價:基線第三次往後移

窗起點從 `10:58` 移到 **`2026-09-02T16:30:00+08:00`**(重新部署在 16:24:26)。10:58–16:24 之間的
**513 列**現在正確地報成 `unspecified` 並拒絕比較。已驗證新起點下
`comparable_to_injections` 為 `true`。

移動基線的成本目前接近零(窗才幾小時),而這正是該做這件事的時機——**在第二個 harness 存在之前**。
若等到接上 Grok 之後才發現,污染的資料無法事後拆開。

指令:`cort-evals adopt-mine --since 2026-09-02T16:30:00+08:00`。

測試:`rust/tests/cli.rs` 一條(claude-code 記到、grok 被隔開、無標記的不被冒領、無 filter 時
`cort usage` 不受影響)、`evals/tests/adopt.rs` 兩條(別的 harness 阻擋比較而非灌水、無標記的不被
記到被挖的 harness 頭上)。全樹 337 + evals 92 + smoke 95,零失敗。

## 11. Grok 不用裝——它早就在跑了,而且被記成 Claude Code(2026-09-02 16:50)

任務是「把 cort 裝進 Grok」。探針的結論是**不要裝**,理由不是不相容,而是相反:它已經接上了。

### 探到什麼

`~/.grok/config.toml` 接一個 dump 用的 PreToolUse hook,跑一次真實 grok 指令:

- **payload 相容**:Grok 同時提供 camelCase 與 snake_case,`tool_input.command` 就在那裡,
  `cort hook-suggest` 的 parser 一行都不用改。
- **注入通道相同**:`hookSpecificOutput.additionalContext` 落在 `chat_history.jsonl`,型別 `user`、
  `synthetic_reason: "system_reminder"`,包成
  `<system-reminder>Context from PreToolUse hook '<來源>':…</system-reminder>`。另外三種形狀實測:
  `systemMessage` 只進 UI 事件流 `updates.jsonl`,頂層 `additionalContext` 與 `message` 哪裡都沒去。
- **來源有兩個。** 在 `~/.grok/hooks/cort.json` 放一份之後,同一輪出現**兩段一模一樣**的
  system-reminder,標籤分別是 `global/cort` 和 **`global/settings`**。

`global/settings` 是關鍵:Grok 為了相容 Claude Code,**會讀 `~/.claude/settings.json`**
(README 的相容表列了 skills / agents / plugins / MCP / CLAUDE.md / settings)。也就是說
`install.sh` 在 10:57 寫進 Claude settings.json 的那一筆,**從那一刻起就在 Grok 裡觸發了**。

刪掉 `~/.grok/hooks/cort.json` 後重測,注入數回到 1。**Grok 的正確安裝步驟是:不做任何事。**
裝了反而是跨 harness 版本的「一份複製觸發一次」。

### 這打穿了 §10 的歸屬設計

§10 說 harness 由 installer 明示,理由是「只有 installer 知道自己在接哪個 harness」。這句話在
一個設定檔只被一個 harness 讀時成立,而它今天被證明不成立:同一筆 entry 帶著
`--harness claude-code` 在 Grok 裡跑,於是**每一次 Grok 觸發都會被記成 Claude Code 的注入,而
Claude 的 transcript 裡沒有對應**。§10 修掉的污染,由另一條路徑照樣進來了。

### 改法:問 harness 自己,不是問 installer 的意圖

`transcript_path` 是 harness 指名自己的 session 檔——不是環境變數那種猜測:

| harness | transcript_path |
|---|---|
| Claude Code | `~/.claude/projects/**/*.jsonl` |
| Grok | `~/.grok/sessions/**/updates.jsonl` |
| Codex | `~/.codex/sessions/**/rollout-*.jsonl` |

`harness_of` 以它判定;認得就**壓過**旗標,並把旗標記進 `harness_declared` 讓分歧留下痕跡;
認不得就沿用旗標(有宣告總比沒有好)。`no_payload` 仍只有旗標可用——那一列連 payload 都沒有。

實地驗證:修好後同一個窗,`claude-code` 視角回報 `other_harness: 16`、
`comparable_to_injections: false`。修之前那 16 列會被算成 Claude Code 的注入。

### 基線第四次往後移

新起點 **`2026-09-02T17:00:00+08:00`**。之前的窗裡混有被標成 `claude-code` 的 Grok 觸發,而且
無法事後拆開——`harness_declared` 只在新列上有。

這是本文件第四次移動基線,四次都是同一個原因:**一個守衛在被問對問題之前都是綠的**。

測試:`rust/tests/cli.rs` 一條(旗標說 claude-code 但 transcript 說 grok → 歸給 grok 且不算進
claude-code;認不得的路徑保留旗標)。全樹 338 + evals 94 + smoke 95,零失敗。

## 12. Codex 的 `Failed`:一個它自己 schema 允許的欄位(2026-09-02 17:15)

§11 留下的問題:Codex 0.152.1 下 hook 會執行(cort 有寫 usage 列)、exit 0、輸出對得上它內嵌的
`pre-tool-use.command.output` schema,但 codex 印 `hook: PreToolUse Failed`,模型什麼都沒收到。

反編譯 strings 查不出 `command_outcome` 的判定,所以改成**二分行為**。先問一個能一刀切開的問題:
讓 hook **完全不輸出** —— 結果是 `Completed`。所以 `Failed` 與解析有關,不是信任或啟用問題。

接著用**一次執行**做二分:讓 hook 每次呼叫換一種輸出形狀,請模型連跑五個指令,codex 會依序印出
五個結果:

| # | 輸出 | 結果 |
|---|---|---|
| 0 | `{}` | Completed |
| 1 | `{"continue":true}` | Completed |
| 2 | `{"hookSpecificOutput":{"hookEventName":"PreToolUse"}}` | Completed |
| 3 | 上者加 `"additionalContext"` | **Completed** |
| 4 | 再加 `"suppressOutput":true` | **Failed** |

**`suppressOutput` 是唯一的變數。** 而 codex 自己內嵌的 schema 明明把它列在頂層允許欄位裡
(`continue` / `decision` / `hookSpecificOutput` / `reason` / `stopReason` / `suppressOutput` /
`systemMessage`,且 `additionalProperties: false`)。宣告與實作不一致,而失敗訊息不說原因。

再一次執行確認變體 3 真的送達:模型回報 `Additional context received: "CORTDELIVERYMARKER42"`。
換上真正的 `cort` 之後端到端也通:`hook: PreToolUse Completed`,模型收到 `cort impact` 建議。

### 改法

`suppressOutput` 的用途是不要把原始 JSON 顯示在使用者的 transcript 裡。Claude Code 與 Grok 都接受
它也都確實抑制了,所以**只在 harness 是 codex 時不輸出**——§11 的 `harness_of` 已經能從 payload
判定,不必猜。

### 順帶記下的兩件設定事實

- Codex 的 hook 設定**只有 `~/.codex/config.toml` 的 `[[hooks.PreToolUse]]` 生效**;
  `~/.codex/hooks/hooks.json` 與 `~/.codex/hooks.json` 兩個位置都試過,**靜默不載入**。
- Codex 的 payload 與 Claude Code 逐字相同:`tool_name`、`tool_input.command`、
  `hook_event_name`、`transcript_path`、`session_id`、`turn_id`、`model`、`permission_mode`、
  `tool_use_id`、`cwd`。

### 沒有做的事

**沒有把 codex hook 留在設定裡。** 它現在可以動,但那是我手接的:不在 `install.sh` 的部署路徑上、
沒有記進 manifest、`--check` 看不到它。留著就是重演本文件 §3 的失效模式——**一個要靠人記得去接的
路由,就是一個沒接上的路由**。要正式支援 codex,`cort hook-install` 需要一個 TOML 模式,而那是
另一件事。

測試:`rust/tests/cli.rs` 一條(codex 拿到 context 但沒有 `suppressOutput`;其他 harness 兩者都
有)。全樹 339 + evals 94 + smoke 95,零失敗。
