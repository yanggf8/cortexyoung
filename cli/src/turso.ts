import { createClient, type Client } from '@libsql/client';
import type { CortexConfig } from './config.js';

let client: Client | null = null;

export function getClient(config: CortexConfig): Client {
  if (!client) {
    client = createClient({
      url: config.turso_url,
      authToken: config.turso_auth_token,
    });
  }
  return client;
}

// Split SQL respecting BEGIN...END blocks (triggers contain inner semicolons)
function splitStatements(sql: string): string[] {
  const statements: string[] = [];
  let current = '';
  let inBlock = false;

  for (const line of sql.split('\n')) {
    const trimmed = line.trim().toUpperCase();
    current += line + '\n';

    if (trimmed.startsWith('CREATE TRIGGER') || trimmed.includes(' BEGIN')) {
      inBlock = true;
    }

    if (inBlock && trimmed === 'END;') {
      statements.push(current.trim().replace(/;$/, ''));
      current = '';
      inBlock = false;
      continue;
    }

    if (!inBlock && current.trimEnd().endsWith(';')) {
      const stmt = current.trim().replace(/;$/, '');
      if (stmt.length > 0) statements.push(stmt);
      current = '';
    }
  }

  const remaining = current.trim().replace(/;$/, '');
  if (remaining.length > 0) statements.push(remaining);
  return statements;
}

const SCHEMA_SQL = `
CREATE TABLE IF NOT EXISTS chunks (
  chunk_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  file_path TEXT NOT NULL,
  symbol_name TEXT,
  chunk_type TEXT,
  start_line INTEGER,
  end_line INTEGER,
  content TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  language TEXT,
  embedding F32_BLOB(384),
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_chunks_project ON chunks(project_id);
CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(project_id, file_path);
CREATE INDEX IF NOT EXISTS idx_chunks_language ON chunks(project_id, language);

CREATE TRIGGER IF NOT EXISTS chunks_updated_at AFTER UPDATE ON chunks FOR EACH ROW BEGIN
  UPDATE chunks SET updated_at = datetime('now') WHERE chunk_id = NEW.chunk_id;
END;

CREATE TABLE IF NOT EXISTS relationships (
  source_chunk_id TEXT NOT NULL,
  target_chunk_id TEXT NOT NULL,
  rel_type TEXT NOT NULL,
  confidence TEXT DEFAULT 'EXTRACTED',
  FOREIGN KEY (source_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  FOREIGN KEY (target_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);

CREATE TABLE IF NOT EXISTS projects (
  project_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT,
  last_indexed TEXT,
  created_at TEXT DEFAULT (datetime('now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
  content, symbol_name, file_path,
  content=chunks, content_rowid=rowid
);

CREATE TRIGGER IF NOT EXISTS chunks_fts_insert AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_delete AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES ('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_update AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, content, symbol_name, file_path)
    VALUES ('delete', OLD.rowid, OLD.content, OLD.symbol_name, OLD.file_path);
  INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
    VALUES (NEW.rowid, NEW.content, NEW.symbol_name, NEW.file_path);
END;

CREATE INDEX IF NOT EXISTS idx_chunks_embedding ON chunks(libsql_vector_idx(embedding));
`;

export async function applySchema(config: CortexConfig): Promise<void> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');
  const statements = splitStatements(SCHEMA_SQL);
  for (const stmt of statements) {
    await db.execute(stmt);
  }
  // Migration: add confidence column to existing relationships tables
  try {
    await db.execute(`ALTER TABLE relationships ADD COLUMN confidence TEXT DEFAULT 'EXTRACTED'`);
  } catch {
    // Column already exists — ignore
  }
  // Migration: add grammar_version to projects table
  try {
    await db.execute(`ALTER TABLE projects ADD COLUMN grammar_version TEXT`);
  } catch {
    // Column already exists — ignore
  }
  // Migration: add chunk_source to chunks table
  try {
    await db.execute(`ALTER TABLE chunks ADD COLUMN chunk_source TEXT DEFAULT 'regex'`);
  } catch {
    // Column already exists — ignore
  }
  // Migration: add git_head to projects table
  try {
    await db.execute(`ALTER TABLE projects ADD COLUMN git_head TEXT`);
  } catch {
    // Column already exists — ignore
  }
  // Migration: add last_indexed_at (epoch ms) to projects table
  try {
    await db.execute(`ALTER TABLE projects ADD COLUMN last_indexed_at INTEGER`);
  } catch {
    // Column already exists — ignore
  }
  // Migration: add confidence_score and confidence_reasoning to relationships table
  try {
    await db.execute(`ALTER TABLE relationships ADD COLUMN confidence_score REAL`);
  } catch {
    // Column already exists — ignore
  }
  try {
    await db.execute(`ALTER TABLE relationships ADD COLUMN confidence_reasoning TEXT`);
  } catch {
    // Column already exists — ignore
  }
}

