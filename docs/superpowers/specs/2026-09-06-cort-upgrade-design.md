# `cort-upgrade`:獨立升級程式(設計,2026-09-06)

狀態:**v2**,經 Codex(機制)與 Kimi(對抗式)各一輪審查並逐條核實。所有對本 repo 程式碼的陳述都附
`file:line` 且經我親自驗證;審查者提出但未經我核實的,列在 §9 並明確標為未採信。

## 0. 這份設計要解決的事件

2026-09-05,schema v5 + type-references 出貨後,這台機器上 **10 個索引裡有 7 個仍停在舊的
extractor**。`./install.sh --check` 對此回報「indexes: all current」。

三件事同時為真,所以沒有人看得見:

* `list_projects` 不讀 `SCHEMA_VERSION` 也不讀 extractor(`db.rs:343-380`,而且它**安靜跳過打不開的
  資料庫**),於是 `cort projects` 只暴露 git head 意義下的 `stale`(`main.rs:1611`),`--check` 把那
  一個布林翻譯成「all current」(`install.sh:873-899`)。
* `incremental_index` 有**三個**退回 full rebuild 的觸發點,不只一個:extractor 不符、`graph_pending`、
  以及 git 候選集合不可信(`incremental.rs:309-332`)。
* 呼叫它的是 `PostToolUse` 編輯 hook(`main.rs:817`),而三種 dialect 的 hook 逾時都是共用的
  `TIMEOUT_SECS`(`settings.rs:208`、`settings_toml.rs:111`、`settings_kimi.rs:68`)。被砍就 rollback,
  下一次編輯再來一遍 —— 永遠。

**升級成了每次編輯都付一次、沒有人看得見的隱藏延遲。** 這不是遺漏的拋光,是診斷正確性的 bug。

## 1. 它是什麼,界線在哪

**`cort-upgrade`:repo-local 的 Rust 執行檔,從不安裝,從新的原始碼樹執行。**

因為它跑的是新版的碼,它天生知道新版需要什麼;它要診斷的只有「裝著的是什麼」。這是 rustup 交棒給新下載
的 `rustup-init`、dpkg 執行新套件的 maintainer script、VS Code 把替換交給獨立 installer 的同一個形狀。

**為什麼是獨立執行檔,而不是 `cort upgrade` 子指令。** v1 給的理由(需要 `db`/`pack`/`settings` 的
crate 內部)**證明太多** —— 同一個 crate 裡的子指令有完全相同的存取。審查點出這點是對的。真正的理由
有兩個:(a) 這是使用者定下的形狀 ——「獨立的升級程式」;(b) 升級是 ops 動詞不是產品動詞,把它放進出貨
的 `cort` 會讓**已安裝的舊版**也長出一個過時的 `upgrade` 子指令,而那正是「舊版決定新版需要什麼」的
倒錯。先例是 `fake_ast_grep`(宣告在 `rust/Cargo.toml`、永遠建置、從不安裝);
`tests/install-smoke.sh` 斷言 payload 只出貨 `cort` 和 pack,本設計不動那條。

**界線**

| | 職責 |
|---|---|
| `install.sh` | **只做全新安裝**。偵測到既有環境就讓路。 |
| `cort-upgrade` | 診斷既有狀態 → 依新版需求逐元件升級 → 單一結論。 |

**反漂移規則(硬約束)。** 升級程式**不得重述任何清單**。這條不是潔癖:`install.sh` 曾把 `HOOK_TARGETS`
重述四次,出貨的是 bash 那份,binary 自己的預設從未被執行過並已腐爛(CLAUDE.md,
`docs/2026-09-03-installer-dedup-and-attribution.md` §2、§3)。

**而審查證明這條規則今天就已經被違反,不是未來風險:**

* **ast-grep 的釘選有兩個家。** `install.sh:12` 的 `AST_GREP_VERSION="0.45.2"`,以及
  `rust/src/ast_grep.rs:13` 的 `AST_GREP_PINNED = "0.45.2"`。兩份手寫字串,今天恰好相等。
  **修法:Rust 是唯一的家,`install.sh` 從 binary 問。**
