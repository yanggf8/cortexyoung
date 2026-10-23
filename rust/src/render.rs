//! JSON / lean rendering.

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

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
}

pub fn render_impact(payload: &Value) -> String {
    let seeds = arr(payload, "seeds");
    let mut lines = vec![format!(
        "# impact {} depth={} seeds={} dependents={} stale={} repair={}",
        as_str(payload, "symbol"),
        js_display(payload, "depth"),
        seeds.len(),
        js_display(payload, "dependent_count"),
        js_display(payload, "index_is_stale"),
        as_str(payload, "repair"),
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
        // Six fixed columns -- hop, file, symbol, definition line, call site, call form. The last two
        // carry a dash rather than disappearing, because a row that changed shape would be read as a
        // different claim; `@-` means "this dependency came in through an import, or through an edge
        // indexed before call sites were recorded", not "line 0".
        let site = d
            .get("call_site_line")
            .and_then(Value::as_i64)
            .map(|l| format!("@{l}"))
            .unwrap_or_else(|| "-".to_string());
        let form = d
            .get("call_form")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("-");
        lines.push(format!(
            "h{}\t{}\t{}\t{}\t{}\t{}",
            as_i64(d, "hop"),
            as_str(d, "file_path"),
            name,
            as_i64(d, "start_line"),
            site,
            form
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
    if coverage["no_seed_resolved"] == Value::Bool(true) || seeds.is_empty() {
        out.push(
            "coverage\tno_seed_resolved\tnot a clean answer: nothing was looked at".to_string(),
        );
    }
    for seed in seeds {
        let no_edge = arr(seed, "mentions_without_edge");
        let dropped = arr(seed, "extracted_but_unresolved");
        // `mentions_without_edge` is capped at `MAX_GAP_ROWS`; `mention_gap_count` is every
        // mention-layer row that was found. Printing the capped length told a lean reader -- the
        // only reader the routing skill creates -- that it had seen the whole list, which is the one
        // thing "read the rows, not the boolean" needs to be true. `gap_count` is deliberately NOT
        // the fallback of first resort any more: it now counts dropped resolutions too (it is the
        // number the boolean reads), and a total that includes them must not drive the truncation
        // claim, or a seed with three printed mention rows and two drops would announce "truncated
        // shown=3 of=5" over a list nothing was cut from. Older payloads without the new field fall
        // back through `gap_count` (which was L1-only before 2026-09-01) and then the rows in hand;
        // a dash would read as "no gaps".
        let gap_total = seed["mention_gap_count"]
            .as_u64()
            .or_else(|| seed["gap_count"].as_u64())
            .unwrap_or(no_edge.len() as u64);
        out.push(format!(
            "seed\t{}\tmentions={}\tno_edge={}\tdropped={}\tincomplete={}",
            as_str(seed, "symbol"),
            js_display(seed, "mentions_on_disk"),
            gap_total,
            dropped.len(),
            js_display(seed, "enumeration_may_be_incomplete")
        ));
        // Derived from the two numbers printed above rather than from `mentions_truncated`, so the
        // cut cannot disagree with the counts beside it. Placed before the rows, not after, because
        // a reader who stops at the twentieth row is exactly the one being misled.
        if gap_total > no_edge.len() as u64 {
            out.push(format!(
                "miss\ttruncated\tshown={}\tof={}\tlowest-ranked rows dropped; -f json for the rest",
                no_edge.len(),
                gap_total
            ));
        }
        for g in no_edge {
            let occurrences = g["occurrences"].as_u64().unwrap_or(1);
            out.push(format!(
                "miss\t{}\t{}:{}\t{}{}",
                as_str(g, "cause"),
                as_str(g, "file_path"),
                js_display(g, "line"),
                if occurrences > 1 {
                    format!("x{occurrences}\t")
                } else {
                    String::new()
                },
                as_str(g, "text")
            ));
        }
        for g in dropped {
            // An import drop has no source symbol (a top-level `use` belongs to no chunk), and a
            // brace import's raw target carries the newlines it was written with. A blank field
            // reads as an empty column and a newline breaks the one-row-per-line contract, so both
            // are collapsed before printing.
            let from = as_str(g, "from_symbol");
            let from = if from.is_empty() { "-" } else { from };
            let target: String = as_str(g, "raw_target")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let target = if target.chars().count() > 72 {
                format!("{}…", target.chars().take(72).collect::<String>())
            } else {
                target
            };
            out.push(format!(
                "drop\t{}:{}\t{} -> {}",
                as_str(g, "file_path"),
                js_display(g, "line"),
                from,
                target
            ));
        }
    }
    let blind = coverage.get("blind_files");
    if let Some(b) = blind {
        // `-` for a count that was never computed, never a blank and never a 0: the no-seed shape of
        // this report reads the tree not at all, so its `scan_skipped` is unknown rather than empty,
        // and "unread=0" there would be the exact false-safe claim the field exists to prevent.
        let count = |key: &str| match b.get(key) {
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Array(list)) => list.len().to_string(),
            _ => "-".to_string(),
        };
        out.push(format!(
            "blind\tunparsed={}\tunindexed={}\tunread={}",
            count("unparsed"),
            count("unindexed"),
            count("scan_skipped")
        ));
        // The one sentence the flag cannot carry: `unparsed` files have no edges but were still read
        // as text, so they contribute rows and no verdict. Without this line, `blind unparsed=2` next
        // to `incomplete=false` reads as a contradiction -- which is how an agent ends up ignoring
        // both.
        if b["unparsed"].as_u64().unwrap_or(0) > 0 {
            out.push(
                "blind\tunparsed\tadvisory: text-scanned, no edges; does not flip incomplete"
                    .to_string(),
            );
        }
        // Paths, not just counts: "1 file is blind" tells an agent nothing it can act on.
        for (kind, key) in [
            ("unparsed", "unparsed_example"),
            ("unindexed", "unindexed_example"),
            // A file this scan could not read was never looked at. Silence here is what let a
            // healthy 2 MB+ file answer "no gaps": K3's worst false negative.
            ("scan_skipped", "scan_skipped_files"),
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

fn render_projects(payload: &Value) -> String {
    let word = payload
        .get("indexes")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let drifted = payload.get("drifted").and_then(Value::as_u64).unwrap_or(0);
    let unreadable = payload
        .get("unreadable")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    format!("indexes\t{word}\t{drifted}\t{unreadable}\n")
}

fn render_internal_manifest_keys(payload: &Value) -> String {
    let mut lines = Vec::new();
    for key in ["known", "legacy"] {
        if let Some(keys) = payload.get(key).and_then(Value::as_array) {
            for k in keys {
                if let Some(k) = k.as_str() {
                    lines.push(format!("{key}\t{k}"));
                }
            }
        }
    }
    format!("{}\n", lines.join("\n"))
}

fn render_internal_ast_grep(payload: &Value) -> String {
    let mut lines = Vec::new();
    for key in ["version", "repo", "crate"] {
        lines.push(format!(
            "{}\t{}",
            key,
            payload.get(key).and_then(Value::as_str).unwrap_or("")
        ));
    }
    if let Some(assets) = payload.get("assets").and_then(Value::as_array) {
        for a in assets {
            let name = a.get(0).and_then(Value::as_str).unwrap_or("");
            let sha = a.get(1).and_then(Value::as_str).unwrap_or("");
            lines.push(format!("asset\t{name}\t{sha}"));
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
        Some("projects") => render_projects(payload),
        Some("internal-ast-grep") => render_internal_ast_grep(payload),
        Some("internal-manifest-keys") => render_internal_manifest_keys(payload),
        _ => pretty(payload),
    }
}

/// Errors render as JSON in every format. The one lean error shape (`validation_error`) belonged to
/// the retired `read`/`recall` store, which was its only producer.
pub fn render_error(_format: Format, err: &CortError) -> String {
    pretty(&err.to_json())
}
