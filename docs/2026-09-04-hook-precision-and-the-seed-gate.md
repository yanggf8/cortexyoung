# hook 問錯了問題,而最直覺的修法會殺掉最有價值的那個案例(2026-09-04)

接續 `2026-09-02-hook-wiring-correction.md`(hook 接了但不會跑)與
`2026-09-03-installer-dedup-and-attribution.md`(同一條規則兩份實作)。那兩份講的是 hook **會不會
跑**;這一份講的是它**跑起來之後對不對**——以及一個看起來乾淨的修法為什麼不能直接上。

起點是一件本來該是好消息的事:Rust 型別參照上線了(schema v5,`eef78709`),`cort impact --symbol
CallForm` 從 `seeds=0 dependents=0` 變成 `seeds=1 dependents=4`,146/146 邊被 `verify-impact` 確認,
幻影 0。然後量路由數字,發現它只動了四次。

## 1. 四次

同一份語料(58,826 條命令、236 次 fire),把改名前的 evals binary 建出來背靠背跑:

| | 舊分類器 | 新分類器 |
|---|---:|---:|
| `confirmed_(function\|seed)` | 59 | 63 |
| `rejected_not_a_(function\|seed)` | 149 | 145 |

計畫預估會移動 ~26 次,依據是「44 個 CamelCase 符號確定不是函式」。實際四次。

估計錯在**按命名形狀分類,而不是按那些樹裡實際的宣告**。抽驗仍被拒的 25 個 CamelCase——
`StellarHazard`、`UnitDestroyed`、`AnalyticsEngine`、`D1ExecResult`——**沒有一個在那些庫裡有宣告**。
它們是依賴與 std 的型別。後來又把 `class ` 加進分類器(TS/JS/Python 的 pack 本來就 chunk
`class_declaration`,儀器少算了),**一個都沒動**,從第二個方向確認了同一件事。

所以型別覆蓋不是瓶頸。問題在別處。

## 2. hook 問的是「這裡能不能回答任何事」,不是「這個能不能回答」

`cmd_hook_suggest` 的閘門是專案有沒有 `projects` 列(`rust/src/main.rs:870`,`index_state()`,
`no_index` outcome)。它**從不把符號交給索引**。

拿 236 次 fire 裡落在真有索引專案的 110 次,在各自的目錄下實跑 `cort impact --symbol X --depth 1`:

```
seeds > 0 :  35  (31.8%)
seeds = 0 :  75  (68.2%)
```

**三分之二的建議把 agent 送去一條 `# impact X depth=1 seeds=0 dependents=0`。** 這比沉默更糟,因為
它教會 agent 這個 hook 是雜訊。

解不出種子的 62 個相異符號,按類別:

* **常數與環境變數名**——`TIMEOUT_S`、`CORT_CACHE_DIR`、`NO_NEWS`、`COST_SERIES`、`CONNECT_BUDGET`、
  `EDGAR_INDEX_FIXTURE`。pack 不 chunk 這些。
* **依賴與 std 型別**——`Option`、`JSON`、`RequestInit`、`Fetch`、`Runtime`、`Secret`、`Var`、
  `CdpError`、`D1ExecResult`。專案內沒有宣告,`impact.rs:113` 的
  `symbol_name IN (...) AND project_id = ?1` 必然落空。
* **欄位、區域變數、領域詞**——`attribute`、`changes`、`canonicalize`、`big5`。

## 3. 觸發它們的命令根本不是 caller-set 問題

抽最大的被拒桶(95 個 snake_case)的實際命令:

```
grep -rn "tide" gwh-rules/src/*.rs
grep -rn "owner" gwh-rules/src/*.rs | grep -i "collapse\|reset\|= None\|= Some"
grep -ril "chart" --include="*.rs" .
grep -n 'episode_attributes' src/*.rs | grep -iE 'insert|update|delete'
```

依序是:領域概念探索、**欄位在哪被賦值**(資料流問題)、**哪些檔案提到**(檔案清單問題)、
資料寫入點。沒有一個在問「誰呼叫它」。

`judge` 看不到這些,因為 `Search` 只有四個欄位——`pattern`、`targets`、`wants_context`、
`recursive`(`rust/src/hook.rs:228-239`)。**沒有大小寫模式,沒有 files-only 模式。** 而
`search_from_shell` 只把「pattern 之前的旗標」跳過(`hook.rs:265-271`),所以 `-i`/`-l` 根本到不了
判決;`-ril` 裡的 `r` 反而讓它看起來像跨檔搜尋。

## 4. 因果鏈

Codex 的獨立診斷與這裡一致,並把它排成:

1. `symbol_of_pattern` 只認詞法形狀——排除 regex meta 與非 ASCII,可選剝掉尾隨 `(`
   (`hook.rs:103-120`)。它不知道宣告種類,也不知道搜尋意圖。
