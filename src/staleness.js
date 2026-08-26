import fs from 'node:fs';
import path from 'node:path';
import { extractFile } from './chunker.js';
import { walkFiles } from './indexer.js';
import { gitCandidates } from './incremental.js';

export function computeStale({ db, bin, root, projectId }) {
  const proj = db.prepare('SELECT path FROM projects WHERE project_id = ?').get(projectId);
  const base = proj ? proj.path : root;                 // always projects.path, never cwd

  const dbFiles = db.prepare('SELECT file_path, file_content_hash FROM file_state WHERE project_id = ?')
    .all(projectId);
  const stored = new Map(dbFiles.map((r) => [r.file_path, r.file_content_hash]));
  const diskFiles = new Set(walkFiles(base));

  const deleted = [...stored.keys()].filter((f) => !diskFiles.has(f)).sort();

  const { changed, gitAvailable } = gitCandidates(base);
  const candidates = gitAvailable
    ? [...new Set([...changed, ...[...diskFiles].filter((f) => !stored.has(f))])].sort()
    : [...diskFiles].sort();

  const changedFiles = [];
  for (const rel of candidates) {
    const abs = path.join(base, rel);
    if (!fs.existsSync(abs)) continue;
    const source = fs.readFileSync(abs, 'utf8');
    const result = extractFile({ bin, projectId, filePath: rel, absPath: abs, source });
    if (stored.get(rel) !== result.file_content_hash) changedFiles.push(rel);
  }

  return {
    index_is_stale: deleted.length > 0 || changedFiles.length > 0,
    deleted_files: deleted,
    changed_files: changedFiles,
  };
}
