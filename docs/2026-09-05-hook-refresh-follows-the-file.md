# 修復 hook 跟著 shell 走,而 agent 編輯的是路徑(2026-09-05)

`PostToolUse` 的修復 hook 用**工作目錄**決定要修哪個專案,而不是用**被編輯檔案的路徑** —— 儘管
payload 裡就寫著那個檔案。接續 `2026-09-03-installer-dedup-and-attribution.md` §9,那份講的是
「候選集合悄悄收窄到零」,這一份是同一個機制的另一條觸發路徑。

## 1. 重現

同一個檔案、同一次編輯、兩個目錄:

```
$ echo '{"tool_input":{"file_path":"rust/src/hook.rs"}}' | cort hook-refresh   # repo root
{}      outcome: already_current
$ cd rust && echo '{"tool_input":{"file_path":"src/hook.rs"}}' | cort hook-refresh
{}      outcome: no_index
```

`/home/yanggf/a/cortexyoung/rust` 在 `projects` 裡沒有列,所以 `index_state()` 回 `Missing`,hook
安靜退出、什麼都沒修。**payload 指名了檔案,hook 沒有看。**

## 2. 它的代價

`usage.db`,2026-09-04 20:35 至 09-05 00:35,四小時的編輯:

| outcome | 次數 | |
|---|---:|---:|
| `already_current` | 184 | 58.4% |
| **`no_index`** | **79** | **25.1%** |
| `db_unavailable` | 20 | 6.3% |
| `busy_or_failed` | 16 | 5.1% |
| `refreshed` | 16 | 5.1% |

那 79 次就是這個 bug。這個 session 整段都在 `cd rust && cargo test`、`cd evals && cargo test`,
於是**之後每一次編輯的修復都跟著跑掉了**。

索引因此落後 `rust/src/hook.rs` **兩行**,而 `cort status` 同時回報:

```
index_is_stale : false
changed_files  : []
git_head       : edfb43f2  (== HEAD)
```

三句都是**真話**,對一個錯的索引而言。

## 3. 為什麼新鮮度判準沒接住

`git_candidates`(`incremental.rs:111-130`)本身是對的:它既 diff 髒樹、也 diff
`indexed_head..HEAD`。但等到某次在 repo 根目錄跑的 refresh 終於執行時,樹已經乾淨,而 head 早被更早
一次根目錄的 refresh 蓋章過了 —— 那一次的 diff 裡沒有 `hook.rs`。**修復欠著,而沒有任何東西記下這件
事。**

## 4. 它是怎麼被抓到的

不是被產品的任何檢查抓到的。是照著建議跑了 `cort impact --symbol Evidence`,拿到兩個 dependent,
然後**逐行去讀**它宣稱的位置:

```
h1  rust/src/hook.rs  judge        435  @435  type    <- 435 行是一句註解
h1  rust/src/hook.rs  evidence_in   70  @74   type    <- 74 行是一個參數
```

兩行都不含 `Evidence`。整個檔案 off-by-2。

這正是這個產品的核心主張 —— **便宜、可逐邊查核** —— 在自己身上兌現。工具的價值不是它永遠對,是它錯
的時候**一行就能發現**。

## 5. 修法

`hook-refresh` 現在從 payload 的 `tool_input.file_path` 決定專案,往上走到最近一個**有 `projects`
列**的祖先。三件事值得寫下來:

**欄位名是攔截來的,不是假設的。** `hook-refresh` 接在 `PostToolUse` 上數月,而沒有人攔截過它收到的
payload。攔截結果記在 `2026-09-05-posttooluse-payloads.md`:Claude Code 的 `Write` 與 `Edit` 送
`tool_input.file_path`,**絕對路徑**;`Bash` 不送路徑;Codex、Kimi、`MultiEdit`、`NotebookEdit`
**一個都沒攔到**,實作對它們降級成走 payload 自己宣告的 `cwd`。

