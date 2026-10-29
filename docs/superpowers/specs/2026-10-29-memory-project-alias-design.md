# repo 改名後讓 ledger 跟著走：專案別名

日期：2026-10-29。分支 `measure`。對應 issue #7（aib #996 的回報）。第 3 版：併入 K3.1 Pro 兩輪設計審查（block 6 點、block 2 點），見文末「第 2 版修正」「第 3 版修正」，與前文衝突處以後者為準。

## 問題

`cort memory` 的專案識別是 `origin` 網址正規化後的雜湊（`memory_store.rs` 的
`project_from_git_origin`）。repo 改名、`origin` 換成新網址後，識別跟著換：改名前記的紀錄還在共享資料庫，
但掛在舊識別底下，`recall`、SessionStart 注入、`status` 都看不到，`--supersedes` 也找不到舊 id。2026-10-28
`lisyanggf/agent-portal-tpm` → `lisyanggf/oec-bsr-revamp-tpm` 實際發生；同日 monorepo 也改名，同樣會斷。
GitHub 會把舊網址轉址，所以 git 本身沒事，只有 cort 斷。

## 決定：別名，不搬資料

共享資料庫加一張別名表，把兩個專案識別連起來；讀取時把連在一起的識別當成一個專案。不改寫任何既有列。

不選「migrate 把舊列的 `project` 改成新識別」：那是對遠端的大量 `UPDATE`，做錯了沒有回頭路；還沒改
`origin` 的機器（或還沒升級的 binary）會一夜之間看不到全部紀錄；而且它一樣得處理「改名後已經在新識別下記了
幾筆」的情況。別名是一列、可以刪、對兩邊的機器都成立。

## 表

```sql
CREATE TABLE IF NOT EXISTS project_aliases (
  old_project TEXT NOT NULL,
  new_project TEXT NOT NULL,
  created_at TEXT NOT NULL,
  machine TEXT NOT NULL,
  PRIMARY KEY (old_project, new_project)
)
```

- 方向只是紀錄（誰是舊的），**讀取時當無向邊**：專案集合是從目前識別出發、沿別名雙向走到底的連通分量。
  這樣改了 `origin` 的機器看得到舊紀錄，還沒改的機器也看得到新紀錄。
- 連通分量用一句遞迴 CTE 在資料庫端算，不多一趟來回：

  ```sql
  WITH RECURSIVE g(p) AS (
    SELECT ?
    UNION
    SELECT CASE WHEN a.old_project = g.p THEN a.new_project ELSE a.old_project END
    FROM project_aliases a JOIN g ON a.old_project = g.p OR a.new_project = g.p
  )
  SELECT body FROM project_memory WHERE project IN (SELECT p FROM g)
  ```

  `UNION`（不是 `UNION ALL`）去重，有環也會停。別名表很小（一次改名一列）。

## 指令

- `cort memory alias add --from-remote <舊網址>`：用與 `origin` 相同的正規化算出舊識別，加一列
  `(舊識別, 目前識別)`。也接受 `--from-project <識別>`（例如 `status` 印出的那串），給不是從網址來的情況。
  - 拒絕：舊識別等於目前識別；舊識別底下 `project_memory` 一列都沒有（多半是網址打錯，錯誤訊息照實說，並
    提示用 `--from-project`）；已經在同一個連通分量裡（冪等，回 `already_linked`，exit 0）。
  - 寫入照 ledger 的規矩：寫完以重讀確認，結果分 `linked`／`not_recorded`／`outcome_unknown`。
- `cort memory alias list`：列出目前專案連通分量裡的每一條別名與每個識別各有幾筆紀錄。
- `cort memory alias remove --from-remote <網址> | --from-project <識別>`：刪掉「這個識別與目前識別」之間的
  那一列（只刪一列，不碰 `project_memory`）。給設錯時撤銷用。
- `cort memory status` 多印 `project_ids`（連通分量）；只有一個時與今天相同。

## 讀寫各處怎麼改

- **讀**（`read_bodies`、`hook-recall` 的 ledger 那句、`status`、衝突判斷）：`project = ?` 換成上面的 CTE。
  同一個 `id` 在兩個識別下都有時只算一次（`id` 是內容雜湊，正常不會撞；撞了視為同一筆）。
- **寫**：新紀錄一律寫在目前識別下，不變。
- **`--supersedes` 與 `resolve`**：檢查被取代的 id 是否存在、以及 `resolve` 的「這個 key 目前所有 active id」，
  都改成在連通分量裡找。寫入用的條件式 `INSERT … WHERE (SELECT COUNT(*) … project = ? AND id IN …)` 裡的
  `project = ?` 換成 `project IN (CTE)`，這樣「新識別的紀錄取代舊識別的紀錄」才成立，且仍是一句原子寫入。
