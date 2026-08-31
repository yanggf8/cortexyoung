# 需求面重驗：日常到底有多常需要呼叫點（2026-08-31）

> 狀態：**可重算**。工具是 `cort-evals demand`，原始產物在 `evals/runs/2026-08-31-demand/`，
> 每一筆命中的人工裁決在 `evals/runs/2026-08-31-demand/adjudication.json`。
> 本文取代 `2026-08-28-real-session-cost.md` §1.1「0 筆在問呼叫關係」的**證據基礎**：
> 那個結論的方向在本機存活樣本上重複成立了，但它引用的資料已經不存在，因此不該再被當成結論引用。

---

## 1. 為什麼非重驗不可

原結論寫的是：兩個最重 repo（finance-engineering 245 MB／764 檔、claw-skills 47 MB／57 檔）合計
1,565 筆真人輸入，真正在問程式碼呼叫關係的 **0 筆**，並自稱「結論，不是 WIP」。

今天的盤點：

- `~/.claude/projects` 只剩 **29 個專案目錄、112 MB**（比那两个目錄合計 292 MB 還小），
  `finance-engineering` 與 `clawd/skills` 的目錄**整個不存在**。
- 但 `~/.cache/claude-cli-nodejs/` 還留著 **63** 個專案痕跡，含 `-home-yanggf-b-finance-engineering`
  與 `-home-yanggf-clawd-skills` → 工作確實發生在這台機器上。
- `~/.claude/.last-cleanup` = `2026-08-31T01:52Z`。也就是说：**transcript 被 30 天留存策略清掉了，
  而不是我算錯。**

這條本身就是發現的一半：**一條會決定功能去留的結論，站在了會被自動刪除的資料上，而且沒有留下任何
可重算的中間產物。**這跟前三輪 `tool_return_tokens: null` 連續三轮沒人發現是同一類錯誤。

## 2. 這次量到什麼（本機、兩套 driver、300 個 transcript 檔）

| 項目 | 數量 |
|---|---|
| 可用「用戶自己的話」（已剝除貼上） | **1,214** |
| 被判定為純貼上、不屬用戶指令的訊息 | **877** |
| 螢幕命中 `ask`（在問關係） | 1（0.08%） |
| 螢幕命中 `task`（需先取得呼叫點才做得對） | 17（1.40%） |
| **人工裁決後真的是呼叫點需求** | **4 嚴格／7 含弱 = 0.33%–0.58%** |

三個要一起讀的點：

1. **877 / 2,090 = 42% 的「user 訊息」其實是 agent 報告被貼回來。** 這個比例本身就說明日常主軸
   不是「問一個問題」，而是「做完 → 貼回來查」。
2. 裁決出來的 4 筆嚴格需求裡，**3 筆長在寫入路線上**（persona-core 刪除前確認無人呼叫
   `SERVICE_ALLOWED_PATHS`；cct 審 `refactor: wire all handlers…` 那個 commit；tc 進行中的重構），
   **1 筆是信任問題**：tsheet 用戶直接不信 agent 講的「No callers use result["by_date"]」，
   要求 fact check。這正是重分析 §6.2 那筆「查證稅」在真實 log 裡的形狀。
3. **零筆**在問 ≥2 跳的傳遞關係。問得到的關係問題都是 1 跳的存在性檢查（有沒有人用 X）。

## 3. 我第一版怎麼量錯的（同一個坑的兩個方向）

第一版對 raw user 訊息做關鍵詞匹配，得到 **7.90%** 關係型提問——比裁決後高約 15 倍。原因很單純：
這些 log 的主導句型是 `review` + 貼上的 agent 報告，而報告裡全是 refactor／impact／callers。

所以這個錯誤有兩面，而兩面都已釘死：

- **過計**（我的第一版）：`own_words` 現在是 `demand.rs` 裡測試最密的函式（9 個 case），
  且每一筆命中都附 `needles` 與原文，`--show` 直接印出來要人Reject。
- **低估**（原文件可能有的）：needle 只有繁體會漏掉簡體打字的同一個問題，所以加了
  簡體→繁體正規化表。漏算需求對這個專案是**危險方向**，因為它正好支持「砍掉圖」。

## 4. 這對「基石」的意思

- 若 cort 的正當性是「用戶會問誰呼叫 X」，**這個理由死了**：存活樣本 1,214 筆只剩 1 筆問句，
  原樣本（不可重算但同向）是 0 筆。它撐不起一個功能。
- 但同批數據支持另一個不需要用戶開口的正當性：**agent 自己每天都在做呼叫點枚舉，而且常做錯、
  也常被要求查證。** 成本面早就量過（基線臂 16.8 turns／7,642 tool-return tokens、6/10 答錯；
  cort 臂 4.2 turns／992 tokens、10/10）；需求面是這次補的。
- 因此長期目標改寫，並已寫進 `AGENTS.md`：
  **把 agent 原本就會做、且常做錯的那次呼叫點枚舉，變便宜且變可查證。**
  這個說法是可驗的，它要求兩件事，而且剛好一件已有證據、一件還欠：
  (a) 一次取得呼叫點集合的成本遠低於 grep 迴圈 — 已量；
  (b) 結果能让 agent 自己快速查證 — **未做**，這是重分析 §6.2 那條缺 `call-site line`／`rel_type`
  的債，現在它有了一個由資料支持的理由，不再是「感覺比較可信」。

## 5. 邊界（別又把話說滿）

- **本機不是主力機**：這 300 檔合計 2,764 次工具呼叫，而原文件引用的是 14,731 次。樣本偏
  「這台機器上做的事」，且 cct 的 386 筆可用指令多數來自 Codex 而非 Claude Code。
- 只讀了 Claude Code 與 Codex 兩套 log；經委派跑的其他 runtime 不在樣本內。
- 螢幕是刻意 over-include 的，裁決是一個人做的；`adjudication.json` 的存在目的就是讓下一個人
  能逐筆推翻它。推翻請改那個檔，不要改敘事。
- `verify-impact` 只證「不是捏造」，`demand` 只證「多久發生一次」。**兩者都不證「值得」**——
  那是 §4 那個改寫後的目標要負責的地方。

## 6. 重現

```bash
cargo run --manifest-path evals/Cargo.toml --release -- demand \
  --exclude yanggf,.claude,b,transcripts,mosh-1.4.0 --show \
  --out evals/runs/2026-08-31-demand/report.json
```

預設讀 `$HOME/.claude/projects` 與 `$HOME/.codex/sessions`；這一台沒裝的那一棵會被跳過並寫進
`notes`，不會假裝量到 0。`--exclude` 的名單是「不是 repo 根目錄的 cwd」（家目錄、`.claude` 設定
目錄、上層目錄、third-party source、媒體資料），逐筆可見於 `report.json` 的 `by_project`。

還活著的風險：**留存機制照樣會刪掉今天的原始 log**（默认 30 天）。這次的對策是把派生的
`report.json` + `adjudication.json` 進 repo，所以就算盤上原始檔消失，命中清單與裁決仍可查；
不可重現的只剩「重新跑一次得到同樣的母數」這件事本身。
