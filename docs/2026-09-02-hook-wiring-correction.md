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

## 13. `cort hook-install` 的 TOML 模式,以及它接上了什麼(2026-09-02 夜)

§12 說的「沒有做的事」在這裡做了:新模組 `rust/src/settings_toml.rs`,`cort hook-install` 依目標
路徑的副檔名分流(`.toml` 走這裡,其餘走既有的 `rust/src/settings.rs`),`install.sh` 的
`deploy_hook`/`remove_hook`/`--check` 三處都各自拆成一個共用函式再各呼叫兩次
(`HOOK_SETTINGS` 給 Claude/Grok,新的 `CODEX_HOOK_SETTINGS="${CODEX_HOME:-$HOME/.codex}/config.toml"`
給 Codex),manifest 新增 `hook_settings_codex`。部署邏輯與 JSON 側同一套哲學:兩個都是**無條件**
部署,跟 `CODEX_SKILL_DEST` 的 skill 一樣——Codex 不在這台機器上也無妨,代價是留白就是重演本文件
反覆講的同一句話。

**TOML 的群組形狀是推來的,不是查來的。** Codex 沒有公開任何 hooks TOML schema 文件。證據鏈:
`codex --strict-config doctor` 能接受候選格式,但複查發現 `doctor` 根本不深入驗證 hooks 子結構
(塞一個亂編的欄位它一樣通過)——這條證據因此降級為佐證,不是證明。更硬的證據來自
`strings` 對已安裝的原生 codex 二進位檔(`@openai/codex-linux-x64`)掃出的 serde 型別名:
`struct ConfiguredHookMatcherGroup with 2 elements`(對應 `matcher` + `hooks` 兩個欄位),以及
一組相鄰字串片段點名 hook 項目的欄位包含 `command`、`type`、`timeout`。據此寫死的形狀:

```toml
[[hooks.PreToolUse]]
matcher = "Bash"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "<cort_bin> hook-suggest --harness codex"
timeout = 5
```

**這個形狀沒有被一次真正的 `codex exec` 觸發驗證過。** 原本要用 §12 同一招(灌好 hook、餵一個
會觸發 grep 的 prompt、看模型有沒有收到)做端到端確認,但 `codex exec --dangerously-bypass-
approvals-and-sandbox --dangerously-bypass-hook-trust` 這個組合被執行環境的權限分類器擋下(自動
模式判定風險過高,不放行)。使用者选择跳過這一步,先用 `strings` 證據落地實作。已經做過、確實
證明了什麼的,只有這些:

- `cort hook-install --settings ~/.codex/config.toml --command '<bin> hook-suggest --harness codex'`
  在這台機器的真實 `~/.codex/config.toml` 上跑過一輪 install → status → 二次 install(冪等)→
  remove,每一步都跟 JSON 側的既有承諾一致(`already_present` 不重寫檔案、`remove` 之後檔案與
  安裝前逐位元組相同、沒有安裝過的 Read/其他 matcher 群組原封不動)。
- 每一步之後 `codex --strict-config doctor --json` 的 `config.load` 都回 `"status": "ok"`,證明
  寫出來的 TOML 語法有效、能被 codex 自己的解析器讀回去——但不證明 hooks 子結構的欄位被正確
  理解或真的會觸發。
- `rust/tests/settings_toml.rs`(21 條,鏡射 `rust/tests/settings.rs` 覆蓋的每一個既有陷阱:
  使用者原有的 hook 存活、二次安裝不重寫、binary 搬家更新原地、`remove` 只拿走自己的、
  手接重複收斂成一筆且不留雜欄位、malformed 群組不擋住後面的條目、只是提到字眼的別人的 hook
  不會被誤判、`hooks`/`PreToolUse` 型別不對就拒絕不覆寫、使用者自己的空群組在 collapse/remove
  兩種操作下都存活)。TOML 特有的一條額外案例:空的 `hooks = []` 在 TOML 語法下只能寫成一個內聯
  空陣列,不可能是「零元素的 array-of-tables」(後者序列化出來就是完全不寫入任何東西,跟沒有這
  個 key 沒兩樣)——`is_ours` 的兩層掃描都用 `continue` 跳過非 array-of-tables 的 `hooks` 值,
  所以這種群組完全不會被觸碰,測試 `the_users_own_empty_group_survives_*` 兩條都覆蓋到。

