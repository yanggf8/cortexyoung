import { CortError } from './errors.js';

export const MAX_OR_TERMS = 20;

export function sanitizeFtsQuery(raw) {
  const terms = String(raw ?? '').trim().split(/\s+/).filter((t) => t.length > 0);
  if (terms.length === 0) throw new CortError('empty_query', { raw });
  const truncated = terms.length > MAX_OR_TERMS;
  const kept = terms.slice(0, MAX_OR_TERMS);
  const quoted = kept.map((t) => `"${t.replaceAll('"', '""')}"`);
  return { query: quoted.join(' OR '), truncated_query: truncated };
}

export function keywordSearch(db, projectId, raw, limit) {
  const { query, truncated_query: truncatedQuery } = sanitizeFtsQuery(raw);
  let rows;
  try {
    rows = db.prepare(`
      SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type, c.start_line, c.end_line,
             c.content, c.language, c.chunk_source, bm25(chunks_fts) AS score
        FROM chunks_fts
        JOIN chunks c ON c.rowid = chunks_fts.rowid
       WHERE chunks_fts MATCH ? AND c.project_id = ?
       ORDER BY score
       LIMIT ?`).all(query, projectId, limit);
  } catch (err) {
    // No embedding arm exists to degrade into, so a broken query must be loud.
    throw new CortError('fts_query_failed', { query, message: String(err && err.message) });
  }
  return { rows, truncated_query: truncatedQuery };
}