export interface ChunkRow {
  chunk_id: string;
  project_id: string;
  file_path: string;
  symbol_name?: string;
  chunk_type?: string;
  start_line: number;
  end_line: number;
  content: string;
  content_hash: string;
  language?: string;
  embedding?: number[];
  chunk_source?: string;
}

export type RelationshipConfidence = 'EXTRACTED' | 'INFERRED' | 'AMBIGUOUS';

export interface RelationshipRow {
  source_chunk_id: string;
  target_chunk_id: string;
  rel_type: string;
  confidence: RelationshipConfidence;
  confidence_score?: number;
  confidence_reasoning?: string;
}

const BATCH_SIZE = 50;

export async function upsertChunks(config: CortexConfig, chunks: ChunkRow[]): Promise<void> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');

  // Batch using db.batch() for single round-trip
  for (let i = 0; i < chunks.length; i += BATCH_SIZE) {
    const batch = chunks.slice(i, i + BATCH_SIZE);
    await db.batch(
      batch.map(c => ({
        sql: `INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type, start_line, end_line, content, content_hash, language, embedding, chunk_source)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, vector(?), ?)
              ON CONFLICT(chunk_id) DO UPDATE SET
                project_id = excluded.project_id,
                file_path = excluded.file_path,
                symbol_name = excluded.symbol_name,
                chunk_type = excluded.chunk_type,
                start_line = excluded.start_line,
                end_line = excluded.end_line,
                content = excluded.content,
                content_hash = excluded.content_hash,
                language = excluded.language,
                embedding = excluded.embedding,
                chunk_source = excluded.chunk_source`,
        args: [
          c.chunk_id, c.project_id, c.file_path,
          c.symbol_name ?? null, c.chunk_type ?? null,
          c.start_line, c.end_line, c.content, c.content_hash,
          c.language ?? null,
          c.embedding ? JSON.stringify(c.embedding) : null,
          c.chunk_source ?? 'regex',
        ],
      })),
      'write'
    );
  }
}

export async function upsertRelationships(config: CortexConfig, rels: RelationshipRow[]): Promise<void> {
  if (rels.length === 0) return;
  const db = getClient(config);
  await db.batch(
    rels.map(r => ({
      sql: `INSERT OR IGNORE INTO relationships (source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score, confidence_reasoning) VALUES (?, ?, ?, ?, ?, ?)`,
      args: [r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, r.confidence_score ?? null, r.confidence_reasoning ?? null],
    })),
    'write'
  );
}

export interface UpsertProjectOptions {
  grammarVersion?: string;
  gitHead?: string;
  /** Unix epoch milliseconds; defaults to Date.now() when omitted */
  lastIndexedAt?: number;
}

export async function upsertProject(
  config: CortexConfig,
  projectId: string,
  name: string,
  path?: string,
  options: UpsertProjectOptions = {}
): Promise<void> {
  const db = getClient(config);
  const lastIndexedAt = options.lastIndexedAt ?? Date.now();
  await db.execute({
    sql: `INSERT INTO projects (project_id, name, path, last_indexed, last_indexed_at, grammar_version, git_head)
          VALUES (?, ?, ?, datetime('now'), ?, ?, ?)
          ON CONFLICT(project_id) DO UPDATE SET
            name = excluded.name,
            path = excluded.path,
            last_indexed = datetime('now'),
            last_indexed_at = excluded.last_indexed_at,
            grammar_version = excluded.grammar_version,
            git_head = excluded.git_head`,
    args: [
      projectId,
      name,
      path ?? null,
      lastIndexedAt,
      options.grammarVersion ?? null,
      options.gitHead ?? null,
    ],
  });
}

export interface ProjectMeta {
  project_id: string;
  name: string;
  path: string | null;
  git_head: string | null;
  last_indexed: string | null;
  last_indexed_at: number | null;
  grammar_version: string | null;
}

export async function getProjectMeta(config: CortexConfig, projectId: string): Promise<ProjectMeta | null> {
  const db = getClient(config);
  const result = await db.execute({
    sql: `SELECT project_id, name, path, git_head, last_indexed, last_indexed_at, grammar_version
          FROM projects WHERE project_id = ?`,
    args: [projectId],
  });
  const row = result.rows[0];
  if (!row) return null;
  return {
    project_id: row.project_id as string,
    name: row.name as string,
    path: (row.path as string | null) ?? null,
    git_head: (row.git_head as string | null) ?? null,
    last_indexed: (row.last_indexed as string | null) ?? null,
    last_indexed_at: row.last_indexed_at != null ? Number(row.last_indexed_at) : null,
    grammar_version: (row.grammar_version as string | null) ?? null,
  };
}

