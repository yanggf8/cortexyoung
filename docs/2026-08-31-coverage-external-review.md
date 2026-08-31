# `impact --coverage` 的外部覆核紀錄（2026-08-31）

> 為什麼存在：這個功能聲稱能回答「枚舉漏了誰」，而寫它的人（我）剛在同一份資料上連兩次自我欺騙
> （30 格 null 指標、把貼上的報告當需求）。所以這一次把判斷交給**不同模型的 agent**去反證，
> 並把結論與處置落檔，不留在對話裡。

- 指令（同一份，送給兩個引擎）：要它找 **false negative**、要它自己建 `/tmp` 小倉庫驗六種呼叫形狀、
  禁止改 repo。開頭就寫「不要附和，你的工作是找出聲稱在哪裡是假的或過頭的」。
- 引擎順序與實測：`agy --model gemini-3.1-pro-high`（完成，第一輪發現全部出自它）→
  `zglmcode`（預設 `glm-5.3` 在 `-p` 路徑被 `unrecognized_model` 拒；`--model glm-5.1` 可用但撞 429
  週上限）→ **Kimi K3**（`-m kimi-code/k3`，登入＋更版後可用，`-p` 模式有 shell）。
  Kimi 的 `-p` 與 `-y`／`--auto` 互斥，但不加這兩個旗標時工具權限本來就通過探測。

## 發現與處置

| # | 覆核結論 | 严重度 | 我的處置 |
|---|---|---|---|
| C2(e) | 有盲檔（chunker 讀不了的檔案）時，per-seed 仍回 `enumeration_may_be_incomplete=false` —— 等於發安全通行证 | **高（真 bug）** | **已修**：盲檔現在會倒向 `incomplete=true`，並新增 `why: [.. blind_files ..]`；`blind_files.unparsed_example` 給路徑不給純計數；回歸測試 `a_blind_file_is_never_a_clean_bill_of_health` |
| C3 | `SKILL.md` 叫 agent「刪除前先跑 `impact`」卻不含 `--coverage`；空 `dependents` 陣列會被当成許可 | **高（引導錯誤）** | **已修**：第 1 條改為 `--coverage` 必帶，並明寫「`--coverage` 是啟發式螢幕、不是完整性證明、看不見 indexer 不讀的檔案（`.sh`/`.txt`/設定檔）」 |
| 殘餘 | 呼叫藏在**非來源檔**（`.sh`、`.txt`、設定檔）時，三層全部看不到 | 中（限制） | 不掩蓋：`reading` 與 skill 都明文寫出這個邊界；擴大到非來源檔需要另案（體積與二分檔） |
| C4 | `extracted_but_unresolved` 不是死重量：它建了 `crate::def::my_func()` 的案例，被 `resolve_targets` 丟掉 | 高（**產品 bug，尚未修**） | 記錄為待修：模組限定呼叫在 Rust 側永不解析。螢幕先讓它可見，修在 `graph.rs` 的解析規則，屬下一個提案 |
| C5 | 成本可忽略（logInfo `--coverage` 約 0.126s）；原因排序確實讓真漏洞壓過雜訊 | — | 採信並更正：我先前在 commit message 把 `--depth 3` 的 4,410→7,428 bytes 與 `--depth 1` 的計數混寫，數字本身對、depth 標錯，此處更正 |
| 措辭 | 「tells you what the enumeration missed」過頭；建議改成「warns you if unmapped mentions or dropped resolutions *suggest* the graph missed a caller」 | — | **採納**，已寫進 skill |

## 第二輪：Kimi **K3**（`kimi -m kimi-code/k3`，同一份反證包）

先前 Kimi 失敗不是 Kimi 的問題：那時候還沒登入，而且別名要寫全 `kimi-code/k3`（`-m k3` 找不到）。
登入＋更版後 `-p` 非互動模式**可以用 shell**（先探測過：它把 `Tally::add` 那兩列 receiver 漏洞原封貼回）。
`zglmcode --model glm-5.1` 那一份則死在 429 週用量上限（2026-09-05 重置），無產出。

K3 跑了約 5 分鐘、48KB，**中途斷掉、沒有最終 C1–C5 裁決**，但過程中抓到三個真問題，全部已修：

