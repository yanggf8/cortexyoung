import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { extractFile } from './chunker.js';
import { extractorVersion } from './pack.js';
import { setMeta } from './db.js';
import { relationshipRowsForFile } from './graph.js';

export const IGNORE_DIRS = new Set([
  'node_modules', 'dist', 'build', '.git', '__pycache__', '.venv', 'venv',
  'target', 'coverage', '.next', '.cache',
]);

export const SOURCE_EXT = new Set(['.ts', '.tsx', '.js', '.jsx', '.mjs', '.cjs', '.py', '.rs']);

export function walkFiles(root) {
  const out = [];
  const walk = (dir) => {
    const entries = fs.readdirSync(dir, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      if (entry.isSymbolicLink()) continue;
      const abs = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (!IGNORE_DIRS.has(entry.name)) walk(abs);
      } else if (entry.isFile() && SOURCE_EXT.has(path.extname(entry.name))) {
        out.push(path.relative(root, abs).split(path.sep).join('/'));
      }
    }
  };
  walk(root);
  return out.sort();
}

export function gitHeadOf(root) {
  const r = spawnSync('git', ['-C', root, 'rev-parse', 'HEAD'], { encoding: 'utf8' });
  if (r.error || r.status !== 0) return null;
  return r.stdout.trim() || null;
}

const INSERT_CHUNK = `INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
  start_line, end_line, content, content_hash, language, chunk_source)
  VALUES (@chunk_id, @project_id, @file_path, @symbol_name, @chunk_type,
  @start_line, @end_line, @content, @content_hash, @language, @chunk_source)`;

export function extractAll({ bin, root, projectId, files }) {
  return files.map((rel) => {
    const abs = path.join(root, rel);
    const source = fs.readFileSync(abs, 'utf8');
    return { rel, result: extractFile({ bin, projectId, filePath: rel, absPath: abs, source }) };
  });
}

export function fullIndex({ db, bin, root, projectId }) {
  const started = Date.now();
  const files = walkFiles(root);
  const version = extractorVersion();
  const head = gitHeadOf(root);

  // Extraction runs outside the transaction: subprocesses must not hold a write lock.
  const extracted = extractAll({ bin, root, projectId, files });

  let chunkCount = 0;
  let unparsedCount = 0;
  let relCount = 0;

  const run = db.transaction(() => {
    db.prepare(`INSERT INTO projects (project_id, name, path, git_head, last_indexed_at, extractor_version)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(project_id) DO UPDATE SET
                  name = excluded.name, path = excluded.path, git_head = excluded.git_head,
                  last_indexed_at = excluded.last_indexed_at, extractor_version = excluded.extractor_version`)
      .run(projectId, path.basename(root), root, head, Date.now(), version);

    db.prepare('DELETE FROM chunks WHERE project_id = ?').run(projectId);
    db.prepare('DELETE FROM file_state WHERE project_id = ?').run(projectId);

    const insertChunk = db.prepare(INSERT_CHUNK);
    const insertState = db.prepare(`INSERT INTO file_state (project_id, file_path, file_content_hash)
      VALUES (?, ?, ?) ON CONFLICT(project_id, file_path)
      DO UPDATE SET file_content_hash = excluded.file_content_hash, updated_at = datetime('now')`);

    for (const { rel, result } of extracted) {
      if (result.unparsed) unparsedCount += 1;
      for (const c of result.chunks) { insertChunk.run(c); chunkCount += 1; }
      insertState.run(projectId, rel, result.file_content_hash);
    }

    const insertRel = db.prepare(`INSERT INTO relationships
      (source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score, confidence_reasoning)
      VALUES (@source_chunk_id, @target_chunk_id, @rel_type, @confidence, @confidence_score, @confidence_reasoning)
      ON CONFLICT(source_chunk_id, target_chunk_id, rel_type) DO NOTHING`);
    for (const { rel, result } of extracted) {
      const rows = relationshipRowsForFile({
        db, projectId, filePath: rel, chunks: result.chunks, edges: result.edges,
      });
      for (const row of rows) { insertRel.run(row); relCount += 1; }
    }

    setMeta(db, 'extractor_version', version);
  });

  run();

  return {
    files: files.length, chunks: chunkCount, unparsed: unparsedCount,
    relationships: relCount, elapsed_ms: Date.now() - started,
  };
}

export function statusOf({ db, root, projectId }) {
  const proj = db.prepare('SELECT * FROM projects WHERE project_id = ?').get(projectId);
  if (!proj) return { project_id: projectId, path: root, indexed: false };
  return {
    project_id: projectId,
    path: proj.path,
    indexed: true,
    files: db.prepare('SELECT COUNT(*) c FROM file_state WHERE project_id = ?').get(projectId).c,
    chunks: db.prepare('SELECT COUNT(*) c FROM chunks WHERE project_id = ?').get(projectId).c,
    readings: db.prepare('SELECT COUNT(*) c FROM reading_notes WHERE project_id = ?').get(projectId).c,
    relationships: db.prepare(`SELECT COUNT(*) c FROM relationships r
      JOIN chunks s ON s.chunk_id = r.source_chunk_id WHERE s.project_id = ?`).get(projectId).c,
    extractor_version: proj.extractor_version,
    git_head: proj.git_head,
    last_indexed_at: proj.last_indexed_at,
  };
}
