//! Per-file extraction. JS `src/chunker.js`.
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
    pub source_symbol: Option<String>,
    pub raw_target: String,
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

/// `metaVariables.single.<name>.text`. Job D can read `"OWNER"` through this
/// without changing the parser; C1 only uses `"NAME"` for `symbol_name`.
pub fn meta_var_text<'a>(rec: &'a Value, name: &str) -> Option<&'a str> {
    rec.get("metaVariables")
        .and_then(|m| m.get("single"))
        .and_then(|s| s.get(name))
        .and_then(|v| v.get("text"))
        .and_then(Value::as_str)
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
            '>'
                if depth > 0 => {
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
    let mut raw_edges: Vec<(String, String, i64)> = Vec::new();
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
            let target = meta_var_text(rec, "SRC").or_else(|| meta_var_text(rec, "CALLEE"));
            if let Some(target) = target {
                raw_edges.push((rest.to_string(), unquote(target), start_line));
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
        .map(|(rel_type, raw_target, start_line)| {
            let containing = deduped
                .iter()
                .filter(|c| c.start_line <= start_line && start_line <= c.end_line)
                .min_by_key(|c| c.end_line - c.start_line);
            Edge {
                rel_type,
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
