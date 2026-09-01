//! Parity probe: does the `ast-grep` *crate* (ast-grep-core/config/language, in-process) produce
//! the same matches as the `ast-grep` *CLI* (`scan --json=stream`) on the same rule pack and the
//! same files?
//!
//! This is a decision probe, not a shipped contract: its numbers decide whether `cort` can wire the
//! pack in-process (removing the npm-installed CLI, the PATH probe, the per-file 30s timeout and the
//! installer's binary download) or must keep the CLI. Run it explicitly while the decision is open:
//!
//! ```sh
//! cargo test --test parity_probe -- --nocapture
//! ```
//!
//! The crates stay dev-dependencies because they add eight tree-sitter grammars and ~2 minutes of
//! build; promoting them to real dependencies is exactly what this probe decides. The traversal
//! mirrors the CLI's scan defaults (every node, reentrant, named and anonymous alike): `Root::dfs`
//! plus one `match_node` per node against each rule's `RuleCore`.

use ast_grep_config::{from_yaml_string, GlobalRules, RuleConfig};
use ast_grep_core::{matcher::MatcherExt, tree_sitter::LanguageExt, tree_sitter::StrDoc, AstGrep};
use ast_grep_language::{JavaScript, Python, Rust, Tsx, TypeScript};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

type Matches = BTreeMap<(String, u64, u64), String>; // (ruleId, start byte, end byte) -> text

const PACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/pack");
const CCT: &str = "/home/yanggf/a/cct";

/// Run the CLI over one file and fold its stream into the canonical match set.
fn cli_matches(bin: &str, config: &Path, path: &Path) -> Matches {
    let out = Command::new(bin)
        .args([
            "scan",
            "--json=stream",
            "--config",
            config.to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()
        .expect("ast-grep CLI runs");
    assert!(
        out.status.success(),
        "CLI scan failed on {path:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut set = Matches::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).expect("cli stream is json");
        let start = v["range"]["byteOffset"]["start"].as_u64().unwrap();
        let end = v["range"]["byteOffset"]["end"].as_u64().unwrap();
        let rule = v["ruleId"].as_str().unwrap().to_string();
        set.insert((rule, start, end), v["text"].as_str().unwrap().to_string());
    }
    set
}

/// One rule bound to one parsed file: run the matcher over every node, exactly once per node.
fn rule_matches<L>(
    matcher: &ast_grep_config::RuleCore,
    text: &str,
    lang: L,
    id: &str,
    out: &mut Matches,
) where
    L: LanguageExt + Copy,
{
    let root: AstGrep<StrDoc<L>> = AstGrep::doc(StrDoc::new(text, lang));
    for node in root.root().dfs() {
        if let Some(m) = matcher.match_node(node) {
            let r = m.get_node().range();
            out.insert(
                (id.to_string(), r.start as u64, r.end as u64),
                m.text().to_string(),
            );
        }
    }
}

/// The whole (language, rule-file) pair through the crate, over every file given.
fn crate_matches<L>(yaml: &str, lang: L, files: &[&Path]) -> (BTreeMap<PathBuf, Matches>, usize)
where
    L: LanguageExt + serde::de::DeserializeOwned + Copy,
{
    let rules: Vec<RuleConfig<L>> =
        from_yaml_string(yaml, &GlobalRules::default()).expect("pack parses as rules");
    let globals = GlobalRules::default();
    let matchers: Vec<(_, ast_grep_config::RuleCore)> = rules
        .iter()
        .map(|r| {
            (
                r.id.clone(),
                r.get_matcher(&globals).expect("rule compiles to a matcher"),
            )
        })
        .collect();
    let mut per_file = BTreeMap::new();
    for path in files {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut set = Matches::new();
        for (id, m) in &matchers {
            rule_matches(m, &text, lang, id, &mut set);
        }
        per_file.insert(path.to_path_buf(), set);
    }
    (per_file, matchers.len())
}

fn diff(a: &Matches, b: &Matches) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in a {
        match b.get(k) {
            None => out.push(format!("  CLI-only: {k:?} {:?}", &v[..v.len().min(60)])),
            Some(t) if t != v => out.push(format!(
                "  text differs: {k:?} {:?} vs {:?}",
                &v[..v.len().min(40)],
                &t[..t.len().min(40)]
            )),
            _ => {}
        }
    }
    for (k, v) in b {
        if !a.contains_key(k) {
            out.push(format!("  crate-only: {k:?} {:?}", &v[..v.len().min(60)]));
        }
    }
    out
}

fn find_files(root: &Path, exts: &[&str], limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || matches!(name, "target" | "node_modules" | "dist") {
                    continue;
                }
                stack.push(p);
            } else if out.len() < limit
                && exts.iter().any(|x| p.to_str().unwrap_or("").ends_with(x))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// One probe leg: for every file, crate vs CLI, and the report of any divergence.
fn leg<L>(yaml: &str, lang: L, label: &str, files: &[&Path]) -> usize
where
    L: LanguageExt + serde::de::DeserializeOwned + Copy,
{
    if files.is_empty() {
        eprintln!("SKIP {label}: no files");
        return 0;
    }
    // Repo convention: a machine without the CLI prints SKIP rather than failing the suite.
    let bin = match cort::ast_grep::resolve_ast_grep_bin() {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP {label}: ast-grep CLI not present");
            return 0;
        }
    };
    let config = Path::new(PACK).join("sgconfig.yml");
    let t0 = Instant::now();
    let (crate_by_file, rule_count) = crate_matches::<L>(yaml, lang, files);
    let crate_us = t0.elapsed().as_millis();
    let mut compared = 0usize;
    for path in files {
        let cli = cli_matches(&bin, &config, path);
        let mine = &crate_by_file[*path];
        compared += cli.len().max(mine.len());
        let d = diff(&cli, mine);
        if !d.is_empty() {
            println!(
                "DIVERGE {label} {} ({} CLI vs {} crate):",
                path.display(),
                cli.len(),
                mine.len()
            );
            for line in d.iter().take(10) {
                println!("{line}");
            }
        }
    }
    println!(
        "leg {label}: {} files, {rule_count} rules, {} comparisons, crate {} ms, CLI-driven",
        files.len(),
        compared,
        crate_us
    );
    compared
}

