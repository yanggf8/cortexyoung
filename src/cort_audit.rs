//! cort-audit：把「與 cortexyoung 整合是否達成」變成可收集、可追蹤的數據
//! （索引健康 / 覆蓋缺口 / FTS 同步 / 用量），全部唯讀；
//! `claudecat cort-audit --track <file>` 寫入長期指標表（原子、同日更新）。
use crate::cort::{CortAudit, CortAuditIndex, UsageWindow};
use crate::dates::today_iso;
use std::path::Path;

pub const TRACK_SECTION: &str = "## 長期指標 (claudecat cort-audit)";

fn core_verb_count(u: &UsageWindow) -> i64 {
    ["context", "impact", "recall", "struct", "read"]
        .iter()
        .map(|v| u.by_command.get(*v).copied().unwrap_or(0))
        .sum()
}

fn deep_verb_count(u: &UsageWindow) -> i64 {
    u.by_command.get("context").copied().unwrap_or(0)
        + u.by_command.get("recall").copied().unwrap_or(0)
}

/// Option<i64> 呈現：None = 無法判讀，顯示 `?`（絕不顯示 0 假裝健康）
fn opt_i64(v: &Option<i64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "?".to_string())
}

fn render_index(i: &CortAuditIndex, s: &mut String) {
    s.push_str("## 索引健康\n");
    s.push_str(&format!(
        "- fresh={}（git head 相符={}，索引距今 {} 天）\n",
        if i.fresh { "fresh" } else { "STALE" },
        i.git_head_matches,
        i.index_age_days
            .map(|d| d.to_string())
            .unwrap_or_else(|| "?".into())
    ));
    s.push_str(&format!(
        "- chunks={} relationships={} name={} path={}\n",
        i.chunk_count, i.relationships_count, i.name, i.path
    ));
    s.push_str(&format!(
        "- schema={} graph_pending={}\n",
        i.schema_version.as_deref().unwrap_or("?"),
        match i.graph_pending {
            Some(true) => "1（圖落後 chunks）",
            Some(false) => "0",
            None => "?（無法判讀）",
        }
    ));
    s.push_str(&format!(
        "- repair={}（cort status 的三態，cortexyoung f4ad4c7d：none=不欠、refreshable=增量可修、rebuild_required=只有全量會修；? = PATH 上沒有 cort 或判讀失敗）\n",
        i.repair.as_deref().unwrap_or("?")
    ));
    s.push_str(&format!(
        "- file_state={} files，chunked={} files，FTS docs={}（{}）\n",
        opt_i64(&i.file_state_files),
        opt_i64(&i.chunked_files),
        opt_i64(&i.fts_docs),
        match i.fts_drift {
            Some(0) => "synced".to_string(),
            Some(n) => format!("drift={n}"),
            None => "unknown".to_string(),
        }
    ));
}

