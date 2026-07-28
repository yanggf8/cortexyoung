// Louvain community detection on the file-level relationship graph.
// Pure TypeScript — no external dependencies.

export interface ModuleInfo {
  id: number;
  label: string;
  file_count: number;
  files: string[];
  key_symbols: string[];
  hub_files: string[];
}

export interface ClusterResult {
  module_count: number;
  algorithm: 'louvain';
  modules: ModuleInfo[];
}

type AdjMap = Map<string, Map<string, number>>;

export function clusterProject(
  chunks: Array<{ chunk_id: string; file_path: string; symbol_name: string | null; chunk_type: string | null }>,
  relationships: Array<{ source_chunk_id: string; target_chunk_id: string; confidence_score: number | null }>,
): ClusterResult {
  // chunk → file mapping + per-file symbol list
  const chunkToFile = new Map<string, string>();
  const fileSymbols = new Map<string, string[]>();

  for (const chunk of chunks) {
    chunkToFile.set(chunk.chunk_id, chunk.file_path);
    if (!fileSymbols.has(chunk.file_path)) fileSymbols.set(chunk.file_path, []);
    if (chunk.symbol_name) fileSymbols.get(chunk.file_path)!.push(chunk.symbol_name);
  }

  const allFiles = [...new Set(chunks.map(c => c.file_path))];

  // Build undirected file-level adjacency, weighted by relationship count × confidence
  const adj: AdjMap = new Map();
  for (const f of allFiles) adj.set(f, new Map());

  for (const rel of relationships) {
    const src = chunkToFile.get(rel.source_chunk_id);
    const tgt = chunkToFile.get(rel.target_chunk_id);
    if (!src || !tgt || src === tgt) continue;
    const w = rel.confidence_score ?? 0.5;

    const srcMap = adj.get(src)!;
    srcMap.set(tgt, (srcMap.get(tgt) ?? 0) + w);

    const tgtMap = adj.get(tgt)!;
    tgtMap.set(src, (tgtMap.get(src) ?? 0) + w);
  }

  // Degree per file and total edge weight m (undirected: sum/2)
  const degrees = new Map<string, number>();
  let m = 0;
  for (const [file, neighbors] of adj) {
    let deg = 0;
    for (const w of neighbors.values()) deg += w;
    degrees.set(file, deg);
    m += deg;
  }
  m /= 2;

  const assignments = louvain(allFiles, adj, degrees, m);

  // Group files by community
  const communities = new Map<number, string[]>();
  for (const [file, cid] of assignments) {
    if (!communities.has(cid)) communities.set(cid, []);
    communities.get(cid)!.push(file);
  }

  const modules: ModuleInfo[] = [];
  let moduleId = 0;

  for (const [, files] of [...communities.entries()].sort((a, b) => b[1].length - a[1].length)) {
    const label = inferLabel(files);

    // Hub files = highest total degree within this module's subgraph
    const filesByDegree = files
      .map(f => ({ file: f, degree: degrees.get(f) ?? 0 }))
      .sort((a, b) => b.degree - a.degree);
    const hubFiles = filesByDegree.slice(0, 3).map(fd => fd.file);

    // Key symbols: top symbols from the most-connected files
    const keySymbols: string[] = [];
    for (const f of hubFiles) {
      for (const sym of fileSymbols.get(f) ?? []) {
        keySymbols.push(sym);
        if (keySymbols.length >= 6) break;
      }
      if (keySymbols.length >= 6) break;
    }

    modules.push({
      id: moduleId++,
      label,
      file_count: files.length,
      files: files.sort(),
      key_symbols: [...new Set(keySymbols)].slice(0, 5),
      hub_files: hubFiles,
    });
  }

  return { module_count: modules.length, algorithm: 'louvain', modules };
}

// ── Louvain Phase 1 ──────────────────────────────────────────────────────────
// Greedy modularity optimisation on an undirected weighted graph.
// Blondel et al. 2008, "Fast unfolding of communities in large networks."

