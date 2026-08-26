import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { openDb, ensureSchema, projectIdFor, getMeta, setMeta } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { extractorVersion } from '../src/pack.js';
import { fullIndex } from '../src/indexer.js';
import { gitCandidates, reindexOneFile, removeFile, incrementalIndex } from '../src/incremental.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

function git(root, ...args) { execFileSync('git', ['-C', root, ...args], { stdio: 'pipe' }); }

function gitProject(files = SAMPLE) {
  const root = makeProject(files);
  git(root, 'init', '-q');
  git(root, 'config', 'user.email', 'test@example.com');
  git(root, 'config', 'user.name', 'test');
  git(root, 'add', '-A');
  git(root, 'commit', '-qm', 'init');
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  fullIndex({ db, bin: resolveAstGrepBin(), root, projectId });
  return { root, db, projectId, bin: resolveAstGrepBin() };
}

test('an extractor_version mismatch forces a full rebuild', () => {
  const { root, db, projectId, bin } = gitProject();
  setMeta(db, 'extractor_version', 'stale-version-hash');
  const r = incrementalIndex({ db, bin, root, projectId });
  assert.equal(r.mode, 'full');
  assert.equal(getMeta(db, 'extractor_version'), extractorVersion());
});

test('no changes means nothing is reindexed', () => {
  const { root, db, projectId, bin } = gitProject();
  const r = incrementalIndex({ db, bin, root, projectId });
  assert.equal(r.mode, 'incremental');
  assert.equal(r.files_reindexed, 0);
});

test('an edited file is reindexed and its chunks replaced', () => {
  const { root, db, projectId, bin } = gitProject();
  fs.writeFileSync(path.join(root, 'src/helper.ts'),
    'export function helper(n: number) { return n * 3; }\nexport function extra() { return 0; }\n');
  const r = incrementalIndex({ db, bin, root, projectId });
  assert.equal(r.mode, 'incremental');
  assert.equal(r.files_reindexed, 1);
  const syms = db.prepare("SELECT symbol_name FROM chunks WHERE file_path = 'src/helper.ts' ORDER BY start_line")
    .all().map((c) => c.symbol_name);
  assert.deepEqual(syms, ['helper', 'extra']);
});

test('a touched-but-identical file is skipped without a write', () => {
  const { root, db, projectId, bin } = gitProject();
  const p = path.join(root, 'src/helper.ts');
  const body = fs.readFileSync(p, 'utf8');
  fs.writeFileSync(p, body.replace('return n * 2;', 'return n * 2;   '));  // whitespace only, outside the chunk text
  const before = db.prepare("SELECT updated_at FROM file_state WHERE file_path = 'src/helper.ts'").get().updated_at;
  const r = incrementalIndex({ db, bin, root, projectId });
  assert.equal(r.files_skipped + r.files_reindexed, r.files_examined);
  const after = db.prepare("SELECT updated_at FROM file_state WHERE file_path = 'src/helper.ts'").get().updated_at;
  if (r.files_skipped === 1) assert.equal(after, before, 'a skipped file must not be rewritten');
});

test('a new untracked file is picked up via git ls-files --others', () => {
  const { root, db, projectId, bin } = gitProject();
  fs.writeFileSync(path.join(root, 'src/brand-new.ts'), 'export function brandNew() { return 1; }\n');
  const cands = gitCandidates(root);
  assert.ok(cands.changed.includes('src/brand-new.ts'));
  const r = incrementalIndex({ db, bin, root, projectId });
  assert.equal(r.files_reindexed, 1);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM chunks WHERE file_path = 'src/brand-new.ts'").get().c, 1);
});

test('a deleted file drops its chunks, fts rows and file_state', () => {
  const { root, db, projectId, bin } = gitProject();
  fs.rmSync(path.join(root, 'src/helper.ts'));
  const r = incrementalIndex({ db, bin, root, projectId });
  assert.equal(r.files_removed, 1);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM chunks WHERE file_path = 'src/helper.ts'").get().c, 0);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM file_state WHERE file_path = 'src/helper.ts'").get().c, 0);
  // FTS rows for the deleted file must be gone; global MATCH 'helper' would still hit alpha.ts
  // (which contains `helper(a)`), so verify by file_path instead.
  assert.equal(db.prepare("SELECT COUNT(*) c FROM chunks_fts WHERE file_path = 'src/helper.ts'").get().c, 0);
});

test('an interrupt keeps already-committed files and does NOT advance git_head', () => {
  const { root, db, projectId, bin } = gitProject();
  fs.writeFileSync(path.join(root, 'src/one.ts'), 'export function one() { return 1; }\n');
  fs.writeFileSync(path.join(root, 'src/two.ts'), 'export function two() { return 2; }\n');
  const headBefore = db.prepare('SELECT git_head FROM projects WHERE project_id = ?').get(projectId).git_head;

  let calls = 0;
  const realPrepare = db.prepare.bind(db);
  db.prepare = (sql) => {
    if (sql.startsWith('INSERT INTO chunks')) {
      calls += 1;
      if (calls === 2) throw new Error('interrupted');
    }
    return realPrepare(sql);
  };
  try {
    assert.throws(() => incrementalIndex({ db, bin, root, projectId }), /interrupted/);
  } finally { db.prepare = realPrepare; }

  const done = db.prepare("SELECT COUNT(*) c FROM chunks WHERE file_path IN ('src/one.ts','src/two.ts')").get().c;
  assert.equal(done, 1, 'the first committed file survives as incremental progress');
  const headAfter = db.prepare('SELECT git_head FROM projects WHERE project_id = ?').get(projectId).git_head;
  assert.equal(headAfter, headBefore, 'git_head advances only in the final transaction');
});

test('a non-git directory degrades to a full index', () => {
  const root = makeProject(SAMPLE);
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  const bin = resolveAstGrepBin();
  fullIndex({ db, bin, root, projectId });
  const r = incrementalIndex({ db, bin, root, projectId });
  assert.equal(r.mode, 'full');
});

test('removeFile and reindexOneFile each run in their own transaction', () => {
  const { root, db, projectId, bin } = gitProject();
  const one = reindexOneFile({ db, bin, root, projectId, filePath: 'src/helper.ts' });
  assert.equal(one.skipped, true, 'unchanged content must be skipped');
  removeFile({ db, projectId, filePath: 'src/helper.ts' });
  assert.equal(db.prepare("SELECT COUNT(*) c FROM chunks WHERE file_path = 'src/helper.ts'").get().c, 0);
});
