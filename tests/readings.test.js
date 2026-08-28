import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import { readFragment, recallReadings } from '../src/readings.js';
import { makeProject } from './helpers/tmp-project.js';

const BODY = [
  'first line',
  'database lookup detail',
  'third line',
  'fourth line',
  '',
].join('\n');

function setup() {
  const root = makeProject({ 'notes.txt': BODY, 'src/seed.ts': 'export function seed() { return 1; }\n' });
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  fullIndex({ db, bin: resolveAstGrepBin(), root, projectId });
  return { root, db, projectId };
}

test('reading notes require an indexed project', () => {
  const root = makeProject({ 'notes.txt': BODY });
  const db = openDb(':memory:');
  ensureSchema(db);
  assert.throws(() => readFragment({
    db, root, projectId: projectIdFor(root), filePath: 'notes.txt',
  }), (err) => err.code === 'project_not_indexed');
});

test('a first fragment read is persisted and an unchanged repeat comes from the store', () => {
  const { root, db, projectId } = setup();
  const first = readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 2, endLine: 3 });
  assert.equal(first.source, 'filesystem');
  assert.equal(first.content, 'database lookup detail\nthird line');
  const second = readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 2, endLine: 3 });
  assert.equal(second.source, 'store');
  assert.equal(second.content, first.content);
  assert.equal(second.read_count, 2);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM reading_notes_fts WHERE reading_notes_fts MATCH 'database'").get().c, 1);
});

test('a stored whole-file reading serves later subranges without another filesystem payload', () => {
  const { root, db, projectId } = setup();
  const whole = readFragment({ db, root, projectId, filePath: 'notes.txt' });
  assert.equal(whole.content, BODY);
  const subset = readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 2, endLine: 2 });
  assert.equal(subset.source, 'store');
  assert.equal(subset.content, 'database lookup detail');
});

test('a partial note never masquerades as a whole-file cache entry', () => {
  const { root, db, projectId } = setup();
  readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 1, endLine: 2 });
  const whole = readFragment({ db, root, projectId, filePath: 'notes.txt' });
  assert.equal(whole.source, 'filesystem');
  assert.equal(whole.content, BODY);
});

test('an omitted end line caches the requested start through EOF', () => {
  const { root, db, projectId } = setup();
  const first = readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 2 });
  assert.equal(first.source, 'filesystem');
  assert.ok(first.content.startsWith('database lookup detail'));
  const second = readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 3 });
  assert.equal(second.source, 'store');
  assert.ok(second.content.startsWith('third line'));
});

test('unchanged reading notes survive a full re-index', () => {
  const { root, db, projectId } = setup();
  readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 2, endLine: 3 });
  fullIndex({ db, bin: resolveAstGrepBin(), root, projectId });
  const second = readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 2, endLine: 3 });
  assert.equal(second.source, 'store');
  assert.equal(second.read_count, 2);
});

test('FTS recall returns stored readings and drops them after the source changes', () => {
  const { root, db, projectId } = setup();
  readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 1, endLine: 3 });
  const found = recallReadings({ db, root, projectId, query: 'database' });
  assert.equal(found.reading_count, 1);
  assert.equal(found.readings[0].file_path, 'notes.txt');
  assert.ok(found.readings[0].content.includes('database lookup detail'));

  fs.writeFileSync(path.join(root, 'notes.txt'), `${BODY}changed\n`);
  const stale = recallReadings({ db, root, projectId, query: 'database' });
  assert.equal(stale.reading_count, 0);
  assert.equal(db.prepare('SELECT COUNT(*) c FROM reading_notes').get().c, 0);
});

test('reading rejects paths outside the indexed project and invalid ranges', () => {
  const { root, db, projectId } = setup();
  assert.throws(() => readFragment({ db, root, projectId, filePath: '../outside' }),
    (err) => err.code === 'file_not_found' || err.code === 'path_outside_project');
  assert.throws(() => readFragment({ db, root, projectId, filePath: 'notes.txt', startLine: 3, endLine: 2 }),
    (err) => err.code === 'invalid_line_range');
});