- **DDL**：`project_aliases` 與 `project_memory` 一樣，在已經會送 `CREATE TABLE IF NOT EXISTS` 的路徑一起建
  （`read_bodies`、寫入路徑、`alias` 指令）。
- **hook-recall 不送 DDL**（它的合約）：別名表還不存在時，CTE 那句會是 SQL 錯誤。處理方式：hook-recall 已有
  「notes 那幾句 SQL 錯誤時、時間還夠就重送只有 ledger 的版本」；這個重送改成用不帶別名的舊句子
  （`project = ?`）。這樣在共享資料庫第一次被新 binary 寫過之前，SessionStart 照舊能讀，只是看不到別名。
- **notes 不在這次範圍**：`harness_notes` 是各機器未審的筆記，以自己的專案識別上傳；改名後它們也會分家，
  但影響的是「其他機器有哪些筆記」的清單，不是 ledger 本身。之後若要，用同一張別名表、同一個 CTE。
- **量測不受影響**：`telemetry_project_aliases` 是量測端「本機專案 id ↔ 共享專案 id」的對照，與這張表無關。

## 舊 binary

還沒升級的機器（i5940R3070 跑 `3ea01b2d`、i51149R3050 跑 `5d3be5ce`）不讀別名表：它們照舊只看自己 `origin`
那個識別。別名表存在不影響它們。升級後自動看得到。

## 測試

- 別名 CTE：舊→新一條、雙向讀得到、兩條串成三個識別、有環（A→B、B→A）會停、沒有別名時結果與今天相同。
- `alias add`：自己對自己被拒；舊識別沒有紀錄被拒並提示 `--from-project`；重複加回 `already_linked`；
  `--from-remote` 用與 `origin` 相同的正規化（ssh 與 https 兩種寫法得到同一個識別）。
- `alias remove` 只刪那一列，`project_memory` 列數不變。
- `record --supersedes <舊識別下的 id>`：加別名後成功，未加別名時是 `supersedes_missing`；被取代的舊紀錄在
  `recall` 裡變成非 active。
- `resolve` 跨兩個識別的同 key 衝突。
- `status` 的 `project_ids` 與 active／total 是連通分量的總和。
- hook-recall：別名表不存在時，ledger 照舊讀得到（走重送那句）；存在時讀到舊識別的紀錄。
- 全部走既有的 `SqliteTransport` 測試 seam，不碰網路與真的資料目錄。

## 部署後

在 `~/c/agent-portal-tpm`（tpm repo）跑一次
`cort memory alias add --from-remote https://github.com/lisyanggf/agent-portal-tpm.git`，確認
`cort memory recall "gateway option B"` 找得到改名前的紀錄；monorepo 若有 ledger 紀錄也做一次。結果回覆
issue #7 與 aib #996。

## 第 2 版修正（K3.1 Pro 設計審查，block，6 點；以下覆蓋前文）

1. **hook-recall 的退路要看是哪一步失敗。** 現在的重送只在 SQL 錯誤落在第 1 步以後（notes 那幾句）才發生
   （`memory_store.rs` `error.step.is_some_and(|n| n >= 1)`，步驟從 0 算）。別名表不存在時失敗的是第 0 步
   （ledger 那句），舊條件不會重送，SessionStart 會整個沒有 ledger。改成：
   - 第 0 步 SQL 錯誤 → 重送**不帶別名**的 `project = ?`（別名表還沒建）；
   - 第 1 步以後 SQL 錯誤 → 重送**帶別名**的 ledger 句（notes 表沒建，與別名無關，不該丟掉改名前的紀錄）；
   - 兩種重送都受同一個「剩餘時間 ≥ 200 ms」條件約束，且都只重送一次。
   測試三種：別名表不存在、notes 表不存在但有別名、兩者都在。
2. **`alias remove` 不能讓 ledger 讀不出來。** 新識別的紀錄取代了舊識別的紀錄之後，拆掉唯一連著兩邊的別名，
   新那筆會指向讀不到的 id，`validate` 會拒絕整份 ledger（`supersedes missing id`），recall、status、
   SessionStart、之後的寫入全部失敗。改成：remove 先讀整個連通分量的紀錄與別名，算出拆掉這一條之後的分量；
   若有任何紀錄的 `supersedes`／`resolves` 指向拆完後不在同一分量的 id，就拒絕（`alias_in_use`，列出那些
   id），不刪。remove 與另一台機器同時做跨識別取代的競態不做成原子：發生時的修復是把別名加回來
   （`alias add` 立刻讓 ledger 讀得出來），這寫進錯誤訊息。測試：跨識別 supersede 與 resolve 之後 remove 被拒；
   沒有跨識別引用時 remove 成功。
