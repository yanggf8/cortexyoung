# 每日 cort-audit 分析 + 開下一條規則（headless 排程任務）

你是排程觸發的 Claude Code session，沒有對話上下文——本檔就是完整指示。
背景：claudecat（`/home/yanggf/a/claudecat`）與 cortexyoung（`/home/yanggf/a/cortexyoung`）是
使用者的兩個 repo。cort-audit 每天量測 cort 整合成效（OS cron 09:17 寫 CORT-AUDIT.md），
本任務做每日分析，並在數據足夠時挑最大的 decline 標籤開下一條 hook 規則。

## 步驟

1. 確認 `/home/yanggf/a/claudecat/CORT-AUDIT.md` 最新列是今天的；缺則：
   `cd /home/yanggf/a/claudecat && /home/yanggf/.cargo/bin/cargo run -q -- cort-audit --root . --track CORT-AUDIT.md`
2. decline/shape 的需求排序看 `cort-audit` 報告本身（步驟 1 的 stdout 就有，或
   `cargo run -q -- cort-audit --root .`），不用再開手工 SQL——報告的「hook census」區
   （claudecat 已自算 cortexyoung 6623113d 口徑）每列恰落一桶、加總＝fires；
   「top declines」已排除 `not_a_search_tool` baseline（那是「本來就不是搜尋」的正確沉默）；
   「top no_shape shapes」＝`工具名|排序後的 top-level key 名`，是 issue #3 的需求排序依據：
   要開哪條規則看哪個 shape 最常被沉默掉，不要只看 decline 標籤（shape 只含欄位名，
   沒有 payload 內容，可以直接貼進報告）。
   已知不需動作的桶：`unparseable_summary`（2026-09-01 writer bug 的歷史列，
   args_summary 是字面 `'hook'`）、`no_shape/decline_absent`（v3 前舊列）。
   census 出現 `unknown/<hook>`＝上游動了詞彙、claudecat 的常數跟丟，要回報並同步；
   任一 census 行加總 ≠ fires＝數據品質問題，按步驟 5 先查根因。
   self-heal 採樣（5f6d5267 起，impact/context 回答前自癒 index）讀報告的
   「self-heal 採樣」行即可、不需手工 SQL；零樣本時 `legacy=N` 表示 heal 欄位還沒
   累積（5f6d5267 之前的舊列），與「0 次自癒」是兩回事。
3. 台灣中文輸出（stdout 進 log 檔）：
   - deep/30d、deep/7d 趨勢（2026-09-06 基線 deep30=3、deep7=2；注意 30 天滾動窗口效應）
   - decline 排序與樣本數
3b. **回報**（最後一步，不可省略）：把結論濃縮成 3–8 條 markdown bullet，寫進文件：
   `/home/yanggf/.cargo/bin/cargo run -q -- findings - --file CORT-AUDIT.md`
   （內容走 stdin）。只寫**判讀與該做什麼**，不要貼原始數字表——數字在同檔的長期指標表裡。
   整段取代、只留最新一天，歷史在 git。**log 檔不算回報**：使用者看的是文件，
   2026-09-09 之前這條循環跑完等於沒回報，就是因為終點只有 log。
4. 樣本 ≥10 時：挑最大的可動作標籤，在 `/home/yanggf/a/cortexyoung/rust` 開下一條規則：
   - 規矩：`cargo fmt --all`、clippy 0 warnings、`cargo test --locked --all-targets` 全綠
   - **不自行 commit / push**——把 `git diff` 與說明寫進輸出，使用者會 gate
   - 脈絡：yanggf8/cortexyoung issue #3，驗收線 = hook-suggest 命中率 ≥5%；
     頭號候選 `context_flag`＝把上下文搜尋導向 `cort context` 而非沉默
     （2026-09-01 的 probe 只證明了不該導向 `impact`，沒證明該沉默）。
   - 標籤採樣用 evals 的 hook-probe（自帶 parsed pattern 與全母體 declines，
     **不需**讀 transcript、不需授權）：
     `/home/yanggf/.cargo/bin/cargo run --quiet --manifest-path /home/yanggf/a/cortexyoung/evals/Cargo.toml -- hook-probe --decline <TAG>`
     （`declines` 恆含全母體 census；`--examples N` 只縮 specimen 清單；跑不動先在 evals/ `cargo build`）。
     **別重開 `pattern_not_symbol`**：2026-09-11 上游已裁決（52,046 搜尋 / 4,418 筆：
     80.6% 真文字、0% 偽裝單 symbol 查詢），regex 剝皮規則 `9e725c76` 已收進 hook，
     驗收線進入觀察期——看 kimi 命中率（3.77%）是否上移即可。
   - hook_row v4（`9820d9f5`）起，probe-paid 列（hit/hit_stale/no_index/no_index_hinted/
     no_evidence）帶 `symbol`，`no_evidence` 另帶 `why`（`leaf_in_index`＝搜尋換了名字、
     `absent`＝任何形狀都沒有），且帶 `project_id`（`613b1ec7`）——refusals 現在可對著
     檔案／其他專案追問；v3 舊列沒有這些鍵，採樣先看 `v`。
   樣本 <10：誠實說還要等，不要硬開規則。
