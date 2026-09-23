//! File discovery (gitignore-aware) + LOC / language stats
//! DirStat：只存「直接子檔」的 code 統計；顯示時 rollup 子樹。
use crate::model::{DirStat, MapProfile, ProjectMap};
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// (副檔名, 語言名, is_code)
///
/// 第三欄只決定「算不算 code」（進 LOC / 目錄樹 / key files），**不代表本地有 grammar**：
/// java/ruby/php/csharp/swift/kotlin/shell 都是 true，但 `symbols::lang_for` 沒有它們。
/// 這兩件事本來就該分開——把 java 改成 false 會讓 Java 專案整個從地圖消失，
/// 比「檔案在、符號空」更騙人；空符號改由 outline 明說（見 `symbols::has_grammar`）。
pub const CODE_EXT: &[(&str, &str, bool)] = &[
    ("js", "javascript", true),
    ("jsx", "javascript", true),
    ("mjs", "javascript", true),
    ("cjs", "javascript", true),
    ("ts", "typescript", true),
    ("tsx", "typescript", true),
    ("mts", "typescript", true),
    ("cts", "typescript", true),
    ("py", "python", true),
    ("rs", "rust", true),
    ("go", "go", true),
    ("c", "c", true),
    ("h", "c", true),
    ("cpp", "cpp", true),
    ("cc", "cpp", true),
    ("cxx", "cpp", true),
    ("hpp", "cpp", true),
    ("hh", "cpp", true),
    ("java", "java", true),
    ("rb", "ruby", true),
    ("php", "php", true),
    ("cs", "csharp", true),
    ("swift", "swift", true),
    ("kt", "kotlin", true),
    ("sh", "shell", true),
    ("toml", "config", false),
    ("json", "config", false),
    ("yaml", "config", false),
    ("yml", "config", false),
];

pub fn lang_for_ext(ext: &str) -> Option<(&'static str, bool)> {
    CODE_EXT
        .iter()
        .find(|(e, _, _)| *e == ext)
        .map(|(_, l, code)| (*l, *code))
}

/// Always-excluded directory names（無 gitignore 也排除）。
/// 注意：排除清單會寫進地圖的 excluded_paths，供使用者檢視。
pub const ALWAYS_EXCLUDE: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".cache",
    "coverage",
    ".idea",
    ".vscode",
    "Pods",
    "DerivedData",
    ".terraform",
    ".claudecat",
    "pids",
    "logs",
    // 封存目錄：排除但會顯示在地圖的 excluded 清單（非靜默）
    "legacy",
    "archive",
    "archived",
    "old",
];

/// 收集全部 code/config 檔案，並回報被排除的目錄名。
pub fn collect_files(root: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .ignore(true)
        .follow_links(false);
    let excluded = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let ex = std::sync::Arc::clone(&excluded);
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && ALWAYS_EXCLUDE.contains(&name.as_str())
        {
            ex.lock().unwrap().push(name);
            return false;
        }
        true
    });
    let mut excluded = match std::sync::Arc::try_unwrap(excluded) {
        Ok(m) => m.into_inner().unwrap(),
        Err(_) => Vec::new(),
    };
    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                if lang_for_ext(ext).is_some() {
                    files.push(entry.into_path());
                }
            }
        }
    }
    files.sort();
    excluded.sort();
    excluded.dedup();
    (files, excluded)
}

pub fn collect_markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .ignore(true)
        .follow_links(false);
    let mut files = Vec::new();
    for entry in builder.build().flatten() {
        if entry.path().components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            ALWAYS_EXCLUDE.iter().any(|excluded| *excluded == name)
        }) {
            continue;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && entry.path().extension().and_then(|e| e.to_str()) == Some("md")
        {
            files.push(entry.into_path());
        }
    }
    files.sort();
    files
}

pub fn count_loc(path: &Path) -> Option<(usize, usize)> {
    let data = std::fs::read(path).ok()?;
    if data.len() > 4 * 1024 * 1024 {
        return None;
    }
    let text = String::from_utf8_lossy(&data);
    let total = text.lines().count();
    let non_blank = text.lines().filter(|l| !l.trim().is_empty()).count();
    Some((total, non_blank))
}

/// 取得 `dir` 的直接子檔（不含子目錄）統計。
fn direct_stat(dir_stats: &BTreeMap<String, DirStat>, dir: &str) -> DirStat {
    dir_stats.get(dir).cloned().unwrap_or_default()
}

