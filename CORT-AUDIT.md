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
## 每日分析發現 (claudecat cort-audit)

_2026-09-14_

- deep30=12 連續第 4 天持平、deep7=11→9 回落（基線 3／2）——09-11 的跳變確認是真實新增的深水區使用而非 30 天窗口效應；今日回落是 7 天窗口的正常進出，recall 側 30 天仍只有 1 次，深挖幾乎全靠 context，召回仍是 adoption 缺口。
- 可動作標籤今日仍從缺，不開新規則：pattern_not_symbol（已裁決、觀察中，今日 773）、concrete_file_read=57（探針看過：多是單檔內 grep＋sed 同一檔，開火只會是噪音，屬精確度閘門）、unparseable_command=30（parser 產品問題，非 hook 規則可解）、context_flag 仍 7（<10 誠實等樣本）。
- kimi 命中率 2.30%（4/174，前日 2.44% 4/164）仍居冠，分子連續多天未動、分母小幅膨脹，剝皮規則觀察期繼續，不下結論；其餘 harness 皆 <1%。
- top no_shape shapes 覆蓋僅 6.7%（492 筆帶 shape），且全是 Bash／Grep 的 envelope 鍵形狀、尚無語義形狀訊號——shape 母體還在累積，issue #3 的需求排序暫無新依據。
- 數據品質全綠：兩 hook census 加總＝fires、無 unknown/ 桶、FTS synced、repair=none、真缺口 0（5 未 chunk 檔全是 chunk_count=0 的正確沉默）；4 檔含 unparsed chunk 屬 advisory，不翻旗標。上游今天 17:44 落地 5f6d5267（query-time self-heal：`impact`/`context` 回答前自癒 index，P2 的 reindex 建議退役，新 payload 欄位 self_healed/heal_mode/heal_ms/heal_deferred）；本機 `~/.local/share/cortexyoung/cort/cort` 17:35 已換成 heal 版 binary、`~/.claude/skills/ast-grep` 17:39 已同步。claudecat 的 audit 自本輪起採樣 heal 欄位——heal 資料尚未累積（今日實測 30 天窗 scanned=495 全是 legacy 舊列、0 次自癒、0 次背景重建），報告呈零樣本屬預期。
- 語意備案：self-heal 上線後 audit 的 `repair=none` 會更常見——查詢自己把 index 養新，不得誤讀為「沒有 staleness 發生過」。
- 語意備案：上游 README 已改 `rebuild_required` 的含義——從「只有前景 `cort index` 會修」變成「下一個 foreground query 自己修」。
