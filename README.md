# ClaudeCat V2 — Project Navigation Map（給 Claude Code 的導航地圖）

**引導型**：ClaudeCat 在啟動時就給 Claude Code 一張**可信任的專案地圖**
（入口、模組樹、主要符號、依賴），讓它導航時**不用盲目摸索**。
與 **cortexyoung/cort**（協助型，插在 rg 上的搜尋)互補：
cort 回答「查詢時的問題」，ClaudeCat 回答「啟動時的導航」。

## 定位（V2，2026-09 重定位）

- ❌ **不做**：「猜測式」auth/response/error pattern 偵測 + 假信心
- ✅ **只做**：從 manifest + AST 產出**可驗證的事實地圖**
- 對齊 Anthropic 官方大型 codebase 策略（agentic search > RAG）：
  給 Claude 精簡地圖（≤150 行），讓它自己做有目標的探索

## 安裝 / 建置

```bash
cargo build --release            # 產生 target/release/claudecat（單一二進位）
```

## 使用

```bash
claudecat scan                   # 輸出導航地圖（markdown，auto 依規模選樣式）
claudecat scan --format json     # JSON 輸出
claudecat scan --format text     # 純文字輸出
claudecat scan --map mini        # 強制迷你地圖（小專案省 token）
claudecat scan --map full        # 強制完整導航地圖
claudecat update                 # 更新 CLAUDE.md 的 claudecat 自動區塊（原子寫入）
claudecat update --dry-run       # 只看會不會變，不寫入
claudecat explore                # 量化探索成本（地圖 token vs 全讀 token）
claudecat explore --json         # 機器可讀指標輸出
claudecat track SESSION-EVIDENCE.md  # 把指標寫入長期指標表（原子、同日不重複）
claudecat navigate "<要找什麼>"      # 從一句話給出目的地符號/檔案 + cort 路線
claudecat navigate --cort "<要找什麼>"  # 優先吃 cort 全量索引（含反向依賴）
claudecat cort-status --root <proj>   # cort 索引新鮮度 / chunks / relationships
claudecat track METRICS.md --root /repo/a --root /repo/b   # 多 repo 一次寫入
claudecat track METRICS.md --roots-file repos.txt          # 從檔案讀 repo 清單
claudecat scan --root /path/to/project
```

**迷你地圖（auto）**：專案 <300 LOC 時，讀全部比導航地圖便宜，`auto` 會自動改用
15 行內的迷你地圖（About/Entry/Run/Build/Structure/Deps/Guardrails），避免負效益。
`--map mini|full` 可手動覆蓋。

### 對 Claude Code 的使用建議

1. 在專案根目錄執行 `claudecat update`（或讓 skill/CI 定期執行）——Project Map 寫到
   **各機器的資料目錄** `${XDG_DATA_HOME:-$HOME/.local/share}/claudecat/projects/<project-id>/map.md`
   （`CLAUDECAT_DATA_DIR` 可覆蓋），輸出印出 `map ->` 確切路徑
2. CLAUDE.md（或其 symlink 本體 AGENTS.md）**只放手寫規則**：v2.1 首次執行會自動移除
   舊版 `claudecat:auto` 區塊，並播下 guardrails 與「地圖位置」兩個種子（種一次、永不改寫）
3. 要看地圖就讀 `map ->` 指到的檔；Claude Code 啟動時載入的 CLAUDE.md 保持精簡，
   `git status` 不再被機器生成的內容弄髒

> 注意：把 `CLAUDECAT_DATA_DIR` 指進專案目錄內，會讓掃描把地圖也算進專案——預設位置
> 在 HOME 下，不受影響。

## 導航能力：全圖之外，還要有「路」

> 與 cortexyoung/cort 的結合使用細節見 [CORT-INTEGRATION.md](CORT-INTEGRATION.md)。

**問題**（2026-09-05）：作為導航工具，光有「全圖」不夠——全圖是靜態的「東西在哪裡」，
真正的導航是**從一句話高速低本到達目的地**。缺少它時，Claude 仍要自己
Glob/Read 繞路（真實 session 59–94% 工具呼叫是搜尋類）。

**解法：`claudecat navigate <query>`**——輸入意圖，輸出：
1. **命中符號表**（檔案:行號 + 種類，精確命中優先）
2. **命中檔案**
3. **低成本路線**：先讀哪個檔案:行號 → 建議 `cort` 精確查詢
   （`cort context <symbol> --content full -f lean`、`cort impact --symbol <symbol>`）
   → 沒中時擴大搜尋指令

```bash
claudecat navigate "guardrail"
# → src/guardrails.rs:4 mod guardrails
# → 路線: cort context guardrails --content full -f lean …
claudecat navigate "auth" --json   # 機器可讀
```

**分工**：`scan/update` = 全圖（場景）；`navigate` = 路線（導航）；
`cortexyoung/cort` = 精準定位（到達後深挖）。三者串成「快速低本到達目的地」。

### 結合 cortexyoung：直接吃 cort 的索引（2026-09-05）

claudecat **不重造索引**——直接唯讀 cort 的 SQLite
（`~/.cache/cortex-ng/<sha256>.db`，schema v5 相容）：`chunks`（全 project 符號）、
`relationships`（calls/references/imports 邊）、`projects`（git_head / 索引時間）、
`_cortex_meta`（schema 版本 / `graph_pending`）。