`is_ours`(判斷一個指令是不是我們自己的 hook,§7、§9 兩輪硬化過的那條規則)從 `settings.rs`
改成 `pub(crate)` 直接 import 進 `settings_toml.rs`,沒有第二份拷貝——重寫一份等於在兩個地方
放同一個未來會走樣的 bug。

測試:`rust/tests/settings_toml.rs` 新增 21 條。全樹 360 + evals 94 + smoke 95,零失敗
(`is_ours` 移出的可見性變更沒有新增測試,因為行為完全沒變,只是換了呼叫路徑)。

### 端到端驗證補上了(同日稍晚)

上一節留的洞:`--dangerously-bypass-approvals-and-sandbox` 這個組合被執行環境的權限分類器擋下。
拿掉它、只留 `--sandbox read-only --dangerously-bypass-hook-trust` 就放行了——真正擋下的是「連
approval 帶 sandbox 一起繞過」這個危險組合,不是「跑 codex exec」本身。OpenAI 官方 provider 先撞
到帳號額度上限(`usage limit`),改用 `bdcodex`(同一支 codex、同一份 `~/.codex/config.toml`,只
是換 ModelStudio provider)才把這次驗證跑完,這件事本身也再次確認了 §5 附錄記錄過的事實:
provider 覆蓋不影響 hook 是否被讀取,兩者是同一份設定檔。

跑法:`bdcodex exec --sandbox read-only --dangerously-bypass-hook-trust --skip-git-repo-check "<prompt
要求模型跑一個 grep 並逐字回報context裡多出來什麼>"`。結果,兩個獨立來源一致:

1. **模型自己的回合記錄**:印出 `hook: PreToolUse` 接著 `hook: PreToolUse Completed`(不是 §11/§12
   之前那個 `Failed`),而且模型的最終回答逐字引用了注入文字:「cort has an index for this
   project. `cort impact --symbol 'harness_of' --depth 1 --coverage -f lean` answers who calls it
   in one call...」——跟 `cmd_hook_suggest` 產生的 `context` 字串一字不差。
2. **usage.db**(獨立於模型自述的第二個來源):同一時刻寫入 `{"harness":"codex","hook":"hit","v":2}`,
   沒有 `harness_declared` 分歧欄位——代表 `--harness codex`(TOML 裡寫死的宣告值)跟
   `harness_of` 從 `transcript_path` 反推出的值一致,兩條路徑互相印證。

**這證實了什麼**:TOML 形狀(`matcher`/`hooks`/`type`/`command`/`timeout`)不只是被 codex 的解析
器接受,是真的被讀進 hook 執行引擎、真的觸發、產生的 `additionalContext` 真的送達模型上下文。
`hook_codex: ... (wired)` 現在跟 `hook: ... (wired)` 站在同一個證據等級上——上一節列的那個
「還開著的一件事」關掉。

> **後續更正(§14):** 上面這句「站在同一個證據等級上」多跨了一步。這次跑法帶了
> `--dangerously-bypass-hook-trust`,而 Codex 的 hook 有一道 persisted trust 閘;拿掉那個 flag 之
> 後,同一個條目一次也沒有執行過。本節證明的是形狀被讀懂、注入送達;「正常路徑會不會執行」由 §14
> 補上(答案:要先在互動式 Codex 裡審過一次)。

## 14. `wired` 回答的是一個差一步的問題(2026-09-02 21:20-21:50)

§13 收尾那句「`hook_codex: ... (wired)` 現在跟 `hook: ... (wired)` 站在同一個證據等級上」是錯的,
而且錯的方式跟本文件反覆講的是同一件事。用**正常路徑**(不帶任何 bypass flag)重跑一次 §13 的端到
端:

```
codex exec --skip-git-repo-check "<要求模型跑一個 grep,並逐字回報 harness 附加了什麼>"
```

模型回 `NONE`。`cort usage` 的 `hook-suggest` 計數 2140 → **2140**,一次都沒動(安靜命中也會寫一
列,所以這是零執行,不是零建議)。

**原因:Codex 的 hook 有一道 persisted trust 閘。** `codex exec --help`:

> `--dangerously-bypass-hook-trust` — Run enabled hooks without requiring persisted hook trust for
> this invocation.

