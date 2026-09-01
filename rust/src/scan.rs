//! The rule pack as in-process matchers, replacing the `ast-grep scan --json=stream` subprocess on
//! the indexing hot path.
//!
//! The probe (`rust/tests/parity_probe.rs`) proved byte-identical output against the CLI on the real
//! pack -- 1,604/1,604 matches across four languages -- before any of this was wired. Direct wiring
//! removes what the subprocess meant: a binary that on this machine only ever shipped via npm, the
//! PATH probe, the per-file 30-second timeout, and the installer's release download. The CLI stays
//! for `struct --pattern` lookup and as the documented escape hatch: `CORT_SCAN_BACKEND=cli` puts
//! the subprocess back, which is also how the failure-injection tests (the `fake_ast_grep` double)
//! reach the subprocess code.
//!
//! Engine identity is part of `pack::extractor_version`: a backend flip forces a full re-index, the
//! same way a pack edit does. A grammar upgrade that changes extraction must not look like a no-op
//! to staleness.

use crate::errors::CortError;
use crate::pack::pack_dir;
use ast_grep_config::{from_yaml_string, GlobalRules, RuleConfig, RuleCore};
use ast_grep_core::meta_var::MetaVariable;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{matcher::MatcherExt, tree_sitter::LanguageExt, AstGrep, NodeMatch, Position};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// The engine behind in-process scans, recorded into `extractor_version`. Keep in step with the
/// `ast-grep-*` entries in `Cargo.lock`; the parity probe re-runs the byte-identity question
/// whenever either moves.
pub const SCAN_ENGINE: &str = "ast-grep-crate/0.45.3 (pinned via Cargo.lock, probed 2026-09-01)";

/// Which backend `extract_file` uses for its scan. `crate` is the default; `cli` is the escape
/// hatch and the failure-injection path. Anything else is refused rather than silently aliased.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Crate,
    Cli,
}

pub fn scan_backend() -> Result<Backend, CortError> {
    match std::env::var("CORT_SCAN_BACKEND").as_deref() {
        Ok("") | Err(_) => Ok(Backend::Crate),
        Ok("crate") => Ok(Backend::Crate),
        Ok("cli") => Ok(Backend::Cli),
        Ok(other) => Err(CortError::new(
            "unknown_scan_backend",
            json!({ "value": other, "valid": ["crate", "cli"] }),
        )),
    }
}

/// One rule compiled against one language: the CLI's `ruleId` and `message` plus the matcher.
struct CompiledRule {
    id: String,
    message: String,
    matcher: RuleCore,
}

/// All pack rules, grouped by the language that runs them.
pub struct RulePack {
    rules: HashMap<&'static str, Vec<CompiledRule>>,
}

/// (extension, key into `RulePack.rules`, the name the CLI reports in the `language` field). The
/// extension set mirrors `indexer::SOURCE_EXT` exactly: an extension one side knows and the other
/// does not would silently change what gets indexed.
const LANG_BY_EXT: &[(&str, &str, &str)] = &[
    ("rs", "rust", "Rust"),
    ("ts", "typescript", "TypeScript"),
    ("tsx", "tsx", "TypeScript"),
    ("js", "javascript", "JavaScript"),
    ("jsx", "javascript", "JavaScript"),
    ("mjs", "javascript", "JavaScript"),
    ("cjs", "javascript", "JavaScript"),
    ("py", "python", "Python"),
];

fn lang_key_and_name(ext: &str) -> Option<(&'static str, &'static str)> {
    let bare = ext.strip_prefix('.').unwrap_or(ext);
    LANG_BY_EXT
        .iter()
        .find(|(e, _, _)| *e == bare)
        .map(|(_, key, name)| (*key, *name))
}

macro_rules! load_lang {
    ($path:expr, $lang:ty) => {
        from_yaml_string::<$lang>(
            &std::fs::read_to_string($path).map_err(|e| {
                CortError::new(
                    "pack_unreadable",
                    json!({ "path": $path.display().to_string(), "message": e.to_string() }),
                )
            })?,
            &GlobalRules::default(),
        )
        .map_err(|e| {
            CortError::new(
                "pack_rule_invalid",
                json!({ "path": $path.display().to_string(), "message": e.to_string() }),
            )
        })?
    };
}

impl RulePack {
    /// Same file set the CLI's `ruleDirs: [rules]` picks up: every `.yml` under `rules/`.
    pub fn load(dir: &Path) -> Result<RulePack, CortError> {
        let rules_dir = dir.join("rules");
        let mut ymls: Vec<PathBuf> = std::fs::read_dir(&rules_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yml"))
                    .collect()
            })
            .map_err(|e| {
                CortError::new(
                    "pack_unreadable",
                    json!({ "path": rules_dir.display().to_string(), "message": e.to_string() }),
                )
            })?;
        ymls.sort();
        if ymls.is_empty() {
            return Err(CortError::new(
                "pack_unreadable",
                json!({ "path": rules_dir.display().to_string(), "message": "no rule files found" }),
            ));
        }
        let mut rules: HashMap<&'static str, Vec<CompiledRule>> = HashMap::new();
        for path in &ymls {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            // Fail closed on an unknown rule file: silently indexing a new language with zero
            // rules would look exactly like "this language has no callers".
            let (key, compiled): (&'static str, Vec<CompiledRule>) = match stem.as_str() {
                "rust" => ("rust", compile(load_lang!(path, ast_grep_language::Rust))),
                "typescript" => (
                    "typescript",
                    compile(load_lang!(path, ast_grep_language::TypeScript)),
                ),
                "tsx" => ("tsx", compile(load_lang!(path, ast_grep_language::Tsx))),
                "javascript" => (
                    "javascript",
                    compile(load_lang!(path, ast_grep_language::JavaScript)),
                ),
                "python" => (
                    "python",
                    compile(load_lang!(path, ast_grep_language::Python)),
                ),
                other => {
                    return Err(CortError::new(
                        "pack_rule_invalid",
                        json!({ "path": path.display().to_string(),
                                "message": format!("unknown rule file stem '{other}': extend scan.rs's language map") }),
                    ));
                }
            };
            rules.entry(key).or_default().extend(compiled);
        }
        Ok(RulePack { rules })
    }
}

