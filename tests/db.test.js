import test from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import {
  SCHEMA_VERSION, projectIdFor, dbPathFor, openDb, ensureSchema, getMeta, setMeta,
} from '../src/db.js';

function fresh() { const db = openDb(':memory:'); ensureSchema(db); return db; }

test('project id is a stable sha256 of the real path', () => {
  const a = projectIdFor('/tmp/some/project');
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.equal(a, projectIdFor('/tmp/some/project'));
  assert.notEqual(a, projectIdFor('/tmp/other/project'));
});

test('db path lands under the cortex-ng cache keyed by project id', () => {
  const p = dbPathFor('/tmp/some/project');
  assert.equal(p, path.join(os.homedir(), '.cache', 'cortex-ng', `${projectIdFor('/tmp/some/project')}.db`));
});

test('ensureSchema is idempotent and records the schema version', () => {
  const db = fresh();
  assert.equal(getMeta(db, 'SCHEMA_VERSION'), String(SCHEMA_VERSION));
  assert.doesNotThrow(() => ensureSchema(db));
  assert.equal(getMeta(db, 'SCHEMA_VERSION'), String(SCHEMA_VERSION));
});

test('schema uses the V6 column names required by the spec', () => {
  const db = fresh();
  const cols = (t) => db.prepare(`PRAGMA table_info(${t})`).all().map((r) => r.name);
  assert.ok(cols('projects').includes('project_id'));
  assert.ok(cols('chunks').includes('chunk_id'));
  assert.ok(cols('chunks').includes('chunk_source'));
  assert.ok(!cols('chunks').includes('embedding'));
  assert.ok(cols('relationships').includes('rel_type'));
  assert.ok(cols('file_state').includes('file_content_hash'));
  const tables = db.prepare("SELECT name FROM sqlite_master WHERE type='table'").all().map((r) => r.name);
  assert.ok(!tables.includes('unresolved_refs'));
});

test('relationships primary key is the composite triple', () => {
  const db = fresh();
  const pk = db.prepare('PRAGMA table_info(relationships)').all()
    .filter((r) => r.pk > 0).sort((a, b) => a.pk - b.pk).map((r) => r.name);
  assert.deepEqual(pk, ['source_chunk_id', 'target_chunk_id', 'rel_type']);
});

test('fts triggers mirror chunk writes', () => {
  const db = fresh();
  db.prepare("INSERT INTO projects (project_id, name, path, extractor_version) VALUES ('p','n','/n','v')").run();
  db.prepare(`INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type,
    start_line, end_line, content, content_hash, language, chunk_source)
    VALUES ('p:a.ts:1','p','a.ts','alpha','function',1,3,'function alpha() {}','h','TypeScript','ast')`).run();
  const hit = db.prepare("SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH 'alpha'").all();
  assert.equal(hit.length, 1);
  db.prepare("DELETE FROM chunks WHERE chunk_id = 'p:a.ts:1'").run();
  assert.equal(db.prepare("SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH 'alpha'").all().length, 0);
});

test('zero-target relationships are impossible: target_chunk_id is NOT NULL', () => {
  const db = fresh();
  const notNull = db.prepare('PRAGMA table_info(relationships)').all()
    .find((r) => r.name === 'target_chunk_id').notnull;
  assert.equal(notNull, 1);
});
