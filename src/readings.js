import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { CortError } from './errors.js';
import { sanitizeFtsQuery } from './fts.js';

export const DEFAULT_RECALL_LIMIT = 5;
export const RECALL_HEAD_LINES = 12;

function sourceHash(source) {
  return createHash('sha256').update(source).digest('hex');
}

function indexedProject(db, projectId) {
  return db.prepare('SELECT 1 FROM projects WHERE project_id = ? AND last_indexed_at IS NOT NULL')
    .get(projectId) !== undefined;
}

function requireIndexed(db, projectId) {
  if (!indexedProject(db, projectId)) {
    throw new CortError('project_not_indexed', { hint: 'run cort index first' });
  }
}

function resolveProjectFile(root, requestedPath) {
  if (typeof requestedPath !== 'string' || requestedPath.length === 0) {
    throw new CortError('missing_file', { hint: 'cort read <file> [--start <line>] [--end <line>]' });
  }
  const candidate = path.resolve(root, requestedPath);
  let abs;
  try { abs = fs.realpathSync(candidate); } catch {
    throw new CortError('file_not_found', { file_path: requestedPath });
  }
  const relative = path.relative(root, abs);
  if (relative === '' || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new CortError('path_outside_project', { file_path: requestedPath });
  }
  if (!fs.statSync(abs).isFile()) throw new CortError('not_a_file', { file_path: requestedPath });
  return { abs, rel: relative.split(path.sep).join('/') };
}

function positiveLine(value, name, fallback) {
  if (value === undefined || value === null) return fallback;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    throw new CortError('invalid_line_range', { [name]: value });
  }
  return parsed;
}

function statMatches(note, stat) {
  return note.source_size === stat.size && note.source_mtime_ms === stat.mtimeMs;
}

function sliceStored(note, startLine, endLine) {
  return note.content.split('\n')
    .slice(startLine - note.start_line, endLine - note.start_line + 1)
    .join('\n');
}

function noteResult(note, content, startLine, endLine, source) {
  return {
    file_path: note.file_path,
    start_line: startLine,
    end_line: endLine,
    content,
    source,
    read_count: note.read_count,
  };
}

export function readFragment({ db, root, projectId, filePath, startLine, endLine }) {
  requireIndexed(db, projectId);
  const { abs, rel } = resolveProjectFile(root, filePath);
  const start = positiveLine(startLine, 'start', 1);
  const requestedEnd = positiveLine(endLine, 'end', null);
  if (requestedEnd !== null && requestedEnd < start) {
    throw new CortError('invalid_line_range', { start, end: requestedEnd });
  }

  const stat = fs.statSync(abs);
  const notes = db.prepare(`SELECT * FROM reading_notes
    WHERE project_id = ? AND file_path = ? ORDER BY (end_line - start_line), start_line`)
    .all(projectId, rel);
  let covering = requestedEnd === null
    ? notes.find((n) => n.start_line <= start && n.ends_at_eof === 1)
    : notes.find((n) => n.start_line <= start && n.end_line >= requestedEnd);

  if (covering && statMatches(covering, stat)) {
    const end = requestedEnd ?? covering.end_line;
    db.prepare('UPDATE reading_notes SET read_count = read_count + 1, last_read_at = ? WHERE reading_id = ?')
      .run(Date.now(), covering.reading_id);
    covering = { ...covering, read_count: covering.read_count + 1 };
    return noteResult(covering, sliceStored(covering, start, end), start, end, 'store');
  }

  const source = fs.readFileSync(abs, 'utf8');
  const hash = sourceHash(source);
  const lines = source.split('\n');
  const end = requestedEnd ?? lines.length;
  if (start > lines.length || end > lines.length) {
    throw new CortError('invalid_line_range', { start, end, file_lines: lines.length });
  }

  if (notes.length > 0 && notes[0].source_hash !== hash) {
    db.prepare('DELETE FROM reading_notes WHERE project_id = ? AND file_path = ?').run(projectId, rel);
    covering = undefined;
  } else if (notes.length > 0) {
    db.prepare(`UPDATE reading_notes SET source_mtime_ms = ?, source_size = ?
      WHERE project_id = ? AND file_path = ?`).run(stat.mtimeMs, stat.size, projectId, rel);
    covering = requestedEnd === null
      ? notes.find((n) => n.start_line <= start && n.ends_at_eof === 1)
      : notes.find((n) => n.start_line <= start && n.end_line >= end);
  }

  if (covering) {
    db.prepare('UPDATE reading_notes SET read_count = read_count + 1, last_read_at = ? WHERE reading_id = ?')
      .run(Date.now(), covering.reading_id);
    const refreshed = { ...covering, read_count: covering.read_count + 1 };
    return noteResult(refreshed, sliceStored(refreshed, start, end), start, end, 'store');
  }

  const content = lines.slice(start - 1, end).join('\n');
  const now = Date.now();
  db.prepare(`INSERT INTO reading_notes
    (project_id, file_path, start_line, end_line, ends_at_eof, content, source_hash,
     source_mtime_ms, source_size, read_count, first_read_at, last_read_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)
    ON CONFLICT(project_id, file_path, start_line, end_line) DO UPDATE SET
      ends_at_eof = excluded.ends_at_eof, content = excluded.content, source_hash = excluded.source_hash,
      source_mtime_ms = excluded.source_mtime_ms, source_size = excluded.source_size,
      read_count = reading_notes.read_count + 1, last_read_at = excluded.last_read_at`)
    .run(projectId, rel, start, end, requestedEnd === null ? 1 : 0,
      content, hash, stat.mtimeMs, stat.size, now, now);
  const inserted = db.prepare(`SELECT * FROM reading_notes
    WHERE project_id = ? AND file_path = ? AND start_line = ? AND end_line = ?`)
    .get(projectId, rel, start, end);
  return noteResult(inserted, content, start, end, 'filesystem');
}