3. **「舊識別沒有紀錄就拒絕」改看整個連通分量。** A→B 之後又改名成 C，期間在 B 沒記任何東西：B 自己沒有列，
   但它連著 A。所以 guard 改成「舊識別所在的連通分量（含它自己）一筆紀錄都沒有」才拒絕。`--from-project`
   只是跳過「網址算雜湊」那一步，guard 一樣適用；錯誤訊息提示核對網址，或從舊名時 `cort memory status`
   印過的 `project_id` 拿識別。測試 A→B→C 的第二次加別名成功。
4. **同一個 id 出現在兩個識別下：用既有的 `same_claim` 合併。** 直接把兩列都交給 `ledger_from_bodies` 會因
   重複 id 被 `validate` 拒絕；隨便挑一列又可能把內容不同的兩列藏起來。改成在交出前依解析後的 `id` 合併：
   `same_claim` 成立（完全相同，或只差舊版 binary 丟掉的機器戳記）就保留一筆（有戳記的那筆）；不成立就報錯，
   與今天讀到損壞列的處理相同。排序不用另外處理：`ledger_from_bodies` 本來就依取代關係做拓撲排序，不靠時鐘。
   測試：完全相同、只差戳記、內容不同三種。
5. **舊 binary 會讓已被取代的 key 分岔。** 新 binary 在 B 寫了「B1 取代 A1」，還在讀 A 的舊 binary 仍看得到
   A1 是 active，可以合法地寫「A2 取代 A1」；合併後 B1 與 A2 都 active，成為衝突。升級後的 client 會照常報衝突、
   要求 `resolve`，不會默默選一個，但分岔本身擋不住。所以：
   - 部署步驟改成「會寫這個專案的機器都升級之後，才 `alias add`」；
   - `alias add` 成功時印一行相容性提醒（舊 binary 只看得到自己的識別，跨識別的更正要等它們升級）；
   - 測試這個衝突：合併後 `status` 列出衝突、`resolve` 能跨兩個識別解決。
6. **remove 不分方向。** 別名列有方向（誰是舊的），但讀取當無向邊；從 A 端或 B 端下 remove 都要刪掉連著兩者的
   那一列，不論它存成 `(A,B)` 還是 `(B,A)`。測試從兩端各 remove 一次。

此外，`alias add` 若發現「目前識別」與「舊識別」之間已經有一條反向的列，視同已連結（`already_linked`），不再
加第二列。

## 第 3 版修正（K3.1 Pro 第二輪，block，2 點；以下覆蓋前文）

1. **原子寫入的條件數「不同的 id」，不數列。** `--supersedes` 與 `resolve` 的條件式
   `(SELECT COUNT(*) FROM project_memory WHERE project IN (…) AND id IN (…)) = N` 改成 `COUNT(DISTINCT id)`。
   第 2 版第 4 點允許同一個 id 出現在兩個識別下（讀取時合併成一筆）；用 `COUNT(*)` 的話，`X` 兩列加 `Y` 一列會數成
   3、不等於 2，寫入影響 0 列，被誤報成 `supersedes_missing`，衝突就解不掉。測試：`X` 同時在 A、B 兩個識別下、
   `Y` 只在 B，合併後衝突 `{X, Y}`，`resolve X Y` 成功。
2. **第 0 步失敗的退路要保留 notes。** 別名表還沒建、notes 表已經在的資料庫上，帶別名的 ledger 句會在第 0 步失敗；
   這時只重送 ledger 會把好好的 notes 一起丟掉（hook-only 的機器可能一直如此，因為 SessionStart 不送 DDL）。改成：
   - 第 0 步 SQL 錯誤 → 重送**整條**管線，ledger 換成不帶別名的 `project = ?`，notes 兩句照舊；
   - 這次重送若在第 1 步以後 SQL 錯誤（notes 表也沒有）→ 再重送一次只有 ledger（不帶別名）；
   - 第 1 步以後的 SQL 錯誤（第一次就發生）→ 重送只有 ledger（帶別名），同第 2 版；
   - 每次重送前都要剩餘時間 ≥ 200 ms，最多兩次重送。
   測試：「別名表沒有、notes 表在」時 notes 照樣出現；「兩者都沒有」時拿到不帶別名的 ledger、沒有 notes。
3. （審查建議）remove 與跨識別寫入的競態補一個測試：直接造出「新識別的紀錄取代舊識別的 id、但別名不在」的狀態，
   確認讀取報 `supersedes missing id`、錯誤訊息提示加回別名，`alias add` 之後讀得出來。