export async function clearProjectChunks(config: CortexConfig, projectId: string): Promise<number> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');
  const result = await db.execute({
    sql: `DELETE FROM chunks WHERE project_id = ?`,
    args: [projectId],
  });
  return result.rowsAffected;
}

export async function deleteStaleProjectChunks(config: CortexConfig, projectId: string, keepChunkIds: string[]): Promise<number> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');

  const existing = await db.execute({
    sql: `SELECT chunk_id FROM chunks WHERE project_id = ?`,
    args: [projectId],
  });

  const keep = new Set(keepChunkIds);
  const staleChunkIds = existing.rows
    .map(row => row.chunk_id as string)
    .filter(chunkId => !keep.has(chunkId));

  if (staleChunkIds.length === 0) return 0;

  let deleted = 0;
  for (let i = 0; i < staleChunkIds.length; i += BATCH_SIZE) {
    const batch = staleChunkIds.slice(i, i + BATCH_SIZE);
    const placeholders = batch.map(() => '?').join(', ');
    const result = await db.execute({
      sql: `DELETE FROM chunks
            WHERE project_id = ?
              AND chunk_id IN (${placeholders})`,
      args: [projectId, ...batch],
    });
    deleted += result.rowsAffected;
  }
  return deleted;
}

export async function replaceProjectRelationships(config: CortexConfig, projectId: string, rels: RelationshipRow[]): Promise<void> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');

  await db.execute({
    sql: `DELETE FROM relationships
          WHERE source_chunk_id IN (
            SELECT chunk_id FROM chunks WHERE project_id = ?
          )`,
    args: [projectId],
  });

  if (rels.length > 0) {
    await upsertRelationships(config, rels);
  }
}

export async function replaceFileRelationships(config: CortexConfig, projectId: string, filePath: string, rels: RelationshipRow[]): Promise<void> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');

  await db.execute({
    sql: `DELETE FROM relationships
          WHERE source_chunk_id IN (
            SELECT chunk_id FROM chunks WHERE project_id = ? AND file_path = ?
          )`,
    args: [projectId, filePath],
  });

  if (rels.length > 0) {
    await upsertRelationships(config, rels);
  }
}

export async function deleteFileChunks(config: CortexConfig, projectId: string, filePath: string): Promise<number> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');
  const result = await db.execute({
    sql: `DELETE FROM chunks WHERE project_id = ? AND file_path = ?`,
    args: [projectId, filePath],
  });
  return result.rowsAffected;
}

export async function deleteStaleFileChunks(config: CortexConfig, projectId: string, filePath: string, currentChunkIds: string[]): Promise<number> {
  if (currentChunkIds.length === 0) return 0;
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');
  const placeholders = currentChunkIds.map(() => '?').join(', ');
  const result = await db.execute({
    sql: `DELETE FROM chunks
          WHERE project_id = ? AND file_path = ? AND chunk_id NOT IN (${placeholders})`,
    args: [projectId, filePath, ...currentChunkIds],
  });
  return result.rowsAffected;
}

/** Get the set of distinct file_paths currently indexed for a project. */
export async function getIndexedFilePaths(config: CortexConfig, projectId: string): Promise<Set<string>> {
  const db = getClient(config);
  const result = await db.execute({
    sql: `SELECT DISTINCT file_path FROM chunks WHERE project_id = ?`,
    args: [projectId],
  });
  return new Set(result.rows.map(r => r.file_path as string));
}

export async function projectExists(config: CortexConfig, projectId: string): Promise<boolean> {
  const db = getClient(config);
  const result = await db.execute({
    sql: `SELECT 1 FROM projects WHERE project_id = ? LIMIT 1`,
    args: [projectId],
  });
  return result.rows.length > 0;
}

// --- P5: AST-aware filter clause builder ---

/**
 * Optional structured filters for `vectorSearch` / `keywordSearch` /
 * `hybridSearch`. All four fields are AND-combined when present. `symbolGlob`
 * and `fileGlob` use SQL LIKE patterns (with `%` already substituted from `*`)
 * — `index.ts:globToLike` is the canonical builder. SQL LIKE wildcards in the
 * raw value must be backslash-escaped before being passed in.
 */
export interface SearchFilters {
  kind?: string;
  language?: string;
  symbolGlob?: string;
  fileGlob?: string;
}

/**
 * Build the trailing WHERE clause for a query that JOINs `chunks` aliased as
 * `<alias>`. Returns `{ sql: '', args: [] }` when no filters are set.
 *
 * Note: returned `sql` always begins with ` AND ` (or is empty), so the caller
 * just appends it after an existing WHERE.
 */
