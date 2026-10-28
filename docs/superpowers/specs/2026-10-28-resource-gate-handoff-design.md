# 資源閘門轉交點：cort 的 `PreToolUse` 把重工交給外部閘門

日期：2026-10-28。分支 `measure`（HEAD `b7d454f4`）。第 1 版：初稿加自審（process group、Kimi 實測的是 `Grep`、出處、同時開工的窗口、措辭）；第 2 版：併入 K3.1 Pro 審查（代 Muse；approve with changes，7 點：`gate run` 段不再放行整條、deny 訊息不說「排隊」、認引號的 lexer、`cargo +toolchain`／絕對路徑／`command`、claudecat 拿掉 `id -u` 的 fork、`--timeout-ms` 放在 `--` 前、孫程序握 pipe 與 stdin 塞滿的測試）。

## 為什麼做

2026-10-24 使用者定的分工：cort 管 memory 和 hook，claudecat 管機器資源（「claudecat 現已無任務，應由
claudecat 來擋」「不想一直加 hook」）。NUC11i5 記憶體吃緊，兩支完整編譯疊在一起時使用者打字會卡；而
cort 的 hook 已經裝在 Claude Code、Codex、Kimi 三邊，是唯一擋得到委派出去的模型的位置。claudecat 那邊
`gate check`／`gate run`／`status` 已經上線（claudecat `115abe5`），缺的是 agent 直接下的指令：那條路
要經 cort 的 `PreToolUse`。

依賴方向只有一個：claudecat → cort。cort 不指名 claudecat；沒有設定閘門時 cort 的行為和今天一模一樣。

這份設計原本只存在 harness 私人記憶、claudecat README 幾行和 `rust/src/main.rs` 一行註解，這裡第一次
寫成 repo 裡的原文，並更新 10/24 以後才知道的事。

## 目標、非目標

- 目標：agent 在三邊 harness 下 `Bash` 指令時，cort 把 payload 交給一個設定好的外部指令；那個指令說
  「擋」時，cort 用 harness 看得懂的 deny 擋下這次呼叫並附上原因；其餘情況一律放行。
- 目標：每次轉交的結果記進這一列 usage，讓「閘門擋了幾次、花多久」可以從 2026-10-28 起算的量測裡讀出來。
- 非目標：cort 不判斷什麼是重工、不讀負載、不碰名額——那些門檻在 claudecat，只有一份。
- 非目標：第一版不改寫指令（見「不做的事」）。
- 反目標：閘門壞掉、慢、不在，絕不能擋人，也不能讓 hook 失去「安靜、exit 0」的承諾。

## 合約

### 設定（cort 擁有格式，claudecat 透過 cort 的指令寫）

- 檔案：`<data_dir>/gate.json`，`data_dir` 是 `rust/src/data_dir.rs` 的 `data_dir()`
  （預設 `~/.local/share/cortexyoung/memory`）。內容：

  ```json
  {"v": 1, "command": ["/home/u/.cargo/bin/claudecat", "gate", "hook"], "timeout_ms": 200}
  ```

  `command` 是 argv 陣列，不經 shell；第一個元素必須是絕對路徑（hook 的 `PATH` 三邊不一致）。
  `timeout_ms` 可省略，預設 200，範圍 50..=1000，超出範圍就當成設定無效。
- 不用環境變數：三邊 harness 給 hook 的環境不同，環境變數設了也不一定到得了 hook。
- 管理指令：`cort hook-gate set [--timeout-ms N] -- <argv…>`（選項一定在 `--` 前面，`--` 之後全是閘門的 argv）、`cort hook-gate clear`、
  `cort hook-gate status`（lean／json，回報設定內容與第一個元素是否存在且可執行）。寫入用
  `data_dir::write_atomic`。claudecat 的安裝流程呼叫 `cort hook-gate set`，不直接寫檔，所以檔案格式
  只有 cort 一個主人。
- 檔案不存在＝沒有閘門（`gate=unconfigured`，不算錯）。檔案存在但讀不了、不是合法 JSON、`v` 不認得、
  `command` 空的或第一個元素不是絕對路徑、`timeout_ms` 超範圍＝`gate=config_invalid`，放行。

### 轉交（cort 端，`cmd_hook_suggest`）

