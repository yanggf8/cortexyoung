//! doctor：體檢本機的追蹤循環（cron / 數據源 / host / cort 索引），並可一鍵部署。
//! 數據（usage.db）天生是每台機器各自的，所以「部署」= 每台機器跑一次
//! `claudecat doctor --install`；各機的列靠 host 欄位在 CORT-AUDIT.md 共存。
use std::path::Path;

/// 每日追蹤的 cron 條目（09:17，避開整點；cargo 用絕對路徑——cron 的 PATH 很瘦）
pub fn track_cron_line(manifest_dir: &str, root: &Path) -> String {
    format!(
        "17 9 * * * cd {manifest_dir} && $HOME/.cargo/bin/cargo run -q --manifest-path \
         {manifest_dir}/Cargo.toml -- cort-audit --root {} --track CORT-AUDIT.md >> \
         {manifest_dir}/cort-audit.log 2>&1",
        root.display()
    )
}

/// 每日分析的 cron 條目（09:29；headless agent，prompt 檔在 repo 裡可審查、可版本化）。
///
/// 兩層 PATH 問題，只解決一層是不夠的：執行檔本身用安裝時解析的絕對路徑（cron 的 PATH 很瘦），
/// 但**它 exec 的下一層仍然靠 PATH**——實測 2026-09-08 09:29 這條每天都失敗在
/// `cc_claude: cannot exec claude: No such file or directory`：`musecode` 轉給 wrapper 去
/// exec `claude`，而 cron 的 PATH 沒有 `~/.local/bin`（互動 shell 有，所以手動跑永遠是綠的）。
/// 因此把執行檔所在目錄放進 PATH——這對任何「wrapper 再 exec 同目錄工具」的組合都成立。
pub fn analysis_cron_line(bin: &str, manifest_dir: &str, extra_dirs: &[String]) -> String {
    let mut dirs: Vec<String> = Vec::new();
    let mut push = |d: String| {
        if !d.is_empty() && !dirs.contains(&d) {
            dirs.push(d);
        }
    };
    if let Some(d) = Path::new(bin).parent() {
        push(d.display().to_string());
    }
    for d in extra_dirs {
        push(d.clone());
    }
    let path_prefix = if dirs.is_empty() {
        String::new()
    } else {
        format!("PATH=\"{}:$PATH\" ", dirs.join(":"))
    };
    format!(
        "29 9 * * * {path_prefix}{bin} -p --dangerously-skip-permissions \
         \"$(cat {manifest_dir}/cort-audit-analysis-prompt.md)\" >> \
         {manifest_dir}/cort-audit-analysis.log 2>&1"
    )
}

/// 條目 PATH 還要帶哪些目錄：harness 的 hook 會用名字 exec `node`
/// （實測 cron 下 `SessionEnd hook ... node: not found`），而 node 常在 nvm 目錄裡。
/// 只收「真的解析得到」的目錄——猜一個不存在的路徑只會讓條目更難讀。
pub fn runtime_dirs() -> Vec<String> {
    ["node"]
        .iter()
        .filter_map(|n| resolve_bin(n))
        .filter_map(|p| Path::new(&p).parent().map(|d| d.display().to_string()))
        .collect()
}

/// 判斷既有 crontab 是否已含追蹤條目（寬鬆比對：cort-audit + --track）
pub fn has_track_entry(crontab: &str) -> bool {
    crontab
        .lines()
        .any(|l| l.contains("cort-audit") && l.contains("--track"))
}

/// 判斷既有 crontab 是否已含分析條目
pub fn has_analysis_entry(crontab: &str) -> bool {
    crontab
        .lines()
        .any(|l| l.contains("cort-audit-analysis-prompt.md"))
}