* **shim 樣板只住在 bash**(`install.sh:753-757`,內嵌 `CORT_VERSION` 與絕對的 `CORT_HOME`)。
  升級程式要比對 shim 內容就得在 Rust 裡重述一份。**修法:shim 由 Rust 產生(一個子指令輸出它),
  `install.sh` 只負責寫檔** —— 與 `HOOK_TARGETS` 當年走的是同一個方向。
* **manifest 的 key-set 沒有家。** v1 寫「目標狀態來源:本設計」—— 設計文件不可執行。全新安裝由
  bash 寫 key,升級由 Rust 寫 key,就是兩個寫者一個 schema。**修法:key-set 定義在 Rust,
  `install.sh` 消費它。**

**建置順序:診斷先,執行者後。** 診斷那一半獨立有價值且便宜 —— 光是讓 `list_projects` 回報 extractor
漂移(那一行本來就存著 `extractor_version`,`indexer.rs:468`)就會在第一天抓到 7-of-10 事件。把它和
昂貴、仍有爭議的執行者綁在一起發布,是讓確定有用的部分等待不確定的部分。

## 2. 元件清單與診斷

| # | 元件 | 目標狀態的家 | 現在怎麼診斷 | 缺什麼 |
|---|---|---|---|---|
| 1 | payload(binary + pack) | 新原始碼樹 | `cort --version` | **假的**:shim 攔截 `--version` 印死字串,不執行真的 binary(`install.sh:753-757`)。這個檢查不可能不一致。 |
| 2 | shim | 待搬進 Rust(§1) | 存在性 | 內容比對 |
| 3 | ast-grep CLI | 待收斂為 `ast_grep.rs:13` | `assert_ast_grep_version` | 版本本身有兩個家 |
| 4 | skills(**3 個** dest) | repo 內的 SKILL.md | `skill_is_managed` | **只證明「是我們寫的」,從不跟新版原始檔比**(`install.sh:189-200`)。舊 skill 配舊 stamp 照樣過。 |
| 5 | hook 條目(3 檔 × 2 事件) | `HOOK_TARGETS`(`main.rs:1187`) | `hook-install --status` | 回報「存在且是我們的」,不是「等於新版的正規形狀」 |
| 6 | manifest **key-set** | 待搬進 Rust(§1) | `manifest_version` | 只描述語法,不描述任何其他元件是否達標 |
| 7 | 每個專案的索引 | `SCHEMA_VERSION` + `extractor_version` | git head 的 `stale` | **完全看不見 schema 與 extractor 漂移** —— 就是 §0 |
| 8 | `usage.db` | `USAGE_SCHEMA_VERSION`(`usage.rs:10`) | 無 | **有版本、且不符時硬拒**(`usage.rs:698-709` → `usage_corrupt`)。也帶著 `MACHINE_ID`(`usage.rs:21`),CLAUDE.md 視之為證據衛生的載重項。 |
| 9 | `hook-gate` 狀態檔 | — | 無 | 每 session 每 symbol 一次的 deny 閘,存在索引快取旁(`main.rs:670-674`) |

第 8、9 列是審查補上的,v1 兩者都沒有。第 4 列 v1 數錯(寫 2 個,實際 3 個:xgrep、ast-grep、
ast-grep for Codex,`install.sh:25`、`:28`、`:32`)。

**`xg` / xgrep 明確劃出範圍外。** 解除安裝仍會移除它們;本設計不診斷、不升級它們。這是一個**明說的**
決定,不是沉默。

**診斷的三條規則**

* **binary 的身分由真的執行檔回答,不由 shim 回答。** 升級程式從 manifest 取得 `CORT_HOME`,直接執行
  那個檔案。回答必須是**行為變了就會變**的東西:`extractor_version` + `SCHEMA_VERSION`,而不是手寫
  字串。
* **「不可讀」不等於「不存在」,且不算通過。** 先例已在本 repo:`RootProbe::Unreadable` 存在的唯一
  理由就是不讓呼叫端把失敗讀成「這裡沒東西」(`db.rs:559-562`)。而 `list_projects` 今天正是安靜跳過
  打不開的檔案(`db.rs:343-380`) —— 那個行為會讓不可讀的索引隱形,必須改。