位置：寫好 `fired` 列的 tag 之後、`collect::kick` 之前（今天那行註解的位置）。只有這些條件都成立才轉交：

1. payload 已解析成 JSON，而且這一列是 `fired`；
2. `tool_name == "Bash"`。Codex 把 `exec_command` 正規化成 `Bash`（2026-09-03 攔到的 payload），Kimi
   的 shell 工具本來就叫 `Bash`（`docs/2026-09-02-hook-wiring-correction.md` §15 的工具統計：
   `Bash 358`）；Kimi 的 `Grep` 和其他工具不轉交（`gate=not_shell`）；
3. 有一份有效設定。

轉交方式：

- spawn 設定的 argv，stdin 寫入 hook 收到的**原始 payload 位元組**（不重新序列化），stdout 收集，
  stderr 丟到 null。
- 寫 stdin、讀 stdout 各用一條執行緒；子程序不讀 stdin 時的 `EPIPE` 忽略。主執行緒只等子程序結束或
  逾時，**逾時之後不 join 這兩條執行緒**（孫程序可能還握著 pipe），hook 直接回答、程序結束時一起收掉。
- 等待上限是 `timeout_ms`，從 spawn 開始算。逾時就 `Child::kill` 直接子程序、reap，記 `gate=timeout`，
  放行。cort 沒有 `libc` 相依，第一版不殺整個 process group，所以**閘門合約要求閘門不 fork 會活過
  自己的子程序**；claudecat `gate hook` 要在 G2 拿掉今天唯一的 fork（見閘門端「不 fork」）。
- stdout 只讀前 4096 位元組，多的丟掉；轉成字串用 lossy UTF-8，去掉頭尾空白。

結果對應：

| 子程序結果 | `gate` tag | hook 輸出 |
|---|---|---|
| exit 0 | `allow` | 安靜（`{}`） |
| exit 75 | `deny` | deny（下節） |
| 其他 exit code、被 signal 殺掉 | `error` | 安靜 |
| spawn 失敗 | `spawn_failed` | 安靜 |
| 逾時 | `timeout` | 安靜 |

每一列另外記 `gate_ms`（spawn 到結束或逾時，整數毫秒），只有真的轉交時才有。

`collect::kick` 和 `notes_kick` 照舊執行，不管閘門怎麼回答——它們本來就是背景的。

### deny 的形狀：三邊同一份

```json
{"hookSpecificOutput": {"hookEventName": "PreToolUse",
  "permissionDecision": "deny", "permissionDecisionReason": "<閘門的 stdout>"}}
```

- Claude Code：官方格式。
- Kimi：形狀已實測，工具還沒。舊的 search 建議（`4042a7a6`，於 `6cf17b14` 移除）在 Kimi 上用的就是
  這個形狀，2026-09-03 實機上 deny 有送到模型、模型照做（`docs/2026-09-02-hook-wiring-correction.md`
  §16）；但那次擋的是 `Grep` 工具，擋 `Bash` 要在 G3 實測一次。
- Codex：**尚未證實會照 deny 做。** 以前只攔到過它的 payload，從沒看過它執行 deny。上線前要在 Codex
  實機跑一次被擋的情境，才能說擋得到 Codex；在那之前文件只能寫「Claude Code、Kimi」。
- 不加 `suppressOutput`：Codex 會因為這個欄位丟掉整份輸出（舊程式的註解）。
- stdout 是空字串時，原因換成固定句 `resource gate denied this command (no reason given)`。

### 閘門端（claudecat `gate hook`，在 claudecat repo 實作）

