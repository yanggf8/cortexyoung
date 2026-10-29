# 資源閘門轉交點：cort 的 `PreToolUse` 把重工交給外部閘門

日期：2026-10-28。分支 `measure`（HEAD `b7d454f4`）。第 5 版：G4 併入 K3.1 Pro 第二輪（block，8 點，見「第 5 版補充」）。第 4 版：G4 併入 K3.1 Pro 審查（block，8 點）。第 3 版：加「第二階段（G4）」（重複）。第 1 版：初稿加自審（process group、Kimi 實測的是 `Grep`、出處、同時開工的窗口、措辭）；第 2 版：併入 K3.1 Pro 審查（代 Muse；approve with changes，7 點：`gate run` 段不再放行整條、deny 訊息不說「排隊」、認引號的 lexer、`cargo +toolchain`／絕對路徑／`command`、claudecat 拿掉 `id -u` 的 fork、`--timeout-ms` 放在 `--` 前、孫程序握 pipe 與 stdin 塞滿的測試）。

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
- Kimi：形狀與 `Bash` 都已實測。舊的 search 建議（`4042a7a6`，於 `6cf17b14` 移除）在 Kimi 上用的就是
  這個形狀，2026-09-03 實機上 deny 有送到模型、模型照做（`docs/2026-09-02-hook-wiring-correction.md`
  §16）；那次擋的是 `Grep` 工具。2026-10-29 G3 實測擋 `Bash` 也成立（見「進度」）。
- Codex：2026-10-29 G3 實測證實會照 deny 做（見「進度」）。在那之前只攔到過它的 payload，從沒看過它執行
  deny，所以設計時這裡寫的是「尚未證實」。
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

### 進度

- 2026-10-28 19:10 台灣時間（11:10Z）G1 進 master：cortexyoung `afca7e2f`（CI 四個 job 全綠；rust 901／evals
  142 測試通過）。K3.1 Pro 程式審查（代 Muse）approve with changes，4 點已併入：`--` 之後的
  `--help`／`--version` 屬於閘門、README／AGENTS.md 四處「從不出聲」改寫、回收檢查改用 `kill -0`（macOS
  沒有 `/proc`）、時間斷言改成 10 秒的 fixture 對 5 秒門檻。
- 同時 G2 進 claudecat `main`：`030cfcd`。除了設計的分類規則，另外處理 heredoc 內容（`<<EOF` 到結束行，
  `<<-` 才去掉開頭 tab）、`#` 註解、開頭的重導向、`2>&1` 裡的 `&`、cargo 的 `-Z`／`--color` 帶值選項，
  deny 原因裡的換行換成空白以保證一行。K3.1 Pro 審查 approve with changes，5 點已併入。
- G3（2026-10-29）：NUC11i5 部署 master `5d3be5ce`（`cort-7a5d501addf2`）與 claudecat `030cfcd`，
  Thinkpade15 同版（`cort-2a217223f835`），兩台 `install.sh --check` OK、`claudecat gate install` 後
  `cort hook-gate status` 是 `configured`（`~/.cargo/bin/claudecat gate hook`，200 ms）。i5940R3070 未部署。
- G3 實測（NUC11i5，08:49 台灣時間）：以 `claudecat gate run --kind build -- sleep 300` 拿住
  `compiler.lock`，三邊各下一次 `cargo check --locked --manifest-path …`，**三邊都沒有執行、都收到同一句
  原因**：Claude Code 顯示 `PreToolUse:Bash hook error: gate denied (compiler-lock) …`；Codex 顯示
  `Command blocked by PreToolUse hook: gate denied (compiler-lock) …`（第一次證實 Codex 照 deny 做）；Kimi
  顯示 `ERROR: Tool execution failed.` 加上同一句原因（Kimi 擋 `Bash` 也成立）。同一段時間輕量指令與
  heredoc 內容照常執行。前一輪以真的 rustc 當佔位時 Claude Code 被擋（`compiler`），Codex 的指令晚了
  一秒、編譯已結束而正確放行，Kimi 則被 kimi plugin 自己的 rescue hook 以「不接受 `&&`」擋下，兩者都不算數。

## 第二階段（G4）：告訴 agent「這是重複的」