**判準是「有 `projects` 列」而不是「db 檔存在」。** 這不是潔癖:`ensure_schema` 會在任何命令開啟專案
時建出檔案和空的 `projects` 表,而 `open_project_tracked` 不檢查那一列,`incremental_index` 在版本或
候選不符時會落到 `full_index` —— 而 `full_index` **會插入那一列**。停在只有 schema 的資料庫,會讓修復
hook **在沒人要求的目錄裡建出索引**,違反它自己的第一條拒絕。

**解析是三態的。** `is_ok()` 會把「沒有那一列」和「資料庫鎖住」和「這不是資料庫」混為一談,而 walk
只在正命中時停止 —— 於是一個暫時被鎖住的內層專案會被跳過,修復落到外層:**修了沒人編輯的專案,usage
列歸到錯的 project_id,而被編輯的專案繼續壞著,卻有一列宣稱修過了。** `RootUnreadable` 停止 walk,
映射到既有的 `db_unavailable`。

## 6. 出貨後的受控量測

環境計數**不可直接比較**,因為 `no_index` 的語意變了:以前它是「cwd 沒索引」,現在是「被編輯檔案的專案
沒索引」,而對一個真的在索引外的檔案,那是正確答案。所以量的是形狀:

| 情境 | outcome |
|---|---|
| repo 檔 / cwd = repo root | `already_current` |
| repo 檔 / cwd = `rust/` | `already_current` |
| repo 檔 / cwd = `evals/` | `already_current` |
| repo 檔 / cwd = `/tmp` | `already_current` |
| **repo 外的檔 / cwd = repo root** | **`no_index`** |

**對一個指名了檔案的 payload,cwd 不再影響結果。** 上表五列都是這種 payload。不指名檔案的形狀
(`Bash`,以及所有還沒攔截到的)cwd 仍然是全部的答案 —— 只是 payload 自己主張的那個 cwd,不是 hook
行程繼承到的那個;相對路徑也以前者為基準(2026-09-05 覆審補上,見 §8)。而原本的重現 —— 從 `rust/` 編輯 `rust/src/hook.rs` —— 從 `no_index` 變成
`refreshed`。

## 7. 沒有修掉的

那四小時裡另外 **36 次拒絕**(20 `db_unavailable` + 16 `busy_or_failed`)不是這個 bug 造成的,這個
改動也修不到它們。對那些情況,失敗仍然**什麼都不記錄**,下一次 refresh 看到乾淨的樹就說
`already_current`,而 `stale=false` 仍然會說謊。

計畫原本有一個 `repair_owed` 標記要補這個洞,審查後**整個刪掉**:它只做了 `graph_pending` 的一半
(強制回報陳舊,卻不強制重建),而一次沒有候選的乾淨增量會成功返回並清掉標記 —— 它會製造它宣稱要防的
失敗。真正的版本得讓 `incremental_index` 真的還債,那是另一個設計。

## 8. 這一輪關於審查的結論

計畫在寫任何程式碼之前經過兩輪外部審查加逐條 corroborate,攔下 14 個缺陷,其中 3 個 blocker、
2 個是**設計錯**而非措辭錯。兩個值得記:

* Codex 指出 `repair_owed` 只做了先例的一半 —— 而那是我照著 `graph_pending` 抄的,抄了形狀沒抄機制。
* Kimi 指出招牌測試的 fixture 讓 cwd 與檔案**解析到同一個專案**,所以一個完全不讀 payload 的實作照樣
  通過。**這個改動存在的唯一理由,沒有被任何測試釘住。**

而 Kimi 在前一輪預言的 blocker,在實作時**真的發生了**:更新完全部呼叫端、`cd rust && cargo test`
419 綠 —— 而 evals crate 編不過。計畫寫「每個 task 驗兩個 crate」擋下了一個會靜默提交的破口。

一個我自己的疏忽也記著:`ddf7cacf` 的 clippy 其實是紅的(`index_state` 成了死碼),但驗證管線尾端用了
`tail -1`,把退出碼吃掉了。閘門只有在失敗能擋下提交時才有價值。

## 9. 出貨後的第三輪審查(Kimi,2026-09-05)

