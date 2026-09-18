//! Deterministic Markdown heading index used as a navigation sidecar.
use std::path::Path;

use crate::model::DocumentHeading;

pub fn index_file(root: &Path, path: &Path) -> Vec<DocumentHeading> {
    let Ok(source) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    index_source(&rel, &source)
}

pub fn index_source(path: &str, source: &str) -> Vec<DocumentHeading> {
    let lines: Vec<&str> = source.lines().collect();
    let mut headings = Vec::new();
    let mut stack: Vec<(usize, String, usize)> = Vec::new();
    let mut fenced = false;
    let mut frontmatter = false;

    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        if line_no == 1 && trimmed == "---" {
            frontmatter = true;
            continue;
        }
        if frontmatter {
            if trimmed == "---" {
                frontmatter = false;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }

        let Some((level, title)) = atx_heading(trimmed) else {
            continue;
        };
        while stack
            .last()
            .is_some_and(|(parent_level, _, _)| *parent_level >= level)
        {
            stack.pop();
        }
        let mut heading_path: Vec<String> =
            stack.iter().map(|(_, title, _)| title.clone()).collect();
        heading_path.push(title.clone());
        stack.push((level, title, headings.len()));
        headings.push(DocumentHeading {
            path: path.to_string(),
            heading_path,
            start_line: line_no,
            end_line: lines.len(),
            preview: String::new(),
        });
    }

    for i in 0..headings.len() {
        let level = heading_level(&headings[i].heading_path, &lines, headings[i].start_line);
        let end = headings
            .iter()
            .skip(i + 1)
            .find(|next| heading_level(&next.heading_path, &lines, next.start_line) <= level)
            .map(|next| next.start_line.saturating_sub(1))
            .unwrap_or(lines.len());
        headings[i].end_line = end.max(headings[i].start_line);
        headings[i].preview = preview(&lines, headings[i].start_line, headings[i].end_line);
    }
    headings
}

fn atx_heading(line: &str) -> Option<(usize, String)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) || line.as_bytes().get(hashes) != Some(&b' ') {
        return None;
    }
    let title = line[hashes..].trim().trim_end_matches('#').trim();
    (!title.is_empty()).then(|| (hashes, title.to_string()))
}

fn heading_level(path: &[String], lines: &[&str], line: usize) -> usize {
    lines
        .get(line.saturating_sub(1))
        .and_then(|line| atx_heading(line.trim()).map(|(level, _)| level))
        .unwrap_or(path.len())
}

fn preview(lines: &[&str], start_line: usize, end_line: usize) -> String {
    lines
        .iter()
        .skip(start_line)
        .take(end_line.saturating_sub(start_line))
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.chars().take(220).collect())
        .unwrap_or_default()
}
