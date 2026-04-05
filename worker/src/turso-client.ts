// Turso client using @libsql/client with WebSocket transport
import { createClient, type Client } from '@libsql/client';
import type { Env } from './types';

let client: Client | null = null;
let foreignKeysEnabled = false;

export function getTursoClient(env: Env): Client {
  if (!client) {
    client = createClient({
      url: env.TURSO_URL,
      authToken: env.TURSO_AUTH_TOKEN,
    });
    foreignKeysEnabled = false;
  }
  return client;
}

// Must be called once after client creation to enable CASCADE behavior
export async function ensureForeignKeys(env: Env): Promise<void> {
  if (foreignKeysEnabled) return;
  const db = getTursoClient(env);
  await db.execute('PRAGMA foreign_keys = ON');
  foreignKeysEnabled = true;
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

// Apply schema (idempotent — uses IF NOT EXISTS)
export async function applySchema(env: Env): Promise<void> {
  const db = getTursoClient(env);
  await ensureForeignKeys(env);
  const schema = await getSchemaSQL();
  const statements = splitStatements(schema);
  for (const stmt of statements) {
    await db.execute(stmt);
  }
}

async function getSchemaSQL(): Promise<string> {
  // Schema is embedded at deploy time
  return SCHEMA_SQL;
}

// Embedded schema (duplicated from schema.sql for Worker bundle)
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
  FOREIGN KEY (source_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  FOREIGN KEY (target_chunk_id) REFERENCES chunks(chunk_id) ON DELETE CASCADE,
  PRIMARY KEY (source_chunk_id, target_chunk_id, rel_type)
);
CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_type_source ON relationships(rel_type, source_chunk_id);
CREATE INDEX IF NOT EXISTS idx_rel_type_target ON relationships(rel_type, target_chunk_id);
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
CREATE TABLE IF NOT EXISTS api_keys (
  api_key TEXT PRIMARY KEY,
  created_at TEXT DEFAULT (datetime('now')),
  active INTEGER DEFAULT 1
);
`;

// Batch upsert chunks into Turso
export async function upsertChunks(
  env: Env,
  chunks: Array<{
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
  }>
): Promise<void> {
  const db = getTursoClient(env);
  await ensureForeignKeys(env);
  for (const chunk of chunks) {
    await db.execute({
      sql: `INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type, start_line, end_line, content, content_hash, language, embedding)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, vector(?))
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
              embedding = excluded.embedding`,
      args: [
        chunk.chunk_id,
        chunk.project_id,
        chunk.file_path,
        chunk.symbol_name ?? null,
        chunk.chunk_type ?? null,
        chunk.start_line,
        chunk.end_line,
        chunk.content,
        chunk.content_hash,
        chunk.language ?? null,
        chunk.embedding ? JSON.stringify(chunk.embedding) : null,
      ],
    });
  }
}

// Batch upsert relationships
export async function upsertRelationships(
  env: Env,
  relationships: Array<{
    source_chunk_id: string;
    target_chunk_id: string;
    rel_type: string;
  }>
): Promise<void> {
  const db = getTursoClient(env);
  for (const rel of relationships) {
    await db.execute({
      sql: `INSERT OR REPLACE INTO relationships (source_chunk_id, target_chunk_id, rel_type)
            VALUES (?, ?, ?)`,
      args: [rel.source_chunk_id, rel.target_chunk_id, rel.rel_type],
    });
  }
}

// Fetch chunks by IDs (batched)
export async function fetchChunksByIds(
  env: Env,
  chunkIds: string[]
): Promise<Array<{
  chunk_id: string;
  file_path: string;
  symbol_name: string | null;
  chunk_type: string | null;
  start_line: number;
  end_line: number;
  content: string;
  content_hash: string;
  language: string | null;
}>> {
  if (chunkIds.length === 0) return [];
  const db = getTursoClient(env);

  // Build parameterized query with placeholders
  const placeholders = chunkIds.map(() => '?').join(',');
  const result = await db.execute({
    sql: `SELECT chunk_id, file_path, symbol_name, chunk_type, start_line, end_line, content, content_hash, language
          FROM chunks WHERE chunk_id IN (${placeholders})`,
    args: chunkIds as any[],
  });

  return result.rows.map(row => ({
    chunk_id: row.chunk_id as string,
    file_path: row.file_path as string,
    symbol_name: row.symbol_name as string | null,
    chunk_type: row.chunk_type as string | null,
    start_line: row.start_line as number,
    end_line: row.end_line as number,
    content: row.content as string,
    content_hash: row.content_hash as string,
    language: row.language as string | null,
  }));
}

// Delete all chunks for a file, return affected chunk_ids
export async function deleteChunksByFile(
  env: Env,
  projectId: string,
  filePath: string
): Promise<string[]> {
  const db = getTursoClient(env);

  // Get affected chunk_ids before deletion
  const ids = await db.execute({
    sql: `SELECT chunk_id FROM chunks WHERE project_id = ? AND file_path = ?`,
    args: [projectId, filePath],
  });

  const chunkIds = ids.rows.map(r => r.chunk_id as string);

  if (chunkIds.length > 0) {
    // CASCADE will handle relationships automatically
    const placeholders = chunkIds.map(() => '?').join(',');
    await db.execute({
      sql: `DELETE FROM chunks WHERE chunk_id IN (${placeholders})`,
      args: chunkIds as any[],
    });
  }

  return chunkIds;
}

// Delete entire project
export async function deleteProjectChunks(
  env: Env,
  projectId: string
): Promise<number> {
  const db = getTursoClient(env);
  const result = await db.execute({
    sql: `DELETE FROM chunks WHERE project_id = ?`,
    args: [projectId],
  });
  await db.execute({
    sql: `DELETE FROM projects WHERE project_id = ?`,
    args: [projectId],
  });
  return result.rowsAffected;
}

// Vector similarity search using Turso native vector_top_k
// Over-fetches globally then filters by project_id, since vector_top_k
// does not support pre-filtering. Uses increasing multipliers to ensure
// enough project-scoped results.
const VECTOR_FETCH_MULTIPLIER = 5;
const VECTOR_MAX_FETCH = 500;

export async function vectorSearch(
  env: Env,
  vector: number[],
  projectId: string,
  topK: number,
  offset: number
): Promise<{ chunks: any[]; total_matches: number; has_more: boolean }> {
  const db = getTursoClient(env);

  const needed = topK + offset;
  // Over-fetch to compensate for cross-project neighbors
  const fetchCount = Math.min(needed * VECTOR_FETCH_MULTIPLIER, VECTOR_MAX_FETCH);

  const result = await db.execute({
    sql: `SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type,
                 c.start_line, c.end_line, c.content, c.language,
                 v.distance
          FROM vector_top_k('idx_chunks_embedding', vector(?), ?) v
          JOIN chunks c ON c.rowid = v.id
          WHERE c.project_id = ?
          ORDER BY v.distance`,
    args: [
      JSON.stringify(vector),
      fetchCount,
      projectId,
    ],
  });

  // Apply offset + limit on the filtered results
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
  env: Env,
  query: string,
  projectId: string,
  limit: number,
  offset: number
): Promise<{ chunks: any[]; total: number }> {
  const db = getTursoClient(env);

  // Use FTS5 with BM25 ranking
  const result = await db.execute({
    sql: `SELECT c.chunk_id, c.file_path, c.symbol_name, c.chunk_type,
                 c.start_line, c.end_line, c.content, c.language,
                 rank
          FROM chunks_fts f
          JOIN chunks c ON c.rowid = f.rowid
          WHERE f.content MATCH ? AND c.project_id = ?
          ORDER BY rank
          LIMIT ? OFFSET ?`,
    args: [query, projectId, limit, offset],
  });

  // Get total count
  const countResult = await db.execute({
    sql: `SELECT COUNT(*) as total FROM chunks_fts f
          JOIN chunks c ON c.rowid = f.rowid
          WHERE f.content MATCH ? AND c.project_id = ?`,
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
      score: -(row.rank as number), // FTS5 rank is negative (lower = better)
    })),
    total: countResult.rows[0]?.total as number ?? 0,
  };
}

// Recursive CTE for relationship traversal
export async function traverseRelationships(
  env: Env,
  projectId: string,
  symbol: string,
  depth: number,
  relTypes: string[]
): Promise<{ nodes: any[]; edges: any[] }> {
  const db = getTursoClient(env);

  const relTypeFilter = relTypes.length > 0
    ? `AND r.rel_type IN (${relTypes.map(() => '?').join(',')})`
    : '';

  // Find starting chunk_ids by symbol name
  const startNodes = await db.execute({
    sql: `SELECT chunk_id FROM chunks WHERE project_id = ? AND symbol_name = ?`,
    args: [projectId, symbol],
  });

  if (startNodes.rows.length === 0) {
    return { nodes: [], edges: [] };
  }

  const startIds = startNodes.rows.map(r => r.chunk_id as string);

  // Recursive CTE: traverse relationships up to N depth
  const startPlaceholders = startIds.map(() => '?').join(',');
  const relTypePlaceholders = relTypes.length > 0
    ? relTypes.map(() => '?').join(',')
    : "'calls','called_by','imports','exports','data_flow'";

  // Build params: startIds for anchor IN, relTypes for anchor filter,
  // depth for recursive limit, relTypes again for recursive filter
  const cteParams: any[] = [
    ...startIds,
    ...(relTypes.length > 0 ? relTypes : []),
    depth,
    ...(relTypes.length > 0 ? relTypes : []),
  ];

  const sql = `
    WITH RECURSIVE graph AS (
      -- Anchor: starting nodes
      SELECT
        r.source_chunk_id,
        r.target_chunk_id,
        r.rel_type,
        1 as depth
      FROM relationships r
      WHERE r.source_chunk_id IN (${startPlaceholders})
        AND r.rel_type IN (${relTypePlaceholders})
      UNION ALL
      -- Recursive: follow edges
      SELECT
        r.source_chunk_id,
        r.target_chunk_id,
        r.rel_type,
        g.depth + 1
      FROM relationships r
      JOIN graph g ON r.source_chunk_id = g.target_chunk_id
      WHERE g.depth < ?
        AND r.rel_type IN (${relTypePlaceholders})
    )
    SELECT DISTINCT
      g.source_chunk_id,
      g.target_chunk_id,
      g.rel_type
    FROM graph g
  `;

  const edgesResult = await db.execute({ sql, args: cteParams });

  // Collect unique chunk_ids
  const allChunkIds = new Set<string>();
  const edges = edgesResult.rows.map(r => {
    allChunkIds.add(r.source_chunk_id as string);
    allChunkIds.add(r.target_chunk_id as string);
    return {
      source: r.source_chunk_id as string,
      target: r.target_chunk_id as string,
      rel_type: r.rel_type as string,
    };
  });

  // Also add starting chunks that might not have edges
  startIds.forEach(id => allChunkIds.add(id));

  // Fetch chunk details
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
      chunk_id: r.chunk_id as string,
      file_path: r.file_path as string,
      symbol_name: r.symbol_name as string | null,
      chunk_type: r.chunk_type as string | null,
    })),
    edges,
  };
}

// Get project status
export async function getProjectStatus(
  env: Env,
  projectId: string
): Promise<{
  project_id: string;
  name: string;
  chunk_count: number;
  relationship_count: number;
  last_indexed: string | null;
  languages: string[];
} | null> {
  const db = getTursoClient(env);

  const project = await db.execute({
    sql: `SELECT * FROM projects WHERE project_id = ?`,
    args: [projectId],
  });

  if (project.rows.length === 0) return null;

  const chunkCount = await db.execute({
    sql: `SELECT COUNT(*) as count FROM chunks WHERE project_id = ?`,
    args: [projectId],
  });

  const relCount = await db.execute({
    sql: `SELECT COUNT(*) as count FROM relationships r
          JOIN chunks c ON r.source_chunk_id = c.chunk_id
          WHERE c.project_id = ?`,
    args: [projectId],
  });

  const langs = await db.execute({
    sql: `SELECT DISTINCT language FROM chunks WHERE project_id = ? AND language IS NOT NULL`,
    args: [projectId],
  });

  return {
    project_id: projectId,
    name: project.rows[0].name as string,
    chunk_count: chunkCount.rows[0].count as number,
    relationship_count: relCount.rows[0].count as number,
    last_indexed: project.rows[0].last_indexed as string | null,
    languages: langs.rows.map(r => r.language as string),
  };
}

// List all projects
export async function listProjects(env: Env): Promise<Array<{
  project_id: string;
  name: string;
  chunk_count: number;
  last_indexed: string | null;
}>> {
  const db = getTursoClient(env);

  const result = await db.execute({
    sql: `SELECT p.project_id, p.name, p.last_indexed,
          (SELECT COUNT(*) FROM chunks c WHERE c.project_id = p.project_id) as chunk_count
          FROM projects p
          ORDER BY p.last_indexed DESC`,
    args: [],
  });

  return result.rows.map(r => ({
    project_id: r.project_id as string,
    name: r.name as string,
    chunk_count: r.chunk_count as number,
    last_indexed: r.last_indexed as string | null,
  }));
}

// Upsert project record
export async function upsertProject(
  env: Env,
  projectId: string,
  name: string,
  path?: string
): Promise<void> {
  const db = getTursoClient(env);
  await db.execute({
    sql: `INSERT OR REPLACE INTO projects (project_id, name, path, last_indexed, created_at)
          VALUES (?, ?, ?, datetime('now'),
            COALESCE((SELECT created_at FROM projects WHERE project_id = ?), datetime('now')))`,
    args: [projectId, name, path ?? null, projectId],
  });
}
