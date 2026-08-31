//! JSON / lean rendering. The `read`/`recall` validation contracts follow
//! `docs/superpowers/plans/2026-08-28-codex-fix-proposal.md` §1 and §3.

use crate::errors::CortError;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Lean,
}

/// JS `parseFormat`: non-string → `'json'`; `'LEAN'` → `'lean'`; unknown → `null`.
pub fn parse_format(raw: Option<&str>) -> Option<Format> {
    let value = match raw {
        Some(s) => s.to_ascii_lowercase(),
        None => "json".to_string(),
    };
    match value.as_str() {
        "lean" => Some(Format::Lean),
        "json" => Some(Format::Json),
        _ => None,
    }
}

fn pretty(payload: &Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| "null".into())
    )
}

fn as_str<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn as_i64(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(Value::as_i64)
        .or_else(|| {
            v.get(key)
                .and_then(Value::as_u64)
                .and_then(|u| i64::try_from(u).ok())
        })
        .or_else(|| v.get(key).and_then(Value::as_f64).map(|f| f as i64))
        .unwrap_or(0)
}

fn js_display(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn js_slice_120(s: &str) -> String {
    let u: Vec<u16> = s.encode_utf16().take(120).collect();
    String::from_utf16_lossy(&u)
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
}

pub fn render_impact(payload: &Value) -> String {
    let seeds = arr(payload, "seeds");
    let mut lines = vec![format!(
        "# impact {} depth={} seeds={} dependents={} stale={}",
        as_str(payload, "symbol"),
        js_display(payload, "depth"),
        seeds.len(),
        js_display(payload, "dependent_count"),
        js_display(payload, "index_is_stale"),
    )];
    for s in seeds {
        lines.push(format!(
            "seed\t{}:{}",
            as_str(s, "file_path"),
            as_i64(s, "start_line")
        ));
    }
    for d in arr(payload, "dependents") {
        let name = d.get("symbol_name").and_then(Value::as_str).unwrap_or("?");
        lines.push(format!(
            "h{}\t{}\t{}\t{}",
            as_i64(d, "hop"),
            as_str(d, "file_path"),
            name,
            as_i64(d, "start_line")
        ));
    }
    for u in arr(payload, "unresolved") {
        lines.push(format!(
            "unresolved\t{}\t{}\t{}",
            as_str(u, "symbol"),
            as_str(u, "rel_type"),
            as_str(u, "confidence")
        ));
    }
    lines.extend(coverage_lines(payload));
    format!("{}\n", lines.join("\n"))
}

/// The recall section. It exists because `dependents=0` and "no caller was ever extracted" are the
/// same bytes otherwise, so the summary header alone can bless a wrong "safe to remove". Rendered as
/// its own block rather than folded into the header because the gap rows are the payload.
fn coverage_lines(payload: &Value) -> Vec<String> {
    let Some(coverage) = payload.get("coverage") else {
        return Vec::new();
    };
    let mut out = vec![format!("# coverage {}", as_str(coverage, "method"))];
    let seeds = arr(coverage, "seeds");
    if seeds.is_empty() {
        out.push(
            "coverage\tseeds=0\tnothing_resolved\tthis is itself a gap, not a clean answer"
                .to_string(),
        );
    }
    for seed in seeds {
        let no_edge = arr(seed, "mentions_without_edge");
        let dropped = arr(seed, "extracted_but_unresolved");
        out.push(format!(
            "seed\t{}\tmentions={}\tno_edge={}\tdropped={}\tincomplete={}",
            as_str(seed, "symbol"),
            js_display(seed, "mentions_on_disk"),
            no_edge.len(),
            dropped.len(),
            js_display(seed, "enumeration_may_be_incomplete")
        ));
        for g in no_edge {
            out.push(format!(
                "miss\t{}\t{}:{}\t{}",
                as_str(g, "cause"),
                as_str(g, "file_path"),
                js_display(g, "line"),
                as_str(g, "text")
            ));
        }
        for g in dropped {
            out.push(format!(
                "drop\t{}:{}\t{} -> {}",
                as_str(g, "file_path"),
                js_display(g, "line"),
                as_str(g, "from_symbol"),
                as_str(g, "raw_target")
            ));
        }
    }
    let blind = coverage.get("blind_files");
    if let Some(b) = blind {
        out.push(format!(
            "blind\tunparsed={}\tunindexed={}",
            js_display(b, "unparsed"),
            js_display(b, "unindexed")
        ));
        // Paths, not just counts: "1 file is blind" tells an agent nothing it can act on.
        for (kind, key) in [
            ("unparsed", "unparsed_example"),
            ("unindexed", "unindexed_example"),
        ] {
            for f in arr(b, key) {
                if let Some(name) = f.as_str() {
                    out.push(format!("blind\t{kind}\t{name}"));
                }
            }
        }
    }
    out
}

pub fn render_struct(payload: &Value) -> String {
    let matches = arr(payload, "matches");
    let mut lines = vec![format!(
        "# struct {} lang={} matches={} shown={} truncated={} stale={}",
        as_str(payload, "pattern"),
        as_str(payload, "lang"),
        js_display(payload, "match_count"),
        matches.len(),
        js_display(payload, "truncated"),
        js_display(payload, "index_is_stale"),
    )];
    for m in matches {
        let neighbors = arr(m, "neighbors")
            .iter()
            .map(|n| {
                let dir0 = as_str(n, "direction").chars().next().unwrap_or('?');
                let label = n
                    .get("symbol_name")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| as_str(n, "file_path"));
                format!("{dir0}{}:{label}", as_str(n, "rel_type"))
            })
            .collect::<Vec<_>>()
            .join(",");
        let symbol = m.get("symbol_name").and_then(Value::as_str).unwrap_or("?");
        let text = js_slice_120(&as_str(m, "text").replace('\n', " "));
        let cols: Vec<String> = [
            format!("{}:{}", as_str(m, "file_path"), as_i64(m, "start_line")),
            symbol.to_string(),
            neighbors,
            text,
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
        lines.push(cols.join("\t"));
    }
    format!("{}\n", lines.join("\n"))
}

pub fn render_context(payload: &Value) -> String {
    let mut lines = vec![format!(
        "# context {} resolution={} seeds={} truncated={} stale={}",
        as_str(payload, "query"),
        as_str(payload, "resolution"),
        js_display(payload, "seed_count"),
        js_display(payload, "truncated"),
        js_display(payload, "index_is_stale"),
    )];
    for s in arr(payload, "seeds") {
        let name = s.get("symbol_name").and_then(Value::as_str).unwrap_or("?");
        lines.push(format!(
            "{}:{}\t{}\t{}",
            as_str(s, "file_path"),
            as_i64(s, "start_line"),
            name,
            as_str(s, "chunk_type")
        ));
        for n in arr(s, "neighbors") {
            let dir0 = as_str(n, "direction").chars().next().unwrap_or('?');
            let conf0 = as_str(n, "confidence").chars().next().unwrap_or('?');
            let nname = n.get("symbol_name").and_then(Value::as_str).unwrap_or("?");
            lines.push(format!(
                "  {dir0}{}\t{}:{}\t{}\t{conf0}",
                as_str(n, "rel_type"),
                as_str(n, "file_path"),
                as_i64(n, "start_line"),
                nname
            ));
        }
        for u in arr(s, "unresolved") {
            lines.push(format!(
                "  unresolved\t{}\t{}\t{}",
                as_str(u, "rel_type"),
                as_str(u, "symbol"),
                as_str(u, "confidence")
            ));
        }
        if let Some(content) = s.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                lines.push("  {".into());
                for line in content.split('\n') {
                    lines.push(format!("  {line}"));
                }
                lines.push("  }".into());
            }
        }
    }
    format!("{}\n", lines.join("\n"))
}

