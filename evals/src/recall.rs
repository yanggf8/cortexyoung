//! The recall counterfactual, computed from **source text alone**.
//!
//! This is the tool that decides whether a call shape is worth indexing, and it exists so the
//! decision can be re-run: `docs/2026-08-31-coverage-external-review.md` settled two questions with
//! numbers produced by a throwaway Python script under `/tmp` that is not in the repo and cannot be
//! re-executed by anyone else. The repo's rule is that analysis lives in `evals/` as Rust, so this
//! is that port.
//!
//! Deliberate limits, because a tool that quietly measures less than it claims is how the last two
//! false-safe fields happened:
//!
//! * It reads files. It does **not** open `cort`'s database and does not link `cort` at all, so it
//!   cannot inherit the product's chunking or re-implement its gate. What it reports is the
//!   *input* to that decision -- how many call sites exist and how many project symbols share their
//!   name -- never the gate's output. Compare against `cort status` /
//!   `evals/runs/2026-08-31-schema-v4/` for what was actually attached.
//! * "Call site" and "declared name" are found textually. Strings and comments count, `x.y.z()`
//!   counts once per segment, and a venue that generates bundles counts them as source. That is
//!   over-inclusive on purpose: the decision this informs was "is the population big enough to be
//!   worth a rule", and an under-count would be the error that matters.

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Mirrors `cort::indexer::{IGNORE_DIRS, SOURCE_EXT}` by value rather than by import: the harness
/// stays free of the product, and a drift between the two trees shows up as a `files_scanned`
/// disagreement rather than as silent agreement built on a shared bug.
const IGNORE_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    ".git",
    "__pycache__",
    ".venv",
    "venv",
    "target",
    "coverage",
    ".next",
    ".cache",
    "vendor",
    ".wrangler",
];
const SOURCE_EXT: &[&str] = &[
    ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".py", ".rs", ".java", ".go",
];

/// `pub fn name`, `fn name`, `class Name`, `const NAME`, `interface X`, `type Y`, methods, and the
/// `trait`/`enum`/`mod`/`struct`/`union`/`impl` heads. Used only to ask "how many symbols in this
/// project answer to the name `m`", which is the gate's whole input.
const DECLARATION_KEYWORDS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "union",
    "trait",
    "type",
    "const",
    "let",
    "var",
    "static",
    "class",
    "interface",
    "mod",
    "impl",
    "func",
    "def",
    "namespace",
    "macro_rules!",
];

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !IGNORE_DIRS.contains(&name.as_str()) {
                walk(&path, out);
            }
        } else if SOURCE_EXT.iter().any(|e| name.ends_with(e)) {
            out.push(path);
        }
    }
}

fn word_at(text: &[char], start: usize, end: usize) -> bool {
    let before_ok = start == 0 || !is_ident_char(text[start - 1]);
    let after_ok = end >= text.len() || !is_ident_char(text[end]);
    before_ok && after_ok
}

/// Every name a declaration keyword introduces on this line -- **all of them**, because one-line
/// items are the norm in Rust tests and in generated JS: `impl T { pub fn take(&self) -> u32 { 1 } }`
/// declares two names, and returning only the first made `take` look like it was never declared,
/// which is the wrong direction for a tool whose whole job is counting what a rule could bind to.
pub fn declared_names(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut names = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i].is_alphabetic() {
            let start = i;
            while i < chars.len() && (is_ident_char(chars[i]) || chars[i] == '!') {
                i += 1;
            }
            // Never let the scan stall on a character that starts a word but cannot form one --
            // `#[derive]` used to spin here forever, because `#` triggered the branch and the inner
            // loop advanced zero characters.
            if i == start {
                i += 1;
                continue;
            }
            let word: String = chars[start..i].iter().collect();
            if !DECLARATION_KEYWORDS.contains(&word.as_str()) {
                continue;
            }
            // `pub(crate)`, `async`, `default`, `get`/`set` accessors, decorators: skip modifiers
            // and decorators until a bare identifier that is the thing being declared.
            let mut j = i;
            loop {
                while j < chars.len() && (chars[j].is_whitespace() || chars[j] == '(') {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ')' {
                    // `pub(...)`: past the visibility argument, keep skipping.
                    while j < chars.len() && chars[j] != '(' && chars[j] != ')' {
                        j += 1;
                    }
                    j += 1;
                    continue;
                }
                break;
            }
            let mut k = j;
            while k < chars.len() && (is_ident_char(chars[k]) || chars[k] == '_') {
                k += 1;
            }
            if k > j {
                names.push(chars[j..k].iter().collect());
                i = k;
                continue;
            }
        }
        i += 1;
    }
    names
}

