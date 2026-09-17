//! 技術決策 / Guardrails：開發者維護、claudecat 永不覆寫
//! 來源優先序：`claudecat-guardrails.md` > CLAUDE.md 的 guardrail marker 區塊
use std::path::Path;

pub const GR_BEGIN: &str = "<!-- claudecat:guardrails:begin -->";
pub const GR_END: &str = "<!-- claudecat:guardrails:end -->";

/// 從 marker 區塊抽出每一條決策（去掉 markdown 前綴與註解）
pub fn extract(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(b) = content.find(GR_BEGIN) else {
        return out;
    };
    let rest = &content[b + GR_BEGIN.len()..];
    let Some(e) = rest.find(GR_END) else {
        return out;
    };
    for line in rest[..e].lines() {
        let l = line.trim().trim_start_matches('-').trim();
        if !l.is_empty() && !l.starts_with("<!--") {
            out.push(l.to_string());
        }
    }
    out
}

pub fn load(root: &Path, claude_content: &str) -> Vec<String> {
    let f = root.join("claudecat-guardrails.md");
    if f.is_file() {
        if let Ok(s) = std::fs::read_to_string(&f) {
            let items: Vec<String> = s
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            if !items.is_empty() {
                return items;
            }
        }
    }
    extract(claude_content)
}

/// seed：第一次 update 時附加的佔位區塊（內容可自由編輯）
pub fn seed_block() -> String {
    format!(
        "{GR_BEGIN}\n<!-- 技術決策 / Guardrails：每行一條，例如 `2D tilemap + Macroquad（禁 Python/3D）`、`插件一律裝在 agent harness 內`。claudecat 只在此區不存在時建立，之後永不覆寫。 -->\n{GR_END}\n"
    )
}

pub fn has_marker(content: &str) -> bool {
    content.contains(GR_BEGIN) && content.contains(GR_END)
}
