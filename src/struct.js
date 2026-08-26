import { execAstGrep } from './ast-grep.js';
import { parseScanStream } from './chunker.js';
import { getNeighbors } from './graph.js';
import { computeStale } from './staleness.js';
import { applyBudget } from './budget.js';
import { CortError } from './errors.js';

export const MAX_MALFORMED_RATIO = 0.10;
export const MAX_NEIGHBORS = 3;
export const UNBOUNDED_SCAN_FILE_LIMIT = 2000;

const ERROR_NODE_MARKER = 'Pattern contains an ERROR node';

export function preflightPattern({ bin, pattern, lang, paths }) {
  const r = execAstGrep(bin, ['run', '--debug-query=ast', '--lang', lang, '-p', pattern, ...paths]);
  if (r.code === 2) {
    throw new CortError('parse_failed', { pattern, lang, detail: r.stderr.trim() });
  }
  if (r.stderr.includes(ERROR_NODE_MARKER)) {
    throw new CortError('parse_failed', { pattern, lang, detail: r.stderr.trim() });
  }
}

export function runPattern({ bin, pattern, lang, paths, rewrite, skipPreflight = false }) {
  if (!skipPreflight) preflightPattern({ bin, pattern, lang, paths });

  const args = ['run', '--json=stream', '--strictness', 'ast', '--lang', lang, '-p', pattern];
  if (rewrite !== undefined) args.push('--rewrite', rewrite);
  args.push(...paths);

  const r = execAstGrep(bin, args);
  // Post-pre-flight semantics: exit 0 means hits; exit 1 with both streams empty means a
  // genuine zero-hit run; a non-empty stderr means something operational went wrong.
  if (r.code !== 0 && r.stdout.length === 0 && r.stderr.trim().length > 0) {
    throw new CortError('ast_grep_run_failed', { code: r.code, detail: r.stderr.trim() });
  }

  const { records, malformed, total } = parseScanStream(r.stdout);
  if (total > 0 && malformed / total > MAX_MALFORMED_RATIO) {
    throw new CortError('run_aborted_malformed', { malformed, total, ratio: MAX_MALFORMED_RATIO });
  }

  const matches = records.map((rec) => ({
    file: rec.file,
    text: rec.text,
    start_line: rec.range.start.line + 1,
    end_line: rec.range.end.line + 1,
    replacement: rec.replacement,
  }));
  return { matches, malformed, total };
}

export function containmentJoin(db, projectId, match) {
  return db.prepare(`
    SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, language
      FROM chunks
     WHERE project_id = ? AND file_path = ? AND start_line <= ? AND end_line >= ?
     ORDER BY (end_line - start_line) ASC, start_line DESC
     LIMIT 1`).get(projectId, match.file_path, match.start_line, match.end_line) ?? null;
}

export function structCommand({ db, bin, root, projectId, pattern, lang, globs, budget, fileLimit = UNBOUNDED_SCAN_FILE_LIMIT }) {
  if (globs.length === 0) {
    const indexedFiles = db.prepare('SELECT COUNT(*) c FROM file_state WHERE project_id = ?').get(projectId).c;
    if (indexedFiles > fileLimit) {
      throw new CortError('scan_too_broad', {
        indexed_files: indexedFiles,
        limit: fileLimit,
        hint: "narrow the scan with -g '<glob>', e.g. cort struct -p '<pattern>' --lang ts -g 'src/**/*.ts'",
      });
    }
  }
  const paths = globs.length > 0 ? globs : [root];
  const { matches, malformed } = runPattern({ bin, pattern, lang, paths });

  const enriched = matches.map((m) => {
    const filePath = m.file.startsWith(root)
      ? m.file.slice(root.length + 1).split('\\').join('/')
      : m.file;
    const chunk = containmentJoin(db, projectId, { ...m, file_path: filePath });
    const neighbors = chunk
      ? getNeighbors(db, chunk.chunk_id, MAX_NEIGHBORS)
        .filter((n) => n.confidence === 'EXTRACTED' || n.confidence === 'INFERRED')
        .slice(0, MAX_NEIGHBORS)
      : [];
    return {
      file_path: filePath,
      start_line: m.start_line,
      end_line: m.end_line,
      text: m.text,
      chunk_id: chunk ? chunk.chunk_id : null,
      symbol_name: chunk ? chunk.symbol_name : null,
      chunk_type: chunk ? chunk.chunk_type : null,
      neighbors,
    };
  });

  const { kept, truncated } = applyBudget(enriched, budget, (m) => JSON.stringify(m));
  const { index_is_stale: stale } = computeStale({ db, bin, root, projectId });

  return {
    pattern, lang,
    matches: kept,
    match_count: enriched.length,
    malformed_lines: malformed,
    truncated,
    index_is_stale: stale,
  };
}