function buildFilterClause(filters: SearchFilters | undefined, alias: string = 'c'): { sql: string; args: any[] } {
  if (!filters) return { sql: '', args: [] };
  const conds: string[] = [];
  const args: any[] = [];
  if (filters.kind) {
    conds.push(`${alias}.chunk_type = ?`);
    args.push(filters.kind);
  }
  if (filters.language) {
    conds.push(`${alias}.language = ?`);
    args.push(filters.language);
  }
  if (filters.symbolGlob) {
    conds.push(`${alias}.symbol_name LIKE ? ESCAPE '\\'`);
    args.push(filters.symbolGlob);
  }
  if (filters.fileGlob) {
    conds.push(`${alias}.file_path LIKE ? ESCAPE '\\'`);
    args.push(filters.fileGlob);
  }
  return { sql: conds.length ? ' AND ' + conds.join(' AND ') : '', args };
}

export function searchFiltersActive(filters: SearchFilters | undefined): boolean {
  if (!filters) return false;
  return filters.kind != null || filters.language != null || filters.symbolGlob != null || filters.fileGlob != null;
}

// Vector search
export async function vectorSearch(
  config: CortexConfig,
  vector: number[],
  projectId: string,
  topK: number,
  offset: number = 0,
  filters?: SearchFilters,
): Promise<{ chunks: any[]; total_matches: number; has_more: boolean }> {
  const db = getClient(config);
  const [projectCountResult, totalCountResult] = await Promise.all([
    db.execute({ sql: `SELECT COUNT(*) as count FROM chunks WHERE project_id = ?`, args: [projectId] }),
    db.execute({ sql: `SELECT COUNT(*) as count FROM chunks`, args: [] }),
  ]);

  const projectChunkCount = Number(projectCountResult.rows[0]?.count ?? 0);
  if (projectChunkCount === 0) {
    return { chunks: [], total_matches: 0, has_more: false };
  }

  const totalChunkCount = Number(totalCountResult.rows[0]?.count ?? 0);
  // When filters are active, post-ANN WHERE filters can drop most results, so
  // over-fetch aggressively to avoid returning a near-empty page.
  const filterMultiplier = searchFiltersActive(filters) ? 10 : 1;
  const fetchCount = totalChunkCount === projectChunkCount
    ? Math.min(projectChunkCount, (topK + offset) * filterMultiplier)
    : totalChunkCount;

  const filterClause = buildFilterClause(filters);
  const vectorJson = JSON.stringify(vector);
  const result = await db.execute({
    sql: `SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type,
                 c.start_line, c.end_line, c.content, c.language,
                 vector_distance_cos(c.embedding, vector(?)) as distance
          FROM vector_top_k('idx_chunks_embedding', vector(?), ?) v
          JOIN chunks c ON c.rowid = v.id
          WHERE c.project_id = ?${filterClause.sql}
          ORDER BY distance`,
    args: [vectorJson, vectorJson, fetchCount, projectId, ...filterClause.args],
  });

  const allMatches = result.rows.map(row => ({
    chunk_id: row.chunk_id,
    file_path: row.file_path,
    symbol_name: row.symbol_name,
    chunk_type: row.chunk_type,
    start_line: row.start_line,
    end_line: row.end_line,
    content: row.content,
    language: row.language,
    score: 1 - (row.distance as number),
  }));

  const paged = allMatches.slice(offset, offset + topK);
  return {
    chunks: paged,
    total_matches: allMatches.length,
    has_more: offset + topK < allMatches.length,
  };
}

// FTS5 keyword search
export async function keywordSearch(
  config: CortexConfig,
  query: string,
  projectId: string,
  limit: number = 15,
  offset: number = 0,
  filters?: SearchFilters,
): Promise<{ chunks: any[]; total: number }> {
  const db = getClient(config);
  const filterClause = buildFilterClause(filters);
  const result = await db.execute({
    sql: `SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type,
                 c.start_line, c.end_line, c.content, c.language, rank
          FROM chunks_fts f
          JOIN chunks c ON c.rowid = f.rowid
          WHERE chunks_fts MATCH ? AND c.project_id = ?${filterClause.sql}
          ORDER BY rank
          LIMIT ? OFFSET ?`,
    args: [query, projectId, ...filterClause.args, limit, offset],
  });

  const countResult = await db.execute({
    sql: `SELECT COUNT(*) as total FROM chunks_fts f
          JOIN chunks c ON c.rowid = f.rowid
          WHERE chunks_fts MATCH ? AND c.project_id = ?${filterClause.sql}`,
    args: [query, projectId, ...filterClause.args],
  });

  return {
    chunks: result.rows.map(row => ({
      chunk_id: row.chunk_id,
      file_path: row.file_path,
      symbol_name: row.symbol_name,
      chunk_type: row.chunk_type,
      start_line: row.start_line,
      end_line: row.end_line,
      content: row.content,
      language: row.language,
      score: -(row.rank as number),
    })),
    total: countResult.rows[0]?.total as number ?? 0,
  };
}