§13 那次跑的正是 `--sandbox read-only --dangerously-bypass-hook-trust`。commit message 有誠實寫出這
個 flag,但結論多跨了一步:它證明的是**形狀被 hook 執行引擎讀懂、`additionalContext` 送得到模型**
——這部分至今仍然成立——而不是「正常叫起來的 Codex 會執行它」。後者當時從未被驗過,答案是不會。

閘的形狀,兩個來源:

- codex 二進位的 serde 型別表:`internally tagged enum HookHandlerConfig` 之後接
  `state` / `matcher` / `hooks` / `HookStateToml` / `trusted_hash`。我們寫出去的群組只有
  `matcher` 和 `hooks`。
- TUI 字串:`' hooks need review before they can run.`、`1 hook is new or changed.`,以及
  `tui/src/bottom_pane/hooks_browser_view.rs`。

使用者在互動式 Codex 裡按下信任之後,真實的 `~/.codex/config.toml` 多出來的正是:

```toml
[hooks.state."/home/yanggf/.codex/config.toml:pre_tool_use:0:0"]
trusted_hash = "sha256:e7c909599d16022fe2a529bccd32cfd347d4b0a1723ae7d1f02f8a3a98d78178"
```

**重驗:通過。** 同一條指令、**不帶任何 bypass flag**,三個獨立訊號一致:模型的回合記錄印出
`hook: PreToolUse` → `hook: PreToolUse Completed`;模型逐字引出注入的 `additionalContext`;
`hook-suggest` 計數 2164 → 2166。這才是正常路徑上的第一次端到端。

### 改了什麼

- `settings_toml::installed_entry` 回傳 `(command, trusted)`,`installed_command` 退化成它的前半。
  `trusted_at` 只比對 key 的 `:pre_tool_use:<gi>:<hi>` 尾段,**不比對前面那半路徑**——那是 codex
  對同一個檔案的拼法(symlink、`CODEX_HOME`),讓「認得自己的條目」取決於兩個行程對路徑拼法的共
  識,正是 §7/§9 在指令行上已經付過一次代價的錯。尾段不會誤撞:`:pre_tool_use:0:0` 不是
  `:pre_tool_use:10:0` 的後綴。
- `hook-install --status` 多一個 `trusted` 欄位。**不適用的地方一律 `null`,不是 `false`**:
  Claude Code 和 Grok 沒有這道閘,沒有我們的條目也就沒有可信任的東西。只有「已接線的 Codex 條目」
  能回答這個問題,而那裡的 `false` 就是一個不會執行的 hook。
- `install.sh --check` 據此印 `(wired, NOT TRUSTED — start codex once and review the hook)`。
  **報告,但不判定失敗**:這個 hook 跟 `CODEX_SKILL_DEST` 一樣是無條件部署的,所以「已接線、從未
  審過」也是一台根本沒裝 Codex 的機器的靜止狀態,在那裡讓 `--check` 失敗是狼來了。信任是使用者
  的權利,把它講出來是我們的義務。
- `deploy_one_hook` 在**寫入或改寫**指令的當下就講。信任綁定在那個指令字串上,所以搬動 binary 必
  然使它失效——而事後沒有任何東西分得出「過期的信任」和「當前的信任」,所以唯一誠實的時機就是我們
  自己動它的那一刻。
- `rust/tests/settings_toml.rs` 新增 7 條(無 state 表、位置命中、路徑半段不比對、位置不符不算、
  空 hash 不算、重裝兩條路徑都不能洗掉別人的信任表、`installed_command` 與 `installed_entry` 不
  得對「什麼被接線了」產生分歧)。全樹綠燈。

### 沒做、且不打算做的一件事

**不逆向 `trusted_hash` 的計算方式,不讓 `install.sh` 自己把信任寫進去。** 那等於替使用者偽造一個
「我審過這個 hook」的安全決定,而那道 review 存在的全部意義,就是不該由被安裝的東西自己蓋章。
連帶的代價要明說:`trusted_at` 只能報「有沒有」,報不了「對不對」——用舊指令審過的條目,在這裡讀
起來和當前的一模一樣。這是接受的限制,不是還沒做的功能。

### 順帶修掉的死碼

