# 索引漂移是看不見的,而檢查說「all current」(2026-09-06)

## 1. 事件

2026-09-05,schema v5 + type-references 出貨後,這台機器上 **10 個索引裡有 7 個仍停在舊的
extractor**。`./install.sh --check` 對此回報:

```
indexes: all current
```

三件事同時為真,所以沒有人看得見:

* `list_projects` 不讀 `SCHEMA_VERSION` 也不讀 `extractor_version`,於是 `cort projects` 只暴露
  git head 意義下的 `stale`,而 `--check` 把那一個布林翻譯成「all current」。
* **那七個專案的 git head 一步都沒動。** 漂移的原因是 pack 規則變了,不是樹變了 —— 所以每一個現有的
  陳舊訊號都誠實地回報「新鮮」。
* `incremental_index` 一發現 extractor 不符就退回 full rebuild,而呼叫它的是每次編輯都開火、有五秒
  逾時的 `PostToolUse` hook。被砍就 rollback,下一次編輯再來一遍 —— **升級成了每次編輯都付一次、
  沒有人看得見的隱藏延遲。**

## 2. 這一輪做了什麼

診斷,不含修復。修復是後續計畫的事,而**這份改動刻意不讓 `--check` 變紅** —— 它沒有出貨任何修復,
紅燈會指向一個不存在的動作,而永久紅的檢查會訓練讀它的人忽略它。

* `ProjectListRow` 帶上 `schema_version` 與 `extractor_version`,從 `_cortex_meta` 讀 —— 就是
  `incremental_index` 比對的同一個鍵。
* `list_projects()` 回傳 `Vec<ProjectEntry>`,其中 `Unreadable` 是一個變體而非一次靜默的 `continue`。
  沒有 `projects` row 的資料庫仍然跳過(那是 schema-only,本來就不是專案)。`usage.db` 依
  `usage_db_path()` 排除。
* `cort projects` 每列多一個 `drifted`;`cort projects --verdict` 印一行
  `indexes\t<word>\t<drifted>\t<unreadable>`。
* `install.sh --check` 多印一條線,消費那個 verdict。既有的 stale / gone 報告一行未動。

## 3. 為什麼是兩個數字而不是一個

第一版的 verdict 是三欄:一個字加一個計數。審查指出**一個計數撐不起兩個字** —— 回報
「3 個由過時 extractor 建的」而其中兩個是**打不開的**資料庫,是對它們的不實陳述。

同一輪還指出一件相關的事:目錄已消失的漂移索引原本被算成 `compatible`,而它自己的 JSON row 卻寫著
`"drifted": true`。**「排除在計數外」和「否認事實」是兩回事**,只有前者站得住。現在漂移就是漂移,
樹還在不在由 `exists` 分開報。

真機在這一輪結束時回報 `indexes drifted 2 0`,而那兩個正是 `exists=false` 的暫存索引 —— 這個
決定在真機上被驗證了。

## 4. 出貨前的兩輪總審找到什麼

**兩家獨立收斂到同一個缺陷,而它是實作者親手造成的。** `get_meta` 回傳
`Result<Option<String>>`,刻意把「key 不存在」(`Ok(None)`)和「讀取失敗」(`Err`)分開 —— 這正是
第一輪審計畫時被要求的。實作採納了 `get_meta`,卻在後面接上 `.ok().flatten()`,**把它保留的區別又
丟掉了**。後果:`_cortex_meta` 讀不到的資料庫回報 `drifted`,對一個從沒讀到的版本做出肯定宣稱。

觸發門檻不高:這條掃描連線是裸的 `SQLITE_OPEN_READ_ONLY`,**沒有 busy timeout**,而 refresh hook
每次編輯都在寫。一次短暫的 `SQLITE_BUSY` 就夠了。

其餘三條:

* `read_dir` 失敗時回空清單,於是 verdict 對一個**從沒讀過**的目錄回答 `compatible 0 0`。
  這違反「儲存失敗是回傳,不是吞掉」。
* bash 只檢查前兩欄,截斷的 `indexes\tcompatible` 被讀成「current」。註解寫著 fail closed,
  程式碼沒有做到。
* 既有的 fallback 訊息怪罪一個從不存在的 `cort projects --stale` 旗標。

## 5. 而測試也有同一個病

審查獨立找到:**`compatible` 這個狀態沒有任何測試釘住。** 形狀測試跑在空族群上並接受三個字裡的任何
一個,而其餘測試全部只驗漂移或不可讀。實測確認:一個寫成
`if n_drifted > 0 { "drifted" } else { "unknown" }` 的實作**通過全部六個 CLI 測試與 25 個 db 測試**,
而一台完全修好的機器會永遠顯示「compatibility unknown」。

`install.sh` 那三個正向分支同樣從未被執行過 —— 一句無條件的
`echo "compatibility unknown (unparsable verdict)"` 可以通過整個 smoke 套件。

**修正的過程本身又找到第五個。** 為了證明 fail-closed 的修正有效而故意拿掉數字檢查時,smoke **沒有
變紅** —— 因為截斷的那條被「字與數一致性」那道擋下了。也就是說**數字檢查沒有被任何東西釘住**:
`indexes\tdrifted\tx\ty` 會讓 `--check` 把 `x` 和 `y` 當成測量值印出來。補上該 fixture 後,拿掉那道
guard 會讓 smoke 出現兩個 FAIL。

## 6. 這一輪關於方法的結論

* **每一條修正都配一個會變紅的測試,而且真的去改壞它。** 這一輪有九次故意改壞;其中一次沒能變紅,
  那一次比其他八次加起來更有價值,因為它指出的是一道沒有人守著的 guard。
* **「拿了修正卻扔掉它的好處」是一種真實的失敗模式。** `.ok().flatten()` 那一行的作者知道
  `get_meta` 保留了區別 —— 那是他自己在計畫裡寫下的理由 —— 然後在實作時把它壓掉了。計畫的散文與
  計畫的程式碼片段互相矛盾,而實作照著程式碼走。**計畫裡的程式碼片段和它的散文一樣需要被審。**
* 執行階段浮現的缺陷有五個是三輪紙上審查都沒抓到的(其中一個是 `git commit` 在空目錄會失敗)。
  **紙上審查抓設計,跑起來抓現實**,兩者不能互相取代。
