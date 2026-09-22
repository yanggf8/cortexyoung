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
| 2026-09-19 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 685 | 450 | 5 | 0 | synced | 44458 | 552 | 12 | 19698 | 0 | pattern_not_symbol=1425 |
| 2026-09-20 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 685 | 450 | 5 | 0 | synced | 48677 | 552 | 12 | 20792 | 0 | pattern_not_symbol=1631 |
| 2026-09-21 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 685 | 450 | 5 | 0 | synced | 50950 | 552 | 12 | 22543 | 0 | pattern_not_symbol=1741 |
| 2026-09-22 | `/home/yanggf/a/claudecat` | NUC11i5 | fresh | 685 | 450 | 5 | 0 | synced | 53651 | 552 | 12 | 22901 | 0 | pattern_not_symbol=1848 |
## 每日分析發現 (claudecat cort-audit)

_2026-09-22_

- deep/30d 持平在 12，但 deep/7d 連五天掛零（09-18 起），30 天的量全落在窗口外半部；再無 deep 使用的話，30d 指標會隨滾動窗口開始掉，adoption 面是未來幾天要盯的數字。
- 可動作標籤今日仍從缺，不開新規則：pattern_not_symbol 已裁決、剝皮規則觀察中不重開；concrete_file_read 是精確度閘門、unindexed_extension 是正確沉默、unparseable_command 是 parser 產品問題；唯一候選 context_flag 僅 7 筆（<10），誠實等樣本。
- kimi 命中率約 0.98%（分子不動、分母膨脹），剝皮規則觀察期繼續、不下結論；其餘 harness 皆低於 1%（unspecified 高命中是手動探針，不計）；grok 宣告值與實測千筆不符，歸因仍不可信。
- top no_shape shapes 覆蓋約兩成，全是 Bash／Grep 的 envelope 鍵形狀、尚無語義形狀訊號——issue #3 的需求排序暫無新依據。
- 數據品質全綠：兩 hook census 加總等於 fires、無 unknown 桶、FTS synced 且 fresh，repair=none 是自養預期不是無 staleness；真缺口為零（未 chunk 檔全是 chunk_count=0 的正確沉默）；self-heal 採樣仍零樣本，legacy 屬舊列預期、不是零自癒。
- 上游 origin/master 超前本地十個 commit（upgrade／install／deps 與 gate-audit／codegraph 文檔），無 hook 規則變動、口徑不變，下次方便時 pull 即可。
