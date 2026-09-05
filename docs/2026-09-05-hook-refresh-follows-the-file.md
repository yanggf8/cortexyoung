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

**cwd 完全不再影響結果。** 而原本的重現 —— 從 `rust/` 編輯 `rust/src/hook.rs` —— 從 `no_index` 變成
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