function louvain(
  nodes: string[],
  adj: AdjMap,
  degrees: Map<string, number>,
  m: number,
): Map<string, number> {
  if (nodes.length === 0) return new Map();
  if (m === 0) return new Map(nodes.map((n, i) => [n, i]));

  // Start: each node is its own singleton community
  const comm = new Map<string, number>(nodes.map((n, i) => [n, i]));
  const totDeg = new Map<number, number>(nodes.map((n, i) => [i, degrees.get(n) ?? 0]));

  let improved = true;
  const MAX_ITER = 50;
  let iter = 0;

  while (improved && iter++ < MAX_ITER) {
    improved = false;

    for (const node of nodes) {
      const ki = degrees.get(node) ?? 0;
      if (ki === 0) continue;

      const ci = comm.get(node)!;

      // Weight from node to each adjacent community
      const neighborW = new Map<number, number>();
      for (const [neighbor, w] of adj.get(node) ?? []) {
        const cn = comm.get(neighbor)!;
        neighborW.set(cn, (neighborW.get(cn) ?? 0) + w);
      }

      // Temporarily remove node from ci
      totDeg.set(ci, (totDeg.get(ci) ?? 0) - ki);

      // ΔQ(move i → C) = k_{i,C}/m  − k_i·∑tot(C)/(2m²)
      // Baseline: isolation (ΔQ = 0); bestC tracks which community wins.
      let bestC = ci;
      let bestGain = 0;

      const candidates = new Set([ci, ...neighborW.keys()]);
      for (const c of candidates) {
        const kiC = neighborW.get(c) ?? 0;
        const tot = totDeg.get(c) ?? 0;
        const gain = kiC / m - (ki * tot) / (2 * m * m);
        if (gain > bestGain) { bestGain = gain; bestC = c; }
      }

      // If no community beats isolation, keep node in ci (original singleton or current cluster)
      if (bestGain <= 0) bestC = ci;

      comm.set(node, bestC);
      totDeg.set(bestC, (totDeg.get(bestC) ?? 0) + ki);

      if (bestC !== ci) improved = true;
    }
  }

  // Renumber community IDs 0…N-1
  const remap = new Map<number, number>();
  let next = 0;
  const result = new Map<string, number>();
  for (const n of nodes) {
    const c = comm.get(n)!;
    if (!remap.has(c)) remap.set(c, next++);
    result.set(n, remap.get(c)!);
  }
  return result;
}

// ── Module labelling ──────────────────────────────────────────────────────────

const GENERIC_DIRS = new Set(['src', 'lib', 'app', 'packages', 'modules', 'core', '']);

function inferLabel(files: string[]): string {
  if (files.length === 0) return 'unknown';
  if (files.length === 1) {
    const parts = files[0].split('/');
    return parts[parts.length - 1].replace(/\.\w+$/, '') || 'unknown';
  }

  // Common directory segments across all files
  const dirs = files.map(f => f.split('/').slice(0, -1).filter(Boolean));
  let common: string[] = dirs[0] ?? [];
  for (const d of dirs.slice(1)) {
    const keep: string[] = [];
    for (let i = 0; i < Math.min(common.length, d.length); i++) {
      if (common[i] === d[i]) keep.push(common[i]);
      else break;
    }
    common = keep;
  }

  // Return the deepest segment that isn't too generic
  for (let i = common.length - 1; i >= 0; i--) {
    if (!GENERIC_DIRS.has(common[i])) return common[i];
  }

  // Fallback: last segment of the common prefix (even if generic)
  if (common.length > 0) return common[common.length - 1];

  // No common directory at all: use the name of the most common parent dir
  const dirCounts = new Map<string, number>();
  for (const parts of dirs) {
    const dir = parts[parts.length - 1] ?? '';
    if (dir) dirCounts.set(dir, (dirCounts.get(dir) ?? 0) + 1);
  }
  let topDir = '';
  let topCount = 0;
  for (const [d, count] of dirCounts) {
    if (count > topCount) { topCount = count; topDir = d; }
  }
  return topDir || files[0].split('/').pop()?.replace(/\.\w+$/, '') || 'unknown';
}