使用者 2026-10-29：擋下來不夠，閘門要告訴 agent 它在重複——同一個 agent 一直重試同一個指令，或者
別人已經在跑同一個指令。兩種都做（「好,兩種都做」）。只動 claudecat 的 `gate hook`；cort 端的合約不變，
仍然只看 exit 75 與 stdout 那一行。

### 指令的身分

重複要先定義「同一個指令」。第一版**只做 Cargo**：Gradle 的 `gradle`／`gradlew` 會 exec 成 Java
（`org.gradle.launcher.GradleMain`），還要經過 daemon，從 `/proc` 拿不到可比對的原始指令；Gradle 仍照原本的
資源門檻擋，只是不會回 duplicate。`classify` 找出重工片段之後，G4 讓它一併回傳那一段的**身分**
`(argv, dir, target)`：

- `argv`：`cargo` 之後的字，原樣保留，但拿掉 `+toolchain`（rustup proxy 在 exec 真正的 cargo 之前就把它
  吃掉了，`/proc/<pid>/cmdline` 裡不會有）和片段裡的重導向（`2>&1`、`> log`）。不同 toolchain 在同一個
  target 目錄會搶同一把 build 鎖，所以忽略 toolchain 與「重複」的理由一致。前面的環境變數、`nice`、
  `timeout`、`env`、`exec`、`command` 不進 `argv`。
- `dir`：指令實際作用的目錄，依序取第一個存在的：`--manifest-path X` 的 `X` 所在目錄；`cargo -C X`；
  同一片段 `env -C X`／`env --chdir X`；重工片段之前最後一個 `cd X`；payload 的 `cwd`。相對路徑以前一個
  基準解析，開頭的 `~` 以 `HOME` 展開，再用 `fs::canonicalize` 解析 symlink（只是系統呼叫，不 fork；
  目錄不存在就退回字面正規化：去掉 `.`、`..`、重複的 `/`）。`/proc/<pid>/cwd` 本來就是解析過的路徑，兩邊
  才比得起來。
- `target`：實際的 target 目錄，依序取：`argv` 裡的 `--target-dir X`；片段前綴（或 `env`）裡的
  `CARGO_TARGET_DIR=X`；hook 程序自己的 `CARGO_TARGET_DIR`（hook 繼承 harness 的環境，和 agent 的 shell
  一樣）；都沒有就是 `<dir>/target`。同樣解析成絕對路徑。
- 有多個重工片段時，用第一個 `test` 片段，否則第一個重工片段（和 kind 的選法一致）。

### 情況二：別人已經在跑同一個指令（先判斷）

在看負載之前先找有沒有**正在跑的同一個指令**，找到就擋，不管機器忙不忙——同一個目錄、同一個 target
同時跑兩次 `cargo test`，第二個本來就會卡在 cargo 的 build 目錄鎖上，只是 agent 看不出來在等什麼。
這個順序也讓 agent 拿到比「機器忙」更具體、做得到的指示。

比對對象，與資源判斷共用**同一次** `/proc` 掃描（`snapshot` 要改成先掃程序、再讀負載，不能多掃一次）：

1. `gate run` 的名額持有者：`Holder` 加 `cwd`（`gate run` 寫入時取 `env::current_dir()` 並 canonicalize）與
   `target`（`gate run` 自己的 `CARGO_TARGET_DIR`，沒有就 null）。持有者的 `command` 依上一節的規則算身分；
   舊檔沒有 `cwd` 就不比對、不出錯。
2. 同 uid 正在跑、basename 是 `cargo` 的程序：用 `cmdline` 算 `argv` 與 `--manifest-path`／`-C`／
   `--target-dir`，`dir` 的基準是程序自己的 `cwd`；`target` 沒有在 argv 裡時讀 `/proc/<pid>/environ` 的
   `CARGO_TARGET_DIR`（同 uid 可讀；讀不到就當成預設 `<dir>/target`）。`cargo-clippy`、`rustc`、
   `clippy-driver` 不算（它們是 cargo 的子程序，外層的 `cargo` 程序本身就在）。

`(argv, dir, target)` 三者都相等才算重複。擋的時候一行：