- 從 stdin 讀 payload，取 `tool_input.command`。讀不到、不是字串＝放行（exit 0）。
- **判斷重工在這裡。** 先用一個認得引號的 lexer 把指令切成 token：單引號、雙引號、反斜線跳脫，引號內的
  `;`、`&&` 不是分隔符。引號不成對或遇到認不得的語法＝整條放行（fail-open，記成判斷不了，不是重工）。
  再依 `&&`、`||`、`;`、`|`、`&`、換行把 token 分成片段；每段去掉開頭的環境變數賦值（`A=b`）、
  `nice [-n N]`、`timeout [選項] N`、`env [A=b…]`、`exec`、`command`，然後看第一個 token 的 basename
  （`/usr/bin/cargo`、`~/.cargo/bin/cargo` 都算 `cargo`）：
  - `cargo`，略過 `+toolchain` 和 `-…` 全域選項後的子命令是 `build`／`test`／`clippy`／`check`／
    `bench`／`run`／`install`／`doc`／`nextest` ＝重工（`test`／`bench`／`nextest` 是 `test`，其餘 `build`）；
  - `gradle`、`gradlew`（含 `./gradlew`）＝`build`；
  - 一段本身是 `claudecat gate run …` ＝**只有這一段**不算重工（它自己會排隊），其他片段照樣判斷——
    `claudecat gate run -- cargo build && cargo test` 的第二段仍然要過閘門；
  - `cd …` 這種片段本身不是重工，不影響後面的片段；
  - 其他＝不是重工。
  任何一段是重工，就用那一段的 kind 跑和 `gate check` 同一個判斷（`deny_reason`）；有 `test` 段就用
  `test`。E2E 沒有可靠的指令形狀，第一版不從 hook 判斷；E2E 照舊用 `gate run --kind e2e`。
  已知漏網（接受）：`$(…)`、反引號、`bash -c '…'`、`make`／`just` 包起來的建置、腳本檔裡的 `cargo`。
- 擋的時候 exit 75，stdout 印 `deny_reason` 那一行，再加一句給 agent 的指示：
  `Denied, not queued: nothing will run this command later. Retry it after the holders above finish; do not poll with sleep in the foreground.`
  不建議「改到背景跑」：背景的 `Bash` 呼叫一樣經過這個 hook，一樣會被擋。
- 內部錯誤一律 exit 0（和 `gate check` 一樣 fail-open）。
- **不 fork。** 今天 `gate.rs` 的 `config()` 在 `CLAUDECAT_GATE_DIR`、`XDG_RUNTIME_DIR` 都沒設時會
  spawn `id -u` 取 uid（`gate.rs:120`）；G2 改成讀 `/proc/self` 的 metadata（`MetadataExt::uid`），
  `gate hook` 的整條路徑不產生任何子程序。這是上一節「只殺直接子程序」能成立的前提。
- 自己的時間預算：要在 cort 的 200 ms 內回答。`gate check` 實測一次不到 10 ms（NUC11i5，101 個
  程序），所以不另設計快取。
- 安裝：`claudecat gate install` 呼叫 `cort hook-gate set -- <自己的絕對路徑> gate hook`；
  `claudecat gate uninstall` 呼叫 `cort hook-gate clear`。找不到 `cort` 就報錯退出，不自己寫檔。

## hook 的契約改寫

AGENTS.md 和 README 今天寫「`PreToolUse` 是薄觸發，什麼都不決定，對 agent 什麼都不說」。改成：
**cort 自己的判斷仍然安靜；唯一能出聲的是外部閘門的 deny，而且只在閘門明確回 75 的時候。**
沒有設定閘門時，這句的後半句不會發生，行為和今天一樣。

## 量測

usage 列的 `gate` tag 和 `gate_ms` 隨 `cort measure` 上傳（`command_log` 本來就上傳）。2026-10-28
起算的新量測要能回答：

- 有設定閘門的機器上，`Bash` 呼叫有幾次轉交、`allow`／`deny`／`timeout`／`error` 各多少；
- `gate_ms` 的 p50／p95（確認轉交沒有拖慢每一次 Bash）；
- 被擋之後同一個 session 是否在名額空出前又被擋（agent 是否在空轉）——這一項要 session id，usage
  列目前沒有，第一版先不答，記為待辦。

報表不在這份設計的範圍；先讓列寫得出來。

## 不做的事

- **改寫指令**：Claude Code 的 `updatedInput` 可以把指令換成 `claudecat gate run -- …`，名額才會真的
  被占、也會自動 `nice`。Codex、Kimi 沒有對應的功能，一邊改寫一邊只擋會讓三邊行為不一致。先量 deny
  的效果再說。
- **等待**：hook 不替 agent 排隊等名額；hook 有時間上限，排隊是 `gate run` 的事。
- **cort 內建任何門檻**：負載、swap、名額的數字只在 claudecat。