/// 分析條目用哪顆執行檔。`CLAUDECAT_ANALYSIS_BIN` 指名（musecode/claude/絕對路徑）；
/// 未指定 = auto：musecode 優先（額度與 claude 訂閱獨立，claude 限額週不會拖垮循環），
/// 退 claude。額度狀況改變時調頭＝換 env 值重跑一次 `--install`，條目會被重寫。
pub fn analysis_bin() -> Result<String, String> {
    let prefer = std::env::var("CLAUDECAT_ANALYSIS_BIN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    let mut order: Vec<String> = Vec::new();
    match &prefer {
        Some(name) => order.push(name.clone()),
        None => {
            order.push("musecode".to_string());
            order.push("claude".to_string());
        }
    }
    for name in &order {
        if let Some(p) = resolve_bin(name) {
            return Ok(p);
        }
    }
    Err(match prefer {
        Some(n) => format!("找不到指定的分析執行檔：{n}"),
        None => "找不到 musecode 也找不到 claude".to_string(),
    })
}

fn resolve_bin(name: &str) -> Option<String> {
    if let Ok(o) = std::process::Command::new("which").arg(name).output() {
        if o.status.success() {
            let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    let p = format!("{}/.local/bin/{name}", std::env::var("HOME").ok()?);
    Path::new(&p).is_file().then_some(p)
}

/// 併入分析條目：既有分析條目**與期望的整行不同就重寫**；完全相同才不動。
/// 回傳 (新內容, 是否有變更)。
///
/// 原本只比對「執行檔字串在不在」，於是模板本身的修正永遠部署不出去：2026-09-09 加上
/// `PATH=` 前綴修 `cannot exec claude` 時，舊條目因為仍含著同一個 musecode 路徑而被判成
/// 「已是最新」，`--install` 會安靜地什麼都不做——一個宣稱幂等、實際是「永不升級」的比對。
/// 整行比對同時涵蓋原本的用途（換執行檔＝換行內容）。
pub fn merge_analysis_entry(existing: &str, line: &str) -> (String, bool) {
    let is_analysis = |l: &str| l.contains("cort-audit-analysis-prompt.md");
    let stale = |l: &str| is_analysis(l) && l.trim() != line.trim();
    let kept: Vec<&str> = existing.lines().filter(|l| !stale(l)).collect();
    let up_to_date = kept.iter().any(|l| is_analysis(l));
    if up_to_date {
        let out = format!("{}\n", kept.join("\n"));
        let changed = existing != out;
        return (out, changed);
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    (out, true)
}

/// 把一條 cron 條目併進既有 crontab（幂等：`present` 判定已存在 → 原樣返回）
pub fn merge_entry(existing: &str, line: &str, present: fn(&str) -> bool) -> String {
    if present(existing) {
        return existing.to_string();
    }
    let mut out = existing.trim_end().to_string();
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    out
}

/// 讀目前使用者的 crontab（無 crontab 或 crontab 不存在 → 空字串）
pub fn read_crontab() -> String {
    std::process::Command::new("crontab")
        .arg("-l")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// 寫回 crontab（完整內容走 stdin 的 `crontab -`）
pub fn write_crontab(content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("crontab")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_ref()
        .expect("stdin must be piped")
        .write_all(content.as_bytes())?;
    if child.wait()?.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("crontab - exited non-zero"))
    }
}

/// host 名：`/etc/hostname`（Linux 慣例）優先；macOS 沒有這個檔，退回 `hostname` 指令。
/// 多機的審計列靠它區分，兩條路都拿不到才回 None——一條永遠 ✗ 的檢查在 Mac 上
/// 等於把這台機器永遠記成 unknown。
pub fn resolve_host() -> Option<String> {
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim();
        if !h.is_empty() {
            return Some(h.to_string());
        }
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
}

/// 體檢報告：循環賴以運作的每一環，逐項 ✓/✗，✗ 帶下一步
pub fn report(root: &Path) -> String {
    let mut s = String::from("# claudecat doctor\n\n");
    let host = resolve_host();
    s.push_str(&format!(
        "- [{}] host 可讀（{}）——多機的列靠它區分\n",
        tick(host.is_some()),
        host.as_deref().unwrap_or("unknown")
    ));

    let cache = crate::cort::cache_dir();
    s.push_str(&format!(
        "- [{}] cort cache dir：{}\n",
        tick(cache.is_dir()),
        cache.display()
    ));
    let usage = cache.join("usage.db");
    s.push_str(&format!(
        "- [{}] usage.db（用量數據源）{}\n",
        tick(usage.is_file()),
        if usage.is_file() {
            String::new()
        } else {
            "— 裝了 cort 並用過之後就會有".to_string()
        }
    ));

    let pid_db = std::fs::canonicalize(root)
        .ok()
        .and_then(|r| r.to_str().map(crate::cort::db_path_for));
    let indexed = pid_db.as_ref().is_some_and(|p| p.is_file());
    s.push_str(&format!(
        "- [{}] 本專案 cort 索引（{}）{}\n",
        tick(indexed),
        root.display(),
        if indexed {
            match crate::cort::index_info(root) {
                Some(info) if info.fresh => "— fresh".to_string(),
                Some(_) => "— STALE（執行 `cort index`）".to_string(),
                None => "— 存在但無法讀取".to_string(),
            }
        } else {
            "— 執行 `cort index` 後可用 navigate --cort".to_string()
        }
    ));

    let track_file = root.join("CORT-AUDIT.md");
    s.push_str(&format!(
        "- [{}] 長期指標檔：{}\n",
        tick(track_file.is_file()),
        track_file.display()
    ));

    let crontab = read_crontab();
    let installed = has_track_entry(&crontab);
    s.push_str(&format!(
        "- [{}] 每日追蹤 crontab {}\n",
        tick(installed),
        if installed {
            String::new()
        } else {
            "— 跑 `claudecat doctor --install` 一鍵安裝".to_string()
        }
    ));
    match analysis_bin() {
        Ok(bin) => {
            let up_to_date = has_analysis_entry(&crontab)
                && crontab
                    .lines()
                    .any(|l| l.contains("cort-audit-analysis-prompt.md") && l.contains(&bin));
            let runner = Path::new(&bin)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| bin.clone());
            s.push_str(&format!(
                "- [{}] 每日分析 crontab（headless {} -p）{}\n",
                tick(up_to_date),
                runner,
                if up_to_date {
                    String::new()
                } else if has_analysis_entry(&crontab) {
                    "— 條目用的不是目前的執行檔，跑 `claudecat doctor --install` 切換".to_string()
                } else {
                    "— 跑 `claudecat doctor --install` 一鍵安裝".to_string()
                }
            ));
        }
        Err(e) => s.push_str(&format!(
            "- [✗] 每日分析 crontab（{e}）——可用 CLAUDECAT_ANALYSIS_BIN 指定執行檔\n"
        )),
    }
    s
}

fn tick(ok: bool) -> char {
    if ok {
        '✓'
    } else {
        '✗'
    }
}