`gate denied (duplicate) the same command is already running: pid=<pid> since=<秒>s dir=<dir> cmd=cargo <argv>. Its output is not visible to you; wait for it to finish, then run this once (the build will be warm). Do not poll with sleep in the foreground.`

`since` 用「幾秒前」，不用時鐘時間，免得時區問題。

### 情況一：同一個 agent 一直重試（每一次 deny 都適用）

每次 `gate hook` 決定擋（包括情況二和原本的資源門檻），讀紀錄計次，再追加一筆：

- 檔案：`$XDG_STATE_HOME/claudecat/gate-denies.jsonl`，沒有 `XDG_STATE_HOME` 就用
  `$HOME/.local/state/claudecat/gate-denies.jsonl`；`CLAUDECAT_GATE_LOG` 可覆寫（測試用）。放在耐久的 state
  目錄，因為它也是量測資料，重開機不該消失。目錄以 `0700` 建立、檔案以 `0600` 建立（`OpenOptions::mode`），
  不靠 umask。
- **不存指令內容。** 一筆一行 JSON：
  `{"ts":<秒>,"session":<session_id 的 SHA-256 前 16 hex 或 null>,"id":<身分的 SHA-256 前 16 hex>,"sub":"<cargo 子命令，如 test>","reason":"<duplicate|compiler|compiler-lock|slots|load|swap 以逗號相接>"}`，
  放行列多一個 `"allowed":true`、沒有 `reason`。沒有 argv、沒有路徑、沒有原始 session id——參數裡可能有
  憑證，路徑是專案名稱；計次只需要能比對，量測只需要子命令與原因。
- 每一列序列化後上限 512 位元組（以上面的欄位一定小於它；超過就不寫）。用 `O_APPEND` 一次 `write_all`
  一行；一般檔案的 `O_APPEND` 不保證多個寫入者不交錯，所以讀的時候**跳過解析失敗的列**。讀、計次、追加
  三步不是原子的：兩個 hook 同時擋同一個 session 的同一個指令時，兩個可能拿到同一個次數。接受——次數是
  建議，不改變擋或放行。
- **只讀檔尾。** 計次與「是否被擋過」只看最後 64 KiB（從檔尾往前讀，丟掉第一個不完整的列），所以不管檔案
  多大，每次 hook 的讀取量都有上限；30 分鐘內的列一定在檔尾。
- 次數：同一個 `session`、同一個 `id`，在最近 30 分鐘內、且最近一筆放行列之後的 deny 筆數，加上這一次。
  `session` 是 null 的不計次數（只記錄）。
- 放行列：hook 決定放行一個 Cargo 重工指令時，若檔尾 64 KiB 裡這個 session 這個 `id` 在 30 分鐘內有 deny
  列，就追加一筆放行列，計次從那之後重新算。沒有 deny 紀錄就不寫。這表示**每一次放行的重工指令都會讀一次
  檔尾**，上限 64 KiB。
- 第 2 次起，訊息前面加 `retry <N> of this command in this session; first denied <S>s ago, still blocked.`；
  第 3 次起再加 `Stop retrying it: do other work or tell the user what is blocking it.`
- **保留與壓縮。** 每次寫入前看檔案大小：超過 256 KiB，或第一列的 `ts` 早於 7 天前（只讀第一列），就改寫成
  「7 天內、而且從最新往回最多 128 KiB」的列（寫暫存檔、`0600`、再 rename）。所以檔案大小上限約 256 KiB
  加一列，保留期限 7 天，和大小無關。和同時追加的 hook 競爭時可能掉幾筆，接受。
- 任何讀寫失敗都當成「沒有紀錄」照常回答：紀錄壞了不能改變擋或放行，只少了計次那段話。

### 預算

情況二不多掃 `/proc`（共用掃描），每個同 uid 的 `cargo` 程序多讀一次 `cmdline` 已有的資料，必要時讀一次
`environ`；情況一每次最多讀 64 KiB 檔尾、寫一列，偶爾改寫 128 KiB。都遠低於 cort 的 200 ms；逾時會讓 cort
放行，連資源門檻都失效，所以 G4 的測試要量 `gate hook` 在大紀錄檔下的耗時。

### 量測