// Relationship traversal
export async function traverseRelationships(
  config: CortexConfig,
  projectId: string,
  symbol: string,
  depth: number = 2,
  relTypes: string[] = ['calls', 'called_by', 'imports', 'exports', 'data_flow']
): Promise<{ nodes: any[]; edges: any[] }> {
  const db = getClient(config);

  const startNodes = await db.execute({
    sql: `SELECT chunk_id FROM chunks WHERE project_id = ? AND symbol_name = ?`,
    args: [projectId, symbol],
  });

  if (startNodes.rows.length === 0) return { nodes: [], edges: [] };

  const startIds = startNodes.rows.map(r => r.chunk_id as string);
  const startPlaceholders = startIds.map(() => '?').join(',');
  const relTypePlaceholders = relTypes.map(() => '?').join(',');

  const cteParams: any[] = [
    ...startIds,
    ...relTypes,
    depth,
    ...relTypes,
  ];

  const sql = `
    WITH RECURSIVE graph AS (
      SELECT r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, r.confidence_score, r.confidence_reasoning, 1 as depth
      FROM relationships r
      WHERE r.source_chunk_id IN (${startPlaceholders})
        AND r.rel_type IN (${relTypePlaceholders})
      UNION ALL
      SELECT r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, r.confidence_score, r.confidence_reasoning, g.depth + 1
      FROM relationships r
      JOIN graph g ON r.source_chunk_id = g.target_chunk_id
      WHERE g.depth < ?
        AND r.rel_type IN (${relTypePlaceholders})
    )
    SELECT DISTINCT g.source_chunk_id, g.target_chunk_id, g.rel_type, g.confidence, g.confidence_score, g.confidence_reasoning FROM graph g
    ORDER BY COALESCE(g.confidence_score,
                      CASE g.confidence
                        WHEN 'EXTRACTED' THEN 0.9
                        WHEN 'INFERRED'  THEN 0.55
                        ELSE 0.35
                      END) DESC
  `;

  const edgesResult = await db.execute({ sql, args: cteParams });

  const allChunkIds = new Set<string>();
  const edges = edgesResult.rows.map(r => {
    allChunkIds.add(r.source_chunk_id as string);
    allChunkIds.add(r.target_chunk_id as string);
    return {
      source: r.source_chunk_id as string,
      target: r.target_chunk_id as string,
      rel_type: r.rel_type as string,
      confidence: (r.confidence as string) || 'EXTRACTED',
      confidence_score: r.confidence_score != null ? Number(r.confidence_score) : null,
      confidence_reasoning: (r.confidence_reasoning as string | null) ?? null,
    };
  });

  startIds.forEach(id => allChunkIds.add(id));

  if (allChunkIds.size === 0) {
    return { nodes: startIds.map(id => ({ chunk_id: id })), edges: [] };
  }

  const idPlaceholders = [...allChunkIds].map(() => '?').join(',');
  const chunksResult = await db.execute({
    sql: `SELECT chunk_id, file_path, symbol_name, chunk_type
          FROM chunks WHERE chunk_id IN (${idPlaceholders})`,
    args: [...allChunkIds] as any[],
  });

  return {
    nodes: chunksResult.rows.map(r => ({
      chunk_id: r.chunk_id, file_path: r.file_path,
      symbol_name: r.symbol_name, chunk_type: r.chunk_type,
    })),
    edges,
  };
}

// Project operations
export async function getProjectStatus(config: CortexConfig, projectId: string): Promise<any | null> {
  const db = getClient(config);
  const project = await db.execute({ sql: `SELECT * FROM projects WHERE project_id = ?`, args: [projectId] });
  if (project.rows.length === 0) return null;

  const [chunks, rels, langs, confidence] = await Promise.all([
    db.execute({ sql: `SELECT COUNT(*) as count FROM chunks WHERE project_id = ?`, args: [projectId] }),
    db.execute({ sql: `SELECT COUNT(*) as count FROM relationships r JOIN chunks c ON r.source_chunk_id = c.chunk_id WHERE c.project_id = ?`, args: [projectId] }),
    db.execute({ sql: `SELECT DISTINCT language FROM chunks WHERE project_id = ? AND language IS NOT NULL`, args: [projectId] }),
    db.execute({
      sql: `SELECT r.confidence, COUNT(*) as count
            FROM relationships r
            JOIN chunks c ON r.source_chunk_id = c.chunk_id
            WHERE c.project_id = ?
            GROUP BY r.confidence`,
      args: [projectId],
    }),
  ]);

  const confidence_breakdown: Record<string, number> = { EXTRACTED: 0, INFERRED: 0, AMBIGUOUS: 0 };
  for (const row of confidence.rows) {
    const key = (row.confidence as string) || 'EXTRACTED';
    confidence_breakdown[key] = Number(row.count);
  }

  return {
    project_id: projectId,
    name: project.rows[0].name,
    chunk_count: chunks.rows[0].count,
    relationship_count: rels.rows[0].count,
    last_indexed: project.rows[0].last_indexed,
    last_indexed_at: project.rows[0].last_indexed_at != null ? Number(project.rows[0].last_indexed_at) : null,
    git_head: (project.rows[0].git_head as string | null) ?? null,
    grammar_version: (project.rows[0].grammar_version as string | null) ?? null,
    languages: langs.rows.map(r => r.language),
    confidence_breakdown,
  };
}

