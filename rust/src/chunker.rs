//! Per-file extraction.
//! argv: `scan --json=stream --config SGCONFIG absPath`. Timeout 30s/file (overridable).
//! 256 MiB maxBuffer is specified on `execAstGrep` (Job B's bridge).

use crate::ast_grep::{exec_ast_grep, ExecOpts};
use crate::errors::CortError;
use crate::pack::sgconfig;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const CHUNK_TAG: &str = "chunk:";
const EDGE_TAG: &str = "edge:";

/// The call shapes the graph resolves differently, and the only values
/// `raw_edges.call_form` / `relationships.call_form` may hold (both columns are CHECK-constrained
/// to this list). A pack rule names its form in its message -- `edge:calls:receiver` -- because the
/// message string is the only channel from a rule to the parser.
/// The last segment of a call target: `Tally::add` -> `add`, `formatter.formatToParts` ->
/// `formatToParts`. Lives here because three modules need the same split -- the receiver gate in
/// `graph`, the mention screen in `coverage`, and the line that names a callee -- and two different
/// answers to "which name is this" is how a gate and a report start disagreeing.
pub fn bare_name(target: &str) -> &str {
    target
        .rsplit([':', '.'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(target)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallForm {
    /// `add()` -- a name with no qualifier (#[default]); the pre-v4 behaviour of every rule.
    #[default]
    Bare,
    /// `t.add()` -- the method name is only worth an edge if it is unique project-wide.
    Receiver,
    /// `Worker::add()` / `crate::m::f()` -- carries its own path, so it is matched exactly.
    Scoped,
}

impl CallForm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bare => "bare",
            Self::Receiver => "receiver",
            Self::Scoped => "scoped",
        }
    }

    /// Public because the database stores the string and `graph` has to read it back.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "bare" => Some(Self::Bare),
            "receiver" => Some(Self::Receiver),
            "scoped" => Some(Self::Scoped),
            _ => None,
        }
    }

    /// Total order for writing edges into `raw_edges`, whose key does not include the form. A bare
    /// `add()` and a receiver `t.add()` no longer collide -- the receiver stores its head -- so what
    /// is left is a genuinely duplicated row, and the rank exists to make *which* row survives a rule
    /// rather than an artefact of the order ast-grep emitted records in. `receiver` first, because it
    /// is the form with the strictest gate behind it.
    pub fn insertion_rank(self) -> u8 {
        match self {
            Self::Receiver => 0,
            Self::Scoped => 1,
            Self::Bare => 2,
        }
    }
}

/// The rel types the schema allows. Checked here so one typo in a pack rule message cannot take
/// down `cort index` for a whole project: the CHECK constraint would reject the insert mid-
/// transaction, and every chunk and edge already gathered would be lost with it.
pub const EDGE_REL_TYPES: &[&str] = &["imports", "exports", "calls"];