`harness_of` 裡有一條 `/.kimi-code/` 的 arm,靠 `transcript_path` 比對。兩種拼法的 transcript path
在 kimi-code 出貨的整份 bundle 裡都是零命中,實機收到的 payload 裡也沒有——那條 arm 永遠不會命中,
而它讓那份 harness 清單看起來涵蓋了一個根本沒接的 harness。刪掉,理由寫進 `harness_of` 的 doc
comment。

> **更正(同日稍晚,實測後):** 本節與 `cb3d04aa` 的 commit message 原本寫「payload 只有
> `tool_name` / `tool_input` / `tool_call_id`」。那是只讀 `runPreToolUse` 得出的,漏了 runner 在
> `main.mjs:276370` 送進 stdin 前補上的欄位。實際收到的是 `hook_event_name` / `session_id` /
> `cwd` / `client_type` 加上那三個 tool 欄位。結論(arm 不會命中)不受影響,推理過程錯了。這不是
> 無關緊要的措辭:`session_id` 的存在正是下一節那個設計可不可行的前提。

## 15. Kimi:母體比想的大,通道比想的窄(2026-09-02 22:00-22:40)

前面幾節推定 Kimi 的接法是「照 Codex 再寫一個 module」。撈了 kimi-code 自己的 session log 之後,
兩個前提都不成立。

**Log 在 `~/.kimi-code/sessions/<workspace>/<session>/agents/main/wire.jsonl`**,178 個 session、
58MB、3,002 次 tool call。工具分布:

```
Read 1086 | Grep 834 | Edit 443 | Bash 358 | Glob 134 | TodoList 65 | Write 47 | …
```

**Kimi 的搜尋主要不走 shell。** `Grep` 是結構化 tool:

```json
{"name":"Grep","args":{"pattern":"enumeration_may_be_incomplete","path":"rust/src","output_mode":"content","-n":true}}
```

所以 `settings_toml.rs` 為 Codex 寫死的 `matcher = "Bash"` 搬到 Kimi 會直接漏掉主要母體。把
`hook.rs` 的判準逐列套上去(**手工 grep 近似,不是那條規則本身**——見下方限制):

| | Grep tool | Bash tool |
|---|---|---|
| 總數 | 834 | 358 |
| 單一符號 pattern | 92 | — |
| 且無 `-A`/`-B`/`-C` | 67 | — |
| 且非單一具體檔案 | 53 | — |
| 且非 non-source 路徑 | **43** | — |
| 開頭是 `rg`/`grep`/`egrep` | — | **32** |

> **這張表已被 §16 的 `hook-probe v2` 取代,兩個數字都高估。** 真規則重放同一批語料的答案是
> **結構化 29、shell 8**,不是 43 和 32。方向的結論(結構化母體比 shell 大,`matcher = "Bash"`
> 會漏掉大半)成立且被 v2 加強;倍率不要引用這一張。留著它是因為它示範了本文件反覆講的那條規矩
> 在同一天內第二次咬人:一份手工近似的規則,產出的就是一組描述別的東西的數字。


43 比 32 大。`matcher = "Bash"` 會漏掉約 57%。對照組:同一套數法在自己的 Claude Code transcript
上是 `Grep` 244 對 Bash 裡的 rg/grep 2,546——**比例反過來**,所以 Claude Code 那邊的 `Bash` 是對
的,這是真的 per-harness 行為差異,不是產品層的普遍漏洞。

**通道確認到底了。** 八個 hook 事件全查過:`PreToolUse` 走 `triggerBlock`(只留 `action === "block"`);
`PostToolUse` / `PermissionRequest` / `Interrupt` 是 `fireAndForget`;`SessionStart` / `SessionEnd` /
`SubagentStart` / `PreCompact` 走 `trigger` 但**回傳值直接被丟棄**(`await trigger(...)` 沒有賦值)。
沒有第二條通道。

### 唯一可能成立的設計,以及它的實測

**「每個 session 每個符號只擋一次,然後讓路。」** 第一次遇到合格搜尋時 deny,建議放進
`permissionDecisionReason`;記下 `(session_id, symbol)`;重試放行。誤判代價從「搜尋被擋掉」降成
「多花一個 turn」,也就是其他 harness 上一則建議本來的成本。

