import { createHash } from 'node:crypto';
import { execAstGrep } from './ast-grep.js';
import { CortError } from './errors.js';
import { SGCONFIG } from './pack.js';

const CHUNK_TAG = 'chunk:';
const EDGE_TAG = 'edge:';

export function chunkIdFor(projectId, filePath, startLine) {
  return `${projectId}:${filePath}:${startLine}`;
}

export function parseScanStream(stdout) {
  const lines = stdout.split('\n').filter((l) => l.trim().length > 0);
  const records = [];
  let malformed = 0;
  for (const line of lines) {
    try { records.push(JSON.parse(line)); } catch { malformed += 1; }
  }
  return { records, malformed, total: lines.length };
}

export function edgeString(edge) {
  return `${edge.rel_type}\t${edge.source_symbol ?? ''}\t${edge.raw_target}`;
}

export function fileContentHash(chunks, edges) {
  const h = createHash('sha256');
  for (const c of [...chunks].sort((a, b) => a.start_line - b.start_line)) h.update(c.content);
  for (const s of edges.map(edgeString).sort()) h.update(s);
  return h.digest('hex');
}

function unquote(text) {
  const t = text.trim();
  if (t.length >= 2 && (t[0] === "'" || t[0] === '"' || t[0] === '`') && t[t.length - 1] === t[0]) {
    return t.slice(1, -1);
  }
  return t;
}

function unparsedResult({ projectId, filePath, source, malformed }) {
  const chunk = {
    chunk_id: chunkIdFor(projectId, filePath, 1),
    project_id: projectId,
    file_path: filePath,
    symbol_name: null,
    chunk_type: 'unparsed',
    start_line: 1,
    end_line: Math.max(1, source.split('\n').length),
    content: source,
    content_hash: createHash('sha256').update(source).digest('hex'),
    language: null,
    chunk_source: 'unparsed',
  };
  return {
    chunks: [chunk], edges: [],
    file_content_hash: fileContentHash([chunk], []),
    unparsed: true, malformed,
  };
}

export function extractFile({ bin, projectId, filePath, absPath, source, timeoutMs }) {
  let r;
  try {
    r = execAstGrep(bin, ['scan', '--json=stream', '--config', SGCONFIG, absPath], { timeoutMs });
  } catch (err) {
    // A per-file scan timeout (e.g. a huge minified bundle) degrades that file to
    // unparsed — the index must not abort on one file. Environment-wide spawn
    // failures stay loud.
    if (err instanceof CortError && err.code === 'ast_grep_timeout') {
      return unparsedResult({ projectId, filePath, source, malformed: 0 });
    }
    throw err;
  }
  if (r.code !== 0) return unparsedResult({ projectId, filePath, source, malformed: 0 });

  const { records, malformed } = parseScanStream(r.stdout);
  if (records.length === 0) return unparsedResult({ projectId, filePath, source, malformed });

  const chunks = [];
  const rawEdges = [];
  for (const rec of records) {
    const tag = rec.message ?? '';
    const startLine = rec.range.start.line + 1;
    const endLine = rec.range.end.line + 1;
    if (tag.startsWith(CHUNK_TAG)) {
      chunks.push({
        chunk_id: chunkIdFor(projectId, filePath, startLine),
        project_id: projectId,
        file_path: filePath,
        symbol_name: rec.metaVariables?.single?.NAME?.text ?? null,
        chunk_type: tag.slice(CHUNK_TAG.length),
        start_line: startLine,
        end_line: endLine,
        content: rec.text,
        content_hash: createHash('sha256').update(rec.text).digest('hex'),
        language: rec.language ?? null,
        chunk_source: 'ast',
      });
    } else if (tag.startsWith(EDGE_TAG)) {
      const single = rec.metaVariables?.single ?? {};
      const target = single.SRC?.text ?? single.CALLEE?.text ?? null;
      if (target !== null) {
        rawEdges.push({ rel_type: tag.slice(EDGE_TAG.length), raw_target: unquote(target), start_line: startLine });
      }
    }
  }

  // ast-grep emits records unordered; sort so chunk_id collisions resolve deterministically.
  chunks.sort((a, b) => a.start_line - b.start_line || a.end_line - b.end_line);
  const byId = new Map();
  for (const c of chunks) if (!byId.has(c.chunk_id)) byId.set(c.chunk_id, c);
  const deduped = [...byId.values()];

  const edges = rawEdges.map((e) => {
    const containing = deduped
      .filter((c) => c.start_line <= e.start_line && e.start_line <= c.end_line)
      .sort((a, b) => (a.end_line - a.start_line) - (b.end_line - b.start_line))[0];
    return { rel_type: e.rel_type, source_symbol: containing?.symbol_name ?? null, raw_target: e.raw_target, start_line: e.start_line };
  });

  return {
    chunks: deduped, edges,
    file_content_hash: fileContentHash(deduped, edges),
    unparsed: false, malformed,
  };
}
