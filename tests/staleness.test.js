import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import { computeStale } from '../src/staleness.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

function git(root, ...args) { execFileSync('git', ['-C', root, ...args], { stdio: 'pipe' }); }

function setup(files = SAMPLE) {
  const root = makeProject(files);
  git(root, 'init', '-q');
  git(root, 'config', 'user.email', 't@e.com');
  git(root, 'config', 'user.name', 't');
  git(root, 'add', '-A');
  git(root, 'commit', '-qm', 'init');
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  const bin = resolveAstGrepBin();
  fullIndex({ db, bin, root, projectId });
  return { root, db, projectId, bin };
}

test('a freshly indexed clean tree is not stale', () => {
  const { root, db, projectId, bin } = setup();
  const s = computeStale({ db, bin, root, projectId });
  assert.equal(s.index_is_stale, false);
  assert.deepEqual(s.deleted_files, []);
});

test('a dirty-but-semantically-identical file is NOT stale', () => {
  const { root, db, projectId, bin } = setup();
  const p = path.join(root, 'src/alpha.ts');
  fs.writeFileSync(path.join(root, 'src/alpha.ts'), `${fs.readFileSync(p, 'utf8')}\n// trailing comment\n`);
  const s = computeStale({ db, bin, root, projectId });
  assert.equal(s.index_is_stale, false,
    'git dirty alone must not mark the index stale — extraction output is unchanged');
});

test('a changed chunk body makes the index stale', () => {
  const { root, db, projectId, bin } = setup();
  fs.writeFileSync(path.join(root, 'src/helper.ts'), 'export function helper(n: number) { return n * 99; }\n');
  const s = computeStale({ db, bin, root, projectId });
  assert.equal(s.index_is_stale, true);
  assert.ok(s.changed_files.includes('src/helper.ts'));
});

test('an edge-only change makes the index stale', () => {
  const { root, db, projectId, bin } = setup();
  fs.writeFileSync(path.join(root, 'src/alpha.ts'), [
    "import { helper } from './helper';",
    'export function alpha(a: number) { return helper(a) + 1; }',
    'export class Beta {',
    '  go() { return helper(2); }',   // was alpha(2): same chunk text length class, different edge
    '}',
  ].join('\n') + '\n');
  const s = computeStale({ db, bin, root, projectId });
  assert.equal(s.index_is_stale, true, 'file_content_hash covers edges, not just chunk contents');
});

test('a deleted file makes the index stale and is reported', () => {
  const { root, db, projectId, bin } = setup();
  fs.rmSync(path.join(root, 'src/helper.ts'));
  const s = computeStale({ db, bin, root, projectId });
  assert.equal(s.index_is_stale, true);
  assert.deepEqual(s.deleted_files, ['src/helper.ts']);
});

test('staleness is computed against projects.path, not the cwd', () => {
  const { root, db, projectId, bin } = setup();
  const elsewhere = makeProject({ 'src/unrelated.ts': 'export function u() {}\n' });
  const prev = process.cwd();
  process.chdir(elsewhere);
  try {
    const s = computeStale({ db, bin, root, projectId });
    assert.equal(s.index_is_stale, false);
  } finally { process.chdir(prev); }
});