pub fn render_read(payload: &Value) -> String {
    let file = as_str(payload, "file_path");
    let start = as_i64(payload, "start_line");
    let end = as_i64(payload, "end_line");
    let source = as_str(payload, "source");
    let reads = as_i64(payload, "read_count");
    let mode = payload
        .get("content_mode")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if payload.get("content").and_then(Value::as_str).is_some() {
                "full"
            } else {
                "receipt"
            }
        });
    let hash = as_str(payload, "content_hash_prefix");
    let header = format!(
        "# read {file}:{start}-{end} source={source} reads={reads} content={mode} hash={hash}"
    );
    if mode == "receipt" {
        format!("{header}\n")
    } else {
        let content = payload.get("content").and_then(Value::as_str).unwrap_or("");
        format!("{header}\n{content}\n")
    }
}

pub fn render_recall(payload: &Value) -> String {
    let mut lines = vec![format!(
        "# recall {} readings={} truncated_query={}",
        as_str(payload, "query"),
        js_display(payload, "reading_count"),
        js_display(payload, "truncated_query"),
    )];
    for reading in arr(payload, "readings") {
        lines.push(format!(
            "{}:{}-{}\treads={}",
            as_str(reading, "file_path"),
            as_i64(reading, "start_line"),
            as_i64(reading, "end_line"),
            as_i64(reading, "read_count")
        ));
        if let Some(content) = reading.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                lines.push(content.to_string());
            }
        }
    }
    format!("{}\n", lines.join("\n"))
}

pub fn render(command: Option<&str>, format: Format, payload: &Value) -> String {
    if format != Format::Lean {
        return pretty(payload);
    }
    match command {
        Some("impact") => render_impact(payload),
        Some("struct") => render_struct(payload),
        Some("context") => render_context(payload),
        Some("read") => render_read(payload),
        Some("recall") => render_recall(payload),
        _ => pretty(payload),
    }
}

pub fn render_error(format: Format, err: &CortError) -> String {
    if format == Format::Lean && err.code == "validation_error" {
        render_validation_error_lean(&err.detail)
    } else {
        pretty(&err.to_json())
    }
}

fn dash_or_display(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::Null) | None => "-".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
    }
}

fn render_validation_error_lean(detail: &Value) -> String {
    format!(
        "! validation_error command={} file={} operation={} errno={} os_code={} retryable={} note={}\n",
        as_str(detail, "command"),
        as_str(detail, "file_path"),
        as_str(detail, "operation"),
        dash_or_display(detail, "errno"),
        dash_or_display(detail, "os_code"),
        js_display(detail, "retryable"),
        as_str(detail, "note_action"),
    )
}
