# ClaudeCat V2 目標：帶給 Claude Code 一張「可信任的專案地圖」

## 重新定位（V2）

### V1 的教訓（為什麼要重定位）
V1 把目標設定為「偵測實作 pattern（auth/API response/error handling）並給信心分數」，
結果產生三個致命問題：
1. **假信心**：對 `Unknown` 仍標「100% High Confidence」，誤導 Claude Code。
2. **誤判**：把除錯腳本當專案慣例（本 repo 被誤判成 Express API，並產生
   `ALWAYS use req.auth` 的錯誤 guardrail）。
3. **介面錯位**：MCP server 持續「主動維護」CLAUDE.md，產出更多雜訊而非精確資訊。

### V2 核心命題
> Claude Code 缺的不是「更多程式碼」，而是一張**精確、可信任、精簡的專案地圖**。
> ClaudeCat 的職責是**自動產出這張地圖**——入口、模組、依賴、符號、專案事實——
> 放進 CLAUDE.md 供啟動時使用，並提供 CLI 隨叫隨查。

地圖只包含**可驗證的事實**（manifest 宣告、AST 符號、檔案統計、依賴清單）；
**不做執行期行為的推論式猜測**，**不標不存在的信心**。

---

## 三大目標

### Goal 1：可信的事實地圖
自動產生專案輪廓，內容全部來自可驗證來源：
- **專案類型**：語言、框架、套件管理器（來自 `package.json` / `Cargo.toml` /
  `pyproject.toml` / `go.mod` 等 manifest）
- **入口點**：`main`/`bin`/`scripts.start`、`main.rs`、`main.py`、`main.go`…
- **模組樹**：pruned directory tree + 各目錄檔案數/LOC（gitignore-aware）
- **符號清單**：tree-sitter AST 抽出主要檔案的 function/class/struct/interface/mod
- **依賴摘要**：manifest 宣告的依賴名稱

**成功標準**：掃描結果中 100% 的陳述可回溯到 manifest 或 AST 證據；
沒有「未偵測到」卻標高信心的欄位。

### Goal 2：精簡、守預算的 CLAUDE.md 自動維護
- 自動區塊目標 **≤ 150 行**（官方建議 CLAUDE.md 約 200 行內）
- **原子寫入**：只更新 `<!-- claudecat:auto:begin/end -->` 區塊，其餘內容不動
- 內容有變才寫；失敗不損壞原檔案
- 偵測不到的項目明確標「未偵測到（需人工確認）」

**成功標準**：`claudecat update` 後 CLAUDE.md 保持可讀、精簡、零假信心。

### Goal 3：純 Rust CLI（快速、可離線、單一二進位）
- `claudecat scan`：輸出地圖（text / markdown / json）
- `claudecat update`：更新 CLAUDE.md auto 區塊
- `claudecat skill`（後續）：安裝 Claude Code skill
- 單一靜態二進位；不需 Node、不需模型 API、不需網路

**成功標準**：`cargo build --release` 產出單一二進位；在大型 repo 掃描
sub-second 到數秒完成。

---

## 明確不做的事（Anti-Goals）

1. ❌ **不推論執行期行為**：不對 auth 流程、API 回應格式、錯誤處理做「猜測式」偵測。
   （「依賴清單有 passport」是事實；「401 用 {error}」是猜測——V1 死在這裡。）
2. ❌ **不打包整個 repo**：不學 repomix/code2prompt 全量塞 context；
   只給「地圖」，讓 Claude 自己做有目標的探索（對齊 Anthropic agentic search 策略）。
3. ❌ **不標假信心**：沒有 Unknown→100% High Confidence。
4. ❌ **不做 RAG / embed 索引**。
5. ❌ **不做 transit 壓縮**：不改寫 agent 與 LLM 之間流動的任何位元組。
   地圖的每一行（`file:symbol:line`）都要能逐字回查；任何中途改寫都摧毀這個性質。
   壓縮是傳輸層（headroom 之類）的事，與本工具互補：它降低「每次讀的成本」，
   地圖減少「盲目讀的次數」。

## 環境對照（2026-09-18）

[Headroom](https://github.com/headroomlabs-ai/headroom)（transit 壓縮層，72.9k stars）wrap agent 時
**預設捆綁 Serena**（語義導航 MCP）並帶 `--code-graph` 旗標——「導航輔助」正在被
壓縮層當成標配出貨。這驗證了導航需求是真的，也代表分發渠道上會有捆綁競品。
claudecat 的位子不被它吃掉的三個理由：**只做可驗證事實**（壓縮是「希望沒壞」，
地圖是「每行可查」）、**離線零遙測單一 Rust binary**（headroom beacon 預設開）、
**接 cort 深挖**（`navigate --cort` → `cort impact` 的 caller-set 可核對主線，無人重疊）。

## 成功度量

| 項目 | 目標 |
|---|---|
| 地圖陳述可驗證性 | 100%（manifest/AST 可回溯） |
| CLAUDE.md 自動區塊 | ≤ 150 行、原子更新、零假信心 |
| 掃描速度 | 中型 repo（<5k files）< 2s |
| 二進位 | 單一 Rust binary，`cargo build --release` 即得 |

## 技術棧

`tree-sitter`（AST 符號抽取）+ `ignore`（gitignore-aware 遍歷）+
`clap`（CLI）+ `serde_json`/`toml`（manifest 解析）。
