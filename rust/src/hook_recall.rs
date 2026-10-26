//! Session-start context from the existing local memory replica.

use crate::memory::MemoryRecall;

const MAX_ENTRIES: usize = 20;
const MAX_BYTES: usize = 6000;
const HEADER: &str = "cort memory: local replica (may be behind the shared database; `cort memory recall \"<topic>\"` refreshes)";

pub fn render(recall: MemoryRecall) -> Option<(String, usize)> {
    render_with_omitted(recall, 0)
}

pub fn render_with_omitted(recall: MemoryRecall, pre_omitted: usize) -> Option<(String, usize)> {
    if recall.hits.is_empty() {
        return None;
    }
    let mut hits = recall.hits;
    hits.sort_by(|a, b| {
        b.entry
            .recorded_at
            .cmp(&a.entry.recorded_at)
            .then(b.entry.id.cmp(&a.entry.id))
    });
    let mut out = String::from(HEADER);
    let mut shown = 0;
    // Reserve enough space for the omitted-count suffix, even for a large ledger.
    let body_limit = MAX_BYTES - 80;
    for hit in &hits {
        if shown == MAX_ENTRIES {
            break;
        }
        let clean = |s: &str| {
            s.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect::<String>()
        };
        let line = format!(
            "\n- [{}] {} ({}:{}, {})",
            clean(&hit.entry.key),
            clean(&hit.entry.claim),
            clean(&hit.entry.source),
            hit.entry.source_line,
            hit.evidence_status
        );
        if out.len() + line.len() > body_limit {
            let room = body_limit.saturating_sub(out.len());
            if room > 4 {
                let mut end = room - 4;
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                out.push_str(&line[..end]);
                out.push_str(" ...");
                shown += 1;
            }
            break;
        }
        out.push_str(&line);
        shown += 1;
    }
    let omitted = hits.len() - shown + pre_omitted;
    if omitted > 0 {
        out.push_str(&format!("\n... {omitted} entries left out"));
    }
    Some((out, shown))
}
