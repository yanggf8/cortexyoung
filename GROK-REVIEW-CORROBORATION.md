# Grok Review — Corroboration 紀錄（2026-09-05）

Grok（grok-4.5，`grok -p`）針對 ClaudeCat V2（commit 7e71704）產出 26 條 review。
我先**實測 corroborate**（不盲信），再修復。

## Corroboration 結果

| # | Grok 指控 | 實測 | 結論 | 修復 |
|---|---|---|---|---|
| 2 | 目錄 LOC 雙重計數 | 2 檔各 1 LOC → `src/ (2 files, 5 LOC)` | ✅ 屬實 | walk.rs 重寫：DirStat 只存直接子檔 + rollup |
| 3 | 大 repo 字母序截斷 | sort()+truncate 註解卻寫 largest | ✅ 屬實 | 全量計 LOC 再 top-k |
| 4 | dual manifest 標成 Node | package.json 蓋掉 Cargo | ✅ 屬實 | primary rank 重寫 |
| 5 | explore「節省%」假指標 | chars/4 vs LOC×6、恒 100 欄 | ✅ 屬實 | 改 `map vs full-read`、去掉裝飾欄、overhead 標記 |
| 6 | track 誤刪兄弟 repo | foo 更新吃掉 foo-bar 列 | ✅ 屬實（實測復現） | exact root 比對 |
| 7 | update --root subdir 汙染父專案 | 向上搜尋 CLAUDE.md | ⚠️ 條件屬實 | 只寫 root/CLAUDE.md |
| 8 | Go deps 未解析 | require 沒讀 | ✅ 屬實 | parse_go_mod 讀 require |
| 9 | config 當 code | toml/json 進 LOC/key_files | ✅ 屬實 | config 分離 |
| 11 | legacy/archive 靜默排除 | excluded_paths 沒用 | ✅ 屬實 | 排除但顯示 |
| 13 | 原子寫入弱 | 無 fsync、固定 tmp | ✅ 屬實 | fsync + 唯一 tmp |
| 16 | package manager 硬編 npm | 不看 lockfile | ✅ 屬實 | lockfile 偵測 |
| 24 | 探索%含 Write 誇大 | SESSION-EVIDENCE 表格 | ✅ 屬實 | 重算不含 Write/Edit（59–94%） |

其餘（#1 footer 措辭、#10 errors 吞掉、#12 300 LOC 門檻、#14 guardrails 鏡像、
#15 top10=0、#17 tsx、#18-26）多為 P1/P2：品質、測試補強、整合完整度，紀錄待辦。

## 待辦（未修）
- #10 解析失敗寫入 `errors` 並顯示
- #12 mini 門檻改「render 後比 token」或改文案
- #17 `.tsx` 用 TSX grammar
- #19 符號截斷標 `+N omitted`
- #21 按語言復用 Parser
- #22 測試補對抗案例（<10 檔 top10、broken manifest）
- #23 `claudecat skill` / hook 整合（README 已標未來）
- #25 Cargo workspace members 彙總（部分已做：entry points）