#[test]
fn the_crate_matches_the_cli_on_the_real_pack_and_real_files() {
    let pack = Path::new(PACK).join("rules");
    let read = |name: &str| std::fs::read_to_string(pack.join(name)).expect(name);

    // Rust: the pack's most used rules over this repo's own, real, large files.
    let rust_files = vec![
        PathBuf::from("src/coverage.rs"),
        PathBuf::from("src/graph.rs"),
        PathBuf::from("src/chunker.rs"),
    ];
    let rust_files: Vec<PathBuf> = rust_files
        .into_iter()
        .map(|p| Path::new(env!("CARGO_MANIFEST_DIR")).join(p))
        .collect();
    let n1 = leg::<Rust>(
        &read("rust.yml"),
        Rust,
        "rust",
        &rust_files.iter().map(|p| p.as_path()).collect::<Vec<_>>(),
    );

    // TS/TSX/JS/Python: the cct venue if it is on this machine (the measured TS venue), else skip.
    let cct = Path::new(CCT);
    let ts = find_files(cct, &[".ts"], 4);
    let tsx = find_files(cct, &[".tsx"], 2);
    let js = find_files(cct, &[".js"], 2);
    let py = find_files(cct, &[".py"], 2);
    let n2 = leg::<TypeScript>(
        &read("typescript.yml"),
        TypeScript,
        "typescript",
        &ts.iter().map(|p| p.as_path()).collect::<Vec<_>>(),
    );
    let n3 = leg::<Tsx>(
        &read("tsx.yml"),
        Tsx,
        "tsx",
        &tsx.iter().map(|p| p.as_path()).collect::<Vec<_>>(),
    );
    let n4 = leg::<JavaScript>(
        &read("javascript.yml"),
        JavaScript,
        "javascript",
        &js.iter().map(|p| p.as_path()).collect::<Vec<_>>(),
    );
    let n5 = leg::<Python>(
        &read("python.yml"),
        Python,
        "python",
        &py.iter().map(|p| p.as_path()).collect::<Vec<_>>(),
    );

    assert!(
        n1 > 0,
        "the rust leg must have produced comparisons; it is the pack's main language"
    );
    println!("total comparisons: {}", n1 + n2 + n3 + n4 + n5);
}