fn render_coverage(i: &CortAuditIndex, s: &mut String) {
    s.push_str("\n## 覆蓋缺口\n");
    match i.not_chunked_total {
        None => {
            s.push_str("- 覆蓋狀態無法判讀（file_state/chunks 查詢失敗）——不假裝「無缺口」\n");
        }
        Some(0) => {
            if i.chunk_count == 0 && i.file_state_files == Some(0) {
                s.push_str(
                    "- 空索引（chunks=0、file_state=0）——索引可能壞了或尚未掃描，不算「無缺口」\n",
                );
            } else {
                s.push_str(&format!(
                    "- 無缺口（file_state 全部都有 chunk；含 unparsed chunk 的檔案 {} 檔）\n",
                    i.files_with_unparsed_chunks
                ));
            }
        }
        Some(total) => {
            // v7 的 chunk_count 之前，這裡只能印「兩種成因都要查」的猜測；
            // 現在同一句話有沒有定論，取決於欄位查不查得到。
            let real_gap = i.real_gap();
            let unknown = i.not_chunked_unknown.unwrap_or(0);
            match real_gap {
                Some(0) => {
                    s.push_str(&format!(
                        "- {}：{total} 檔未 chunk，其中 {} 檔 `chunk_count = 0`——extractor 掃過、檔內沒有可 chunk 的宣告（cortexyoung#2 定義的正確沉默）\n",
                        if unknown > 0 {
                            "無已證實的真缺口"
                        } else {
                            "無真缺口"
                        },
                        opt_i64(&i.not_chunked_scanned_empty),
                    ));
                }
                Some(n) => {
                    s.push_str(&format!(
                        "- 真缺口 {n} 檔（{total} 檔未 chunk 中，{} 檔是正確沉默）：file_state 說檔裡有宣告、chunks 卻沒有 → cortexyoung#5 的形狀，增量因 git diff 為空永不重看，只有全量 `cort index` 會修\n",
                        opt_i64(&i.not_chunked_scanned_empty),
                    ));
                }
                None => {
                    s.push_str(&format!(
                        "- {total} 檔在 file_state 但從未被 chunk，無法用欄位判讀成因（舊 schema 沒有 `chunk_count`，或查詢失敗）——不假裝沒有缺口\n"
                    ));
                }
            }
            s.push_str(&format!(
                "- 未 chunk 檔清單（前 {} 檔{}）：\n",
                i.not_chunked_files.len(),
                if real_gap == Some(0) {
                    "；皆為正確沉默"
                } else {
                    ""
                }
            ));
            for f in &i.not_chunked_files {
                s.push_str(&format!("  - `{f}`\n"));
            }
            if unknown > 0 {
                s.push_str(&format!(
                    "- 其中 {unknown} 檔 `chunk_count = -1`：v7 之前寫入、之後從未重寫的列，**不是掃描結果**——重新全量 `cort index` 後才會有定論\n"
                ));
            }
        }
    }
    s.push_str(&format!(
        "- 含 unparsed chunk 的檔案：{} 檔\n",
        i.files_with_unparsed_chunks
    ));
    if i.indexed_uncommitted_files.is_some_and(|n| n > 0) {
        s.push_str(&format!(
            "- 警示：{} 檔的索引建立自未提交內容（`indexed_uncommitted`，cortexyoung#5/v6）——git 還原那些變更後，增量看不到 diff，索引就留在一個不存在的版本上\n",
            opt_i64(&i.indexed_uncommitted_files)
        ));
    }
}