function trimContent(row, fullContent) {
  const lines = row.content.split('\n');
  const truncated = !fullContent && lines.length > RECALL_HEAD_LINES;
  return {
    file_path: row.file_path,
    start_line: row.start_line,
    end_line: row.end_line,
    content: truncated ? `${lines.slice(0, RECALL_HEAD_LINES).join('\n')}\n…` : row.content,
    content_truncated: truncated,
    read_count: row.read_count,
    last_read_at: row.last_read_at,
  };
}

export function recallReadings({ db, root, projectId, query, limit = DEFAULT_RECALL_LIMIT, fullContent = false }) {
  requireIndexed(db, projectId);
  const parsedLimit = Number(limit);
  if (!Number.isSafeInteger(parsedLimit) || parsedLimit < 1 || parsedLimit > 100) {
    throw new CortError('invalid_limit', { limit });
  }
  const { query: ftsQuery, truncated_query: truncatedQuery } = sanitizeFtsQuery(query);
  let candidates;
  try {
    candidates = db.prepare(`SELECT n.*, bm25(reading_notes_fts) AS score
      FROM reading_notes_fts
      JOIN reading_notes n ON n.reading_id = reading_notes_fts.rowid
      WHERE reading_notes_fts MATCH ? AND n.project_id = ?
      ORDER BY score LIMIT ?`).all(ftsQuery, projectId, parsedLimit * 4);
  } catch (err) {
    throw new CortError('fts_query_failed', { query: ftsQuery, message: String(err && err.message) });
  }

  const checked = new Map();
  const staleFiles = new Set();
  const results = [];
  for (const row of candidates) {
    if (results.length >= parsedLimit || staleFiles.has(row.file_path)) continue;
    let state = checked.get(row.file_path);
    if (!state) {
      const abs = path.join(root, row.file_path);
      try {
        const stat = fs.statSync(abs);
        if (!stat.isFile()) throw new Error('not file');
        state = statMatches(row, stat)
          ? { valid: true, stat }
          : { valid: sourceHash(fs.readFileSync(abs, 'utf8')) === row.source_hash, stat };
      } catch { state = { valid: false }; }
      checked.set(row.file_path, state);
    }
    if (!state.valid) {
      staleFiles.add(row.file_path);
      db.prepare('DELETE FROM reading_notes WHERE project_id = ? AND file_path = ?')
        .run(projectId, row.file_path);
      continue;
    }
    if (!statMatches(row, state.stat)) {
      db.prepare(`UPDATE reading_notes SET source_mtime_ms = ?, source_size = ?
        WHERE project_id = ? AND file_path = ?`)
        .run(state.stat.mtimeMs, state.stat.size, projectId, row.file_path);
    }
    results.push(trimContent(row, fullContent));
  }
  return { query, readings: results, reading_count: results.length, truncated_query: truncatedQuery };
}
