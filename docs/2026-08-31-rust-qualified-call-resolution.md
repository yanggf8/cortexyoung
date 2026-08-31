# Rust 模組限定呼叫永不解析：根因與解方（2026-08-31）

> 這是外部覆核（K3 與 agy 兩輪）在验 `impact --coverage` 時挖出的**產品缺陷**，不是螢幕的缺陷。
> 螢幕只是第一次把它照出來：`extracted_but_unresolved` 明確列出「抽到了、但解析階段掉了」。

## 最小重現（已在 `/tmp/rq` 验過）

```rust
// src/def.rs
pub fn my_func() -> u32 { 7 }
// src/lib.rs
pub mod def;
pub fn caller()  -> u32 { crate::def::my_func() }   // ← 圖看不見
pub fn caller2() -> u32 { my_func() }               // ← 同檔裸呼叫，看得見
// src/other.rs
use crate::def::my_func;
pub fn caller3() -> u32 { my_func() }               // ← use 之後裸呼叫，看得見
```

修正前 `dependents = [caller2, caller3]`；`caller` 消失，且 `stale=false`（它不知道自己漏）。

## 真根因（一行）

`rust/src/graph.rs` 的 `resolve_targets()` 開頭就是：

```sql
WHERE project_id = ?1 AND symbol_name = ?2
```

`symbol_name` 存的是 `my_func`（或 Rust 方法的 `Type::method`），而 raw target 存的是**含限定詞的原字串**
`crate::def::my_func`。全字串精確比對永遠比不中 → `all.is_empty()` → 回傳空 →
`relationship_rows_for_symbol_map()` 的 `if targets.is_empty() { continue; }` 把這條邊**靜默丟掉**。

三個附帶事實，解釋為什麼只有這一路徑壞：
- `Type::method()` 會中，是因為 Rust 的方法 chunk 名稱本身就是 `Type::method`（精確比對成立）。
- `use …; f()` 會中，是因為 `use` 之後呼叫點寫的是裸 `f()`。
- TS/JS/Python 大部分會中，是因為它們的 import 讓呼叫點通常就是裸識別符。
- 換言之：這是「**Rust 的寫法**」撞上「**為裸名設計解析**」，不是 ast-grep 抽錯。

## 已做（A：窄修）

只对**能證明是專案內部**的限定前綴做最後一段退回：`crate::`、`self::`、`super::`、`::`。
理由與邊界都寫在 `internal_rust_path_target()` 的註解裡，重點是：

- `Vec::new`、`formatter.formatToParts`、`serde::deserialize` 這類外部呼叫**不退回**，維持
  `unresolved` 可見狀態。若把它們也退回裸名，會憑空連出一堆「剛好同名的」邊，而且讓原本坦白的
  `unresolved` 列消失——用透明換召回，這筆交易不划算。
- 退回後仍走原有收窄順序（同檔 → import 前綴 → 全部候選/AMBIGUOUS），不新增第二套語意。
- cct 是 TypeScript 場域，此修不影響它的任何基線（已複核 `getCurrentTimeET --depth 3` 仍是 8 個依賴者）。

測試：`an_internal_rust_path_call_resolves_to_the_bare_symbol`、
`a_dependency_call_stays_unresolved_and_says_so_instead_of_becoming_an_edge`、
`only_the_three_internal_rust_prefixes_are_rescued`。

## 尚未做（B：真正完整的解法）

`mod::f()` 這種**靠 `use` 帶進來**的路徑仍然不解析，因為 Rust 側根本沒有 import 圖：

1. pack 沒有 Rust 的 `edge:imports` 規則（README 限制 #8 記著），`build_import_map()` 對 Rust 永遠是空的。
2. `pub mod def;` 不是 chunk——`rust/src/lib.rs` 這種「只有 `pub mod` 行」的檔案在限制 #9 裡甚至被歸為
   `unparsed`。所以 `imported_path_prefixes()` 在 Rust 沒有任何可比對的東西。

要做的是：為 `use_declaration` 產 `imports` 邊、並讓 module 宣告成為可參照的節點（`mod` 是否算 chunk 是
一個設計問題，會動到 `chunks` 語意與所有場域的計數）。這是**會移動基線**的變更，得先定義清楚再動。

## 尚未做（C：另一個洞，且不同性質）

`x.method()` receiver 呼叫是**抽取器**層級的缺失：`raw_edges` 裡根本沒有這條記錄，所以任何解析层的
改動都救不了它——這也是為什麼「每條邊補 call-site 行號」對真正的漏報無能為力。
把它抽出來的代價是名稱碰撞爆炸（`.get()`、`.add()`、`.map()` 會去比對所有同名 chunk，AMBIGUOUS 大量增生），
需要一個策略（只在名稱於專案內唯一時連邊，否則留給 `--coverage` 報為漏報）。這要用數據决定，不能凭感觉。