/// Split an `edge:` tag remainder into (rel_type, call_form).
///
/// `calls` -> (calls, bare), `calls:receiver` -> (calls, receiver). `None` -- an unknown rel type or
/// an unknown form -- drops the edge rather than guessing: a dropped edge leaves the call site
/// uncovered in the coverage screen, so a rule typo surfaces as a reported gap, whereas defaulting a
/// mistyped `reciever` to `bare` would have attached it under the looser policy and *hidden* the hole.
pub fn parse_edge_tag(rest: &str) -> Option<(String, CallForm)> {
    let (rel_type, form) = match rest.split_once(':') {
        Some((rel, form)) => (rel, CallForm::parse(form)?),
        None => (rest, CallForm::Bare),
    };
    EDGE_REL_TYPES
        .contains(&rel_type)
        .then(|| (rel_type.to_string(), form))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub chunk_id: String,
    pub project_id: String,
    pub file_path: String,
    pub symbol_name: Option<String>,
    pub chunk_type: String,
    pub start_line: i64,
    pub end_line: i64,
    pub content: String,
    pub content_hash: String,
    pub language: Option<String>,
    pub chunk_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub rel_type: String,
    /// Which call shape produced this edge. Only meaningful for `calls`; an import carries `bare`.
    pub call_form: CallForm,
    pub source_symbol: Option<String>,
    /// What the code calls, with whitespace removed: `helper` for a bare call, `crate::m::f` for a
    /// qualified one, `tally.add` for a receiver call. A receiver target keeps its receiver because
    /// that is the only evidence `graph::receiver_binds` has for whether the call can bind to the
    /// symbol it is about to attach to; dropping it is what made the first cut of this gate invent
    /// edges from `e.kind()` to a test fixture's `FailFs::kind`.
    pub raw_target: String,
    /// The line that names the callee: for a call record this is the `CALLEE` capture's own line,
    /// not the matched node's, because a chained `a.b()\n .c()` names `c` two lines below where the
    /// outer call starts. This is what `relationships.call_site_line` ends up storing.
    pub start_line: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractResult {
    pub chunks: Vec<Chunk>,
    pub edges: Vec<Edge>,
    pub file_content_hash: String,
    pub unparsed: bool,
    pub malformed: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanStream {
    pub records: Vec<Value>,
    pub malformed: usize,
    pub total: usize,
}

pub fn chunk_id_for(project_id: &str, file_path: &str, start_line: i64) -> String {
    format!("{project_id}:{file_path}:{start_line}")
}

pub fn parse_scan_stream(stdout: &str) -> ScanStream {
    let lines: Vec<&str> = stdout
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .collect();
    let total = lines.len();
    let mut records = Vec::new();
    let mut malformed = 0usize;
    for line in lines {
        match serde_json::from_str::<Value>(line) {
            Ok(v) => records.push(v),
            Err(_) => malformed += 1,
        }
    }
    ScanStream {
        records,
        malformed,
        total,
    }
}

pub fn edge_string(edge: &Edge) -> String {
    format!(
        "{}\t{}\t{}",
        edge.rel_type,
        edge.source_symbol.as_deref().unwrap_or(""),
        edge.raw_target
    )
}

/// The form a call arrived as is *not* part of this hash. A form can only change two ways: the text
/// changed (which changes the target set or the recorded lines, and so the hash), or the rules that
/// assign forms changed -- and that moves `pack::extractor_version()`, which forces a full re-index
/// on its own. Hashing the form as well would churn every file of every project at the v3/v4
/// boundary and buy nothing, because nothing can reach this hash without one of those two changes.
pub fn file_content_hash(chunks: &[Chunk], edges: &[Edge]) -> String {
    let mut ordered: Vec<&Chunk> = chunks.iter().collect();
    ordered.sort_by_key(|c| c.start_line);
    let mut h = Sha256::new();
    for c in ordered {
        h.update(c.content.as_bytes());
    }
    let mut edge_s: Vec<String> = edges.iter().map(edge_string).collect();
    edge_s.sort();
    for s in edge_s {
        h.update(s.as_bytes());
    }
    hex_sha256(h)
}

/// `metaVariables.single.<name>`. Job D can read `"OWNER"` through this
/// without changing the parser; C1 only uses `"NAME"` for `symbol_name`.
fn meta_var<'a>(rec: &'a Value, name: &str) -> Option<&'a Value> {
    rec.get("metaVariables")
        .and_then(|m| m.get("single"))
        .and_then(|s| s.get(name))
}

pub fn meta_var_text<'a>(rec: &'a Value, name: &str) -> Option<&'a str> {
    meta_var(rec, name)
        .and_then(|v| v.get("text"))
        .and_then(Value::as_str)
}

/// 1-based line a capture sits on, so a call edge can be pinned to the line that names its callee.
fn meta_var_line(rec: &Value, name: &str) -> Option<i64> {
    meta_var(rec, name)
        .and_then(|v| v.get("range"))
        .and_then(|r| r.get("start"))
        .and_then(|p| p.get("line"))
        .and_then(json_i64)
        .map(|n| n + 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeError {
    MethodMissingOwner,
}

/// Collapse runs of whitespace, trim, and drop spaces around `::`.
fn normalize_ws(s: &str) -> String {
    let mut out = String::new();
    let mut started = false;
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if started {
                prev_space = true;
            }
        } else {
            if prev_space && started {
                out.push(' ');
            }
            out.push(c);
            started = true;
            prev_space = false;
        }
    }
    let chars: Vec<char> = out.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' && i + 2 < chars.len() && chars[i + 1] == ':' && chars[i + 2] == ':' {
            i += 1;
            continue;
        }
        if chars[i] == ':' && i + 1 < chars.len() && chars[i + 1] == ':' {
            result.push(':');
            result.push(':');
            i += 2;
            if i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

fn split_colons_outside_generics(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '<' => {
                depth += 1;
                cur.push('<');
            }
            '>' => {
                if depth > 0 {
                    depth -= 1;
                }
                cur.push('>');
            }
            ':' if depth == 0 && i + 1 < chars.len() && chars[i + 1] == ':' => {
                parts.push(cur.trim().to_string());
                cur.clear();
                i += 1;
            }
            c => cur.push(c),
        }
        i += 1;
    }
    parts.push(cur.trim().to_string());
    parts
}