fn render_usage(u: &UsageWindow, s: &mut String) {
    s.push_str(&format!("\n## 用量（last {} 天）\n", u.window_days));
    s.push_str(&format!(
        "- 總命令數 {}；核心動詞（context/impact/recall/struct/read）{}；saved_bytes={}\n",
        u.total_commands,
        core_verb_count(u),
        u.saved_bytes
    ));
    s.push_str("\n| command | count |\n|---|---:|\n");
    for (k, v) in &u.by_command {
        s.push_str(&format!("| `{k}` | {v} |\n"));
    }
    // query-time self-heal（cortexyoung 5f6d5267 起）：impact/context 回答前自癒 index。
    // 零樣本也要印 scanned——「0 次自癒」（legacy=0）與「還沒資料」（legacy=N）是兩回事，
    // 混在一起會把機制上線初期讀成「沒用」。
    s.push_str(
        "\nself-heal 採樣（impact/context 回答前自癒 index；cortexyoung 5f6d5267 起新列才帶 heal 欄）：\n",
    );
    let modes: Vec<String> = u
        .heal_modes
        .iter()
        .map(|(m, c)| format!("{m}={c}"))
        .collect();
    let reasons: Vec<String> = u
        .heal_deferred
        .iter()
        .map(|(r, c)| format!("{r}={c}"))
        .collect();
    s.push_str(&format!(
        "- scanned={}（impact+context）：self_healed={}{}、heal_deferred={}{}、legacy={}\n",
        u.heal_scanned,
        u.heal_self_healed,
        if modes.is_empty() {
            String::new()
        } else {
            format!("（{}）", modes.join("、"))
        },
        u.heal_deferred.values().sum::<i64>(),
        if reasons.is_empty() {
            String::new()
        } else {
            format!("（{}）", reasons.join("、"))
        },
        u.heal_legacy,
    ));
    s.push_str(&format!(
        "- heal_ms 合計={} max={}；背景重建（index --heal-background）={} 次\n",
        u.heal_ms_total, u.heal_ms_max, u.heal_background
    ));
    if u.heal_unparseable > 0 {
        s.push_str(&format!(
            "- 另有 {} 筆 impact/context 列 args_summary 為 NULL／非法 JSON——無法判讀 heal 欄，不混進上面任一桶\n",
            u.heal_unparseable
        ));
    }
    if !u.census.is_empty() {
        // census（cortexyoung 6623113d 口徑）取代舊的 suggest/refresh outcome 清單：
        // 每列恰落一桶、加總＝fires，對帳一眼可查；舊清單把 status_error、
        // 非 JSON、沒 hook 欄三種情況全擠在一個 "unparsed" 假鍵裡，看不見殘留。
        // 閉合檢查跨兩條獨立路徑：by_command 走 GROUP BY、_total 走逐列掃描，
        // 兩者不符＝有列在掃描路上被丟掉，先查數據品質再談其他數字。
        s.push_str(
            "\nhook census（每列恰落一桶，加總＝fires；口徑 cortexyoung 6623113d/4b895589）：\n",
        );
        for (cmd, buckets) in &u.census {
            let total = buckets.get("_total").copied().unwrap_or(0);
            let fires = u.by_command.get(cmd).copied().unwrap_or(0);
            if total != fires {
                s.push_str(&format!(
                    "- `{cmd}`：**分割不閉合**（census 數到 {total}，command_log 有 {fires} 列）——有列在掃描路上消失，先查數據品質\n"
                ));
            }
            let mut ranked: Vec<_> = buckets.iter().filter(|(k, _)| *k != "_total").collect();
            ranked.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
            let parts: Vec<String> = ranked.iter().map(|(k, c)| format!("{k}={c}")).collect();
            s.push_str(&format!(
                "- `{cmd}`（{total} fires）：{}\n",
                parts.join("、")
            ));
        }
    }
    if !u.declines.is_empty() {
        s.push_str("\ntop declines（no_shape 歸因；cortexyoung c290c383 起有數據）：\n");
        // not_a_search_tool 是「本來就不是搜尋」的正確沉默（baseline），不是 tuning 目標——
        // 混進排序會把行動靶心擠掉（2026-09-07 實測 165 筆 baseline 壓過一切），另行呈報。
        let baseline = u
            .declines
            .get("no_shape/not_a_search_tool")
            .copied()
            .unwrap_or(0);
        let actionable: Vec<_> = u
            .declines
            .iter()
            .filter(|(k, _)| !k.ends_with("/not_a_search_tool"))
            .collect();
        let no_shape = u.suggest_outcomes.get("no_shape").copied().unwrap_or(0);
        let mut ranked: Vec<_> = actionable.to_vec();
        ranked.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        for (k, c) in ranked.iter().take(5) {
            // 一位小數，理由與 shape 段同一條：實測 30/9126＝0.3%、18/9126＝0.2%，
            // `{:.0}` 會把三個不同量級的標籤全印成 0%，那一欄就不再提供任何排序資訊。
            s.push_str(&format!(
                "- {k}: {c}（佔 no_shape {:.1}%）\n",
                **c as f64 / no_shape.max(1) as f64 * 100.0
            ));
        }
        if baseline > 0 {
            s.push_str(&format!(
                "- （baseline not_a_search_tool: {baseline}，不列入排序）\n"
            ));
        }
    }
    if !u.no_shape_shapes.is_empty() {
        // decline 只說「為什麼沒開口」，shape 說「是什麼形狀的呼叫沒開口」——
        // 後者才是能直接對著改 router 規則的靶心。口徑與上面的排序一致（排除 baseline）。
        s.push_str(
            "\ntop no_shape shapes（工具名|top-level key，不含 payload 內容；cortexyoung 09f55136 起有數據）：\n",
        );
        let no_shape = u.suggest_outcomes.get("no_shape").copied().unwrap_or(0);
        let baseline = u
            .declines
            .get("no_shape/not_a_search_tool")
            .copied()
            .unwrap_or(0);
        let actionable = (no_shape - baseline).max(0);
        // 分母是**實際帶 shape 的列數**，不是 actionable no_shape：shape 從 09f55136
        // 才開始寫，窗內絕大多數 actionable 列根本沒這個欄位（實測 20/6904）。
        // 拿 actionable 當分母會把 11 筆算成 0.2%、被 {:.0} 印成 0%，等於親手把唯一的
        // 行動靶心標成「無關緊要」——這正是這個 repo 一路在防的那種假數字。
        // 樣本小是事實，但要用「覆蓋率另印一行」講出來，不是把比例稀釋掉。
        let with_shape: i64 = u.no_shape_shapes.values().sum();
        s.push_str(&format!(
            "- （母體：{with_shape} 筆帶 shape ÷ {actionable} 筆 actionable no_shape{}；shape 自 cortexyoung 09f55136 起才寫入，舊列沒有——下列百分比的分母是前者，不是後者）\n",
            if actionable > 0 {
                // 覆蓋率本身也不准被四捨五入成 0%——那是同一個謊言的另一半：
                // 實測 20/6904＝0.29%，`{:.0}` 會印成「覆蓋 0%」，讀起來像完全沒資料。
                format!("＝覆蓋 {:.1}%", with_shape as f64 / actionable as f64 * 100.0)
            } else {
                String::new()
            }
        ));
        let mut ranked: Vec<_> = u.no_shape_shapes.iter().collect();
        ranked.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        for (shape, c) in ranked.iter().take(5) {
            s.push_str(&format!(
                "- `{shape}`: {c}（佔帶 shape 的 {with_shape} 筆 {:.0}%）\n",
                **c as f64 / with_shape.max(1) as f64 * 100.0
            ));
        }
    }
    if !u.by_harness.is_empty() {
        // `harness` 只在 hook payload 上（動詞命令沒有），所以這張表只能講
        // 「router 對誰開了口」，不能講「誰真的用了 cort」——標題就說清楚。
        s.push_str("\nharness 切面（僅 hook payload：router 面對誰；v3 payload 起）：\n");
        s.push_str(
            "\n| harness | hook-suggest | 命中 | 命中率 | no_shape | top decline（排除 baseline） | refresh |\n|---|---:|---:|---:|---:|---|---:|\n",
        );
        let mut rows: Vec<_> = u.by_harness.iter().collect();
        rows.sort_by_key(|(_, st)| std::cmp::Reverse(st.suggests));
        for (h, st) in rows {
            s.push_str(&format!(
                "| `{h}` | {} | {} | {} | {} | {} | {} |\n",
                st.suggests,
                st.hits,
                if st.suggests > 0 {
                    format!("{:.2}%", st.hits as f64 / st.suggests as f64 * 100.0)
                } else {
                    "-".to_string()
                },
                st.no_shape,
                st.top_decline
                    .as_ref()
                    .map(|(t, c)| format!("{t}={c}"))
                    .unwrap_or_else(|| "-".into()),
                st.refreshes,
            ));
        }
        if u.harness_unknown > 0 {
            s.push_str(&format!(
                "- 另有 {} 筆 hook 列沒有 `harness` 欄（v3 前的歷史列），不計入上表\n",
                u.harness_unknown
            ));
        }
        for (h, st) in &u.by_harness {
            if st.declared_mismatch > 0 {
                s.push_str(&format!(
                    "- `{h}` 有 {} 筆 `harness_declared` 與實際不符——信任 declared 值的歸因會被汙染\n",
                    st.declared_mismatch
                ));
            }
        }
    }
    s.push_str(&format!(
        "\nerrors={} index_stale_queries={}\n",
        u.errors, u.index_stale_queries
    ));
}