export async function listProjects(config: CortexConfig): Promise<any[]> {
  const db = getClient(config);
  const result = await db.execute({
    sql: `SELECT p.project_id, p.name, p.last_indexed,
          (SELECT COUNT(*) FROM chunks c WHERE c.project_id = p.project_id) as chunk_count
          FROM projects p ORDER BY p.last_indexed DESC`,
    args: [],
  });
  return result.rows.map(r => ({
    project_id: r.project_id, name: r.name,
    chunk_count: r.chunk_count, last_indexed: r.last_indexed,
  }));
}

// Hybrid search: vector + FTS fused via Reciprocal Rank Fusion
export interface HybridSearchResult {
  chunk_id: string;
  file_path: string;
  symbol_name: string | null;
  chunk_type: string | null;
  start_line: number;
  end_line: number;
  content: string;
  language: string | null;
  rrf_score: number;
  source: 'vec' | 'fts' | 'both';
  vec_rank: number | null;
  fts_rank: number | null;
}

/**
 * Sanitize a free-form search query into a valid FTS5 MATCH expression.
 *
 * FTS5 treats `. ( ) - / "` and other punctuation as syntax, so raw queries like
 * `useEffect(`, `foo.bar`, or `my-component` throw a parse error. We extract
 * alphanumeric/underscore tokens and wrap each as a phrase, joined with OR for
 * higher recall. Returns null when nothing indexable remains.
 */
export function sanitizeFtsQuery(query: string): string | null {
  const tokens = query.match(/[A-Za-z0-9_]+/g);
  if (!tokens || tokens.length === 0) return null;
  // Quote each token as an FTS5 phrase (double-quotes inside are impossible because we stripped them)
  // and OR-join for recall. FTS5 phrase syntax: "word"
  return tokens.map(t => `"${t}"`).join(' OR ');
}

export async function hybridSearch(
  config: CortexConfig,
  vector: number[],
  query: string,
  projectId: string,
  topK: number = 15,
  rrfK: number = 60,
  offset: number = 0,
  filters?: SearchFilters,
): Promise<{ chunks: HybridSearchResult[]; total_matches: number; has_more: boolean }> {
  // Over-fetch enough from each branch so offset-based slicing has headroom.
  // Both branches contribute, so topK*2 + offset gives the fused set enough candidates.
  const fetchSize = (topK + offset) * 2;

  // Vector branch: always runs
  const vecPromise = vectorSearch(config, vector, projectId, fetchSize, 0, filters);

  // FTS branch: sanitize, and fall back to empty on any error so a malformed
  // query never tears down the whole hybrid path. When the user only supplied
  // filter tokens (no free text), the sanitized query is null — in that case
  // skip FTS and let the vector branch carry the filters alone.
  const ftsQuery = sanitizeFtsQuery(query);
  const ftsPromise: Promise<{ chunks: any[]; total: number }> = ftsQuery
    ? keywordSearch(config, ftsQuery, projectId, fetchSize, 0, filters).catch(() => ({ chunks: [], total: 0 }))
    : Promise.resolve({ chunks: [], total: 0 });

  const [vecResult, ftsResult] = await Promise.all([vecPromise, ftsPromise]);

  // Build rank maps (1-indexed)
  const vecRank = new Map<string, number>();
  vecResult.chunks.forEach((c: any, i: number) => vecRank.set(c.chunk_id, i + 1));

  const ftsRank = new Map<string, number>();
  ftsResult.chunks.forEach((c: any, i: number) => ftsRank.set(c.chunk_id, i + 1));

  // Collect all unique chunk_ids and their data
  const chunkData = new Map<string, any>();
  for (const c of vecResult.chunks) chunkData.set(c.chunk_id, c);
  for (const c of ftsResult.chunks) {
    if (!chunkData.has(c.chunk_id)) chunkData.set(c.chunk_id, c);
  }

  // Compute RRF scores
  const scored: HybridSearchResult[] = [];
  for (const [chunkId, data] of chunkData) {
    const vr = vecRank.get(chunkId) ?? null;
    const fr = ftsRank.get(chunkId) ?? null;

    let rrfScore = 0;
    if (vr !== null) rrfScore += 1 / (rrfK + vr);
    if (fr !== null) rrfScore += 1 / (rrfK + fr);

    const source: 'vec' | 'fts' | 'both' =
      vr !== null && fr !== null ? 'both' :
      vr !== null ? 'vec' : 'fts';

    scored.push({
      chunk_id: chunkId,
      file_path: data.file_path,
      symbol_name: data.symbol_name ?? null,
      chunk_type: data.chunk_type ?? null,
      start_line: data.start_line,
      end_line: data.end_line,
      content: data.content,
      language: data.language ?? null,
      rrf_score: rrfScore,
      source,
      vec_rank: vr,
      fts_rank: fr,
    });
  }

  // Sort by RRF score descending, then paginate
  scored.sort((a, b) => b.rrf_score - a.rrf_score);
  const paged = scored.slice(offset, offset + topK);

  return {
    chunks: paged,
    total_matches: scored.length,
    has_more: offset + topK < scored.length,
  };
}

