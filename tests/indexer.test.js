import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { openDb, ensureSchema, projectIdFor, getMeta } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { extractorVersion } from '../src/pack.js';
import { walkFiles, fullIndex, statusOf } from '../src/indexer.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

function setup(files = SAMPLE) {
  const root = makeProject(files);
  const db = openDb(':memory:');
  ensureSchema(db);
  return { root, db, projectId: projectIdFor(root), bin: resolveAstGrepBin() };
}

test('walkFiles skips ignored dirs and non-source extensions', () => {
  const { root } = setup();
  assert.deepEqual(walkFiles(root), ['src/alpha.ts', 'src/helper.ts']);
});

test('a full index writes chunks, fts rows, file_state and meta', () => {
  const { root, db, projectId, bin } = setup();
  const stats = fullIndex({ db, bin, root, projectId });
  assert.equal(stats.files, 2);
  assert.ok(stats.chunks >= 4);
  assert.equal(stats.unparsed, 0);

  const chunks = db.prepare('SELECT * FROM chunks WHERE project_id = ? ORDER BY file_path, start_line').all(projectId);
  assert.ok(chunks.some((c) => c.symbol_name === 'alpha' && c.chunk_type === 'function'));
  assert.ok(chunks.some((c) => c.symbol_name === 'Beta' && c.chunk_type === 'class'));
  assert.ok(chunks.some((c) => c.symbol_name === 'go' && c.chunk_type === 'method'));
  assert.ok(chunks.every((c) => c.chunk_source === 'ast'));

  assert.ok(db.prepare("SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH 'helper'").all().length > 0);

  const states = db.prepare('SELECT * FROM file_state WHERE project_id = ?').all(projectId);
  assert.equal(states.length, 2);
  assert.ok(states.every((s) => /^[0-9a-f]{64}$/.test(s.file_content_hash)));

  assert.equal(getMeta(db, 'extractor_version'), extractorVersion());
  const proj = db.prepare('SELECT * FROM projects WHERE project_id = ?').get(projectId);
  assert.equal(proj.path, root);
  assert.equal(proj.extractor_version, extractorVersion());
  assert.ok(proj.last_indexed_at > 0);
});

test('re-indexing is idempotent — no duplicate chunks, no orphan fts rows', () => {
  const { root, db, projectId, bin } = setup();
  fullIndex({ db, bin, root, projectId });
  const first = db.prepare('SELECT COUNT(*) c FROM chunks').get().c;
  const ftsFirst = db.prepare('SELECT COUNT(*) c FROM chunks_fts').get().c;
  fullIndex({ db, bin, root, projectId });
  assert.equal(db.prepare('SELECT COUNT(*) c FROM chunks').get().c, first);
  assert.equal(db.prepare('SELECT COUNT(*) c FROM chunks_fts').get().c, ftsFirst);
});

test('an unparsable file is indexed as unparsed without failing the run', () => {
  const { root, db, projectId, bin } = setup({
    'src/ok.ts': 'export function ok() { return 1; }\n',
    'src/bad.ts': 'function (((\n',
  });
  const stats = fullIndex({ db, bin, root, projectId });
  assert.equal(stats.files, 2);
  assert.equal(stats.unparsed, 1);
  const bad = db.prepare("SELECT * FROM chunks WHERE file_path = 'src/bad.ts'").all();
  assert.equal(bad.length, 1);
  assert.equal(bad[0].chunk_source, 'unparsed');
  assert.equal(db.prepare("SELECT COUNT(*) c FROM chunks WHERE file_path = 'src/ok.ts'").get().c, 1);
});

test('the whole index is one transaction — a mid-run failure leaves the db untouched', () => {
  const { root, db, projectId, bin } = setup();
  fullIndex({ db, bin, root, projectId });
  const before = db.prepare('SELECT COUNT(*) c FROM chunks').get().c;
  fs.writeFileSync(path.join(root, 'src/new.ts'), 'export function added() {}\n');

  const realPrepare = db.prepare.bind(db);
  let inserts = 0;
  db.prepare = (sql) => {
    const stmt = realPrepare(sql);
    if (!sql.startsWith('INSERT INTO chunks')) return stmt;
    const realRun = stmt.run.bind(stmt);
    stmt.run = (...args) => {
      inserts += 1;
      if (inserts === 2) throw new Error('boom');
      return realRun(...args);
    };
    return stmt;
  };
  try {
    assert.throws(() => fullIndex({ db, bin, root, projectId }), /boom/);
  } finally { db.prepare = realPrepare; }

  assert.equal(db.prepare('SELECT COUNT(*) c FROM chunks').get().c, before,
    'a failed full index must roll back entirely, leaving the previous index readable');
  assert.equal(db.prepare("SELECT COUNT(*) c FROM chunks WHERE file_path = 'src/new.ts'").get().c, 0);
});

test('statusOf reports the indexed project without touching ast-grep', () => {
  const { root, db, projectId, bin } = setup();
  fullIndex({ db, bin, root, projectId });
  const s = statusOf({ db, root, projectId });
  assert.equal(s.project_id, projectId);
  assert.equal(s.path, root);
  assert.equal(s.files, 2);
  assert.equal(s.extractor_version, extractorVersion());
  assert.equal(s.git_head, null, 'the fixture is not a git repo');
});