2. `judge` 接著檢查 context 旗標、來源路徑與副檔名、是否跨檔(`hook.rs:329-373`)。它不問這個名字
   是不是 chunk,也不問這個命令是要 caller 還是要出現位置。
3. `cmd_hook_suggest` 只問專案列(`main.rs:870`),然後注入。
4. `impact` 用精確 `symbol_name` 相等解析種子(`impact.rs:113`),落空就必然 `seed_count=0`
   (`rust/tests/impact.rs:111` 釘住)。

**正確的謂詞是三者合取**:形狀像 caller-set **且** 沒有反向訊號 **且** 至少有一個種子。缺種子是
「空答案」這個症狀的直接病因;它**不是** caller 意圖的充分判準——一個概念搜尋可能剛好撞到真符號而
通過種子閘門。

## 5. 最直覺的修法會殺掉最有價值的案例

候選解法:在「索引存在」閘門與注入之間,用 `impact` 的同一條解析路徑查一次 `chunks`,零則安靜,記
`no_seed` outcome。查詢由 `idx_chunks_symbol ON chunks(project_id, symbol_name)`
(`rust/src/schema.sql:33`)完整覆蓋。

我原本以為「問索引」逃得過 `evals/src/hook.rs` 裡 `index_check_reading` 那段既有反論——它說 gating
「在兩個方向上都錯」,因為**正在被完整刪除的符號在樹裡已經沒有宣告,而那正是 agent 搜尋的原因**。

**逃不過。** `reindex_one_file` 做的是

```rust
DELETE FROM chunks WHERE project_id = ?1 AND file_path = ?2   // rust/src/incremental.rs:243
```

再重插;`PostToolUse` 的 `hook-refresh` 每次編輯都跑。所以符號一被刪、refresh 一跑,索引裡的 chunk
就沒了。而這個案例有測試釘著,並且測試自己寫明它就是目標句點名的任務:

```rust
// rust/tests/hook.rs:107-116
grep -rn "ensureSeedUserPasswords" backend/src frontend/src 2>/dev/null; echo "---(empty = fully removed)---"
    "verifying a deletion is complete is the exact task the goal sentence names"
```

`index_check_reading` 那句 "fixes the second case and not the first" 是精確的。我讀成了「大致逃得
過」,那是誤讀。

## 6. 判決單一性的兩難

CLAUDE.md 要求**解析可以多份,判決只能一份**——`hook-probe` 重放 `judge` 本身,絕不重新實作,因為
手刻近似值曾在兩個介面上高估 48% 和 4 倍(`2026-09-02-hook-wiring-correction.md` §15、§16)。

於是:

* 種子閘門放進 `cmd_hook_suggest` → `tests/hook.rs:107-116` 保持綠,但**測試宣稱的行為與出貨的行為
  分岔**,而 `hook-probe` 量不到閘門。這正是那條規則要防的事。
* 種子閘門放進 `hook.rs` 的 `judge` → 判決單一、`hook-probe` 看得到,但那個測試會紅,而它釘的是
  目標句親自點名的任務。

兩條路都要付代價,而代價不在同一個維度上。這不是實作細節,是設計決策。

## 7. `-i` 訊號:實測完全冗餘

`-i` 看起來是乾淨訊號:15 次命中被拒、**0 次誤傷**。算了重疊之後:

| `-i` 的 15 次 | |
|---|---|
| 落在有索引專案 | 8 —— **全部** `seeds=0`,種子閘門已涵蓋 |
| 種子閘門放行而 `-i` 會擋(淨新增) | **0** |
| 落在無索引專案 | 7 —— 種子閘門無效,`-i` 仍有效 |

**在有索引的地方它完全冗餘。** 唯一價值是那 53%(126/236)沒有索引的專案裡的 7 次。

統計上它也撐不起「無損」:0/15 的 95% 單側二項上界是 **18.1%**;要把上界壓到 5% 以下需要 59 個獨立
樣本,1% 以下需要 299 個。而實作它要同時擴充 `Search` 與兩個 parser(shell 的 `hook.rs:243-286`,以及 Kimi 結構化欄位那條),
因為現在兩邊都不保留這個旗標。

不能一律拒絕管線或多重 grep:`grep -rn 'deliver_news' … | grep -v '…'` 被釘為必要正例
(`rust/tests/hook.rs:100-107`),用第二個 grep 排除宣告本身正是 caller 搜尋的形狀。

## 8. 量測方法學:`hook-probe` 沒有時間窗,而它咬了我兩次

`HOOK_PROBE_FLAGS` 只有 `--claude-dir --codex-dir --kimi-dir --examples`
(`evals/src/main.rs:81`);`adopt-mine` 有 `--since`(`evals/src/main.rs:83`、`:853`)。所以:

1. 我先前把「236 次 fire」與「2,249 次搜尋 / 11 次注入」講成同一份語料。**不是。** 前者掃整棵樹,
   後者套了 `--since 2026-06-06`。兩個母體不能相減。
2. README 曾寫「over the same 236 hook fires … 60 → 63」。那兩次量測相隔約 1,500 條命令——這台機器
   一直在產生 transcript。`fired` 剛好都停在 236 是巧合,不是凍結。重建舊 binary 背靠背重跑後真值
   是 **59 → 63**。同一段裡的 `raw edges 12,463` 也是取自 Task 4 之前的資料庫快照,隔離重測是
   **12,523 → 15,850**。兩處已於 `87b2a98d` 更正。

**任何 before/after 都必須兩個 binary 背靠背跑**,否則語料會在中間漂掉。這個坑是工具自己缺一個旗標
造成的,所以它會反覆咬人。

## 9. 一條被實證排除的路

在寫下兩條出路之前,有第三條看起來能同時解決兩難:**也許 `seeds=0` 的建議對刪除驗證並不是空答案。**
`cort impact --symbol X --coverage` 對一個不存在的符號,直覺上該回報 mention 列與 blind files——
而「這個名字還在哪些行出現」正是「刪除完成了嗎」要問的東西。若成立,閘門的條件就該是
「`seeds=0` **且沒有 mention**」而非「`seeds=0`」,測試不必紅,刪除驗證不必犧牲。

**不成立。** 實測一個有 3 處提及、沒有種子的符號:

```
$ cort impact --symbol HOOK_TARGETS --depth 1 --coverage -f lean
# impact HOOK_TARGETS depth=1 seeds=0 dependents=0 stale=false
coverage	no_seed_resolved	not a clean answer: nothing was looked at
blind	unparsed=2	unindexed=0	unread=-
```

一條 mention 列都沒有。病因在 `rust/src/coverage.rs:204`:種子為空時 `attach` 直接 `return Ok(())`,
做 mention 掃描的 `coverage_for`(`:206`)根本不會被呼叫。`render.rs` 那句
"not a clean answer: nothing was looked at" 說的是字面意思。

所以 `seeds=0` 是一個**誠實的成功非答案**,不是刪除驗證。第三條路關閉。

## 10. 四條出路,選定丁

四條都寫在這裡,因為選定的那條(丁)的理由,是它同時避開了前三條各自的代價。

**甲:接受損失。** 種子閘門放進 `judge`(判決必須單一,見 §6),`tests/hook.rs:108-116` **移到
`rust/tests/cli.rs` 並反轉**——建一個不含該符號的已索引專案,跑同一條刪除驗證 grep,斷言 payload 為
空加上 `no_seed` outcome。從「必須開火」變成「必須沉默」。SKILL 與 README 要明說刪除驗證不再由 hook
提示。換到的是 110 次 fire 裡 75 次轉為安靜、0 次空答案、35 次保留。

**乙:先給 `impact` 墓碑能力。** 讓被刪除的符號留下可回答的殘跡(懸空目標解析),閘門才不會殺掉刪除
驗證。這會動到 `incremental.rs:243` 的刪除語意,是另一個計畫。

**丙(降級,而非沉默)。** `Option<HookHit>` 換成 `HookDecision::{Impact, NoSeed}`,`NoSeed` 不注入
`impact` 建議而注入警語:「沒有索引種子,`impact` 無法列舉呼叫者——保留字面 grep,不要把空結果當成
編譯器級證明」。沒有別的 cort 指令能更好地做刪除檢查,所以這是**警語不是替代指令**;它是否比沉默好
要量了才知道。

**丁:把謂詞放寬到「有種子 **或** 有 raw edge 指名它」。這是選定的方向。**

甲的兩難建立在一個未經檢查的前提上:被刪除的符號在索引裡什麼都不剩。**不對。** `raw_edges` 因為
跨檔重建的理由獨立於 `chunks` 存活(schema 的 F-01 註解),所以倖存的呼叫端還指名著被刪掉的目標。
實測——建一個專案、刪掉定義、跑 `index --incremental`(就是 `PostToolUse` 做的事):

```
chunks 有 ensure_seed_user_passwords : 0     <- 種子沒了,甲案會在此沉默
raw_edges 指名它                     : 1     <- ('src/user.rs', 'boot', 'ensure_seed_user_passwords', 'calls')
```

這給了一個索引已經知道、目前沒人問的判準。對 110 次落在有索引專案的 fire 量(索引狀態:`dbd5bf13`
當下,一次跑完以免漂移):