* **診斷要有 manifest 這個 catch-all。** 解除安裝信任 manifest 作為「存在什麼」的紀錄
  (`install.sh:590-606`),診斷只信任一張 9 列的表。未來 `install.sh` 學會記錄的任何新產物,會被正確
  解除安裝而完全不被診斷。診斷必須同樣以 manifest 為底,表是它的解釋層。

## 3. 執行中的單元:通知、安全停止、或強制

### (a) 替換 payload —— v1 的原子性宣稱是錯的

v1 寫「整代備好再原子 rename,任何行程在任何瞬間看到的都是完整的一代」。**`rename(2)` 不能替換非空
目錄。** `CORT_HOME` 內有 `cort` 與 `pack/`,所以那是兩次 rename,中間 `$CORT_HOME/pack` **不存在**。

而那個視窗的後果,審查與我的核實合起來比 v1 寫的更嚴重:

* `pack.rs:47-67` 的 `walk()` 對失敗**全部安靜跳過**(`read_dir` 失敗直接 `return`,`file_type` 失敗
  `continue`)。所以缺檔的主要後果**不是 panic,是檔案清單變短、雜湊看起來完全合法的假 extractor
  identity**。最壞情況:`sgconfig.yml` 在而規則檔只複製一半 → 掃描成功、抽出較少的邊、蓋上假版本號。
  **安靜地寫入錯的資料。**
* `pack.rs:13-14` 的註解宣稱 fail-closed 由 `sgconfig()` 保證 —— 但 `extractor_version()` 不呼叫
  `sgconfig()`,那個保證不涵蓋雜湊這條路徑。
* 檔案在**列舉之後、讀取之前**消失才會 `panic!`(`pack.rs:80-81`),而它是 `incremental_index` 的
  **第三個**敘述(`incremental.rs:305-309`;v1 誤寫為第一個)。

**正確的機制:世代目錄 + symlink 翻轉。** `CORT_HOME` 是一個 symlink,指向 `cort-<generation>`。
新一代寫進新目錄、驗證完整,再以一次 `rename` 把 symlink 換掉 —— **symlink 換 symlink 的 rename 是
原子的**,這才是 Homebrew 的 `opt/`、Nix、Capistrano 用的形狀。shim 內嵌的
`CORT_PACK_DIR="$CORT_HOME/pack"` 每次呼叫都重新解析,所以翻轉即刻生效且沒有混代。

**還在範圍外的兩個東西,必須明說:**

* **shim 自己在 `$BIN_DIR`,不在 `CORT_HOME`**,是分開 `mv` 的(`install.sh:759`)。§5 必須有它的步驟。
* **ast-grep CLI 也在 `$BIN_DIR`**(`install.sh:720`),不被 symlink 翻轉涵蓋。**但混代不是無聲的** ——
  `pin_bin()` 會 `assert_ast_grep_version`(`main.rs:503-507`),版本不符是明確失敗。審查說「沒有東西
  會標記它」這點**不成立**,我驗過。

### (b) 遷移使用者資料 —— 這裡才需要協調

「執行中單元」在這裡的特殊性:**大部分還不存在**。hook 是三個 harness 在每次搜尋、每次編輯時生出來的
短命行程,活躍時每分鐘數十個。沒有 daemon、沒有 supervisor、沒有可列舉的單元集合。**你沒辦法通知一個
還沒被生出來的行程。**

**機制:兩個 flock,不是標記檔加 TTL 租約。**

* **admission 鎖** 與 **activity 鎖**,都放在會被替換的世代目錄**之外**的穩定位置。
* 一般受保護的操作:短暫取得 admission(shared)→ 取得 activity(shared)→ 放掉 admission → 做完
  pack 與資料庫的工作 → 放掉 activity。
* 升級程式:先取得 admission(**exclusive**),於是**新生出來的 hook 一律讓路**;再帶期限地取得
  activity(exclusive),這會**排空所有已經通過 admission 的在途工作**。兩把鎖持有到切換與遷移完成。
* **不需要列舉 pid。**