`claudecat gate log [--json] [--days N]`（預設 7）：讀整個紀錄檔，報每個 reason 的 deny 次數、每個子命令的
deny 次數、重試次數分布（同一 session 同一 `id` 連續 deny 數：1、2、3+）、從第一次 deny 到放行的秒數
（有放行列的才算；p50／p95）。這回答原本「被擋之後是否空轉」的問題，usage 列不必加 session id。

### 測試（claudecat）

- 身分：`cd x && cargo test`、`cargo test --manifest-path x/Cargo.toml`、`cargo -C x test`、`env -C x cargo test`、
  `RUST_TEST_THREADS=2 nice -n 15 cargo test 2>&1 | tail` 得到相同 `argv=["test"]` 與 `dir=x`；`cargo +stable test`
  的 `argv` 也是 `["test"]`；`cd` 的相對路徑以 payload `cwd` 為基準、`~` 展開、`..` 正規化；symlink 目錄與它
  的目標得到相同 `dir`（canonicalize）；`CARGO_TARGET_DIR=/tmp/a` 與 `/tmp/b` 的 `target` 不同；`--target-dir`
  優先於環境變數。
- 情況二（fixture `/proc`）：同 uid、同 cwd 的 `cargo test` → duplicate；`env -C /b cargo test` 對上 `/a` 的
  程序 → 不算；cwd 不同、argv 多一個參數、不同 uid、`environ` 的 `CARGO_TARGET_DIR` 不同 → 不算；
  `cmdline` 是 rustup 執行後的 `…/toolchains/stable-…/bin/cargo test` 對上 `cargo +stable test` → duplicate；
  `gate run` 持有者帶 `cwd` 的比對；舊格式持有者不比對也不出錯；`cargo-clippy`／`rustc` 程序不當成比對對象。
  機器閒置時 duplicate 仍然擋；機器也忙時訊息是 duplicate（順序）。
- 情況一：同一 session 連續三次 deny → 第二次有 `retry 2`、第三次有 `Stop retrying`；不同 session 各自計次；
  一次放行之後重新計次；超過 30 分鐘的不算；`session` 為 null 不計次；檔案與目錄權限是 `0600`／`0700`；
  檔案裡沒有指令字串與路徑；紀錄檔不可寫時照常 deny、訊息沒有計次段；夾雜一列壞 JSON 時照常計次。
- 上限：先灌一個 1 MiB 的紀錄檔，`gate hook` 在 deny 與放行兩條路上都在 50 ms 內回答、寫完後檔案在 256 KiB
  附近；第一列超過 7 天時觸發改寫；改寫後只剩 7 天內的列。
- 一行：所有 deny 訊息仍然只有一行。`gate log` 的彙總。

### 分包

- **G4a**：身分（`classify` 回傳身分，含 canonicalize 與 target）、`snapshot` 改成單次掃描、情況二、
  `Holder.cwd`／`target`。
- **G4b**：deny 紀錄（權限、雜湊、檔尾讀、保留）、情況一、`gate log`。
- 部署：兩台重裝 claudecat 即可（cort 不動），再做一次三邊實測：同一 session 連下兩次被擋的指令，
  第二次要看到 `retry 2`；背景跑一個 `cargo test`，另一邊下同一個指令要看到 `duplicate`。

第 4 版：併入 K3.1 Pro 對 G4 的設計審查（代 Muse；block，8 點：`env -C`、`CARGO_TARGET_DIR`、Gradle 改成不做
duplicate、`+toolchain`、canonicalize、紀錄檔真正的上限與只讀檔尾、不存指令內容與權限、`O_APPEND` 的
保證與非原子計次）。

### 第 5 版補充（K3.1 Pro 第二輪，block，8 點；以下覆蓋前文衝突處）

1. **`environ` 讀不到就不比對。** 跑中的 `cargo` 程序 argv 沒有 `--target-dir`、而 `/proc/<pid>/environ`
   讀不到時，這個程序**不當成比對對象**（寧可漏抓，不誤擋），不再假設 `<dir>/target`。