出貨後把 `f8795581..c6d359c2` 整段送 Kimi 覆審,六個定向問題。六條回覆,**兩條成立、三條不成立、一條
不動並記下理由** —— 比例本身不是重點,重點是不成立的那三條若照抄都會讓程式碼變差。

**成立(已修)**

1. **相對 `file_path` 的基準用錯了。** `None` 分支早就讓 payload 的 `cwd` 勝過行程的 cwd,`Some`
   分支卻把原始路徑直接丟給 `project_root_for_path`,於是相對路徑靠 `exists()`/`canonicalize` 對
   **行程**的 cwd 解析。Claude Code 送絕對路徑所以目前碰不到,但會送相對路徑的正是那兩個還沒攔截到的
   harness —— 也就是說,這個 bug 會在測試全綠的情況下修錯專案。修法是把同一條優先序套到 `Some` 分支。
   新測試 `a_relative_path_resolves_against_the_payload_cwd_before_the_process_cwd` 把兩個基準指向
   **不同專案**;把修正拿掉,它以 `{"no_index": 1}` 變紅,而舊的相對路徑測試照樣綠 —— 舊的那個抓不到。
2. **README 的分母是錯的。** 寫「79 of 318」,而本文 §2 的表加起來是 315,百分比也是對 315 算的。重新
   查 `usage.db`(2026-09-04 20:35–09-05 00:35,`command_log`)得 **315**,184/79/20/16/16 逐列吻合;
   318 哪裡都對不上。改成 315。順帶一提,把窗口往前推五分鐘會得到 320 —— 一個沒帶窗口的數字不可比,
   這正是 §2 的表要連時間一起寫的原因。
3. 同一條裡的第二半:「cwd 完全不再影響結果」**說得比量到的寬**。§6 那五列全是指名檔案的 payload;
   不指名檔案時 cwd 仍是全部的答案。README 與本文的句子都已收窄到量測涵蓋的範圍。

**不成立(逐條驗過)**

4. 「非 UTF-8 路徑元件會讓 `probe_root` 回 `Absent`,是把失敗讀成不存在」—— 不是。`path_to_utf8`
   (`indexer.rs:108`)對非 UTF-8 路徑直接 panic,所以這種根**從來不可能被索引**;那裡真的沒有索引,
   `Absent` 是正解。反過來說,是 `probe_root` 的 `to_str` 那道閘讓那個 panic 在 `hook-refresh` 這條
   路徑上不可達。
5. 「不可讀的目錄會被走過去,可能接到外層專案」—— 不是。Rust 的 `Path::exists()` 在 `EACCES` 時回
   false,所以 `d/file` 走上一層到 `d`;而 `d` 自己 `stat` 得到(權限看的是父目錄),`canonicalize(d)`
   成功,迴圈的**第一次** `probe_root` 問的就是 `d`。結果與可讀情況逐字相同。實測確認。
6. 「沒有任何測試釘住『path key 不認得時仍從 cwd 解析』」—— 有。
   `a_refresh_with_no_path_in_the_payload_still_uses_the_working_directory` 送的就是 `tool_input:{}`。
   這條真正剩下的半邊是「Codex 會不會把路徑放在別的 key」,而那**不准猜** —— 專案規則是欄位名只從攔截到
   的 payload 判斷,`main.rs` 的註解與 `2026-09-05-posttooluse-payloads.md` 都已經明寫哪些沒攔到。

**不動(記下理由)**

7. 「`probe_root` 的 5 秒 busy timeout 違反『寧可放棄也不等待』,而且沿祖先累加成 N×5s」—— 保留。
   資料庫是 WAL(`db.rs:130`),讀者不會被寫者擋住;索引檔放在**扁平的** cache 目錄、以路徑雜湊命名,
   所以只有真的被開過的祖先才有檔案可探(`db_path_for`,`db.rs:97`),典型是 0–1 個而不是 N 個;而真正
   要幹活的 `open_db` 本來就設同一個 5 秒(`db.rs:131`),探測並沒有把最壞情況拉長。要改的話該連
   `open_db` 一起改,那是另一個題目。