**崩潰安全由 OS 免費提供。** 升級程式以開啟的 fd 持有鎖,行程死亡即由核心釋放 —— 不需要清理程序、
計時器或 supervisor。v1 的 TTL 租約**較差**:沒有獨立的續約者,而牆鐘到期會把一個合法但慢的遷移誤判
為死亡。

**成本。** 每次受保護的呼叫多兩次 open 加兩次 shared lock。被 shape gate 擋掉的 `hook-suggest` 呼叫
**不必付**(那條路徑本來就不開資料庫、不做 canonicalize);只有真的碰資料庫或 pack 的才付。

**誠實的邊界:第一次升級無法排空舊 binary**,因為舊版從不取這把鎖。SQLite 的重試可以排除舊的**寫者**,
但排除不了它的 WAL **讀者**。那一次要嘛帶期限後中止並指名這個風險,要嘛遷移到複製出來的世代再切換路徑。
**宣稱從 SQLite 狀態就能證明排空,是假的。**

### (c) 「通知 → 安全停止 → 強制」對照到這個架構

| 原始框架 | 這裡有沒有對應 |
|---|---|
| 通知單元 | **有,但反向** —— 關閉 admission 閘;未來的單元自己在入場時發現 |
| 安全停止 | **有,而且免費** —— `hook-refresh` 的 quiet 路徑本來就把失敗映射成安靜成功(`main.rs:757-769`) |
| 強制停止(30ms 的 hook) | **沒有意義**。殺一個短命 hook 比等它的鎖掉下來更複雜也更不安全 |
| 強制停止(長跑的 `cort index`) | **有意義但不做**。帶期限地等,逾時就**中止升級**並回報,而不是殺使用者的前景工作 |

## 4. 遷移狀態:讓「需要遷移」持久且**出現在答案裡**

**這是本輪最重要的修正。** v1 只說 hook 拒絕 full rebuild 並記下「看得見的 outcome」,沒說記在哪、
誰把它秀出來。而現有的陳舊訊號全是 git/內容導向的:

```
index_is_stale = graph_pending || !deleted.is_empty() || !changed_files.is_empty()
                                                            (staleness.rs:95-98)
```

§0 的事件裡**專案樹的 git 完全沒動**。所以在 v1 的設計下,被拒絕重建的專案會用**舊 extractor 的語意**
持續回答 `impact`,而每一列旁邊印著 `index_is_stale: false`,且唯一的修復路徑(編輯 hook)剛被關掉。
**那是把吵鬧的跑步機換成安靜的錯答案** —— 依這個產品自己的教條,那是退步不是修復。

**所以拒絕與陳舊訊號必須綁在一起,這是設計的一部分而不是實作細節。**

**做法:把 `graph_pending` 擴充成帶目標的「重建需求集合」,不是重用那個布林,也不是另立半個旗標。**

`graph_pending` 今天的語意正好是所需的**成對**性質:每次提交前設定(`incremental.rs:184-202`、
`:242-262`)、`compute_stale` 把它 OR 進去(`staleness.rs:95-98`)、下一次 incremental 強制 full
rebuild(`incremental.rs:320-325`)、而只有成功的最終交易才清掉它(`incremental.rs:398-420`)。

但名字太窄:extractor 改變會使 chunks、檔案雜湊、raw edges、relationships 全部失效,而 `graph_pending`
描述的是「檔案仍有效但圖可能不完整」。所以擴充成**理由集合 + 目標世代**:

| 理由 | 誰設定 | 誰可以清 |
|---|---|---|
| `graph_incomplete` | 既有邏輯 | 一次成功的 incremental(重建關係後) |
| `extractor_changed` | 升級程式 | **只有**針對該目標的成功 full rebuild |
| `schema_changed` | 升級程式(結構遷移後) | 同上 |

一個共用的讀取器同時供應 `index_is_stale` 的可見性**和** full rebuild 的決策 —— 保住那個雙面合約。
**這正是被刪掉的 `repair_owed` 沒做到的事**:它強制回報陳舊卻不強制償還,而一次乾淨的 incremental
就能清掉它的宣告(`docs/2026-09-05-hook-refresh-follows-the-file.md` §7)。