/// One textual call site: the line, the thing before the callee's name, the name, and whether the
/// separator was a Rust path (`::`) or a member access (`.`). The separator belongs in the answer
/// because the two populations mean different things: `fs::write` is a module-path call the
/// resolution rule reasons over, `out.push` is a receiver call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub line: usize,
    pub qualifier: String,
    pub name: String,
    pub path_sep: bool,
}

impl Call {
    /// How the call was written, for display: `fs::write` or `out.push`.
    pub fn target(&self) -> String {
        format!(
            "{}{}{}",
            self.qualifier,
            if self.path_sep { "::" } else { "." },
            self.name
        )
    }
}

/// Every `.name(` receiver call in a text. `qualifier` is the dotted head before the method name --
/// the same evidence `cort`'s edge stores.
pub fn receiver_calls(text: &str) -> Vec<Call> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] != '.' {
                i += 1;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && (is_ident_char(chars[end]) || chars[end] == '_') {
                end += 1;
            }
            if end == start || !word_at(&chars, start, end) {
                i += 1;
                continue;
            }
            // Skip decimal literals (`0.5`) and member access that is not a call.
            let method: String = chars[start..end].iter().collect();
            let after = chars[end..].iter().find(|c| !c.is_whitespace()).copied();
            if after != Some('(') || method.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                i = end;
                continue;
            }
            let mut head: Vec<char> = Vec::new();
            let mut q = start - 1;
            while q > 0
                && (is_ident_char(chars[q - 1]) || chars[q - 1] == '_' || chars[q - 1] == '.')
            {
                q -= 1;
            }
            head.extend(chars[q..start - 1].iter().copied());
            out.push(Call {
                line: index + 1,
                qualifier: head.into_iter().collect::<String>(),
                name: method,
                path_sep: false,
            });
            i = end;
        }
    }
    out
}

/// Every `qualifier::name(` / `qualifier.name(` call where the qualifier is a single segment -- the
/// population the module-path rule reasons over (`path_sep`) and the member-call population
/// alongside it, distinguished by `Call::path_sep` rather than merged into one number.
pub fn qualified_calls(text: &str) -> Vec<Call> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for sep in ["::", "."] {
            let sep_len = sep.chars().count();
            let mut i = 0usize;
            while i + sep_len <= chars.len() {
                let seg: String = chars[i..i + sep_len].iter().collect();
                if seg != sep {
                    i += 1;
                    continue;
                }
                let start = i + sep_len;
                let mut end = start;
                while end < chars.len() && (is_ident_char(chars[end]) || chars[end] == '_') {
                    end += 1;
                }
                if end > start && word_at(&chars, start, end) {
                    let name: String = chars[start..end].iter().collect();
                    let after = chars[end..].iter().find(|c| !c.is_whitespace()).copied();
                    if after == Some('(')
                        && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
                    {
                        let mut q = i;
                        while q > 0 && (is_ident_char(chars[q - 1]) || chars[q - 1] == '_') {
                            q -= 1;
                        }
                        let qualifier: String = chars[q..i].iter().collect();
                        if !qualifier.is_empty() && !name.is_empty() {
                            out.push(Call {
                                line: index + 1,
                                qualifier,
                                name,
                                path_sep: sep == "::",
                            });
                        }
                    }
                }
                i += 1;
            }
        }
    }
    out
}

/// What a `use` line introduces into a file's scope, as `(root, name)`: `use std::fs;` is
/// `("std", "fs")`, `use crate::db::{open_db, set_meta};` is `("crate", "db")`. Brace items other
/// than the head are ignored, which is the conservative direction -- an unparsed alias can only
/// *reduce* the risk population below, and a measure that over-counts risk gets ignored too.
pub fn use_introductions(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let head = line.trim();
        let rest = match head
            .strip_prefix("use ")
            .or_else(|| head.strip_prefix("use\t"))
        {
            Some(rest) => rest,
            None => continue,
        };
        let path = rest
            .split('{')
            .next()
            .unwrap_or("")
            .trim_end_matches(';')
            .trim();
        let segments: Vec<&str> = path
            .split("::")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let (Some(root), Some(last)) = (segments.first(), segments.last()) else {
            continue;
        };
        if root.is_empty() || last.is_empty() || root == last && segments.len() == 1 {
            continue;
        }
        out.push((root.to_string(), last.to_string()));
    }
    out
}