fn strip_segment_generics(seg: &str) -> String {
    let seg = seg.trim();
    let mut depth = 0i32;
    let mut start = None;
    for (i, c) in seg.char_indices() {
        match c {
            '<' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '>' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(st) = start {
                        let rest = seg[i + c.len_utf8()..].trim();
                        if rest.is_empty() {
                            return seg[..st].trim().to_string();
                        }
                    }
                }
            }
            _ => {}
        }
    }
    seg.to_string()
}

/// Type-path owners keep the path, drop per-segment generic args, and drop
/// inessential whitespace. Non-type-path owners keep syntax and only normalize ws.
pub fn canonical_owner(raw: &str) -> String {
    let normalized = normalize_ws(raw);
    split_colons_outside_generics(&normalized)
        .into_iter()
        .map(|seg| strip_segment_generics(&seg))
        .collect::<Vec<_>>()
        .join("::")
}

/// Free function: `NAME`. Method with `$OWNER`: `canonical_owner(OWNER)::NAME`.
/// A **Rust** method record without OWNER fails closed — never falls back to a
/// bare NAME. JS/TS `chunk:method` records have no OWNER capture and keep NAME.
pub fn compose_symbol_name(
    chunk_type: &str,
    name: Option<&str>,
    owner: Option<&str>,
    language: Option<&str>,
) -> Result<Option<String>, ComposeError> {
    if chunk_type == "method" {
        if let Some(owner) = owner {
            let name = name.unwrap_or("");
            return Ok(Some(format!("{}::{name}", canonical_owner(owner))));
        }
        if language.is_some_and(|l| l.eq_ignore_ascii_case("rust")) {
            return Err(ComposeError::MethodMissingOwner);
        }
        return Ok(name.map(str::to_string));
    }
    Ok(name.map(str::to_string))
}