**hook 必須同時停止做結構遷移。** `open_project_tracked` **無條件**呼叫 `ensure_schema`
(`main.rs:141`),而 `hook-refresh` 走的就是它。所以就算擋掉 full rebuild,hook 仍是結構遷移器。
hook 的開啟路徑必須能在**不執行遷移**的前提下檢查 schema 相容性。

**政策由呼叫端宣告,不在 `incremental_index` 裡再寫一份。** 型別化的呼叫策略
(`AllowFullRebuild` / `ForbidFullRebuild`),前景索引與 hook 各有自己的呼叫點
(`main.rs:823-847` 與 `main.rs:1487-1514`),不需要從環境變數或指令字串推斷。被拒絕時回傳
`FullRebuildRequired { reasons, target }`。且要拒絕**全部三個**觸發點,不只 extractor。

**eager vs lazy:兩位審查者結論相反,我採取中間但有理由的立場。** Codex 主張標記+延後(衍生資料、
專案可能再也不會用到);Kimi 指出這個 codebase **量過**沒有執行者的訊號一文不值(19 次 `cort index`
對 2,700+ 次 hook 開火,`main.rs:706-712`)。兩邊其實指向同一件事:**訊號必須有執行者**。所以:
**目錄還在的專案預設 eager 重建**(實測每個 1.4–3.3 秒,10 個約 20 秒 —— 升級是一個明確的前景動作,
這個成本可以接受);目錄消失的只標記;`--defer` 只標記不重建。無論哪條路,**持久標記一定寫下**,
所以沒有任何專案會回到「無聲」。

**標記本身可能失敗。** 若標記是寫進專案資料庫,它會撞 `SQLITE_BUSY`。既有的 `db_unavailable` 安靜
路徑(`main.rs:815`)是形狀範本;設計必須指明哪種失敗會記下標記、哪種不會,而不是假設寫入必成功。

**使用者前景的 `cort index` 在升級持有鎖時該不該被拒?** 不該無條件拒絕 —— §3(c) 自己的排序是
「砍掉使用者前景的工作比延後那個專案更糟」。前景動作等鎖,並告知原因。

## 5. 順序與崩潰安全

1. **診斷**(唯讀)
2. **取得並建置**新 payload 到 `cort-<generation>`(新目錄,不碰現行世代)
3. **驗證**新世代:pack 檔數與雜湊、binary 能回答身分、**拒絕不完整的 pack**(§3a 的假身分風險)
4. **取得 admission(exclusive)**,再帶期限取得 **activity(exclusive)**;逾時 → 中止,不殺任何行程
5. **翻轉 symlink**(原子);舊世代保留
6. **更新 shim**(v1 漏了這一步)與 **ast-grep**,兩者都在 `$BIN_DIR`,不在世代目錄內
7. **重新斷言 hook 佈線**:執行**新的**(已上架的)binary 的 `--status`,把條目指令與它自己由
   `HOOK_TARGETS` 導出的期望比對 —— 不是「存在且是我們的」
8. **重新部署 skills**:以**內容**比對新原始檔,不是以擁有權
9. **遷移索引**:結構遷移;full rebuild 依 §4 政策
10. **釋放鎖**,清除舊世代
11. **單一結論**

**「診斷一次然後全程相信它」是錯的。** 每個提交邊界都要重新驗證。

## 6. 結論必須單一、誠實,而且**可以被使用者處理**

現況:`hook-install --all` 把單一目標的失敗變成 row 欄位而**仍回傳成功**(`main.rs:1258`),bash 記成
`NOT wired` 的 info 後**繼續跑完並印 `Done`**(`install.sh:537`)。那個 per-target 行為是**對的**且
刻意的;錯的是整體結論。

但 v1 的「任何元件未達標就非零結束」沒有逃生口,審查給了三個會讓機器**永久紅燈**的真實情境:

