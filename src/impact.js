import fs from 'node:fs';
import path from 'node:path';
import { extractFile } from './chunker.js';
import { buildImportMap, resolveTargets, getTransitiveDependents, unresolvedInline } from './graph.js';
import { computeStale } from './staleness.js';

export const DEFAULT_DEPTH = 3;

export function impactCommand({ db, bin, root, projectId, symbol, depth = DEFAULT_DEPTH }) {
  // --symbol accepts a comma-separated batch: one call answers multi-symbol questions.
  const names = String(symbol ?? '').split(',').map((s) => s.trim()).filter(Boolean);
  const seeds = names.length === 0 ? [] : db.prepare(`
    SELECT chunk_id, file_path, symbol_name, start_line, end_line
      FROM chunks WHERE project_id = ? AND symbol_name IN (${names.map(() => '?').join(',')})
     ORDER BY file_path, start_line`)
    .all(projectId, ...names);

  const merged = new Map();
  for (const seed of seeds) {
    for (const dep of getTransitiveDependents(db, seed.chunk_id, depth)) {
      const prev = merged.get(dep.chunk_id);
      if (!prev || dep.hop < prev.hop) merged.set(dep.chunk_id, dep);
    }
  }
  const seedIds = new Set(seeds.map((s) => s.chunk_id));
  const dependents = [...merged.values()]
    .filter((d) => !seedIds.has(d.chunk_id))
    .sort((a, b) => a.hop - b.hop || a.chunk_id.localeCompare(b.chunk_id));

  // Unresolved outgoing references, computed on the fly; never stored, never FK-bearing.
  const unresolved = [];
  const seenSymbols = new Set();
  for (const seed of seeds) {
    const abs = path.join(root, seed.file_path);
    if (!fs.existsSync(abs)) continue;
    const source = fs.readFileSync(abs, 'utf8');
    const { edges } = extractFile({ bin, projectId, filePath: seed.file_path, absPath: abs, source });
    const importMap = buildImportMap(edges);
    for (const e of edges) {
      if (e.source_symbol !== seed.symbol_name) continue;
      if (seenSymbols.has(e.raw_target)) continue;
      seenSymbols.add(e.raw_target);
      const targets = resolveTargets({ db, projectId, filePath: seed.file_path, importMap, symbol: e.raw_target });
      if (targets.length === 0) {
        unresolved.push({ symbol: e.raw_target, rel_type: e.rel_type, ...unresolvedInline(e.raw_target) });
      }
    }
  }

  const { index_is_stale: stale } = computeStale({ db, bin, root, projectId });
  return {
    symbol,
    depth,
    seed_count: seeds.length,
    seeds: seeds.map((s) => ({ chunk_id: s.chunk_id, file_path: s.file_path, start_line: s.start_line })),
    dependents,
    dependent_count: dependents.length,
    unresolved,
    index_is_stale: stale,
  };
}