fn hex_sha256(h: sha2::Sha256) -> String {
    format!("{:x}", h.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn unquote(text: &str) -> String {
    let t = text.trim();
    let b = t.as_bytes();
    if b.len() >= 2 {
        let first = b[0];
        if matches!(first, b'\'' | b'"' | b'`') && b[b.len() - 1] == first {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

/// Remove every whitespace character: a call head split across lines is still the same call.
fn compact_ws(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn json_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
}

fn line_1based(rec: &Value, which: &str) -> Option<i64> {
    rec.get("range")
        .and_then(|r| r.get(which))
        .and_then(|p| p.get("line"))
        .and_then(json_i64)
        .map(|n| n + 1)
}

fn unparsed_result(
    project_id: &str,
    file_path: &str,
    source: &str,
    malformed: usize,
) -> ExtractResult {
    let chunk = Chunk {
        chunk_id: chunk_id_for(project_id, file_path, 1),
        project_id: project_id.to_string(),
        file_path: file_path.to_string(),
        symbol_name: None,
        chunk_type: "unparsed".to_string(),
        start_line: 1,
        end_line: (source.split('\n').count() as i64).max(1),
        content: source.to_string(),
        content_hash: sha256_hex(source.as_bytes()),
        language: None,
        chunk_source: "unparsed".to_string(),
    };
    let hash = file_content_hash(std::slice::from_ref(&chunk), &[]);
    ExtractResult {
        chunks: vec![chunk],
        edges: vec![],
        file_content_hash: hash,
        unparsed: true,
        malformed,
    }
}

pub struct ExtractFileArgs<'a> {
    pub bin: &'a str,
    pub project_id: &'a str,
    pub file_path: &'a str,
    pub abs_path: &'a str,
    pub source: &'a str,
    pub timeout_ms: Option<u64>,
}

pub fn extract_file(args: ExtractFileArgs<'_>) -> Result<ExtractResult, CortError> {
    let sg = sgconfig();
    let sg_s = sg.to_str().expect("SGCONFIG path is UTF-8");
    let r = match exec_ast_grep(
        args.bin,
        &["scan", "--json=stream", "--config", sg_s, args.abs_path],
        ExecOpts {
            timeout_ms: args.timeout_ms,
            cwd: None,
        },
    ) {
        Ok(r) => r,
        Err(err) if err.code == "ast_grep_timeout" => {
            return Ok(unparsed_result(
                args.project_id,
                args.file_path,
                args.source,
                0,
            ));
        }
        Err(err) => return Err(err),
    };
    if r.code != 0 {
        return Ok(unparsed_result(
            args.project_id,
            args.file_path,
            args.source,
            0,
        ));
    }

    let parsed = parse_scan_stream(&r.stdout);
    if parsed.records.is_empty() {
        return Ok(unparsed_result(
            args.project_id,
            args.file_path,
            args.source,
            parsed.malformed,
        ));
    }

    let project_id = args.project_id;
    let file_path = args.file_path;

    let mut chunks = Vec::new();
    let mut raw_edges: Vec<(String, CallForm, String, i64)> = Vec::new();
    let mut extra_malformed = 0usize;
    for rec in &parsed.records {
        let tag = rec.get("message").and_then(Value::as_str).unwrap_or("");
        let Some(start_line) = line_1based(rec, "start") else {
            continue;
        };
        let Some(end_line) = line_1based(rec, "end") else {
            continue;
        };
        if let Some(rest) = tag.strip_prefix(CHUNK_TAG) {
            let text = rec.get("text").and_then(Value::as_str).unwrap_or("");
            let chunk_type = rest.to_string();
            let language = rec.get("language").and_then(Value::as_str);
            match compose_symbol_name(
                &chunk_type,
                meta_var_text(rec, "NAME"),
                meta_var_text(rec, "OWNER"),
                language,
            ) {
                Ok(symbol_name) => {
                    chunks.push(Chunk {
                        chunk_id: chunk_id_for(project_id, file_path, start_line),
                        project_id: project_id.to_string(),
                        file_path: file_path.to_string(),
                        symbol_name,
                        chunk_type,
                        start_line,
                        end_line,
                        content: text.to_string(),
                        content_hash: sha256_hex(text.as_bytes()),
                        language: language.map(str::to_string),
                        chunk_source: "ast".to_string(),
                    });
                }
                Err(_) => {
                    extra_malformed += 1;
                }
            }
        } else if let Some(rest) = tag.strip_prefix(EDGE_TAG) {
            let Some((rel_type, call_form)) = parse_edge_tag(rest) else {
                // A rule cort cannot interpret. Counted rather than silently obeyed: `malformed` is
                // the field that says "this extraction is not complete".
                extra_malformed += 1;
                continue;
            };
            let callee = meta_var_text(rec, "CALLEE");
            let target = meta_var_text(rec, "SRC").or(callee);
            if let Some(target) = target {
                // Pin the edge to the line that *names* the callee, not to the first line of the
                // matched node: `builder\n    .foo()` names `foo` on the second line, and a call site
                // an agent cannot read at the stated line is not evidence. A rule that captures the
                // head and the name separately (`$CALLEE` + `$METHOD`) says which one is the name.
                let line = ["METHOD", "CALLEE"]
                    .iter()
                    .find_map(|var| meta_var_line(rec, var))
                    .unwrap_or(start_line);
                // Whitespace inside a call head is formatting, not identity: `tally\n  .add` and
                // `tally.add` are the same edge, and a stored target containing a newline would
                // never survive a LIKE match against a symbol name.
                let text = if callee.is_some() {
                    compact_ws(unquote(target).as_str())
                } else {
                    unquote(target)
                };
                raw_edges.push((rel_type, call_form, text, line));
            }
        }
    }

    chunks.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then(a.end_line.cmp(&b.end_line))
    });
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for c in chunks {
        if seen.insert(c.chunk_id.clone()) {
            deduped.push(c);
        }
    }

    let edges: Vec<Edge> = raw_edges
        .into_iter()
        .map(|(rel_type, call_form, raw_target, start_line)| {
            let containing = deduped
                .iter()
                .filter(|c| c.start_line <= start_line && start_line <= c.end_line)
                .min_by_key(|c| c.end_line - c.start_line);
            Edge {
                rel_type,
                call_form,
                source_symbol: containing.and_then(|c| c.symbol_name.clone()),
                raw_target,
                start_line,
            }
        })
        .collect();

    let hash = file_content_hash(&deduped, &edges);
    Ok(ExtractResult {
        chunks: deduped,
        edges,
        file_content_hash: hash,
        unparsed: false,
        malformed: parsed.malformed + extra_malformed,
    })
}