| 情境 | 為什麼 v1 會錯 |
|---|---|
| 離線機器的 ast-grep | 這是**離線**產品,而 ast-grep 來自 GitHub release 或需要網路的 `cargo install`(`install.sh:710`、`:723-725`)。防火牆後的機器永遠非零,而且重跑一樣。 |
| 使用者自己改過 SKILL.md | §5 第 8 步以內容比對 ⇒ 合法的手改變成永久失敗,而「修復」是覆蓋掉使用者的編輯 —— 破壞性的。 |
| 一個壞掉的 harness 設定檔 | 每次執行都報失敗。**永遠紅的結論會訓練使用者忽略它**,那就變成狼來了。 |

**所以需要:**

* **結束碼分類**:`0` 全部達標 / `1` 部分未達標但可繼續使用 / `2` 致命(無法安全繼續)。單一非零不夠。
* **每個元件的持久確認**(`--ack <component>`):使用者可以說「我知道,這樣就好」,而該元件之後降為
  資訊列而非失敗。
* **`--keep-mine`**:手改過的 skill 不被覆蓋,回報為「已偏離,依使用者要求保留」。
* 失敗必須**指名元件與可執行的下一步**。沒有歸屬的結束碼只有一半的價值。

`unreadable` 不算通過、目錄消失不算失敗 —— 這兩條 v1 是對的,保留。

## 7. 測試

紀律:**每個性質都要有一個「故意改壞實作就會變紅」的 fixture**。前三輪外部審查各找到一個測不到自己的
測試,所以下面直接寫出**弱 fixture 會長什麼樣**,避免實作時重蹈。

| 性質 | 弱 fixture(會過但性質是壞的) | 必須改成 |
|---|---|---|
| 逐元件診斷 | 用 `cort --version` 的差異 —— **永遠不可能紅**,因為 shim 攔截它印死字串(`install.sh:753-757`) | 兩個 payload 目錄,crate 版本相同但 **pack 差一個 byte**,加一個說謊的 shim;斷言在**判決欄位**上。skill 那格的弱 fixture 是「缺 stamp」(那是已知壞掉的檢查),必須改成**舊內容 + 有效的當前 stamp** |
| 原子切換 | 切換後讀一次 pack,斷言不 panic | 一條讀取執行緒跨越切換持續呼叫 `extractor_version()`,斷言**不 panic 且身分 ∈ {舊, 新}**;另加一個「pack 少一檔」的暫存世代,驗證第 3 步必須拒絕它。且必須跑**真實佈局的真實切換**,玩具 temp-dir 測試會過而正式的兩次 rename 會 panic |
| 鎖與崩潰安全 | 過期租約 + 死 pid → 斷言有進展(**沒有任何 pid 存活檢查也會過**) | **未過期**但持有者已死(驗 OS 釋放鎖);**pid 重用**(未過期、pid 活著但屬於別的行程)必須拒絕;兩個升級程式競爭 |
| 單一結論 | 開跑前先弄壞一個元件,斷言非零(**「診斷到任何東西就非零」也會過**) | 在**診斷之後、重新佈線之前**弄壞(驗證 §5「每個提交邊界重新驗證」),斷言失敗**並指名該元件** |
| 拒絕不製造錯答案(§4) | 斷言 hook 回 `needs_rebuild` | 斷言該專案的 `impact` 輸出 **`index_is_stale: true`** —— 這是 §4 存在的理由,也是唯一能證明它沒退化成安靜錯答案的斷言 |

## 8. 為什麼不是「三個小改動就好」

誠實的對照組:只改 `cort projects` 報漂移、讓 hook 拒絕 full rebuild、修掉 `rm -rf`。

它在自己的條件下就不成立:**`install.sh` 收斂成只做全新安裝之後,那三個改動裡沒有任何一個會讓
`CORT_HOME` 前進**;修 `rm -rf` 修的是一條在新約束下只在乾淨機器上跑的路徑。而 (i)+(ii) 的結果是一個
永久、可見、**沒有執行者**的 `needs_rebuild` 橫幅 —— 而這個 codebase 已經量過沒有執行者的訊號值多少
(`main.rs:706-712`)。

只有一個轉換的擁有者才能 stage-validate payload(半複製的 pack 會算出假身分,§3a —— 三行的
`rm -rf` 修正不處理這件事)、才能持有鎖並以正確順序執行遷移。