/// 把 CortAudit 渲染成可讀報告（含規則式「解讀 / 行動」）
pub fn render(a: &CortAudit) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# claudecat cort-audit — {}（用量窗口 {} 天）\n\n",
        a.root, a.window_days
    ));
    match &a.index {
        Some(i) => {
            render_index(i, &mut s);
            render_coverage(i, &mut s);
        }
        None => {
            if a.db_exists {
                s.push_str(
                    "## 索引健康\n- cort DB 存在但無法讀取（schema 不相容或檔案損毀？）——不假裝「尚未索引」\n",
                );
            } else {
                s.push_str("## 索引健康\n- 尚未對本專案建立 cort 索引。\n");
            }
        }
    }
    match &a.usage {
        Some(u) => render_usage(u, &mut s),
        None => {
            s.push_str("\n## 用量\n- usage.db 不存在（cort 尚未被使用過）\n");
        }
    }

    // 7 天早期訊號（與 --window 的長期趨勢互補）
    if let Some(u7) = &a.usage_7d {
        s.push_str(&format!(
            "\n## 7 天早期訊號\n- 命令 {}、deep（context+recall）{}\n",
            u7.total_commands,
            deep_verb_count(u7)
        ));
    }

    s.push_str("\n## 解讀 & 行動（規則式）\n");
    let mut hints: Vec<String> = Vec::new();
    if let Some(i) = &a.index {
        // 圖落後與 HEAD/age 落後是兩件事，行動也不同——不混成同一句
        match i.graph_pending {
            Some(true) => hints.push(
                "graph_pending=1：relationships 是升級或中斷前的舊邊 → 反向依賴（`cort impact` / `navigate --cort` 路線）先別信，執行 `cort index` 全量重建"
                    .to_string(),
            ),
            None => hints.push(
                "`_cortex_meta` 無法判讀（表不存在或查詢失敗）→ 不視為圖已重建；舊版 cort DB 才會沒有這張表"
                    .to_string(),
            ),
            Some(false) => {}
        }
        if !i.fresh && i.graph_pending != Some(true) {
            hints
                .push("索引 STALE → 執行 `cort index`（或檢查 hook-refresh 是否在跑）".to_string());
        }
        if i.chunk_count == 0 && i.file_state_files == Some(0) {
            hints.push(
                "空索引（0 chunks、0 file_state）→ 執行 `cort index` 或檢查 extractor/路徑"
                    .to_string(),
            );
        }
        match i.not_chunked_total {
            // v7 的 chunk_count 讓這裡能給定論：正確沉默不是行動項，真缺口才是。
            // 只有欄位讀不到（real_gap=None）才退回舊的「兩種成因都要查」。
            Some(n) if n > 0 => match i.real_gap() {
                Some(0) => {
                    if i.not_chunked_unknown.is_some_and(|u| u > 0) {
                        hints.push(format!(
                            "未 chunk 的 {n} 檔裡有 {} 檔 `chunk_count = -1`（v7 前寫入、未重寫）→ 那不是掃描結果，要定論就跑一次全量 `cort index`",
                            opt_i64(&i.not_chunked_unknown)
                        ));
                    }
                }
                Some(g) => hints.push(format!(
                    "coverage 真缺口 {g} 檔（共 {n} 檔未 chunk，其餘是 cortexyoung#2 的正確沉默）→ file_state 說有宣告、chunks 沒有，就是 cortexyoung#5：增量的 git diff 永遠是空的，只有全量 `cort index` 會修"
                )),
                None => hints.push(format!(
                    "coverage 缺口：{n} 檔在 file_state 但從未被 chunk，且這個 DB 沒有 v7 的 `chunk_count` 可判讀 → 兩種成因都要查：①本來就沒有可 chunk 的宣告（只 import 後呼叫的 driver 檔，見 cortexyoung#2）②索引停在一個從未提交、後來被 git 還原的版本——增量因 git diff 為空而永不重看（cortexyoung#5；解法是全量 `cort index`）。都不是才查 extractor 規則"
                )),
            },
            None => hints.push(
                "覆蓋查詢失敗 → 不視為「無缺口」；檢查 cort schema 版本差異".to_string(),
            ),
            _ => {}
        }
        match i.fts_drift {
            Some(0) => {}
            Some(n) => hints.push(format!(
                "FTS 索引與 chunks 不同步（drift={n}）→ cort 端重建 FTS"
            )),
            None => {
                hints.push("FTS 同步無法判讀（chunks_fts 查詢失敗）→ 不假裝 synced".to_string())
            }
        }
    }
    if let Some(u) = &a.usage {
        let suggests = u.suggest_outcomes.get("hit").copied().unwrap_or(0)
            + u.suggest_outcomes.get("hit_yielded").copied().unwrap_or(0)
            + u.suggest_outcomes.get("hit_stale").copied().unwrap_or(0);
        let suggest_total = u
            .by_command
            .get("hook-suggest")
            .copied()
            .unwrap_or(0)
            .max(u.suggest_outcomes.values().sum());
        if u.total_commands > 0 {
            let core = core_verb_count(u);
            if core * 100 < u.total_commands {
                hints.push(format!(
                    "核心動詞用量低（{core}/{total}）→ adoption 瓶頸：讓 claudecat navigate 當 front door 帶入 cort",
                    core = core,
                    total = u.total_commands
                ));
            }
            let deep = u.by_command.get("context").copied().unwrap_or(0)
                + u.by_command.get("recall").copied().unwrap_or(0);
            if deep < 5 {
                hints.push(format!(
                    "深挖動詞用量極低（context+recall={deep}）→ agent 幾乎不問「這個符號還有誰在用/上下文」"
                ));
            }
            if suggest_total >= 100 && suggests * 100 < suggest_total {
                hints.push(format!(
                    "hook-suggest 命中率 {suggests}/{suggest_total}（<1%）→ router 大多 no_shape，檢查 hook shape 規則"
                ));
            }
            // `unspecified`＝旗標與 transcript 都認不出來源（cort main.rs 的 fallback）。
            // 本機三個接線的 harness 全都帶 `--harness`，所以 agent 流量落不進這格：
            // 實測那些列是開發/安裝時手打的探針（2026-09-09 查證，見 CORT-INTEGRATION.md）。
            // 分母不動（動了跨日不可比），但污染要講出來——30d 實測 40 命中裡有 9 筆是自測。
            if let Some(st) = u.by_harness.get("unspecified") {
                if st.hits > 0 {
                    hints.push(format!(
                        "命中數含 {} 筆 `unspecified`（非 agent 流量的手動探針）→ agent 實際命中 {}／{}；比率請用這組",
                        st.hits,
                        suggests - st.hits,
                        suggest_total - st.suggests
                    ));
                }
            }
            // harness 切面：有量卻 0 命中的 harness 是可指名的靶心（總命中率看不出來）
            for (h, st) in &u.by_harness {
                if st.suggests >= 100 && st.hits == 0 {
                    hints.push(format!(
                        "`{h}` {} 筆 hook-suggest、0 命中 → router 對這個 harness 從沒開過口（top decline: {}）",
                        st.suggests,
                        st.top_decline
                            .as_ref()
                            .map(|(t, c)| format!("{t}={c}"))
                            .unwrap_or_else(|| "無".into())
                    ));
                }
                if st.declared_mismatch > 0 {
                    hints.push(format!(
                        "`{h}` 有 {} 筆 `harness_declared` 不符 → 任何按宣告值分群的歸因都要改用實測值",
                        st.declared_mismatch
                    ));
                }
            }
        }
    }
    if hints.is_empty() {
        s.push_str("- 目前沒有需要行動的異常\n");
    } else {
        for h in &hints {
            s.push_str(&format!("- {h}\n"));
        }
    }
    s
}