/// Every `[package] name` in a `Cargo.toml` at the venue root or one directory below it.
fn own_crate_names(root: &Path) -> HashSet<String> {
    let mut manifests = vec![root.join("Cargo.toml")];
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                manifests.push(path.join("Cargo.toml"));
            }
        }
    }
    let mut names = HashSet::new();
    for manifest in manifests {
        let Ok(toml) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let mut in_package = false;
        for line in toml.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_package = line == "[package]";
                continue;
            }
            if !in_package {
                continue;
            }
            if let Some(value) = line
                .strip_prefix("name")
                .and_then(|rest| rest.split('=').nth(1))
            {
                let name = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !name.is_empty() {
                    names.insert(name.replace('-', "_"));
                }
            }
        }
    }
    names
}

/// One file's worth of measurements, folded into the venue totals.
#[derive(Debug, Default)]
pub struct Venue {
    /// name -> how many project declarations introduce it
    pub declarations: HashMap<String, usize>,
    pub receiver_sites: usize,
    pub qualified_sites: usize,
    pub files: usize,
}

impl Venue {
    /// The bucket a method name falls into: no project symbol, exactly one, or several.
    pub fn bucket(&self, name: &str) -> &'static str {
        match self.declarations.get(name).copied().unwrap_or(0) {
            0 => "no_project_symbol",
            1 => "unique",
            _ => "ambiguous",
        }
    }
}

/// Read the venue once and collect the declaration population. Returns `Err` with the offending path
/// rather than skipping a file it could not read -- a partial count is how a null metric happens.
pub fn scan_venue(root: &Path) -> Result<Venue, String> {
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    let mut venue = Venue::default();
    for path in files {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        venue.files += 1;
        for line in text.lines() {
            for name in declared_names(line) {
                *venue.declarations.entry(name).or_insert(0) += 1;
            }
        }
        venue.receiver_sites += receiver_calls(&text).len();
        venue.qualified_sites += qualified_calls(&text).len();
    }
    Ok(venue)
}

/// The three-bucket split the gate decision was argued from: no project symbol by that name,
/// exactly one, or several.
fn tally<'a>(
    venue: &Venue,
    calls: impl Iterator<Item = &'a Call>,
) -> BTreeMap<&'static str, usize> {
    let mut out: BTreeMap<&'static str, usize> = BTreeMap::new();
    for call in calls {
        *out.entry(venue.bucket(&call.name)).or_insert(0) += 1;
    }
    out
}

/// The name -> count list, highest first, capped: the "top three are `String::new`, `PathBuf::from`,
/// `fs::write`" paragraph in the review doc is what this is for.
fn top(counts: HashMap<String, usize>, limit: usize) -> Vec<Value> {
    let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows.into_iter()
        .take(limit)
        .map(|(name, n)| json!({ "target": name, "sites": n }))
        .collect()
}