| K3 發現 | 性質 | 處置 |
|---|---|---|
| 單引號字串（`from './alpha'`）被標成 `mention` 而非 `quoted`，因為引號奇偶只算 `"` | 真 bug（TS/JS/Python 大量用 `'`，會把字串雜音頂到前面） | **已修**：`"` 與 `'` 都算；并把 `comment`／`import` 判定移到引號算術**之前**（散文裡有引號不該先被引號規則吃掉）。Rust 生命週期標在前頭可能被誤判 `quoted`——這是**降級**方向，安全 |
| 同一行兩個提及產生兩條重複列 | 真 bug（一行檔案看起來像兩個洞） | **已修**：依 `(file, line, cause)` 去重並顯示 `occurrences`，lean 輸出以 `x2` 標示 |
| seed 完全沒入索引時 `--coverage` 回 `error: nothing_indexed` 且 **exit 1** | 設計錯誤（「無法回答」變成「工具失敗」，把檢查當成不適用就跳過） | **已修**：改為正常輸出 `no_seed_resolved: true` + `enumeration_may_be_incomplete: true` + `why: [no_seed_resolved]`，exit 0；lean 印 `coverage\tno_seed_resolved\tnot a clean answer: nothing was looked at` |
| 再導出鏈（`export { x as y }` 而後 `y()`）只在 barrel 那行报警，真正的呼叫者不出現 | 邊界（不是 bug） | 寫進 README 限制 #11 與 skill：螢幕報的是 barrel，不是最終呼叫者 |
| `blind_files` 因「只有 export、零符號」的檔案而觸發 | 上一輪修法的正向確認 | 保留；並補上 `unparsed_example` 路徑（純計數不可執行） |

K3 也自己踩到我沒想到的實作缺陷：no-seed 那條路徑我傳了**空的已索引集合**給 `blind_files`，
結果把專案裡每個來源檔都報成 `unindexed=3`。是它把 `blind unparsed=1 unindexed=3` 貼出來才看見的，
已修並由 `a_symbol_that_is_not_indexed_reports_itself_instead_of_failing` 與 `blind_count` 注記守住。

## 帶走的規則

兩輪外部覆核（不同模型家族）各自抓到一個我自己沒發現的問題：一個是**語意**（盲檔發安全通行证），
一個是**實作**（空集合傳錯、報錯變 exit 1）。規則不是「多問幾個模型」，而是：**凡是「沒有信號」
可能被讀成「安全」的欄位，命名、預設值與錯誤路徑都要倒向不安確。**

## 我從這件事帶走的規則

外部 agent 一次就抓到兩個我自己沒發現的問題（一個在語意、一個在引導）。規則不是「多問一個模型」，
而是：**凡是「沒有信號」被讀成「安全」的欄位，必須在命名與預設值上都倒向不安確**（這次是 `false`
的語意，上兩次是 `null` 指標與貼上的報告）。


---

# 第二輪：Kimi **K3**（`kimi -m kimi-code/k3`，同一份反證包，HEAD `9743f65`）

前一輪 Kimi 不可用是我的關係：別名要寫全 `kimi-code/k3`，且當時尚未登入/更版。登入後 `-p` 模式
**有 shell 工具**（先探測過才送正式包），107KB、五項全部走完並交出最終裁決。

| 項 | K3 裁決 | 我要回應的重點 |
|---|---|---|
| C1 | **confirmed** | `Tally::add` 那兩列 `receiver` 如實抓到，原始聲稱成立 |
| C2 | **partly** | 值用途（`arr.map(f)`、`setTimeout(f,1)`、`let g=f;g()`）**有抓到**；再導出鏈只在 barrel 報警（該限制已寫在 skill）；`dyn`／泛型 trait 呼叫有抓到。**兩個靜默洞**見下 |
| C3 | **fix confirmed，但同一個錯誤往下留了一層** | 見「最嚴重的假性安全」 |
| C4 | **partly** | 判因大致正確；一個誤判級：trait 裡的**方法宣告** `fn add(&self, x: i32) -> i32;` 被標成 `call`（定義比對要求精確 `start_line`）。方向是過度提報，不危險但標錯。常見字 `get`（cct）：22 seeds × ~149 列，`quoted` 75 + `comment` 35 → **布林值完全失去鑑別力**，只有排序後的列還有用。無 panic；22 seeds ~1.0s |
| C5 | **partly** | skill 已強制刪除/改名要帶 `--coverage`，但**只講 `incomplete=true` 怎麼辦，沒講 `false` 該怎麼讀**（危險的那個方向）；限制清單漏了三條（>2MB 被跳過、`dist`/`target`/`node_modules` 下的來源檔、±2 行容差吞掉鄰近呼叫點） |

