# ClaudeCat V2 — 方法研究報告（2026-09）

本文件是「重新定位 + 純 Rust CLI 改造」前的方法研究結論。目標：確認
**什麼方法能真正、可靠地幫助 Claude Code 獲得專案架構/輪廓資訊**，
並據此決定 V2 的技術路線。

## 1. Claude Code 怎麼吃 context（事實層面）

| 機制 | 角色 |
|---|---|
| `CLAUDE.md` | 啟動時自動載入的持久 context，是最重要的切入點 |
| `.claude/rules/*.md` | 專案規則，可與 CLAUDE.md 分工 |
| `.claude/skills/<name>/SKILL.md` | 漸進式揭露的技能（用到才讀） |
| MCP tools / CLI | 執行期查詢介面 |
| grep / rg / glob | Claude Code 自己會用的探索工具 |

**關鍵事實（來自 Anthropic 2026-05《How Claude Code works in large codebases》）**：
- Claude Code **刻意不做預先索引 / RAG**；它用 grep/rg/glob 即時探索檔案系統。
- Anthropic 內部測量：**agentic search 勝過 RAG**。
- 官方建議：CLAUDE.md 保持**精簡（約 200 行內）**，太長會稀釋重點、浪費 token。
- 大型 codebase 的正確用法：**給 Claude 一張「地圖」+ 規則，讓它自己做有目標的探索**，而不是餵它全部程式碼。

**推論（V2 核心命題）**：
> Claude Code 缺的不是「更多程式碼」，而是一張**精確、可信任、精簡的專案地圖**——
> 入口在哪、有哪些模組、模組間怎麼依賴、用了哪些框架、程式碼分佈如何。
> ClaudeCat V2 的職責就是**自動、準確地產出這張地圖**，並放進 CLAUDE.md / 隨叫隨查。

## 2. 現有工具與做法的對照

| 工具/做法 | 哲學 | 對照結論 |
|---|---|---|
| **repomix** | 把整個 repo 壓成單一 context 檔（tree-sitter `--compress` 減 token） | 走「全量打包」路線；V2 相反：**精簡地圖**，適合 CLAUDE.md 預算 |
| **code2prompt** | 檔案清單 + 內容打包 | 同上，一次性餵入 |
| **ast-grep** | Rust 原生結構化搜尋/重構 | 證實 Rust + tree-sitter 是成熟路線 |
| **Claude Code `/init`** | 產生基本 CLAUDE.md（what 專案是什麼） | V2 要補的是 **how/where**（結構層）而不是重做 /init |
| **RAG / embed-index** | 語意檢索 | Anthropic 明示不優先用；V2 不採用 |

## 3. 技術路線：Rust + tree-sitter + ignore

選定的 stack（全部有本機 crate cache，可離線建置）：

- **`ignore`**（BurntSushi）：gitignore-aware 遍歷，天然排除 `node_modules/.git/target/dist` 等。
- **`tree-sitter` + 語言 grammar**：真正的 AST 解析，用於**確定性地**抽取
  檔案內的符號（function/class/struct/interface/mod 等）——這些是**事實**，
  不是猜測。支援 JS/TS/Python/Rust/Go/C/C++ 等主流語言。
- **`clap`**：CLI 參數。
- **`serde_json` / `toml`**：manifest 解析（package.json / Cargo.toml / pyproject.toml / go.mod）。
- 不必引入 RAG、不必需要模型 API、本機即可跑。

## 4. 誠實原則（修復 V1 的「假信心」問題）

V1 的致命傷：對 `Unknown` 仍標「100% High Confidence」，且偵測**除錯腳本**
當作專案慣例（把本 repo 誤判成 Express API，還產生 `ALWAYS use req.auth` 的
錯誤 guardrail）。

**V2 誠實原則**：
1. **只報事實**：manifest 宣告的語言/框架/依賴、入口檔案、AST 抽出的符號、
   檔案統計——全部可驗證。
2. **不猜「執行期行為」**：不再對 auth/response/error pattern 做推論式偵測。
   框架只從依賴清單宣告（例如 `dependencies` 有 `passport` → 「使用 passport」是事實）。
3. **沒有 Unknown 卻標高信心**：偵測不到就明說「未偵測到/需人工確認」。
4. **精簡**：CLAUDE.md 自動區塊目標 ≤ 150 行，守官方 200 行預算。
5. **ATOMIC 更新**：只動 marker 區塊，其餘內容不動；失敗不損壞原檔案。

## 5. V2 交付介面（純 Rust CLI）

```
claudecat scan                # 掃描並輸出專案地圖（text/markdown/json）
claudecat update              # 更新 CLAUDE.md 的 auto 區塊（原子寫入）
claudecat skill               # （後續）安裝 Claude Code skill
```

地圖內容（全部是事實）：
- 專案類型/語言/框架/套件管理器（來自 manifest）
- 入口點（package.json main/bin、Cargo.toml bin、main.py、main.go…）
- 模組樹（pruned directory tree + 各目錄檔案數/LOC）
- 主要檔案符號清單（tree-sitter AST）
- 依賴摘要（宣告的依賴名稱）
- 「未偵測到」誠實標記

## 6. 結論

- **方法成立**：產生「精簡事實地圖給 Claude 導航」是對齊 Anthropic 官方
  codebase 策略（agentic search > RAG）的真實可行方法。
- **Rust 可行**：tree-sitter/ignore/clap 皆成熟且本機可離線建置。
- **V2 與 V1 的界線**：V1 的 pattern 猜測引擎（auth/response/error 推論）
  降級為「依賴清單事實 + 符號清單」；CLI 取代 MCP server 成為主介面；
  雲端同步（Turso）保留為後續選項，不阻塞核心價值。
