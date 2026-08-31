# `impact --coverage` 的外部覆核紀錄（2026-08-31）

> 為什麼存在：這個功能聲稱能回答「枚舉漏了誰」，而寫它的人（我）剛在同一份資料上連兩次自我欺騙
> （30 格 null 指標、把貼上的報告當需求）。所以這一次把判斷交給**不同模型的 agent**去反證，
> 並把結論與處置落檔，不留在對話裡。

- 指令（同一份，送給兩個引擎）：要它找 **false negative**、要它自己建 `/tmp` 小倉庫驗六種呼叫形狀、
  禁止改 repo。開頭就寫「不要附和，你的工作是找出聲稱在哪裡是假的或過頭的」。
- 送給 Kimi / K3 失敗：`-m k3` 未在 `~/.kimi-code/config.toml`；登入後 `-p` 仍報
  `No model configured`；且 `-p` 與 `-y`／`--auto` 互斥，非互動模式拿不到工具核准。
- 实际跑的：**agy + `gemini-3.1-pro-high`**（已完成，下列發現全部出自它）；**zglmcode + `glm-5.1`**
  （在跑；`zglmcode` 預設的 `glm-5.3` 被端點以 `unrecognized_model` 拒掉，需 `--model` 覆寫）。

## 發現與處置

| # | 覆核結論 | 严重度 | 我的處置 |
|---|---|---|---|
| C2(e) | 有盲檔（chunker 讀不了的檔案）時，per-seed 仍回 `enumeration_may_be_incomplete=false` —— 等於發安全通行证 | **高（真 bug）** | **已修**：盲檔現在會倒向 `incomplete=true`，並新增 `why: [.. blind_files ..]`；`blind_files.unparsed_example` 給路徑不給純計數；回歸測試 `a_blind_file_is_never_a_clean_bill_of_health` |
| C3 | `SKILL.md` 叫 agent「刪除前先跑 `impact`」卻不含 `--coverage`；空 `dependents` 陣列會被当成許可 | **高（引導錯誤）** | **已修**：第 1 條改為 `--coverage` 必帶，並明寫「`--coverage` 是啟發式螢幕、不是完整性證明、看不見 indexer 不讀的檔案（`.sh`/`.txt`/設定檔）」 |
| 殘餘 | 呼叫藏在**非來源檔**（`.sh`、`.txt`、設定檔）時，三層全部看不到 | 中（限制） | 不掩蓋：`reading` 與 skill 都明文寫出這個邊界；擴大到非來源檔需要另案（體積與二分檔） |
| C4 | `extracted_but_unresolved` 不是死重量：它建了 `crate::def::my_func()` 的案例，被 `resolve_targets` 丟掉 | 高（**產品 bug，尚未修**） | 記錄為待修：模組限定呼叫在 Rust 側永不解析。螢幕先讓它可見，修在 `graph.rs` 的解析規則，屬下一個提案 |
| C5 | 成本可忽略（logInfo `--coverage` 約 0.126s）；原因排序確實讓真漏洞壓過雜訊 | — | 採信並更正：我先前在 commit message 把 `--depth 3` 的 4,410→7,428 bytes 與 `--depth 1` 的計數混寫，數字本身對、depth 標錯，此處更正 |
| 措辭 | 「tells you what the enumeration missed」過頭；建議改成「warns you if unmapped mentions or dropped resolutions *suggest* the graph missed a caller」 | — | **採納**，已寫進 skill |

## 覆核同業比對（同一份包、第二個模型家族）

`zglmcode --model glm-5.1` 於 `0e64e56` 之後重跑，並特別要求它驗證 C2(e) 是否真的被修掉、以及
是否仍有殘餘洞。結果待補在本節。

## 我從這件事帶走的規則

外部 agent 一次就抓到兩個我自己沒發現的問題（一個在語意、一個在引導）。規則不是「多問一個模型」，
而是：**凡是「沒有信號」被讀成「安全」的欄位，必須在命名與預設值上都倒向不安確**（這次是 `false`
的語意，上兩次是 `null` 指標與貼上的報告）。
