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
      sql: `INSERT OR IGNORE INTO relationships (source_chunk_id, target_chunk_id, rel_type, confidence) VALUES (?, ?, ?, ?)`,
      args: [r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence],
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

// Vector search
export async function vectorSearch(
  config: CortexConfig,
  vector: number[],
  projectId: string,
  topK: number,
  offset: number = 0
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
  const fetchCount = totalChunkCount === projectChunkCount
    ? Math.min(projectChunkCount, topK + offset)
    : totalChunkCount;

  const vectorJson = JSON.stringify(vector);
  const result = await db.execute({
    sql: `SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type,
                 c.start_line, c.end_line, c.content, c.language,
                 vector_distance_cos(c.embedding, vector(?)) as distance
          FROM vector_top_k('idx_chunks_embedding', vector(?), ?) v
          JOIN chunks c ON c.rowid = v.id
          WHERE c.project_id = ?
          ORDER BY distance`,
    args: [vectorJson, vectorJson, fetchCount, projectId],
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
  offset: number = 0
): Promise<{ chunks: any[]; total: number }> {
  const db = getClient(config);
  const result = await db.execute({
    sql: `SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type,
                 c.start_line, c.end_line, c.content, c.language, rank
          FROM chunks_fts f
          JOIN chunks c ON c.rowid = f.rowid
          WHERE chunks_fts MATCH ? AND c.project_id = ?
          ORDER BY rank
          LIMIT ? OFFSET ?`,
    args: [query, projectId, limit, offset],
  });

  const countResult = await db.execute({
    sql: `SELECT COUNT(*) as total FROM chunks_fts f
          JOIN chunks c ON c.rowid = f.rowid
          WHERE chunks_fts MATCH ? AND c.project_id = ?`,
    args: [query, projectId],
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
      SELECT r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, 1 as depth
      FROM relationships r
      WHERE r.source_chunk_id IN (${startPlaceholders})
        AND r.rel_type IN (${relTypePlaceholders})
      UNION ALL
      SELECT r.source_chunk_id, r.target_chunk_id, r.rel_type, r.confidence, g.depth + 1
      FROM relationships r
      JOIN graph g ON r.source_chunk_id = g.target_chunk_id
      WHERE g.depth < ?
        AND r.rel_type IN (${relTypePlaceholders})
    )
    SELECT DISTINCT g.source_chunk_id, g.target_chunk_id, g.rel_type, g.confidence FROM graph g
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

export async function deleteProject(config: CortexConfig, projectId: string): Promise<number> {
  const db = getClient(config);
  await db.execute('PRAGMA foreign_keys = ON');
  const result = await db.execute({ sql: `DELETE FROM chunks WHERE project_id = ?`, args: [projectId] });
  await db.execute({ sql: `DELETE FROM projects WHERE project_id = ?`, args: [projectId] });
  return result.rowsAffected;
}