```bash
claudecat cort-status --root <project>    # 索引新鮮度 / chunks / relationships
claudecat navigate --cort "extractSymbolDefinitions"   # 優先查 cort 全量索引（tree-sitter 只掃 top-N）
claudecat cort-audit --root <project>    # 驗證整合成效：健康 / 覆蓋缺口 / FTS / 用量 / harness 切面；--track 寫長期指標
```

- `cort-status fresh`：git HEAD 相符 + 索引 ≤7 天 + `graph_pending != 1`；STALE 時提示 `cort index`。
  `graph_pending=1`（schema 遷移後未重建、或增量中斷）代表 relationships 是舊邊——
  **反向依賴先別信**，且此時 HEAD/時戳可能仍是新的，所以另行提示、不與 STALE 混講；
  讀不到 `_cortex_meta`（舊版 DB）→ 顯示 `?`，不視為健康、也不據此判 STALE。
- `navigate --cort`：命中 cort `chunks`（**比 tree-sitter top-N 更完整**——實測 legacy 檔的
  `extractSymbolDefinitions`：tree-sitter 0 命中、cort 全量索引 1 命中）。
  symbol_name 未命中時自動 fallback 到 **`chunks_fts` 全文**（content/file 命中；
  與 `cort context` 的 `resolution=fts` 同源；注意 `cort recall` 是搜 reading_notes，非 code 索引）。
  路線自動含 **content 摘要**（省一次 read）/ `cort context` / `cort impact` / **反向依賴清單**。
- 唯讀雙保險：一般 `SQLITE_OPEN_READ_ONLY`；失敗（唯讀 FS / 缺 sidecar / 寫入端持鎖 BUSY）
  自動退回 `immutable=1` 直接讀主檔。claudecat 永不寫 cort 的 DB；
  DB 不存在或未命中自動回退 tree-sitter。
- `cort-audit`（收集數據驗證 → 據此改善）：唯讀彙整 ①索引健康（fresh/HEAD/age）
  ②覆蓋缺口（file_state 有、chunks 無的檔案＝completeness 缺口）③FTS 同步
  ④用量（cort usage.db：命令分佈、hook-suggest/refresh 結果、errors、index_stale、saved_bytes）
  ⑤**harness 切面**（`harness` 只在 hook payload 上，所以這張表講的是「router 面對誰」，
  不是「誰用了 cort」；無 `harness` 欄的舊列另計，`harness_declared` 不符的列數也照實列出），
  並依規則給「解讀 & 行動」；`--track <file>` 把每天一列寫進長期指標表（同日更新），
  例如 `CORT-AUDIT.md`，讓「有沒有達成」變成可看的趨勢。

## 導航地圖內容（全部是事實）

| 區塊 | 來源 |
|---|---|
| 專案類型 / 語言 / 框架 / 套件管理器 | manifest（package.json / Cargo.toml / pyproject.toml / go.mod） |
| Entry points / Run / Build | manifest 宣告 |
| 目錄結構（檔案數、LOC） | gitignore-aware walk（`ignore` crate） |
| 主要檔案 & 符號（fn/class/struct/…） | tree-sitter AST |
| Dependencies（宣告） | manifest |

- 支援語言：JS/TS、Python、Rust、Go、C/C++（tree-sitter grammar）
- 自動排除：node_modules、target、dist、.git、legacy 等

## 技術決策 Guardrails（開發者維護，永不覆寫）

`claudecat update` 首次執行會在 CLAUDE.md 附加：

```markdown
<!-- claudecat:guardrails:begin -->
- 2D tilemap + Macroquad（禁 Python/3D）  ← 例：一行一條技術決策
<!-- claudecat:guardrails:end -->
```

此區塊**只存在時不動、不存在才建立**，開發者自由編輯；`scan`/`explore` 會讀出
顯示。也可用根目錄 `claudecat-guardrails.md` 優先提供。
真實案例佐證：GalaxyWarHero session 中 Claude 因缺「技術決策」把 2D 專案當 3D
分析、裝錯工具——見 [SESSION-EVIDENCE.md](SESSION-EVIDENCE.md)。

**2026-09-05 獨立評審（Grok）**：抓到 26 條問題，其中 6 個 P0（目錄 LOC 雙計、
大 repo 截斷、dual-manifest 標錯語言、explore 假指標、track 誤刪兄弟 repo、
update 汙染父專案）全部實測屬實並已修復＋回歸測試；每條 corroboration 與
剩餘待辦見 [GROK-REVIEW-CORROBORATION.md](GROK-REVIEW-CORROBORATION.md)。

## 誠實原則（修復 V1 假信心）

- 只報可驗證事實，明確標 `*Generated at …* no inference`
- 偵測不到就空白，不標「100% High Confidence」
- CLAUDE.md 更新為原子寫入（temp + rename），無變動不寫

## V1 歷史

V1 是 TypeScript MCP server（「主動偵測 pattern + 信心分數」），因假信心與
誤判問題於 V2 重定位；舊程式碼封存於 [legacy/](legacy/)，研究說明見
[RESEARCH-V2.md](RESEARCH-V2.md)、[CLAUDECAT-GOALS.md](CLAUDECAT-GOALS.md)。

## 授權

MIT
