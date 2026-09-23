//! navigate：從「意圖」到「目的地」的低成本路線指引
//! 輸入一句話（例如 "auth"、"find user creation"），輸出：
//! - 命中的符號（檔案:行號 + kind）
//! - 命中的檔案
//! - 建議的 cort 命令路線（cortexyoung 的精準查詢）
use crate::cort::{CortDependent, CortHit};
use crate::model::ProjectMap;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct NavigateHit {
    pub kind: String,
    pub name: String,
    pub file: String,
    pub line: usize,
    pub exact: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentHit {
    pub path: String,
    pub heading_path: Vec<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NavigateResult {
    pub query: String,
    pub symbols: Vec<NavigateHit>,
    pub files: Vec<String>,
    pub documents: Vec<DocumentHit>,
    pub route: Vec<String>,
}

/// 把查詢拆成 token（小寫、去符號）
fn tokens(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

fn symbol_matches(name: &str, kind: &str, toks: &[String]) -> bool {
    let n = name.to_lowercase();
    let k = kind.to_lowercase();
    toks.iter().any(|t| n.contains(t.as_str())) || toks.iter().any(|t| k.contains(t.as_str()))
}

pub fn navigate(map: &ProjectMap, query: &str) -> NavigateResult {
    let toks = tokens(query);
    let mut symbols: Vec<NavigateHit> = Vec::new();
    let mut files_hit: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut documents = Vec::new();

    for f in &map.key_files {
        let path_low = f.path.to_lowercase();
        let file_match = toks.iter().any(|t| path_low.contains(t.as_str()));
        if file_match {
            files_hit.insert(f.path.clone());
        }
        for sym in &f.symbols {
            if symbol_matches(&sym.name, &sym.kind, &toks) {
                let exact = toks.iter().any(|t| sym.name.to_lowercase() == *t);
                symbols.push(NavigateHit {
                    kind: sym.kind.clone(),
                    name: sym.name.clone(),
                    file: f.path.clone(),
                    line: sym.line,
                    exact,
                });
                files_hit.insert(f.path.clone());
            }
        }
    }

    for heading in &map.document_headings {
        let haystack = format!(
            "{} {} {}",
            heading.path,
            heading.heading_path.join(" "),
            heading.preview
        )
        .to_lowercase();
        if toks.iter().any(|token| haystack.contains(token)) {
            documents.push(DocumentHit {
                path: heading.path.clone(),
                heading_path: heading.heading_path.clone(),
                start_line: heading.start_line,
                end_line: heading.end_line,
                preview: heading.preview.clone(),
            });
        }
    }
    documents.sort_by(|a, b| {
        b.heading_path
            .last()
            .map(|title| toks.iter().any(|t| title.to_lowercase() == *t))
            .cmp(
                &a.heading_path
                    .last()
                    .map(|title| toks.iter().any(|t| title.to_lowercase() == *t)),
            )
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.start_line.cmp(&b.start_line))
    });

    // 排序：精確命中優先，其次種類，再行號
    symbols.sort_by(|a, b| {
        b.exact
            .cmp(&a.exact)
            .then_with(|| b.kind.cmp(&a.kind))
            .then_with(|| a.line.cmp(&b.line))
    });

    let mut route: Vec<String> = Vec::new();
    if symbols.is_empty() && files_hit.is_empty() && documents.is_empty() {
        route.push(format!(
            "在地圖（top-{} 大檔）沒找到「{}」——試 cort 精確查詢：`cort context \"{query}\"` 或 `cort struct -p '{}'`",
            map.key_files.len(),
            query,
            query
        ));
    } else {
        if let Some(first) = symbols.first() {
            route.push(format!(
                "先讀 {}:{}（{} {}）",
                first.file, first.line, first.kind, first.name
            ));
        } else if let Some(first_file) = files_hit.iter().next() {
            route.push(format!("先讀 {}（檔案命中）", first_file));
        }
        if !symbols.is_empty() {
            let top = &symbols[0];
            route.push(format!(
                "接著用 cort 深挖符號：`cort context {} --content full -f lean`",
                top.name
            ));
            route.push(format!(
                "改動前檢查影響：`cort impact --symbol {} --depth 1 -f lean`",
                top.name
            ));
        } else if let Some(f) = files_hit.iter().next() {
            route.push(format!("用 cort 讀檔：`cort read {} -f lean`", f));
        }
        route.push(format!(
            "若仍不中，擴大：`cort struct -p '{}' --lang <lang>`",
            query
        ));
        if !documents.is_empty() {
            route.insert(
                route.len().saturating_sub(1),
                format!(
                    "文件候選：`cort read {} --start {} --end {}`",
                    documents[0].path, documents[0].start_line, documents[0].end_line
                ),
            );
        }
    }

    NavigateResult {
        query: query.to_string(),
        symbols: symbols.into_iter().take(20).collect(),
        files: files_hit.into_iter().take(10).collect(),
        documents: documents.into_iter().take(20).collect(),
        route,
    }
}

pub fn render(r: &NavigateResult) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# claudecat navigate \"{}\" — {} 命中\n\n",
        r.query,
        r.symbols.len() + r.documents.len()
    ));

    if r.symbols.is_empty() && r.files.is_empty() && r.documents.is_empty() {
        s.push_str("未命中。\n\n");
        for step in &r.route {
            s.push_str(&format!("- {step}\n"));
        }
        return s;
    }

    if !r.symbols.is_empty() {
        s.push_str("## 符號\n");
        s.push_str("| 位置 | 種類 | 符號 |\n|---|---|---|\n");
        for h in &r.symbols {
            s.push_str(&format!(
                "| `{}:{}` | {} | {} |\n",
                h.file, h.line, h.kind, h.name
            ));
        }
    }
    if !r.files.is_empty() {
        s.push_str("\n## 檔案\n");
        for f in &r.files {
            s.push_str(&format!("- `{f}`\n"));
        }
    }
    if !r.documents.is_empty() {
        s.push_str("\n## 文件段落\n");
        for d in &r.documents {
            s.push_str(&format!(
                "- `{}`:{}-{} — {}\n",
                d.path,
                d.start_line,
                d.end_line,
                d.heading_path.join(" > ")
            ));
        }
    }
    s.push_str("\n## 路線（低成本到目的地）\n");
    for (i, step) in r.route.iter().enumerate() {
        s.push_str(&format!("{}. {step}\n", i + 1));
    }
    s
}