/// rollup：dir + 所有子目錄的 code 統計總和（用於顯示）。
pub fn rollup(dir_stats: &BTreeMap<String, DirStat>, prefix: &str) -> DirStat {
    let mut out = direct_stat(dir_stats, prefix);
    for (k, s) in dir_stats {
        if k == prefix {
            continue;
        }
        if let Some(rest) = k.strip_prefix(prefix) {
            if let Some(rest) = rest.strip_prefix('/') {
                // 直接子目錄（不含更深層；深層會在遞迴時處理）→ 這裡不遞迴，改為全部算一次
                if !rest.contains('/') {
                    out.files += s.files;
                    out.loc += s.loc;
                }
            }
        }
    }
    out
}

pub fn analyze_project(root: &Path, top_n: usize, map_flag: Option<MapProfile>) -> ProjectMap {
    let mut map = ProjectMap {
        root: root.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let (files, excluded_dirs) = collect_files(root);

    let mut dir_stats: BTreeMap<String, DirStat> = BTreeMap::new();
    let mut languages: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_files = 0usize;
    let mut total_code_loc = 0usize;
    let mut candidates: Vec<crate::model::FileInfo> = Vec::new();

    for f in &files {
        let Some((_, non_blank)) = count_loc(f) else {
            continue;
        };
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Some((lang, is_code)) = lang_for_ext(ext) else {
            continue;
        };
        total_files += 1;
        *languages.entry(lang.to_string()).or_insert(0) += 1;

        let rel = f.strip_prefix(root).unwrap_or(f);
        let rel_str = rel.to_string_lossy().into_owned();
        let dir = rel
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = if dir.is_empty() { ".".to_string() } else { dir };

        if is_code {
            total_code_loc += non_blank;
            // DirStat：只記直接子檔（code）
            let ds = dir_stats.entry(dir.clone()).or_default();
            ds.files += 1;
            ds.loc += non_blank;
            candidates.push(crate::model::FileInfo {
                path: rel_str,
                language: Some(lang.to_string()),
                loc: non_blank,
                symbols: vec![],
            });
        }
    }

    // top-k by loc（全量統計，不截斷）
    candidates.sort_by_key(|f| std::cmp::Reverse(f.loc));
    let top_n_files: Vec<crate::model::FileInfo> = candidates.into_iter().take(top_n).collect();

    map.total_files = total_files;
    map.total_loc = total_code_loc;
    map.languages = languages;
    map.dir_stats = dir_stats;
    map.key_files = top_n_files;
    map.excluded_paths = excluded_dirs;
    map.profile_used = crate::model::resolve_profile(total_code_loc, total_files, map_flag);
    map
}

/// 渲染目錄樹：top-level 目錄（rollup 子樹）+ 直接子目錄摘要。
pub fn tree_lines(
    dir_stats: &BTreeMap<String, DirStat>,
    max_depth: usize,
    budget: usize,
    total_loc: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut roots: Vec<String> = Vec::new();
    for dir in dir_stats.keys() {
        let comps: Vec<&str> = dir.split('/').filter(|c| !c.is_empty()).collect();
        if comps.len() == 1 && dir != "." {
            roots.push(comps[0].to_string());
        }
    }
    roots.sort();
    let mut remaining = budget;
    for root_name in roots {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        let stats = rollup(dir_stats, &root_name);
        let mut line = format!(
            "{root_name}/ ({files} files, {loc} LOC)",
            files = stats.files,
            loc = stats.loc
        );
        if max_depth >= 2 {
            let mut subs: Vec<(String, &DirStat)> = Vec::new();
            for (k, s) in dir_stats {
                if let Some(rest) = k.strip_prefix(&format!("{root_name}/")) {
                    if !rest.contains('/') {
                        subs.push((k.clone(), s));
                    }
                }
            }
            subs.sort_by_key(|(_, s)| std::cmp::Reverse(s.loc));
            if !subs.is_empty() {
                let take = subs.len().min(6);
                let parts: Vec<String> = subs[..take]
                    .iter()
                    .map(|(d, s)| {
                        let name = d.rsplit('/').next().unwrap_or(d).to_string();
                        format!("{name}({})", s.files)
                    })
                    .collect();
                line.push_str(&format!("  [{}]", parts.join(", ")));
            }
        }
        out.push(line);
    }
    if total_loc > 0 {
        out.push(format!(
            "Total: {} files, {} LOC (code)",
            total_loc_files(dir_stats),
            total_loc
        ));
    }
    out
}

fn total_loc_files(dir_stats: &BTreeMap<String, DirStat>) -> usize {
    // 全部 code 檔案數 = 所有直接目錄 files 加總（含 "."）
    let mut n = 0;
    for s in dir_stats.values() {
        n += s.files;
    }
    n
}