// Neighbor lookup for context command: direct relationships of given chunk_ids
export async function getDirectNeighbors(
  config: CortexConfig,
  chunkIds: string[],
  maxNeighbors: number = 10,
): Promise<{ nodes: any[]; edges: any[] }> {
  if (chunkIds.length === 0) return { nodes: [], edges: [] };
  const db = getClient(config);

  const placeholders = chunkIds.map(() => '?').join(',');

  // Get edges where source or target is in our set
  const edgesResult = await db.execute({
    sql: `SELECT r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, r.confidence_score, r.confidence_reasoning
          FROM relationships r
          WHERE r.source_chunk_id IN (${placeholders})
             OR r.target_chunk_id IN (${placeholders})
          ORDER BY COALESCE(r.confidence_score,
                            CASE r.confidence
                              WHEN 'EXTRACTED' THEN 0.9
                              WHEN 'INFERRED'  THEN 0.55
                              ELSE 0.35
                            END) DESC
          LIMIT ?`,
    args: [...chunkIds, ...chunkIds, maxNeighbors * 2],
  });

  const neighborIds = new Set<string>();
  const edges = edgesResult.rows.map(r => {
    neighborIds.add(r.source_chunk_id as string);
    neighborIds.add(r.target_chunk_id as string);
    return {
      source: r.source_chunk_id as string,
      target: r.target_chunk_id as string,
      rel_type: r.rel_type as string,
      confidence: (r.confidence as string) || 'EXTRACTED',
      confidence_score: r.confidence_score != null ? Number(r.confidence_score) : null,
      confidence_reasoning: (r.confidence_reasoning as string | null) ?? null,
    };
  });

  // Remove primary chunk ids from neighbor set — we already have those
  for (const id of chunkIds) neighborIds.delete(id);

  if (neighborIds.size === 0) return { nodes: [], edges };

  const neighborPlaceholders = [...neighborIds].map(() => '?').join(',');
  const nodesResult = await db.execute({
    sql: `SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line
          FROM chunks WHERE chunk_id IN (${neighborPlaceholders})`,
    args: [...neighborIds] as any[],
  });

  return {
    nodes: nodesResult.rows.map(r => ({
      chunk_id: r.chunk_id,
      file_path: r.file_path,
      symbol_name: r.symbol_name,
      chunk_type: r.chunk_type,
      start_line: r.start_line,
      end_line: r.end_line,
    })),
    edges,
  };
}