/// 用 cort 索引強化導航：cort 命中 → 附 content 摘要（省一次 read）+ 反向依賴。
/// `from_fts` 表示命中來自 FTS 全文（symbol_name 未命中、content/file 命中）。
pub fn navigate_with_cort(
    map: &ProjectMap,
    query: &str,
    cort_hits: Vec<CortHit>,
    from_fts: bool,
) -> NavigateResult {
    let mut r = navigate(map, query);
    // 把 cort 命中疊進 symbols（去重：同 file+symbol 只留 cort 的行號，較精確）
    for h in &cort_hits {
        if let Some(sym) = &h.symbol {
            let exists = r.symbols.iter().any(|s| s.file == h.file && s.name == *sym);
            if !exists {
                r.symbols.push(NavigateHit {
                    kind: h.chunk_type.clone(),
                    name: sym.clone(),
                    file: h.file.clone(),
                    line: h.start_line as usize,
                    exact: sym.to_lowercase() == query.to_lowercase(),
                });
            }
        }
    }
    // cort hits 是 push 到已排序清單後面，這裡重整 exact（併入 cort 後可能比 tree-sitter
    // 的子字串命中更精確，例如查 with_readonly 時 tree-sitter 因 token with 命中 render_with_profile）
    let toks = tokens(query);
    for sym in &mut r.symbols {
        sym.exact = sym.name.to_lowercase() == query.to_lowercase()
            || toks.iter().any(|t| sym.name.to_lowercase() == *t);
    }
    r.symbols.sort_by(|a, b| {
        b.exact
            .cmp(&a.exact)
            .then_with(|| b.kind.cmp(&a.kind))
            .then_with(|| a.line.cmp(&b.line))
    });
    // 依賴路線：cort 命中取第一個主要符號，查反向依賴
    if r.symbols.is_empty() {
        let mut route = Vec::new();
        if let Some(document) = r.documents.first() {
            route.push(format!(
                "文件候選：`cort read {} --start {} --end {}`",
                document.path, document.start_line, document.end_line
            ));
        }
        route.push(format!(
            "cort 索引也沒找到「{}」——試 `cort context \"{query}\"` or `cort recall \"{query}\"`",
            query
        ));
        r.route = route;
        return r;
    }
    let primary = r.symbols.first().unwrap();
    let root = std::path::Path::new(&map.root);
    let primary_hit = cort_hits
        .iter()
        .find(|h| h.symbol.as_deref() == Some(primary.name.as_str()) && h.file == primary.file);
    // cort 命中時：第一條路線改為「先讀 cort 命中」，而非 fallback 文案
    let source_label = if from_fts {
        "cort FTS 全文命中"
    } else {
        "cort 全量索引命中"
    };
    let summary = primary_hit
        .and_then(|h| h.content.as_deref())
        .map(|c| crate::cort::content_summary(c, 220))
        .filter(|s| !s.is_empty());
    let mut new_route = vec![format!(
        "先讀 {}:{}（{}：{} {}）",
        primary.file, primary.line, source_label, primary.kind, primary.name
    )];
    if let Some(document) = r.documents.first() {
        new_route.push(format!(
            "文件候選：`cort read {} --start {} --end {}`",
            document.path, document.start_line, document.end_line
        ));
    }
    if let Some(s) = summary {
        new_route.push(format!("內文摘要（省一次 read）：{s}"));
    }
    new_route.push(format!(
        "深挖符號：`cort context {} --content full -f lean`",
        primary.name
    ));
    new_route.push(format!(
        "改動前檢查影響：`cort impact --symbol {} --depth 1 -f lean`",
        primary.name
    ));
    if let Some(deps) = crate::cort::dependents(root, &primary.name) {
        new_route.push(format!(
            "反向依賴（誰在用它，改動前必看）：{}",
            render_dependents(&deps)
        ));
    }
    new_route.push(format!(
        "若仍不中，擴大：`cort struct -p '{}' --lang <lang>`",
        query
    ));
    r.route = new_route;
    r
}

fn render_dependents(deps: &[CortDependent]) -> String {
    let parts: Vec<String> = deps
        .iter()
        .take(8)
        .map(|d| {
            let sym = d.source_symbol.as_deref().unwrap_or("(file-level)");
            format!(
                "{}:{} {} ({})",
                d.source_file, d.source_start_line, sym, d.rel_type
            )
        })
        .collect();
    parts.join("; ")
}