/// 每日分析的發現區塊標題（與長期指標表同檔共存，各自 section）
pub const FINDINGS_SECTION: &str = "## 每日分析發現 (claudecat cort-audit)";

/// 把每日分析的發現寫進文件的發現 section（原子；section 外的內容原樣保留）。
///
/// 為什麼要有這個：排程分析原本只把結論印到 `cort-audit-analysis.log`，等於沒回報
/// （2026-09-09 使用者指出）。發現屬於文件、不屬於 CLAUDE.md——CLAUDE.md 只留規則，
/// 最多放一行指引。**整段取代、只留最新一天**：歷史在 git，文件裡疊成流水帳只會沒人看。
pub fn findings_update(path: &Path, body: &str) -> std::io::Result<(bool, String)> {
    let existing = if path.is_file() {
        std::fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    };
    let block = format!(
        "{FINDINGS_SECTION}\n\n_{}_\n\n{}\n",
        today_iso(),
        body.trim()
    );
    // section 邊界與長期指標表同一套規則：標題到下一個 `#` 標題（或 EOF）
    let new_content = match existing.find(FINDINGS_SECTION) {
        Some(start) => {
            let after_title = start + FINDINGS_SECTION.len();
            let end = after_title
                + crate::explore::next_heading_offset(&existing[after_title..])
                    .unwrap_or(existing.len() - after_title);
            format!("{}{}{}", &existing[..start], block, &existing[end..])
        }
        None => {
            let mut out = existing.clone();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push_str(&block);
            out
        }
    };
    let changed = new_content != existing;
    if changed {
        crate::explore::atomic_write(path, &new_content)?;
    }
    Ok((changed, new_content))
}

