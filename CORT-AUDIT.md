## 長期指標 (claudecat cort-audit)

| 日期 | 專案 | host | fresh | chunks | relationships | 未chunk檔 | 真缺口 | FTS drift | 命令數/30d | core/30d | deep/30d | 命令數/7d | deep/7d | decline-top |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 2026-09-06 | `/home/yanggf/a/claudecat` | fresh | 586 | 310 | 5 | ? | synced | 12051 | 473 | 3 | 12035 | 2 |
| 2026-09-07 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 605 | 341 | 5 | ? | synced | 13413 | 473 | 3 | 13397 | 2 | not_a_search_tool=165 |
| 2026-09-08 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 609 | 345 | 5 | ? | synced | 16152 | 473 | 3 | 16128 | 2 | pattern_not_symbol=213 |
| 2026-09-09 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 625 | 369 | 6 | ? | synced | 18298 | 474 | 3 | 17283 | 2 | pattern_not_symbol=324 |
| 2026-09-10 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 639 | 395 | 5 | 0 | synced | 19939 | 474 | 3 | 16301 | 2 | pattern_not_symbol=430 |
| 2026-09-11 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 24101 | 545 | 12 | 17250 | 11 | pattern_not_symbol=614 |
| 2026-09-12 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 26022 | 546 | 12 | 15531 | 11 | pattern_not_symbol=657 |
| 2026-09-13 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 27885 | 546 | 12 | 17310 | 11 | pattern_not_symbol=737 |
| 2026-09-14 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 645 | 401 | 5 | 0 | synced | 28407 | 546 | 12 | 14994 | 9 | pattern_not_symbol=773 |
| 2026-09-15 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 647 | 405 | 5 | 0 | synced | 30750 | 552 | 12 | 14597 | 9 | pattern_not_symbol=939 |
## 每日分析發現 (claudecat cort-audit)

_2026-09-15_

- deep30=12 連續第 5 天持平、deep7=9 持平（基線 3／2）——09-11 的跳變確認是真實新增的深水區使用而非 30 天窗口效應；recall 側 30 天仍只有 1 次，深挖幾乎全靠 context，召回仍是 adoption 缺口。
- 可動作標籤今日仍從缺，不開新規則：pattern_not_symbol（已裁決、剝皮規則觀察中，不重開）、concrete_file_read=68（探針看過：多是單檔內 grep＋sed 同一檔，開火只會是噪音，屬精確度閘門）、unparseable_command=30（parser 產品問題，非 hook 規則可解）、context_flag 仍 7（<10 誠實等樣本）。
- kimi 命中率 3.20%（7/219，前日 2.30% 4/174）仍居冠、分子終於動了 +3，剝皮規則觀察期繼續、不下結論；其餘 harness 皆 <1%（unspecified 高命中是手動探針，不計）。
- top no_shape shapes 覆蓋 9.0%（681 筆帶 shape），且全是 Bash／Grep 的 envelope 鍵形狀、尚無語義形狀訊號——shape 母體還在累積，issue #3 的需求排序暫無新依據。
- 數據品質全綠：兩 hook census 加總＝fires、無 unknown/ 桶、FTS synced、repair=none、真缺口 0（5 未 chunk 檔全是 chunk_count=0 的正確沉默）；self-heal 採樣仍零樣本（scanned=495 全是 legacy 舊列，屬預期）；上游 HEAD 仍是 5f6d5267、fetch 後無新 commit。
