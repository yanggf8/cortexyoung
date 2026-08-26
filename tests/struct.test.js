import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import {
  MAX_MALFORMED_RATIO, MAX_NEIGHBORS,
  preflightPattern, runPattern, containmentJoin, structCommand,
} from '../src/struct.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

const FAKE = path.join(process.cwd(), 'tests/fixtures/fake-ast-grep.js');

function indexed(files = SAMPLE) {
  const root = makeProject(files);
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  const bin = resolveAstGrepBin();
  fullIndex({ db, bin, root, projectId });
  return { root, db, projectId, bin };
}

test('constants match the spec', () => {
  assert.equal(MAX_MALFORMED_RATIO, 0.10);
  assert.equal(MAX_NEIGHBORS, 3);
});

test('a malformed pattern is caught by the pre-flight, not by the exit code', () => {
  const { root, bin } = indexed();
  let err;
  try { preflightPattern({ bin, pattern: 'function (', lang: 'ts', paths: [root] }); assert.fail('should throw'); } catch (e) { err = e; }
  assert.equal(err.code, 'parse_failed');
  assert.equal(err.detail.pattern, 'function (');
  assert.equal(err.detail.lang, 'ts');
});

test('a valid pattern passes the pre-flight', () => {
  const { root, bin } = indexed();
  assert.doesNotThrow(() => preflightPattern({ bin, pattern: 'helper($A)', lang: 'ts', paths: [root] }));
});

test('zero matches is a clean empty result, never parse_failed', () => {
  const { root, bin } = indexed();
  const r = runPattern({ bin, pattern: 'zzzNoSuchFunction($A)', lang: 'ts', paths: [root] });
  assert.deepEqual(r.matches, []);
  assert.equal(r.malformed, 0);
});

test('matches are returned with 1-indexed lines', () => {
  const { root, bin } = indexed();
  const r = runPattern({ bin, pattern: 'helper($A)', lang: 'ts', paths: [root] });
  assert.ok(r.matches.length >= 1);
  assert.ok(r.matches.every((m) => m.start_line >= 1));
  assert.ok(r.matches.every((m) => typeof m.file === 'string'));
});

test('a few malformed JSON lines are skipped and counted', () => {
  const good = JSON.stringify({ text: 'x', file: 'a.ts', range: { start: { line: 0 }, end: { line: 0 } } });
  const stream = `${good}\n`.repeat(19) + 'junk\n';        // 1 of 20 = 5%, under the ratio
  process.env.FAKE_AG_MODE = 'emit:' + Buffer.from(stream).toString('base64');
  try {
    const r = runPattern({ bin: FAKE, pattern: 'x', lang: 'ts', paths: ['.'], skipPreflight: true });
    assert.equal(r.malformed, 1);
    assert.equal(r.matches.length, 19);
  } finally { delete process.env.FAKE_AG_MODE; }
});

test('more than 10% malformed aborts THIS query only', () => {
  const good = JSON.stringify({ text: 'x', file: 'a.ts', range: { start: { line: 0 }, end: { line: 0 } } });
  const stream = `${good}\n`.repeat(8) + 'junk\njunk\n';   // 2 of 10 = 20%, over the ratio
  process.env.FAKE_AG_MODE = 'emit:' + Buffer.from(stream).toString('base64');
  try {
    let err;
    try { runPattern({ bin: FAKE, pattern: 'x', lang: 'ts', paths: ['.'], skipPreflight: true }); assert.fail('should throw'); } catch (e) { err = e; }
    assert.equal(err.code, 'run_aborted_malformed');
    assert.equal(err.detail.malformed, 2);
    assert.equal(err.detail.total, 10);
  } finally { delete process.env.FAKE_AG_MODE; }
});

test('containmentJoin picks the smallest chunk that contains the match', () => {
  const { db, projectId } = indexed();
  // `go() { return alpha(2); }` sits inside both the Beta class chunk and the go method chunk.
  const hit = containmentJoin(db, projectId, { file_path: 'src/alpha.ts', start_line: 4, end_line: 4 });
  assert.ok(hit);
  assert.equal(hit.symbol_name, 'go', 'the method must win over the enclosing class');
});

test('containmentJoin returns null when no chunk contains the match', () => {
  const { db, projectId } = indexed();
  assert.equal(containmentJoin(db, projectId, { file_path: 'src/alpha.ts', start_line: 9999, end_line: 9999 }), null);
});

test('structCommand attaches at most MAX_NEIGHBORS neighbours and reports staleness', () => {
  const { root, db, projectId, bin } = indexed();
  const out = structCommand({
    db, bin, root, projectId, pattern: 'helper($A)', lang: 'ts', globs: [], budget: 1500,
  });
  assert.ok(out.matches.length >= 1);
  const m = out.matches[0];
  assert.equal(m.symbol_name, 'alpha');
  assert.ok(Array.isArray(m.neighbors));
  assert.ok(m.neighbors.length <= MAX_NEIGHBORS);
  assert.equal(typeof out.index_is_stale, 'boolean');
  assert.equal(typeof out.truncated, 'boolean');
});

test('structCommand surfaces parse_failed as a structured error and runs nothing', () => {
  const { root, db, projectId, bin } = indexed();
  let err;
  try { structCommand({ db, bin, root, projectId, pattern: 'function (', lang: 'ts', globs: [], budget: 1500 }); assert.fail('should throw'); } catch (e) { err = e; }
  assert.equal(err.code, 'parse_failed');
  assert.deepEqual(err.toJSON().error, 'parse_failed');
});

test('an unglobbed scan of a large project is refused with actionable advice', () => {
  const files = {};
  for (let i = 0; i < 12; i += 1) files[`src/f${i}.ts`] = `export function f${i}() { return ${i}; }\n`;
  const { root, db, projectId, bin } = indexed(files);
  let err;
  try { structCommand({ db, bin, root, projectId, pattern: 'f0()', lang: 'ts', globs: [], budget: 1500, fileLimit: 10 }); assert.fail('should throw'); } catch (e) { err = e; }
  assert.equal(err.code, 'scan_too_broad');
  assert.equal(err.detail.indexed_files, 12);
  assert.equal(err.detail.limit, 10);
  assert.match(err.detail.hint, /-g/);
});

test('the same scan succeeds once a glob narrows it', () => {
  const files = {};
  for (let i = 0; i < 12; i += 1) files[`src/f${i}.ts`] = `export function f${i}() { return ${i}; }\n`;
  const { root, db, projectId, bin } = indexed(files);
  assert.doesNotThrow(() => structCommand({
    db, bin, root, projectId, pattern: 'f0()', lang: 'ts',
    globs: [`${root}/src/f0.ts`], budget: 1500, fileLimit: 10,
  }));
});