/// cort-audit 長期指標列（單行）：長期趨勢（30d）+ 早期訊號（7d）+ deep 動詞
pub fn row_md(a: &CortAudit) -> String {
    let idx = a.index.as_ref();
    // 不加欄（09-06 那列少一格 host 已經證明加欄的代價），改讓既有 fresh 格說得更準：
    // 圖落後寫 STALE/graph（跟 HEAD/age 的 STALE 分得開），讀不到 meta 寫 fresh?
    // repair（f4ad4c7d，問 PATH 上的 cort）比本地 fresh 誠實——extractor/schema 變了
    // 本地仍說 fresh，rebuild_required 只有 binary 看得見，覆寫在前。
    let fresh = idx
        .map(|i| match i.repair.as_deref() {
            Some("rebuild_required") => "STALE/rebuild",
            Some("refreshable") => "STALE/refresh",
            _ => match (i.fresh, i.graph_pending) {
                (_, Some(true)) => "STALE/graph",
                (false, _) => "STALE",
                (true, None) => "fresh?",
                (true, Some(false)) => "fresh",
            },
        })
        .unwrap_or("no-index");
    let chunks = idx
        .map(|i| i.chunk_count.to_string())
        .unwrap_or_else(|| "-".into());
    let rels = idx
        .map(|i| i.relationships_count.to_string())
        .unwrap_or_else(|| "-".into());
    // 未 chunk 總數原樣保留（它的歷史值才連續），真缺口另開一欄：後者扣掉 cortexyoung#2
    // 的正確沉默，才是會對應到行動的那條曲線。換欄名會讓舊列坐在新語意底下——
    // 兩欄並存是唯一不說謊的作法。舊 schema 讀不到時照既有慣例寫 `?`，不拿 0 假裝沒缺口。
    let not_chunked = idx
        .map(|i| opt_i64(&i.not_chunked_total))
        .unwrap_or_else(|| "-".into());
    let real_gap = idx
        .map(|i| opt_i64(&i.real_gap()))
        .unwrap_or_else(|| "-".into());
    let fts = idx
        .map(|i| match i.fts_drift {
            Some(0) => "synced".to_string(),
            Some(n) => format!("drift={n}"),
            None => "unknown".to_string(),
        })
        .unwrap_or_else(|| "-".into());
    let usage = a.usage.as_ref();
    let cmds = usage
        .map(|u| u.total_commands.to_string())
        .unwrap_or_else(|| "-".into());
    let core = usage
        .map(|u| core_verb_count(u).to_string())
        .unwrap_or_else(|| "-".into());
    let deep = usage
        .map(|u| deep_verb_count(u).to_string())
        .unwrap_or_else(|| "-".into());
    let u7 = a.usage_7d.as_ref();
    let cmds7 = u7
        .map(|u| u.total_commands.to_string())
        .unwrap_or_else(|| "-".into());
    let deep7 = u7
        .map(|u| deep_verb_count(u).to_string())
        .unwrap_or_else(|| "-".into());
    let decline_top = usage
        .and_then(|u| {
            u.declines
                .iter()
                // baseline（本來就不是搜尋的命令）不是行動靶心——排序排除
                .filter(|(k, _)| !k.ends_with("/not_a_search_tool"))
                .max_by_key(|(_, c)| **c)
                .map(|(k, c)| (k.rsplit('/').next().unwrap_or(k).to_string(), *c))
        })
        .map(|(tag, c)| format!("{tag}={c}"))
        .unwrap_or_else(|| "-".into());
    format!(
        "| {} | `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        today_iso(),
        a.root,
        a.host,
        fresh,
        chunks,
        rels,
        not_chunked,
        real_gap,
        fts,
        cmds,
        core,
        deep,
        cmds7,
        deep7,
        decline_top,
    )
}

/// 把多個 cort-audit 寫進文件的長期指標表（原子、同日更新；語意同 explore track）
pub fn track_update(path: &Path, audits: &[&CortAudit]) -> std::io::Result<(bool, String)> {
    let window = audits.first().map(|a| a.window_days).unwrap_or(30);
    let header = format!(
        "{}\n\n| 日期 | 專案 | host | fresh | chunks | relationships | 未chunk檔 | 真缺口 | FTS drift | 命令數/{window}d | core/{window}d | deep/{window}d | 命令數/7d | deep/7d | decline-top |\n|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|\n",
        TRACK_SECTION
    );
    let rows: Vec<String> = audits.iter().map(|a| row_md(a)).collect();
    crate::explore::track_table(path, TRACK_SECTION, &header, &rows, Some(3))
}