已知限制：hook 的判斷和 `gate check` 一樣只看「現在」，不占名額。兩個 agent 在同一瞬間各下一支
`cargo build`，兩個都會通過，因為那時還沒有 `rustc` 在跑。要堵這個窗口需要改寫成 `gate run`
（上一點），第一版接受它。

## 失敗與安全

- 任何錯誤路徑都放行、hook exit 0，不 panic；讀設定失敗用回傳值處理（AGENTS.md：storage failures
  are returned, never panicked on）。
- 閘門指令由 cort 直接 spawn，不經 agent 的工具呼叫，所以不會再觸發 hook，沒有遞迴。
- payload 原樣交給閘門：裡面有指令內容與 `cwd`。閘門是使用者自己設定的本機程式，和 hook 本身同一個
  信任等級；cort 不把 payload 送到網路。
- 使用者要暫時關掉：`cort hook-gate clear`。使用者在另一個終端機（不是 agent 的工具呼叫）打的指令
  不經 hook，本來就不受影響。

## 測試

cort 端（不碰真的 `~/.local/share/cortexyoung`、不碰網路，`CORT_MEMORY_DIR` 指到暫存目錄）：

- 新的測試 bin `fake_gate`（和 `fake_ast_grep` 一樣宣告在 `rust/Cargo.toml`、不安裝），依參數決定：
  exit 0、exit 75 加原因、exit 1、睡過逾時、不讀 stdin、印超過 4096 位元組、被 signal 殺掉，以及
  **fork 一個繼承 stdin／stdout、比自己活得久的孫程序後馬上 exit 0**（孫程序握著 pipe 的情境）。
- 每一種對應到上表的 `gate` tag 與輸出；逾時的測試要確認直接子程序被殺掉、hook 在 `timeout_ms`
  加一小段餘裕內回答。
- 「不讀 stdin」配一份**大於 pipe 緩衝（至少 1 MiB）**的 payload，確認寫入端被擋住時 hook 仍然準時回答。
- 孫程序握著 pipe：hook 照子程序的 exit code 回答（`allow`），不等 pipe 關閉，也在上限內回答。
- 設定的第一個元素是不存在的絕對路徑＝`spawn_failed`。
- 沒有設定檔：輸出和今天完全一樣（`{}`），列上 `gate=unconfigured`。
- 非 `Bash` 工具不 spawn。
- 設定無效的每一種形狀＝`config_invalid`。
- `hook-gate set/clear/status` 的往返。
- `tests/install-smoke.sh` 確認 payload 仍然只出貨 `cort` 和 pack（`fake_gate` 不進安裝）。

claudecat 端：指令拆段與 kind 判斷的表格測試，至少包含：

| 指令 | 判斷 |
|---|---|
| `cd x && cargo test` | test |
| `RUST_TEST_THREADS=2 nice -n 15 cargo test` | test |
| `cargo +stable build`、`/usr/bin/cargo build`、`command cargo build` | build |
| `claudecat gate run -- cargo build` | 不是重工 |
| `claudecat gate run -- cargo build && cargo test` | test（第二段） |
| `echo cargo build`、`git commit -m "cargo test"` | 不是重工 |
| `git commit -m "docs; cargo build; why"` | 不是重工（引號內的 `;`） |
| `echo "unterminated` | 判斷不了，放行 |
| `cargo fmt --all` | 不是重工 |

以及 `gate hook` 的 exit code 與 stdout，和「`gate hook` 的路徑上不 spawn 子程序」（`CLAUDECAT_GATE_DIR`、
`XDG_RUNTIME_DIR` 都不設時也一樣）。

## 分包

1. **G1（cortexyoung）**：`gate.json`、`cort hook-gate set/clear/status`、`cmd_hook_suggest` 轉交、
   `fake_gate`、測試、AGENTS.md／README 契約改寫。
2. **G2（claudecat）**：`gate hook`（含認引號的 lexer）、`config()` 拿掉 `id -u`、`gate install/uninstall`、表格測試、README。
3. **G3（部署與實測）**：兩台部署 G1、G2，`claudecat gate install`；在 Claude Code、Kimi、Codex 各做一次
   「有編譯在跑時下 `cargo build`」的實測，Codex 那次決定文件能不能寫「擋得到 Codex」。

G1、G2 可以平行；G3 等兩包都進 master。
