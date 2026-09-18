## 長期指標 (claudecat cort-audit)

| 日期 | 專案 | host | fresh | chunks | relationships | 未chunk檔 | 真缺口 | FTS drift | 命令數/30d | core/30d | deep/30d | 命令數/7d | deep/7d | decline-top |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 2026-09-06 | `/home/yanggf/a/claudecat` | fresh | 586 | 310 | 5 | ? | synced | 12051 | 473 | 3 | 12035 | 2 |
| 2026-09-07 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 605 | 341 | 5 | ? | synced | 13413 | 473 | 3 | 13397 | 2 | not_a_search_tool=165 |
| 2026-09-07 | `/home/yanggf/a/claudecat` | i51149R3050 | fresh | 602 | 337 | 5 | ? | synced | 859 | 16 | 2 | 828 | 0 | unparseable_command=28 |
| 2026-09-08 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 609 | 345 | 5 | ? | synced | 16152 | 473 | 3 | 16128 | 2 | pattern_not_symbol=213 |
| 2026-09-09 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 625 | 369 | 6 | ? | synced | 18298 | 474 | 3 | 17283 | 2 | pattern_not_symbol=324 |
| 2026-09-10 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 639 | 395 | 5 | 0 | synced | 19939 | 474 | 3 | 16301 | 2 | pattern_not_symbol=430 |
| 2026-09-11 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 24101 | 545 | 12 | 17250 | 11 | pattern_not_symbol=614 |
| 2026-09-12 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 26022 | 546 | 12 | 15531 | 11 | pattern_not_symbol=657 |
| 2026-09-13 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 27885 | 546 | 12 | 17310 | 11 | pattern_not_symbol=737 |
| 2026-09-14 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 28407 | 546 | 12 | 14994 | 9 | pattern_not_symbol=773 |
| 2026-09-15 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 647 | 405 | 5 | 0 | synced | 30750 | 552 | 12 | 14597 | 9 | pattern_not_symbol=939 |
| 2026-09-16 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 647 | 405 | 5 | 0 | synced | 34896 | 552 | 12 | 16598 | 9 | pattern_not_symbol=1135 |
| 2026-09-17 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 647 | 405 | 5 | 0 | synced | 39070 | 552 | 12 | 19354 | 9 | pattern_not_symbol=1198 |
| 2026-09-18 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 647 | 405 | 5 | 0 | synced | 41091 | 552 | 12 | 16990 | 0 | pattern_not_symbol=1248 |
| 2026-09-18 | `/home/yanggf/a/claudecat` | Thinkpade15 | fresh | 663 | 422 | 5 | 0 | synced | 6947 | 14 | 0 | 5112 | 0 | pattern_not_symbol=189 |
## 每日分析發現 (claudecat cort-audit)

_2026-09-18 — Thinkpade15_

- 第三台機器 Thinkpade15 首次入表：fresh、FTS synced、真缺口 0、graph_pending=0，數據品質全綠；本機 30d 命令 6947、7d 5112。跨機口徑提醒：各機 DB 獨立，kimi 分母從 219（NUC11i5）回落到 145 是換機器不是流失。
- deep/30d=0：三台機器中首見深挖歸零（NUC11i5 為 12），本機 30 天內 context+recall 完全沒用過、只有 14 次 impact——adoption 缺口在新機器上最鮮明，「navigate 當 front door 帶入 cort」的行動方向再添一票。
- self-heal 仍零樣本：scanned=14 全是 legacy 舊列、0 自癒、0 背景重建，機制仍未被真實樣本驗證（索引一路 fresh，今日工作只有文件與設定，.md 不在 CODE_EXT 內）。
- hook-suggest 命中率 12/3323（<1%）、hook-refresh no_index=1766 佔 52%——與 NUC11i5 同構的訊號，router 的 shape 規則與 no_index 場景仍是主要改善面。top no_shape shapes 覆蓋 10.5%（210 筆帶 shape），仍全是 Bash／Grep envelope 鍵形狀，無語義形狀訊號。
- 不開新規則：可動作標籤皆欠樣本或已裁決（pattern_not_symbol=189 已裁決觀察中、concrete_file_read=9、context_flag 低於誠實門檻）；`grok` 有 17 筆 harness_declared 與實測不符，按宣告值分群的歸因要改用實測值。
- 本日 repo 事件（非用量數據）：升級到 v2.1.0（CLAUDE.md→AGENTS.md symlink、地圖改寫 ~/.local/share/claudecat/）、殘留的 V1 MCP 註冊已從 ~/.claude.json 移除——今天 CLAUDE.md 被污染蓋掉 symlink 的元兇就是它。

_2026-09-18 — NUC11i5_

- deep30=12 連續第 8 天持平（基線 3，09-11 跳變仍在 30 天窗口內），但 deep7 從 9 掉到 0——過去 7 天零 context／recall 深水區使用；recall 側 30 天仍僅 1 次，深挖幾乎全靠 context，召回仍是 adoption 缺口。該做：繼續觀察 deep7 是否回升，若連續低迷再判讀是否使用模式轉移。
- 可動作標籤今日仍從缺，不開新規則：pattern_not_symbol（已裁決、剝皮規則觀察中，不重開）、concrete_file_read=87（精確度閘門，開火只會是噪音）、unindexed_extension=42（正確沉默，extractor 產品功能）、unparseable_command=30（parser 產品問題）、context_flag 仍 7（<10 誠實等樣本）。
- kimi 命中率 2.09%（9/430，前日 2.31% 9/354）仍居冠，但分子連三天不動、分母膨脹，剝皮規則觀察期繼續、不下結論；其餘 harness 皆 <1%（unspecified 高命中是手動探針，不計）；grok 497 筆 harness_declared 與實測不符，歸因仍不可信。
- top no_shape shapes 覆蓋 13.0%（1032 筆帶 shape），全是 Bash／Grep 的 envelope 鍵形狀、尚無語義形狀訊號——issue #3 的需求排序暫無新依據。
- 數據品質全綠：兩 hook census 加總＝fires、無 unknown/ 桶、FTS synced、repair=none、真缺口 0（5 未 chunk 檔全是 chunk_count=0 的正確沉默）；self-heal 採樣仍零樣本（scanned=495 全是 legacy 舊列，屬預期）。
- 上游今日再新增 4 個 commit（ast-grep pin 升級、install 修復、deps semver-major、docs 專案地圖），本地 HEAD f7a50464 累計落後 origin 565b48ee 共 8 個，未動 hook.rs、census 口徑不變；下次方便時 pull 即可，本次不動工作樹。