5. 數據品質優先（使用者的政策：初期 bug 先修）：CORT-AUDIT.md 出現「無法判讀」/`?`、
   FTS drift>0、fresh 翻 STALE（5f6d5267 起 foreground 查詢會自癒 index，正常應自行
   回落——持續 STALE 就是部署問題；反之 `repair=none` 從此更常見，查詢自己把 index
   養新，不得誤讀為「沒有 staleness 發生過」）、decline 欄整批消失（hooks 可能跑回舊 binary →
   提醒先跑 `cort_upgrade --check` 診斷，再 `cort_upgrade` 修——**必須用樹內 binary**
   `/home/yanggf/a/cortexyoung/rust/target/release/cort_upgrade`：PATH 上 `~/.cargo/bin`
   那份的 `repo_root()` 從 `current_exe()` 往上找樹（找 `src/pack`/`skills`），永遠走到
   `/home` 就 fatal；且 target 可能比樹舊，先 `cargo build --release`（`15487664` 起
   `--check` 自己會把 target 比樹舊判成 Drifted，修法仍是先 build）。它不動才退回
   `cargo install --path /home/yanggf/a/cortexyoung/rust --force`）——先查根因再回報。
5b. **未chunk檔用欄位判形狀，不要再開檔人工猜**（cortexyoung schema v7 起；09-09 那條人工走法
   已被一個欄位取代）：`file_state.chunk_count` 是三態，cort-audit 報告直接給分類——
   - `0`＝extractor 掃過、檔內沒有可 chunk 的宣告，是**正確的沉默**（cortexyoung#2），不必動作。
   - `>0` 卻不在 chunks＝**真缺口**，就是 cortexyoung#5 的形狀（索引停在一個從未提交、
     後來被 git 還原的版本，增量因 `git diff` 為空而永不重看）。修復只能靠**全量**
     `cort index`（5f6d5267 的 query-time self-heal 只回應 staleness、候選集仍是
     git diff——真缺口的漂移不在其中，這條修法不變）；修完複查缺口數並在發現裡
     寫明「哪個檔、修好沒」。
   - `-1`＝v7 之前寫入且從未重寫，**不是掃描結果**；要全量索引一次才有定論，不可當成缺口或無缺口。
   另有 v6 的 `file_state.indexed_uncommitted`：>0 表示有檔案索引自未提交內容（#5 的漂移來源），
   看到就報（工作樹有未提交編輯時是預期現象，commit 後自癒）。報告若說「無法用欄位判讀」＝
   DB 還是舊 schema 或查詢失敗，先按步驟 5 查根因。
   全量重索引若報 `chunk_id_collision`（cortexyoung 34e33a1d 新增的拒絕，兩列同名）＝
   兩條規則把同一個名字描述成同一位置，extractor 拒絕而非挑一個——大聲回報，不要沉默略過。
   chunks/relationships 在 2026-09-11 之後的跳動是 1d54400b/34e33a1d 塌縮修復的預期結果
   （chain/minified 註冊不再擠成同一列），不是數據品質事件。
   **只有真缺口 >0 時**才回報 #5 的影響面（與昨日的差、修復後是否回落、是否有新檔踩進同一形狀）。
6. `git -C /home/yanggf/a/cortexyoung fetch` 後看 `HEAD..origin/master` 有無新 commit。
   （上次追到 `5f6d5267`，2026-09-14；對應關係見 CORT-INTEGRATION.md 同日節。）

## 環境

- cargo：`/home/yanggf/.cargo/bin/cargo`（cron 的 PATH 很瘦）
- usage.db 全程唯讀（`file:...?immutable=1`）
- 兩個 repo 的 CLAUDE.md / AGENTS.md 都要遵守
