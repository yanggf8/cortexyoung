import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { extractFile } from './chunker.js';
import { relationshipRowsForFile } from './graph.js';
import { extractorVersion } from './pack.js';
import { getMeta, setMeta } from './db.js';
import { fullIndex, gitHeadOf, SOURCE_EXT, IGNORE_DIRS } from './indexer.js';

function git(root, args) {
  const r = spawnSync('git', ['-C', root, ...args], { encoding: 'utf8' });
  if (r.error || r.status !== 0) return null;
  return r.stdout;
}

function isIndexable(rel) {
  if (!SOURCE_EXT.has(path.extname(rel))) return false;
  return !rel.split('/').some((seg) => IGNORE_DIRS.has(seg));
}

export function gitCandidates(root) {
  const diff = git(root, ['diff', '--name-status', '-M', 'HEAD']);
  if (diff === null) return { changed: [], deleted: [], gitAvailable: false };
  const changed = new Set();
  const deleted = new Set();
  for (const line of diff.split('\n').filter(Boolean)) {
    const parts = line.split('\t');
    const status = parts[0];
    if (status.startsWith('R')) {              // rename: old path dies, new path is rebuilt
      if (isIndexable(parts[1])) deleted.add(parts[1]);
      if (isIndexable(parts[2])) changed.add(parts[2]);
    } else if (status.startsWith('D')) {
      if (isIndexable(parts[1])) deleted.add(parts[1]);
    } else if (isIndexable(parts[1])) {
      changed.add(parts[1]);
    }
  }
  const others = git(root, ['ls-files', '--others', '--exclude-standard']) ?? '';
  for (const rel of others.split('\n').filter(Boolean)) if (isIndexable(rel)) changed.add(rel);
  return { changed: [...changed].sort(), deleted: [...deleted].sort(), gitAvailable: true };
}

const INSERT_CHUNK = `INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
  start_line, end_line, content, content_hash, language, chunk_source)
  VALUES (@chunk_id, @project_id, @file_path, @symbol_name, @chunk_type,
  @start_line, @end_line, @content, @content_hash, @language, @chunk_source)`;

export function removeFile({ db, projectId, filePath }) {
  db.transaction(() => {
    db.prepare('DELETE FROM chunks WHERE project_id = ? AND file_path = ?').run(projectId, filePath);
    db.prepare('DELETE FROM file_state WHERE project_id = ? AND file_path = ?').run(projectId, filePath);
  })();
}

export function reindexOneFile({ db, bin, root, projectId, filePath }) {
  const abs = path.join(root, filePath);
  if (!fs.existsSync(abs)) { removeFile({ db, projectId, filePath }); return { chunks: 0, unparsed: 0, relationships: 0, skipped: false, removed: true }; }

  const source = fs.readFileSync(abs, 'utf8');
  const result = extractFile({ bin, projectId, filePath, absPath: abs, source });

  const prior = db.prepare('SELECT file_content_hash FROM file_state WHERE project_id = ? AND file_path = ?')
    .get(projectId, filePath);
  if (prior && prior.file_content_hash === result.file_content_hash) {
    return { chunks: 0, unparsed: 0, relationships: 0, skipped: true, removed: false };
  }

  let relationships = 0;
  db.transaction(() => {
    db.prepare('DELETE FROM chunks WHERE project_id = ? AND file_path = ?').run(projectId, filePath);
    const insertChunk = db.prepare(INSERT_CHUNK);
    for (const c of result.chunks) insertChunk.run(c);
    db.prepare(`INSERT INTO file_state (project_id, file_path, file_content_hash) VALUES (?, ?, ?)
      ON CONFLICT(project_id, file_path) DO UPDATE SET
        file_content_hash = excluded.file_content_hash, updated_at = datetime('now')`)
      .run(projectId, filePath, result.file_content_hash);

    const insertRel = db.prepare(`INSERT INTO relationships
      (source_chunk_id, target_chunk_id, rel_type, confidence, confidence_score, confidence_reasoning)
      VALUES (@source_chunk_id, @target_chunk_id, @rel_type, @confidence, @confidence_score, @confidence_reasoning)
      ON CONFLICT(source_chunk_id, target_chunk_id, rel_type) DO NOTHING`);
    for (const row of relationshipRowsForFile({
      db, projectId, filePath, chunks: result.chunks, edges: result.edges,
    })) { insertRel.run(row); relationships += 1; }
  })();

  return {
    chunks: result.chunks.length, unparsed: result.unparsed ? 1 : 0,
    relationships, skipped: false, removed: false,
  };
}

export function incrementalIndex({ db, bin, root, projectId }) {
  const started = Date.now();
  const version = extractorVersion();
  const stored = getMeta(db, 'extractor_version');

  if (stored !== null && stored !== version) {
    process.stderr.write(`extractor_version mismatch: ${stored} -> ${version}, full reindex required\n`);
    const full = fullIndex({ db, bin, root, projectId });
    return { mode: 'full', ...full, elapsed_ms: Date.now() - started };
  }

  const { changed, deleted, gitAvailable } = gitCandidates(root);
  if (!gitAvailable) {
    const full = fullIndex({ db, bin, root, projectId });
    return { mode: 'full', ...full, elapsed_ms: Date.now() - started };
  }

  let reindexed = 0;
  let skipped = 0;
  let removed = 0;
  let relationships = 0;

  for (const filePath of deleted) { removeFile({ db, projectId, filePath }); removed += 1; }
  for (const filePath of changed) {
    const r = reindexOneFile({ db, bin, root, projectId, filePath });
    if (r.removed) removed += 1;
    else if (r.skipped) skipped += 1;
    else { reindexed += 1; relationships += r.relationships; }
  }

  // Final, separate transaction: the freshness markers advance only once every file landed.
  db.transaction(() => {
    db.prepare('UPDATE projects SET git_head = ?, last_indexed_at = ?, extractor_version = ? WHERE project_id = ?')
      .run(gitHeadOf(root), Date.now(), version, projectId);
    setMeta(db, 'extractor_version', version);
  })();

  return {
    mode: 'incremental',
    files_examined: changed.length + deleted.length,
    files_reindexed: reindexed,
    files_skipped: skipped,
    files_removed: removed,
    relationships,
    elapsed_ms: Date.now() - started,
  };
}
