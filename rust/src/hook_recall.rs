//! Session-start context from the shared memory database.

use crate::memory::MemoryRecall;
use crate::notes_upload::NoteBrief;

const MAX_ENTRIES: usize = 20;
const MAX_BYTES: usize = 6000;
/// Other machines' notes listed at session start, on a budget separate from the ledger's.
pub const MAX_NOTES: usize = 15;
const MAX_NOTE_BYTES: usize = 2000;
const HEADER: &str = "cort memory (shared database, read at session start; `cort memory recall \"<topic>\"` searches it)";
const NOTES_HEADER: &str = "unreviewed harness notes from other machines (not checked against source; read one with `cort memory notes show <id>`):";

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

pub fn render_with_notes(
    recall: MemoryRecall,
    omitted: usize,
    notes: &[NoteBrief],
    total: usize,
) -> Option<(String, usize, usize)> {
    let ledger = render_with_omitted(recall, omitted);
    if notes.is_empty() {
        return ledger.map(|(text, entries)| (text, entries, 0));
    }
    let (mut text, entries) = ledger.unwrap_or_else(|| (HEADER.to_string(), 0));
    let mut section = format!("\n{NOTES_HEADER}");
    let mut shown = 0;
    for note in notes.iter().take(MAX_NOTES) {
        let host = note
            .host
            .as_deref()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| note.machine.get(..8).unwrap_or(&note.machine));
        let line = format!(
            "\n- {} [{}] {} ({})",
            crate::notes::sanitize(&note.note_id, Some(64)),
            crate::notes::sanitize(host, Some(64)),
            crate::notes::sanitize(&note.file_name, Some(80)),
            crate::notes::sanitize(&note.updated_at, Some(64)),
        );
        // Keep room for the count suffix, including large totals.
        if section.len() + line.len() + 64 > MAX_NOTE_BYTES {
            break;
        }
        section.push_str(&line);
        shown += 1;
    }
    if total > shown {
        section.push_str(&format!("\n... {} more notes", total - shown));
    }
    text.push_str(&section);
    Some((text, entries, shown))
}