/// The report. `top` caps the frequency lists; every count is reported in full regardless.
pub fn report(root: &Path, top_n: usize) -> Result<Value, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    // `scan_venue` walks the same tree; the list is needed again here for per-file attribution.
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    let venue = scan_venue(&root)?;

    let mut receiver_calls_all: Vec<Call> = Vec::new();
    let mut qualified_calls_all: Vec<(String, Call)> = Vec::new();
    let mut external_names: HashMap<String, HashSet<String>> = HashMap::new();
    let mut zero_names: HashMap<String, usize> = HashMap::new();
    let mut collision = 0usize;
    let mut collision_examples: Vec<Value> = Vec::new();
    let mut into_local_module = 0usize;
    let mut local_modules: HashSet<String> = HashSet::new();
    // The venue's own crate names, so `use cort::usage::args_summary` in `rust/src/main.rs` is not
    // mistaken for a dependency path. Cargo.toml one level down counts too: a multi-crate repo has no
    // manifest at its root, and reading only the root's reported 19 "exposures" here -- every one of
    // them an internal path call, which is exactly the kind of wolf this metric should not cry at.
    let own_crate = own_crate_names(&root);
    let mut shadowed_by: HashMap<String, usize> = HashMap::new();
    for path in &files {
        let text = std::fs::read_to_string(path).map_err(|e| {
            let rel = path.strip_prefix(&root).unwrap_or(path.as_path());
            format!("{}: {e}", rel.display())
        })?;
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        // A file is a module, and so is every directory above it: `src/fs.rs`, `src/fs/mod.rs` and
        // `src/fs/util.rs` all make `fs` a legal local qualifier.
        let segments: Vec<&str> = rel.split('/').collect();
        if let Some(stem) = segments.last().and_then(|s| s.split('.').next()) {
            local_modules.insert(stem.to_string());
        }
        for dir in &segments[..segments.len().saturating_sub(1)] {
            if *dir != "src" {
                local_modules.insert(dir.to_string());
            }
        }
        let external: HashSet<String> = use_introductions(&text)
            .into_iter()
            // Rust turns `-` into `_` in a use path, so a crate named `cort-evals` is imported as
            // `cort_evals`. Comparing the raw strings called every internal `cort_evals::arms::x()`
            // a dependency shadow -- three false exposures in this repo's own report.
            .filter(|(head, _)| {
                head != "crate"
                    && head != "self"
                    && head != "super"
                    && !own_crate.contains(&head.replace('-', "_"))
                    && !own_crate.contains(head)
            })
            .map(|(_, name)| name)
            .collect();
        if !external.is_empty() {
            external_names.insert(rel.clone(), external);
        }
        receiver_calls_all.extend(receiver_calls(&text));
        qualified_calls_all.extend(
            qualified_calls(&text)
                .into_iter()
                .map(|call| (rel.clone(), call)),
        );
    }
    for (file, call) in &qualified_calls_all {
        // Only `::` paths are in scope here; `out.push` is a receiver question, counted above.
        if !call.path_sep {
            continue;
        }
        let local = local_modules.contains(&call.qualifier);
        if local {
            into_local_module += 1;
        }
        match venue.bucket(&call.name) {
            "no_project_symbol" => {
                *zero_names.entry(call.target()).or_insert(0) += 1;
            }
            _ => {
                // The shadowing risk needs both halves to be true: the qualifier names a module this
                // project *also* has, and this file pulled that name in from a dependency
                // (`use std::fs;`). With only the first condition every legitimate internal path call
                // (`usage::now_ms`) counted as an exposure -- 44 here where the real number is 0, and
                // a metric that cries wolf on the good cases is how the next real one gets ignored.
                let shadowed = local
                    && external_names
                        .get(file)
                        .is_some_and(|names| names.contains(&call.qualifier));
                if shadowed {
                    collision += 1;
                    *shadowed_by.entry(call.target()).or_insert(0) += 1;
                    if collision_examples.len() < top_n {
                        collision_examples.push(json!({
                            "target": call.target(),
                            "file": file,
                            "line": call.line,
                        }));
                    }
                }
            }
        }
    }

    Ok(json!({
        "venue": root.to_string_lossy(),
        "method": "recall-exp-v1 (text-side counterfactual: no cort database, no gate re-implementation)",
        "files_scanned": venue.files,
        "declared_symbols": venue.declarations.len(),
        "receiver_calls": {
            "sites": venue.receiver_sites,
            "by_candidate_count": tally(&venue, receiver_calls_all.iter()),
        },
        "qualified_calls": {
            "sites": venue.qualified_sites,
            "by_candidate_count": tally(&venue, qualified_calls_all.iter().map(|(_, call)| call)),
            "no_local_symbol_top": top(zero_names, top_n),
        },
        "module_path_calls_into_a_local_module": into_local_module,
        "dependency_shadowed_by_local_module_sites": collision,
        "dependency_shadowed_top": top(shadowed_by, top_n),
        "shadowed_examples": collision_examples,
        "reading": "This is the population a rule would have to work on, not a verdict on any rule. \
                    'unique' is an upper bound on what a uniqueness gate can attach: it counts names \
                    with exactly one project declaration, and says nothing about whether the receiver \
                    can bind to it (that is `cort`'s `receiver_binds`, which this tool deliberately \
                    does not re-implement). `module_path_calls_into_a_local_module` is the population the \
                    Rust module-suffix rule can reach; `dependency_shadowed_by_local_module_sites` is the \
                    subset that a `use std::x;`-style import makes genuinely ambiguous with a local module \
                    of the same name, which is the hole README limitation #8 describes. Strings and \
                    comments are counted; generated trees are excluded by directory name only. Nothing \
                    here opens a cort index, so 'attached' numbers must come from `cort status` or a \
                    verify-impact run.",
    }))
}
