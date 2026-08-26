import test from 'node:test';
import assert from 'node:assert/strict';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import { DEFAULT_DEPTH, impactCommand } from '../src/impact.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

function indexed(files = SAMPLE) {
  const root = makeProject(files);
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  const bin = resolveAstGrepBin();
  fullIndex({ db, bin, root, projectId });
  return { root, db, projectId, bin };
}

const CHAIN = {
  'src/d.ts': 'export function d() { return 1; }\n',
  'src/c.ts': "import { d } from './d';\nexport function c() { return d(); }\n",
  'src/b.ts': "import { c } from './c';\nexport function b() { return c(); }\n",
  'src/a.ts': "import { b } from './b';\nexport function a() { return b(); }\n",
};

test('the default depth is 3', () => {
  assert.equal(DEFAULT_DEPTH, 3);
});

test('dependents are returned with their hop distance', () => {
  const { root, db, projectId, bin } = indexed(CHAIN);
  const out = impactCommand({ db, bin, root, projectId, symbol: 'd', depth: DEFAULT_DEPTH });
  const byName = Object.fromEntries(out.dependents.map((x) => [x.symbol_name, x.hop]));
  assert.deepEqual(byName, { c: 1, b: 2, a: 3 });
});

test('depth is respected', () => {
  const { root, db, projectId, bin } = indexed(CHAIN);
  const out = impactCommand({ db, bin, root, projectId, symbol: 'd', depth: 1 });
  assert.deepEqual(out.dependents.map((x) => x.symbol_name), ['c']);
});

test('a symbol with no dependents returns an empty list, not an error', () => {
  const { root, db, projectId, bin } = indexed(CHAIN);
  const out = impactCommand({ db, bin, root, projectId, symbol: 'a', depth: DEFAULT_DEPTH });
  assert.deepEqual(out.dependents, []);
  assert.equal(out.seed_count, 1);
});

test('an unknown symbol reports zero seeds without throwing', () => {
  const { root, db, projectId, bin } = indexed(CHAIN);
  const out = impactCommand({ db, bin, root, projectId, symbol: 'nosuchsymbol', depth: DEFAULT_DEPTH });
  assert.equal(out.seed_count, 0);
  assert.deepEqual(out.dependents, []);
});

test('an ambiguous symbol seeds from every matching chunk', () => {
  const { root, db, projectId, bin } = indexed({
    'src/a.ts': 'export function dup() { return 1; }\n',
    'src/b.ts': 'export function dup() { return 2; }\n',
    'src/c.ts': 'export function caller() { return dup(); }\n',
  });
  const out = impactCommand({ db, bin, root, projectId, symbol: 'dup', depth: DEFAULT_DEPTH });
  assert.equal(out.seed_count, 2);
  assert.ok(out.dependents.some((d) => d.symbol_name === 'caller'));
});

test('unresolved references are inlined on the fly and nothing is persisted', () => {
  const { root, db, projectId, bin } = indexed({
    'src/only.ts': 'export function solo() { return externalThing(1); }\n',
  });
  const out = impactCommand({ db, bin, root, projectId, symbol: 'solo', depth: DEFAULT_DEPTH });
  assert.equal(out.unresolved.length, 1);
  assert.equal(out.unresolved[0].confidence_reasoning, 'unresolved: externalThing');
  assert.equal(db.prepare('SELECT COUNT(*) c FROM relationships').get().c, 0);
});

test('the packet reports index staleness', () => {
  const { root, db, projectId, bin } = indexed(CHAIN);
  const out = impactCommand({ db, bin, root, projectId, symbol: 'd', depth: DEFAULT_DEPTH });
  assert.equal(out.index_is_stale, false);
});