**但診斷那一半確實應該先出貨**,見 §1 的建置順序。

## 9. 審查者提出但我未核實,故不納入

Codex 第一輪 11 條裡的五條:manifest 在 preflight 前就變動、manifest 寫入非原子、hook currentness 比
輸出弱、skill 與 stamp 非單一可復原單元、缺乏安裝器層級並行控制。這些看起來合理,但我沒有親自驗過,
所以不寫進上面任何一節。實作前應各自驗一次。

**已驗證為不成立的審查意見(記下來,避免重來):**

* 「ast-grep 混代會產生資料列而沒有東西標記它」—— 不成立。`pin_bin()` 會
  `assert_ast_grep_version`(`main.rs:503-507`),版本不符是明確失敗。
* 「`SCAN_ENGINE` 的 0.45.3 與 `Cargo.toml` 的 0.45.2 不一致」—— 我自己一度懷疑,查了 `Cargo.lock`:
  caret 需求解析到 **0.45.3**,字串是對的。

## 10. 執行階段暴露、已排入後續計畫的

計畫一(診斷)實作與兩輪總審過程中浮現,**在紙上審查階段沒有被抓到**。記在這裡而不是記在計畫一裡:
一份已完成的計畫不會再有人回去讀,而「唯一的紀錄躺在沒人讀的地方」正是這個 repo 反覆吃虧的形狀。

| # | 事實 | 排入 | 為什麼不是現在 |
|---|---|---|---|
| 1 | **掃描連線沒有 busy timeout。** `list_projects` 的 `SQLITE_OPEN_READ_ONLY` 連線是裸的(`db.rs`),而這個 module 其餘每一處都走 `with_busy_retry` 或設 5 秒。refresh hook 每次編輯都在寫,所以一次短暫的 `SQLITE_BUSY` 就會讓一個健康的索引被報成 `unreadable`。 | **計畫二** | 現在的行為是**誠實但過度悲觀** —— 它報「讀不到」而不是撒謊說漂移,方向是對的。加上重試會把它變成「讀得到」,那是改善不是修錯。 |
| 2 | **`extractor_version` 在資料庫裡存兩份。** `projects.extractor_version` 欄位與 `_cortex_meta` 的同名鍵,由 `indexer.rs:436-437` 與 `:377-381` 在同一交易寫入。今天兩者必然一致,因為只有一個寫者。 | **計畫三** | 這是資料庫內部的反漂移違規,與 §1 那三條同類(shim 樣板、ast-grep 釘選、manifest key-set),應該一起收斂到單一的家。分開修會做兩次遷移。 |
| 3 | **`pack.rs:47-67` 的 `walk()` 對失敗全部安靜跳過**,於是半個 pack 會算出看起來合法的假 extractor identity(§3a 已載)。 | **計畫三** | 由第 3 步「驗證新世代完整、拒絕不完整的 pack」涵蓋。但 `walk()` 本身的安靜跳過應一併改成回傳失敗 —— §3a 只擋住了升級這條路徑,任何其他呼叫者仍會拿到短掉的清單。 |
| 4 | **Codex 首輪五條未核實**(§9)。 | **計畫三開工前** | §9 只說「未採信」,沒說誰去驗。實作計畫三之前必須各驗一次,因為其中四條(manifest 在 preflight 前變動、manifest 寫入非原子、skill 與 stamp 非單一可復原單元、缺安裝器層級並行控制)都直接落在計畫三要動的表面上。 |

**一條方法上的結論,寫在這裡因為它會影響後續兩個計畫怎麼被審:**

計畫一的實作出現過一次「拿了修正卻扔掉它的好處」—— 審查要求用 `get_meta`(它保留了「key 不存在」與
「讀取失敗」的區別),實作採納了,然後在下一行接上 `.ok().flatten()` 把區別壓掉。**計畫的散文與計畫
自己的程式碼片段互相矛盾,而實作照著程式碼走。** 後續計畫的審查必須把程式碼片段當成規範來審,不能只審
散文;而實作者遇到兩者矛盾時,必須停下來回報而不是自行擇一。