唯一的未知數是:**模型被 deny 之後會不會重試?** 用一支 scratchpad 的 stub hook 實測
(`KIMI_CODE_HOME` 指向 config 副本,真實設定與 plugin 的 managed block 都不參與)。hook 端記錄:

| # | tool | 內容 | 決策 |
|---|---|---|---|
| 1 | `Grep` | `pattern=cmd_hook_install` | **DENIED** |
| 2 | `Bash` | `cort impact --symbol cmd_hook_install --depth 1 --coverage -f lean` | ALLOWED |
| 3-5 | `Grep` | `pattern=cmd_hook_install` ×3 | ALLOWED |

**模型不但重試,還先去跑了被建議的指令。** 沒有放棄,沒有反覆撞同一個 pattern;它自己的回答裡寫
「my first grep was blocked by the hook with a suggestion; my retry ran」。

同一次跑還意外驗到 coverage 的價值:`cort impact` 的答案是**錯的**(索引 stale,說 caller 在 453、
call site @467;實際 `dispatch` 在 468、call 在 482),而 `--coverage` 回 `incomplete=true` 並把真正
的 call site 列在 `miss` 裡,模型讀到之後用 grep 覆核才給出正確答案。這正是「不保證答案對,但能說出
答案可能不完整」。

### 限制,以及為什麼先擴 `hook-probe` 再建 module

上表的漏斗是**手工 grep 近似**,不是 `hook.rs` 那條規則。本文件反覆講的規矩就是不准有第二份規則
拷貝——要拿這個母體去證成一個改契約的決定,得讓 `cort-evals hook-probe` 讀 Kimi 的 `wire.jsonl`,
用同一個函式重放。實測本身也是 **n = 1**:一個 session、一個符號、一個模型(k3),回答了是非題,
沒有建立任何比率。

所以順序是:先擴 `hook-probe`,再談 module。而若真要建,「擋一次」必須當成明文的 harness 例外寫進
`hook.rs`,連同本節的實測序列當證據——不能讓兩個 harness 的行為在同一份文件裡看起來一樣。

## 16. Kimi 接上去了:一條規則、兩個 parser、一個只在這裡成立的契約(2026-09-02 23:00 - 09-03 01:30)

§15 收在「先擴 `hook-probe`,再談 module」。兩件都做完了,而且第一件立刻改寫了第二件的前提。

### `hook-probe v2`:兩個表面,一個判斷

`suggests_impact` 拆成 parse 與 decide。**解析本來就該各家一份**——這個 codebase 早就有兩種方言
(`"command":"…"` 與 `["bash","-lc","…"]`),Kimi 的結構化 `Grep` 是第三種——**判斷只能有一份**,
因為它的全部價值就是那組校準數字,有第二份就等於量的東西不是裝出去的東西:

```rust
pub fn search_from_shell(&str)      -> Option<Search>
pub fn search_from_grep_fields(...)  -> Option<Search>   // Kimi 自己一份,不經過 shell
pub fn judge(&Search)                -> Option<HookHit>  // 校準量的就是它
```

中間曾經有一版是把結構化欄位**渲染回 shell 字串**再丟給既有的解析器。那個接縫選錯了,三個具體
後果:`-C: 4` 的值被丟掉只為騙過一個為 shell 寫的 tokenizer;憑空生出一個 `structured_unrenderable`
失敗類別(pattern 同時含兩種引號就拒絕——結構化資料本來沒有引號問題);以及證據鏈正中間多一層可能
出錯的翻譯。拆成兩個 parser 之後三個一起消失。

真規則重放的結果(55,458 個指令,三個 harness):

| | shell | 結構化 | 合計 |
|---|---|---|---|
| 搜尋 | 3,417 | 1,078 | 4,495 |
| 開火 | 161 | 47 | 208 |
| 開火率 | 4.71% | 4.36% | 4.63% |

**兩個表面的開火率幾乎相同**,這是「判斷共用一份」不只是教條的實證:同一套判準在兩種輸入形狀上
的選擇性一樣。`--kimi-dir` 預設 `~/.kimi-code/sessions`;順帶修掉一個讓報告不可用的缺陷——Kimi
每個 session 的檔案都叫 `wire.jsonl`,原本的 `session` 欄位對 178 個 session 印同一個字串,改成
相對於掃描根的路徑。

### 接線:第三種方言,以及副檔名規則的死亡