**最嚴重的假性安全（K3 的 worst false negative）**：`MAX_SCAN_BYTES`（2MB）把檔案跳過時**任何地方都不說**——
該檔有索引、解析正常、裡面有真實呼叫，螢幕回 `incomplete=false`。這是 `78fad50` 那個 bug 的孿生兄弟，
只是換了一層。它同時點出我的修法反了：`unparsed`（零 ast-grep 記錄，包含 barrel、只有型別的模組、
只有 `pub mod` 的 lib.rs）**本來就已经被文字掃描**、其漏洞已出現在列裡，我卻拿它去強制每個 seed 都
`true`；而**真的**看不見的 >2MB 跳過反倒什麼都不觸發。

## 由這裡定的下一步（旗標重新設計，尚未實作）

1. 新增獨立的 `scan_skipped`（>2MB、`read_to_string` 失敗）計數與路徑，並**由它**決定 `incomplete=true`。
2. `unparsed` **不再**單獨翻轉布林值：那些檔案已經被掃過、漏洞會以列的形式出現；改成頂層建議資訊。
   （本倉庫 2 個、cct 4 個 unparsed → 現在每個 seed 都掛 `blind_files`，等於沒有資訊。）
3. 採納 K3 的措辭：「**讀列，不要讀布林值**」，並在 skill 與 README 補上那三條漏掉的邊界。
4. 順手修 trait 宣告被標 `call`（定義判定放宽：行首是 `fn`/`pub fn`/`trait` 內容即 `definition`）。

---

# 反事實實驗：B 與 C 到底值不值得做（不改 pack，用現況 DB + 盤上文字計數）

腳本在 `/tmp/exp/exp.py`（分析用，不進 repo；若要長期重跑應做成 `cort-evals` 子命令）。
分類的依據是「該名字在專案 chunks 裡的候選數」：0=無符號、1=唯一、>1=多候選。

## C：抽出 receiver 呼叫（`x.m()`）

| 場域 | 盤上 receiver 呼叫 | 唯一可連（乾淨新增） | 多候選（會變 AMBIGUOUS） | 無此符號（std/噪音） |
|---|---|---|---|---|
| Rust（本倉庫，53 檔 773 chunks） | 5,522 | **160** | 32 次 → 一律連會 +~100 條 | 5,330（**96.5%**） |
| TS（cct，198 檔 2,702 chunks） | 11,408 | 2,408 | 1,847 次 → 一律連會 +~11,164 條 | 7,153 |

判讀（含一個必須講明的不對稱）：
- **Rust 側這 160 條是真新增**（pack 目前完全不抽 receiver，`raw_edges` 裡沒有）。佔現有邊數約 +13%。
- **TS 那行是上界、不是新增**：TS pack 本來就抽 member 呼叫，2,408 多數已存在；它告訴我們的是
  「若放寬一律連，`get` 這種名字會一次產生 22 候選 × 212 次的 AMBIGUOUS 海」——正是 README 限制 #7 的惡夢。
- 所以 C 的結論不是「做/不做」，而是**政策**：只在**專案內唯一**時連邊，其餘一律留給 `--coverage`
  報成 `receiver` 漏洞（現有排序已經把它放第一位）。這樣 Rust 淨得 160 條真邊，不帶 100 條模糊邊、
  也不會把 5,330 條 std 呼叫攪成一團 unresolved。

## B：為 Rust 補 import 邊（`use`／`mod`）

- `crate::`／`self::`／`super::` 的殘留未成邊呼叫：**0 種 / 0 次** ← `3fd9d9b` 那一修在真實倉庫上也清乾了。
- 剩下的限定式未成邊：67 種 / 241 次，高頻前三名是 `String::new`(19)、`PathBuf::from`(15)、`fs::write`(13)、
  `Vec::new`(12)、`fs::canonicalize`(11)——**全部是 std／依賴，本來就不該連**。
- 「第一段有出現在 use/mod 聲明」的只有 78 次（32%），而口語上多數是 `use std::fs` 這種別名，
  連上去仍然是錯的。抽取工作量参考：348 條 use/mod 聲明。

**B 的負面結果值得明說**：在唯一量測過的 Rust 場域上，A 之後 B 的邊際收益接近零、且會把 std 模組
別名變成假邊；代價是「mod 宣告算不算 chunk」這種會移動**所有場域**基線的語意變動。
**建議：不做 B**，把 force 留給 C 的「唯一才連」政策。若要翻案，條件是在一個**內部 mod 路徑呼叫占比明顯
更高**的 Rust 倉庫上重跑這個計數（本倉庫是 CLI 型、模塊間呼叫少，不是代表性樣本）。
