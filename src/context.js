import fs from 'node:fs';
import path from 'node:path';
import { keywordSearch } from './fts.js';
import { extractFile } from './chunker.js';
import { buildImportMap, resolveTargets, getNeighbors, unresolvedInline } from './graph.js';
import { applyBudget } from './budget.js';
import { computeStale } from './staleness.js';

export const DEFAULT_BUDGET = 1500;
export const NEIGHBORS_PER_SEED = 8;
export const CONTENT_HEAD_LINES = 12;
const MAX_SEEDS = 5;

function exactSymbolSeeds(db, projectId, query) {
  return db.prepare(`
    SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, content, language
      FROM chunks WHERE project_id = ? AND symbol_name = ? ORDER BY file_path, start_line
     LIMIT ?`).all(projectId, query, MAX_SEEDS);
}

// Recompute this seed's outgoing references and inline the ones that resolve to nothing.
// Nothing here is read from or written to a table: zero-target edges have no row by design.
function unresolvedFor({ db, bin, root, projectId, seed }) {
  const abs = path.join(root, seed.file_path);
  if (!fs.existsSync(abs)) return [];
  const source = fs.readFileSync(abs, 'utf8');
  const { edges } = extractFile({ bin, projectId, filePath: seed.file_path, absPath: abs, source });
  const importMap = buildImportMap(edges);
  const out = [];
  const seen = new Set();
  for (const e of edges) {
    if (e.source_symbol !== seed.symbol_name) continue;
    if (seen.has(e.raw_target)) continue;
    seen.add(e.raw_target);
    const targets = resolveTargets({ db, projectId, filePath: seed.file_path, importMap, symbol: e.raw_target });
    if (targets.length === 0) out.push({ symbol: e.raw_target, rel_type: e.rel_type, ...unresolvedInline(e.raw_target) });
  }
  return out;
}

export function contextCommand({ db, bin, root, projectId, query, budget = DEFAULT_BUDGET, includeAmbiguous = false, fullContent = false }) {
  let resolution = 'exact_symbol';
  let seedRows = exactSymbolSeeds(db, projectId, query);
  let truncatedQuery = false;

  if (seedRows.length === 0) {
    const { rows, truncated_query: tq } = keywordSearch(db, projectId, query, MAX_SEEDS);
    seedRows = rows;
    truncatedQuery = tq;
    resolution = rows.length > 0 ? 'fts' : 'none';
  }

  const seeds = seedRows.map((row) => {
    const neighbors = getNeighbors(db, row.chunk_id, NEIGHBORS_PER_SEED)
      .filter((n) => includeAmbiguous || n.confidence !== 'AMBIGUOUS');
    const unresolved = unresolvedFor({ db, bin, root, projectId, seed: row });
    // Seed bodies dominate packet size; keep a head by default (--content full restores).
    const lines = String(row.content ?? '').split('\n');
    const contentTruncated = !fullContent && lines.length > CONTENT_HEAD_LINES;
    const content = contentTruncated
      ? lines.slice(0, CONTENT_HEAD_LINES).join('\n') + '\n…'
      : row.content;
    return {
      chunk_id: row.chunk_id,
      file_path: row.file_path,
      symbol_name: row.symbol_name,
      chunk_type: row.chunk_type,
      start_line: row.start_line,
      end_line: row.end_line,
      content,
      content_truncated: contentTruncated,
      neighbors,
      unresolved,
    };
  });

  let { kept, truncated } = applyBudget(seeds, budget, (s) => JSON.stringify(s));
  // If the rendered packet still exceeds the budget, trim neighbours per-seed until it fits.
  // applyBudget only drops whole seeds; a single large seed (e.g. hub with 8 neighbours) would otherwise always exceed a tight budget.
  const packetTokens = (ks) => Math.ceil(JSON.stringify({ query, resolution: ks.length === 0 ? 'none' : resolution, seeds: ks, seed_count: seeds.length, truncated: true, truncated_query: truncatedQuery, index_is_stale: false }).length / 4);
  if (kept.length > 0 && packetTokens(kept) > budget * 1.15) {
    let best = kept.map((s) => ({ ...s, neighbors: [...s.neighbors] }));
    // Shrink neighbours uniformly until the packet fits or no neighbours remain.
    for (let keep = NEIGHBORS_PER_SEED - 1; keep >= 0; keep -= 1) {
      const trimmed = kept.map((s) => ({ ...s, neighbors: s.neighbors.slice(0, keep) }));
      if (packetTokens(trimmed) <= budget * 1.15) { best = trimmed; break; }
      best = trimmed;
    }
    kept = best;
    truncated = true;
  }
  const { index_is_stale: stale } = computeStale({ db, bin, root, projectId });

  return {
    query,
    resolution: kept.length === 0 ? 'none' : resolution,
    seeds: kept,
    seed_count: seeds.length,
    truncated,
    truncated_query: truncatedQuery,
    index_is_stale: stale,
  };
}