2. **計次讀整個檔，並承認是近似值。** 不再只讀檔尾 64 KiB：檔案在壓縮後最多約 256 KiB，整檔讀取的成本有
   上限。壓縮只保留最新 128 KiB，所以 30 分鐘內的列極多時（一般列約 150 位元組，約 850 列以上）最舊的會被
   丟掉，計次與 `first denied` 因此是近似值；文件與訊息都不宣稱精確。測試要放超過 128 KiB、全部在 30 分鐘內
   的列，確認仍然照常回答。
3. **`target` 是近似值。** 只看 `--target-dir`、`CARGO_TARGET_DIR` 與 `<dir>/target`，不讀 Cargo 設定檔
   （`build.target-dir`）也不解析 workspace 根目錄。後果只會是**漏抓**（兩個其實共用 target 的指令被當成不同），
   不會誤擋——兩個身分相同的指令在同一個目錄用同樣的環境，讀到的設定檔也相同。
4. **`=` 寫法。** `--manifest-path=X`、`--target-dir=X`、`env --chdir=X`、`cargo --config=…` 與分開寫的形式等價，
   兩種都要測。
5. **情況一的範圍。** 計次只對 Cargo 指令。Gradle 被擋時也寫一列，但 `id` 是 null、`sub` 是 `"gradle"`、不計
   次數、訊息沒有計次段；`gate log` 仍把它算進每個 reason 的 deny 次數。
6. **`canonicalize` 的延遲。** 先用字面正規化的 `dir` 與候選者比，相等就不碰檔案系統；不相等時才對新指令的
   `dir` 呼叫一次 `canonicalize`（每次 hook 最多一次）。卡在無回應的網路檔案系統時仍可能拖過 cort 的
   200 ms，cort 會放行——這是已知限制，寫進 README。
7. **保留期限的措辭。** 7 天以上的列在「下一次寫入或下一次 `gate log`」時刪除；沒有任何寫入也沒人看
   `gate log` 時舊列會留著。README 照這句寫，不宣稱定時刪除。
8. **`since` 只對 `gate run` 持有者。** 持有者有 `started_at`；一般 `cargo` 程序第一版不讀啟動時間，訊息裡
   沒有 `since=`，只有 `pid`、`dir`、`cmd`。

### G4 進度

- 2026-10-29 claudecat `main` 推到 `2a39387`：G4a `623a0f0`（身分、單次 `/proc` 掃描、情況二）、G4b
  `9da1b5b`（deny 紀錄、情況一、`gate log`）、審查修正 `2a39387`。我逐行審時另修一點：紀錄的 `sub` 原本取
  身分 argv 第一個字，`cargo --manifest-path=/路徑 test` 會把路徑寫進紀錄，改成跳過選項與其值取子命令。
  K3.1 Pro 程式審查（代 Muse）approve with changes，4 點已併入：身分改取段落實際執行的字（`env`／`nice`／
  `timeout` 之後），`node server.js cargo test` 不再算成 `cargo test`、`env -u cargo cargo test` 認得出來；
  只有 canonical 目錄相同時，目錄底下的 target（含相對 `--target-dir`、`CARGO_TARGET_DIR`）跟著換；放行路徑
  先壓縮再讀；1 MiB 計時測試取三次最快。K3.1 沒回報非 ASCII 切片、append／壓縮競爭、重試計數對規格的部分，
  這幾塊視為未審。測試 106 通過，fmt／clippy 乾淨。
- 部署：NUC11i5 與 Thinkpade15 重裝 claudecat `2a39387`（cort 不動，`hook-gate status` 仍是 `configured`）。
- 三邊實測（NUC11i5，09:50–09:53 台灣時間）：背景跑 `cargo test`（測試睡 600 秒）佔住。Claude Code 下同一指令
  第一次收到 `gate denied (duplicate) the same command is already running: pid=… dir=… cmd=cargo test.`，
  第二次前面多了 `retry 2 of this command in this session; first denied 3s ago, still blocked.`；Codex 與 Kimi
  以 `cargo test --manifest-path …/Cargo.toml`（佔位用同一形式）各下兩次，結果相同（Kimi 是 8s）——三邊的
  payload 都帶 `session_id`。紀錄檔目錄 `0700`、檔案 `0600`，內容只有雜湊、`sub` 與原因，沒有路徑；
  `claudecat gate log` 彙總為 `duplicate: 6`、`test: 6`、重試分布 `2=3`。