`rust/src/settings_kimi.rs`,扁平 `[[hooks]]`,`matcher = "Bash|Grep"`(regex,涵蓋兩個表面)。
兩件這個檔獨有的事:

**`--format` 取代了副檔名分流。** `is_toml_settings` 的理由是「`.toml` 是明確的」。Kimi 的檔案也叫
`config.toml`,那句話不再成立,所以 `install.sh` 明講三種格式。不嗅探路徑裡有沒有 `.kimi-code`——
`KIMI_CODE_HOME` 可以指到任何地方,那等於把答案交給一個沒人控制的字串。

**這個檔已經有別的主人,而第一次寫進去時位置是錯的。** Kimi plugin 用
`# === BEGIN kimi-plugin-cc-managed:<host> ===` … `# === END … ===` 圍住自己的 entry,而那行 END 是
document trailer。`toml_edit` 把 push 的元素放在 trailer **之前**,所以我們的 entry 落進了他們的
區塊裡——他們的 uninstall 會靜默把我們一起帶走。`push_after_trailing` 把 trailer 搬到我們新 entry
的前面,等於讓那行註解回到它本來要關閉的位置;他們的 bytes 一個字沒動。

### 只在 Kimi 成立的契約:擋一次,然後讓路

Kimi 的 `PreToolUse` 只保留 `action === "block"` 的結果,`additionalContext` 在那裡到不了任何人
(§15 已把八個事件全查過,沒有第二條通道)。所以這一個 harness——**而且只有這一個**——的建議必須以
deny 送出,配 `permissionDecisionReason`,並且**每 session 每符號只擋一次**,之後放行。誤判的代價
因此從「搜尋被擋掉」降成「多花一個 turn」,也就是建議在其他 harness 上本來的成本。狀態是
`$CORT_CACHE_DIR/hook-gate/<session_id>` 一個檔一行符號:它是閘,不是量測,弄丟只多擋一次。
`no_other_harness_ever_receives_a_deny` 這條測試釘住另外三家永遠拿不到 deny。

### 實測:機制 2/2,說服力 1/2

裝好之後在真實 Kimi 跑一次(`kimi -p`,要求它找 `cmd_hook_install` 的呼叫點),四個獨立證據一致:

- 模型自述:「The PreToolUse hook fired… It says to issue the same search again and it will run.」
- `wire.jsonl`:兩次**完全相同**的 `Grep pattern=cmd_hook_install`,第一次被擋、第二次通過
- `hook-gate/session_ee356d64-…` 內容只有一行 `cmd_hook_install`,證明只擋一次
- `usage.db` 的 `hook-suggest` 計數 2684 → 2700

**機制成立兩次,說服力兩次裡一次。** 這次模型沒有去跑 `cort impact`——它重發 grep、自己推理完,
只在結尾提了一句索引 stale、`--coverage` 會是可檢查的做法(答案是對的:一個 call site,並正確把
doc、測試、文件裡的提及排除)。稍早那次 stub 實測它是真的跑了。這個 case 本身也容易,grep 完全
夠用,規則開火在這裡接近一次假陽性。

要判斷 deny 值不值得,需要的正是更多這種紀錄,而現在它會自己累積:每次開火一列 `usage.db`
帶 `--harness kimi-code`,每個 session 的 `wire.jsonl` 也留著給 `hook-probe` 重放。這一節記的是
一個開始收資料的裝置,不是一個結論。

### 這一輪自己弄壞又修好的兩件事

- `--format` 那輪讓非搜尋的 Bash 指令從 `no_shape` 退化成 `no_payload`,被既有測試
  `the_usage_row_records_which_outcome_the_hook_reached` 抓到。那條區分不能丟:一個是「讀不到指令」,
  一個是「讀到了、規則正確地不出手」,而後者正是證明規則有選擇性的數字。
- 為了趕快測 entry 位置,手動 `cp` raw binary 蓋掉了 `install.sh` 裝在 `~/.cargo/bin/cort` 的 shim,
  `--version` 因此壞掉、`--check` 報 MISMATCH。部署路徑上的東西不要繞過 installer。

### 還開著的

`tests/chunker.rs` 或 `tests/coverage.rs` 之中有一條偶發失敗,出現過一次,之後連跑七次未重現。範圍
縮到兩個 suite,原因未知。