| 桶 | 次數 | 是什麼 |
|---|---:|---|
| A 有種子 | 36 (32.7%) | `impact` 答得出來 |
| B 無種子但有 raw edge | 21 (19.1%) | 外部型別(`Option`、`JSON`)、已刪除的符號 |
| C 兩者皆無 | 53 (48.2%) | 概念(`tide`)、欄位(`owner`)、檔案清單(`chart`) |

| | 開火 | 空答案 | 刪除驗證 | 釘住的測試 | 需要新能力 |
|---|---:|---:|---|---|---|
| 現況 | 110 | **74 (67%)** | 開火 | 綠 | — |
| 甲 嚴格種子閘門 | 36 | 0 | **被扼殺** | **要反轉** | — |
| **丁 放寬謂詞** | 57 | **21 (19%)** | **保留** | **保持綠** | **不需要** |

C 桶正是問題的來源——`tide`、`owner`、`chart` 在 `raw_edges` 裡一列都沒有,因為它們從來不是呼叫或
參照的目標。**用一個既有的、免費的訊號砍掉 72% 的空答案。**

**丁案保住的是那個測試的字面,不是它的價值。** hook 會繼續對刪除驗證開火,而被建議的指令依然回
`seeds=0 / nothing was looked at`(§9)。那不是退步——那是刪除案例的現狀。丁案不宣稱修好刪除驗證;
它宣稱在不弄壞刪除驗證的前提下,砍掉大部分的空答案。真正修好它仍然是乙(墓碑)。

丁也繞開了 §6 的兩難:謂詞在刪除案例上回答「開火」,所以放 `judge` 或放 `cmd_hook_suggest` 都不會
讓 `tests/hook.rs:108-116` 變紅。判決仍應單一(進 `judge`),但那是為了 `hook-probe` 能重放,不再是
一個要付代價的選擇。

### 角度三(注入答案而非指令)已排除

hook 的 timeout 是 **5 秒**(`rust/src/settings.rs:80`)。`impact_command` 會對每個 seed 檔重跑
`extract_file`/ast-grep(`rust/src/impact.rs:171-178`)並跑 `compute_stale`(`:205`),而
`main.rs:973-975` 的註解自己就寫著 "the full `compute_stale` walk is too expensive for a hook with a
5s budget"——作者早就為了同一個預算拒絕過它,所以 `index_state` 只比對 git head 且只給 400ms。
相對地,一次走 `idx_chunks_symbol` 的 `chunks.symbol_name` SELECT 是便宜的,而那正是甲與丁需要的。

### 新約束:種子閘門一旦上線,歷史重放就不再誠實

這是原本沒寫到的,而它比表面上重。`judge` 現在只吃一個 `Search`;要讓它問種子,得把證據傳進去
(`judge(search, seed_state)`——資料庫不該進 `judge`)。但**歷史語料拿不回當時的索引狀態**:
`hook-probe` 只能從 transcript 還原命令與 cwd,然後看**今天**的檔案系統
(`evals/src/hook.rs:353-412`)。那 236 次 fire 發生時,各專案的索引是什麼狀態,不可回溯。

所以舊記錄只能給 `SeedState::Unknown` 或整批排除在種子閘門的指標之外。而 `hook-probe` 是這個產品
**唯一**的校準工具。這一條與 §8 的「沒有時間窗」疊起來:重放本來就讀今天的磁碟,現在還要多一個
不可回溯的維度。

任何採用甲或丙的計畫,都必須先回答「改完之後要拿什麼量它」。

## 11. 這一輪關於審查的結論

三輪外部審查(Codex ×3、Kimi ×1)加上逐條 corroborate,規律很一致:

* **Codex 在讀 code 找矛盾上很強**,四個 stop-ship 級發現全是它的(限定路徑丟修飾、`SELECT *` 欄位
  錯位、第四個 `'calls'` 過濾器、刪除案例)。
* **它會誇大結論的涵蓋範圍**。它說評測分類器「invalidates the plan's claim that this feature
  addresses the measured population」——不成立,出貨的 `judge` 沒有 function 閘門,production 今天
  就對 `FeedSpec` 開火。它打到的是**可量測性**,不是**有沒有打中**。另一次它把 `impact.rs:37` 說成
  走圖,實際上遞迴查詢(`graph.rs:781`)根本沒有 `rel_type` 條件。
* **所以 corroborate 不能省**,而且要對源碼逐行,不是重讀它的論證。
* **但方向是雙向的。** §9 那條路是我提的,我還特地在提問時寫明「這是我最想被攻擊的一點」而不先講
  結論。Codex 直球否定,附上 `coverage.rs:150-204` 的 early return,實測一跑就證實了。**先假設自己
  對、只用審查者找對方的錯,會漏掉這種。** 把自己的假設也送進去被打,成本一樣低。