// Impact analysis: transitive dependents (fan-in) for a symbol or file
export async function getTransitiveDependents(
  config: CortexConfig,
  projectId: string,
  seedChunkIds: string[],
  maxDepth: number = 3,
  maxResults: number = 30,
): Promise<{ nodes: any[]; edges: any[]; depth_reached: number }> {
  if (seedChunkIds.length === 0) return { nodes: [], edges: [], depth_reached: 0 };
  const db = getClient(config);

  const seedPlaceholders = seedChunkIds.map(() => '?').join(',');

  // Traverse reverse edges: who depends on these chunks?
  const sql = `
    WITH RECURSIVE impact AS (
      SELECT r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, r.confidence_score, r.confidence_reasoning, 1 as depth
      FROM relationships r
      WHERE r.target_chunk_id IN (${seedPlaceholders})
      UNION ALL
      SELECT r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, r.confidence_score, r.confidence_reasoning, i.depth + 1
      FROM relationships r
      JOIN impact i ON r.target_chunk_id = i.source_chunk_id
      WHERE i.depth < ?
    )
    SELECT DISTINCT source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score, confidence_reasoning, MIN(depth) as depth
    FROM impact
    GROUP BY source_chunk_id, target_chunk_id, rel_type
    ORDER BY COALESCE(confidence_score,
                      CASE confidence
                        WHEN 'EXTRACTED' THEN 0.9
                        WHEN 'INFERRED'  THEN 0.55
                        ELSE 0.35
                      END) DESC
    LIMIT ?
  `;

  const edgesResult = await db.execute({
    sql,
    args: [...seedChunkIds, maxDepth, maxResults * 3],
  });

  const allChunkIds = new Set<string>(seedChunkIds);
  let maxDepthReached = 0;
  const edges = edgesResult.rows.map(r => {
    allChunkIds.add(r.source_chunk_id as string);
    allChunkIds.add(r.target_chunk_id as string);
    const d = Number(r.depth);
    if (d > maxDepthReached) maxDepthReached = d;
    return {
      source: r.source_chunk_id as string,
      target: r.target_chunk_id as string,
      rel_type: r.rel_type as string,
      confidence: (r.confidence as string) || 'EXTRACTED',
      confidence_score: r.confidence_score != null ? Number(r.confidence_score) : null,
      confidence_reasoning: (r.confidence_reasoning as string | null) ?? null,
      depth: d,
    };
  });

  if (allChunkIds.size === 0) return { nodes: [], edges: [], depth_reached: 0 };

  const idPlaceholders = [...allChunkIds].map(() => '?').join(',');
  const nodesResult = await db.execute({
    sql: `SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line
          FROM chunks WHERE chunk_id IN (${idPlaceholders}) AND project_id = ?`,
    args: [...allChunkIds, projectId] as any[],
  });

  return {
    nodes: nodesResult.rows.map(r => ({
      chunk_id: r.chunk_id,
      file_path: r.file_path,
      symbol_name: r.symbol_name,
      chunk_type: r.chunk_type,
      start_line: r.start_line,
      end_line: r.end_line,
    })),
    edges,
    depth_reached: maxDepthReached,
  };
}

// Find chunks by symbol name (used by context/impact to seed lookups)
export async function findChunksBySymbol(
  config: CortexConfig,
  projectId: string,
  symbol: string,
): Promise<any[]> {
  const db = getClient(config);
  const result = await db.execute({
    sql: `SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, content, language
          FROM chunks WHERE project_id = ? AND symbol_name = ?`,
    args: [projectId, symbol],
  });
  return result.rows.map(r => ({
    chunk_id: r.chunk_id,
    file_path: r.file_path,
    symbol_name: r.symbol_name,
    chunk_type: r.chunk_type,
    start_line: r.start_line,
    end_line: r.end_line,
    content: r.content,
    language: r.language,
  }));
}

// Find chunks by file path (used by impact --from-diff)
export async function findChunksByFile(
  config: CortexConfig,
  projectId: string,
  filePath: string,
): Promise<any[]> {
  const db = getClient(config);
  const result = await db.execute({
    sql: `SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line
          FROM chunks WHERE project_id = ? AND file_path = ?`,
    args: [projectId, filePath],
  });
  return result.rows.map(r => ({
    chunk_id: r.chunk_id,
    file_path: r.file_path,
    symbol_name: r.symbol_name,
    chunk_type: r.chunk_type,
    start_line: r.start_line,
    end_line: r.end_line,
  }));
}

// Graph data for Louvain clustering
export interface ProjectGraphData {
  chunks: Array<{ chunk_id: string; file_path: string; symbol_name: string | null; chunk_type: string | null }>;
  relationships: Array<{ source_chunk_id: string; target_chunk_id: string; confidence_score: number | null }>;
}

export async function getProjectGraphData(config: CortexConfig, projectId: string): Promise<ProjectGraphData> {
  const db = getClient(config);
  const [chunksResult, relsResult] = await Promise.all([
    db.execute({
      sql: `SELECT chunk_id, file_path, symbol_name, chunk_type FROM chunks WHERE project_id = ?`,
      args: [projectId],
    }),
    db.execute({
      sql: `SELECT r.source_chunk_id, r.target_chunk_id, r.confidence_score
            FROM relationships r
            JOIN chunks c ON r.source_chunk_id = c.chunk_id
            WHERE c.project_id = ?`,
      args: [projectId],
    }),
  ]);
  return {
    chunks: chunksResult.rows.map(r => ({
      chunk_id: r.chunk_id as string,
      file_path: r.file_path as string,
      symbol_name: (r.symbol_name as string | null),
      chunk_type: (r.chunk_type as string | null),
    })),
    relationships: relsResult.rows.map(r => ({
      source_chunk_id: r.source_chunk_id as string,
      target_chunk_id: r.target_chunk_id as string,
      confidence_score: r.confidence_score != null ? Number(r.confidence_score) : null,
    })),
  };
}

export async function deleteProject(config: CortexConfig, projectId: string): Promise<number> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');
  const result = await db.execute({ sql: `DELETE FROM chunks WHERE project_id = ?`, args: [projectId] });
  await db.execute({ sql: `DELETE FROM projects WHERE project_id = ?`, args: [projectId] });
  return result.rowsAffected;
}
