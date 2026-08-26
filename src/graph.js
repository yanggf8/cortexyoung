import path from 'node:path';

export const CONFIDENCE_SCORE = { EXTRACTED: 1.0, INFERRED: 0.7, AMBIGUOUS: 0.5 };

export function buildImportMap(edges) {
  const map = new Map();
  for (const e of edges) if (e.rel_type === 'imports') map.set(e.raw_target, e.raw_target);
  return map;
}

// './helper' seen in 'src/alpha.ts' becomes the extension-less candidate 'src/helper'.
function importedPathPrefixes(filePath, importMap) {
  const dir = path.posix.dirname(filePath);
  const prefixes = [];
  for (const spec of importMap.keys()) {
    prefixes.push(spec.startsWith('.') ? path.posix.normalize(path.posix.join(dir, spec)) : spec);
  }
  return prefixes;
}

export function resolveTargets({ db, projectId, filePath, importMap, symbol }) {
  const all = db.prepare(
    'SELECT chunk_id, file_path FROM chunks WHERE project_id = ? AND symbol_name = ? ORDER BY chunk_id',
  ).all(projectId, symbol);
  if (all.length === 0) return [];

  const sameFile = all.filter((r) => r.file_path === filePath);
  if (sameFile.length > 0) return sameFile.map((r) => r.chunk_id);

  const prefixes = importedPathPrefixes(filePath, importMap);
  const viaImport = all.filter((r) => {
    const noExt = r.file_path.replace(/\.[^./]+$/, '');
    return prefixes.some((p) => noExt === p || noExt.endsWith(`/${p}`));
  });
  if (viaImport.length > 0) return viaImport.map((r) => r.chunk_id);

  return all.map((r) => r.chunk_id);
}

export function relationshipRowsForFile({ db, projectId, filePath, chunks, edges }) {
  const importMap = buildImportMap(edges);
  const chunkBySymbol = new Map();
  for (const c of chunks) if (c.symbol_name) chunkBySymbol.set(c.symbol_name, c.chunk_id);

  const rows = [];
  const seen = new Set();
  for (const e of edges) {
    if (e.source_symbol === null) continue;            // file-level imports have no source chunk
    const sourceChunkId = chunkBySymbol.get(e.source_symbol);
    if (!sourceChunkId) continue;

    const targets = resolveTargets({ db, projectId, filePath, importMap, symbol: e.raw_target })
      .filter((id) => id !== sourceChunkId);           // no self-edges
    if (targets.length === 0) continue;                // zero targets: no row, ever

    const n = targets.length;
    const confidence = n === 1 ? 'INFERRED' : 'AMBIGUOUS';
    const score = n === 1 ? CONFIDENCE_SCORE.INFERRED : CONFIDENCE_SCORE.AMBIGUOUS * (1 / n);
    for (const target of targets) {
      const key = `${sourceChunkId} ${target} ${e.rel_type}`;
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({
        source_chunk_id: sourceChunkId,
        target_chunk_id: target,
        rel_type: e.rel_type,
        confidence,
        confidence_score: score,
        confidence_reasoning: n === 1
          ? `resolved: ${e.raw_target}`
          : `ambiguous: ${e.raw_target} (${n} candidates)`,
      });
    }
  }
  return rows;
}

export function unresolvedInline(symbol) {
  return {
    confidence: 'AMBIGUOUS',
    confidence_score: CONFIDENCE_SCORE.AMBIGUOUS,
    confidence_reasoning: `unresolved: ${symbol}`,
  };
}

export function getNeighbors(db, chunkId, limit) {
  return db.prepare(`
    SELECT c.chunk_id, c.symbol_name, c.file_path, c.start_line, c.end_line,
           r.rel_type, r.confidence, r.confidence_score, 'outgoing' AS direction
      FROM relationships r JOIN chunks c ON c.chunk_id = r.target_chunk_id
     WHERE r.source_chunk_id = ?
    UNION ALL
    SELECT c.chunk_id, c.symbol_name, c.file_path, c.start_line, c.end_line,
           r.rel_type, r.confidence, r.confidence_score, 'incoming' AS direction
      FROM relationships r JOIN chunks c ON c.chunk_id = r.source_chunk_id
     WHERE r.target_chunk_id = ?
     ORDER BY confidence_score DESC, chunk_id
     LIMIT ?`).all(chunkId, chunkId, limit);
}

export function getTransitiveDependents(db, chunkId, depth) {
  return db.prepare(`
    WITH RECURSIVE dependents(chunk_id, hop) AS (
      SELECT r.source_chunk_id, 1 FROM relationships r WHERE r.target_chunk_id = ?
      UNION
      SELECT r.source_chunk_id, d.hop + 1
        FROM relationships r JOIN dependents d ON r.target_chunk_id = d.chunk_id
       WHERE d.hop < ?
    )
    SELECT c.chunk_id, c.symbol_name, c.file_path, c.start_line, c.end_line, MIN(d.hop) AS hop
      FROM dependents d JOIN chunks c ON c.chunk_id = d.chunk_id
     WHERE c.chunk_id != ?
     GROUP BY c.chunk_id
     ORDER BY hop, c.chunk_id`).all(chunkId, depth, chunkId);
}