fn compile<L: LanguageExt + serde::de::DeserializeOwned + Copy>(
    configs: Vec<RuleConfig<L>>,
) -> Vec<CompiledRule> {
    let globals = GlobalRules::default();
    configs
        .into_iter()
        .map(|c| {
            let matcher = c
                .get_matcher(&globals)
                .expect("every shipped rule must compile to a matcher");
            CompiledRule {
                id: c.id.clone(),
                // Pack messages are static strings (no $ metavariables) -- verified 2026-09-01 --
                // so the field on the rule is the message verbatim, same bytes the CLI emits.
                message: c.message.clone(),
                matcher,
            }
        })
        .collect()
}

static PACK: OnceLock<(PathBuf, Arc<RulePack>)> = OnceLock::new();

/// The process's rule pack: first caller loads, later callers reuse. Keyed on the resolved pack
/// directory at load time; a later `CORT_PACK_DIR` change reuses the first-loaded pack rather than
/// silently mixing two, and the recorded path makes that visible when debugging.
fn pack() -> Result<Arc<RulePack>, CortError> {
    let dir = pack_dir();
    if let Some((loaded_dir, p)) = PACK.get() {
        let _ = loaded_dir;
        return Ok(p.clone());
    }
    let loaded = Arc::new(RulePack::load(&dir)?);
    let _ = PACK.set((dir, loaded.clone()));
    Ok(loaded)
}

/// The CLI's position shape: 0-based line, 0-based *char* column (`Position::column`, not the
/// private byte column). The chunker reads only the lines, but a record whose columns disagreed
/// with the CLI's would make backend diffs lie about identity.
fn pos<L: LanguageExt>(p: Position, node: &ast_grep_core::Node<'_, StrDoc<L>>) -> Value {
    json!({ "line": p.line(), "column": p.column(node) })
}

/// The variables one match bound, in the CLI's `metaVariables.single` shape. Transformed and
/// multi-capture variables have no entry here, same as the CLI's `single` object.
fn meta_variables<L: LanguageExt>(m: &NodeMatch<'_, StrDoc<L>>) -> Value {
    let mut single = serde_json::Map::new();
    for var in m.get_env().get_matched_variables() {
        let MetaVariable::Capture(name, _) = var else {
            continue;
        };
        let Some(node) = m.get_env().get_match(&name) else {
            continue;
        };
        let r = node.range();
        single.insert(
            name,
            json!({
                "text": node.text(),
                "range": {
                    "byteOffset": { "start": r.start, "end": r.end },
                    "start": pos(node.start_pos(), node),
                    "end": pos(node.end_pos(), node),
                },
            }),
        );
    }
    json!({ "single": single })
}

/// Scan one file's text in-process. The returned records are the same JSON shapes
/// `parse_scan_stream` folds out of the CLI's stdout -- `text`, `range`, `message`, `language`,
/// `metaVariables` -- so everything downstream of the stream is unchanged, and the two backends
/// stay comparable record by record.
///
/// An unparseable file is an `Err` with code `scan_parse_failed`, which the caller degrades to an
/// `unparsed` chunk exactly as it degrades a failed CLI run.
pub fn scan_in_process(text: &str, ext: &str) -> Result<Vec<Value>, CortError> {
    let Some((key, lang_name)) = lang_key_and_name(ext) else {
        // Not a source extension: the CLI would have inferred no language either.
        return Ok(Vec::new());
    };
    let pack = pack()?;
    let Some(rules) = pack.rules.get(key) else {
        return Ok(Vec::new());
    };
    // Each arm passes its concrete language: the tree is parsed once per file, every rule walks
    // the same tree -- the CLI's own economy.
    let records = match key {
        "rust" => scan_lang(text, lang_name, rules, ast_grep_language::Rust),
        "typescript" => scan_lang(text, lang_name, rules, ast_grep_language::TypeScript),
        "tsx" => scan_lang(text, lang_name, rules, ast_grep_language::Tsx),
        "javascript" => scan_lang(text, lang_name, rules, ast_grep_language::JavaScript),
        _ => scan_lang(text, lang_name, rules, ast_grep_language::Python),
    };
    records.map_err(|e| {
        CortError::new(
            "scan_parse_failed",
            json!({ "message": e, "engine": SCAN_ENGINE }),
        )
    })
}

fn scan_lang<L: LanguageExt + Copy>(
    text: &str,
    lang_name: &str,
    rules: &[CompiledRule],
    lang: L,
) -> Result<Vec<Value>, String> {
    let root: AstGrep<StrDoc<L>> = AstGrep::doc(StrDoc::try_new(text, lang)?);
    let mut records = Vec::new();
    for rule in rules {
        for node in root.root().dfs() {
            if let Some(m) = rule.matcher.match_node(node) {
                let r = m.get_node().range();
                records.push(json!({
                    "text": m.text(),
                    "range": {
                        "byteOffset": { "start": r.start, "end": r.end },
                        "start": pos(m.get_node().start_pos(), m.get_node()),
                        "end": pos(m.get_node().end_pos(), m.get_node()),
                    },
                    "ruleId": rule.id,
                    "message": rule.message,
                    "language": lang_name,
                    "metaVariables": meta_variables(&m),
                }));
            }
        }
    }
    Ok(records)
}
