import test from 'node:test';
import assert from 'node:assert/strict';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import { estimateTokens } from '../src/budget.js';
import { DEFAULT_BUDGET, contextCommand } from '../src/context.js';
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

test('the default budget is 1500 tokens', () => {
  assert.equal(DEFAULT_BUDGET, 1500);
});

test('an exact symbol name resolves without touching FTS', () => {
  const { root, db, projectId, bin } = indexed();
  const out = contextCommand({ db, bin, root, projectId, query: 'helper', budget: DEFAULT_BUDGET });
  assert.equal(out.resolution, 'exact_symbol');
  assert.equal(out.seeds.length, 1);
  assert.equal(out.seeds[0].symbol_name, 'helper');
});

test('a non-symbol query falls back to FTS', () => {
  const { root, db, projectId, bin } = indexed();
  const out = contextCommand({ db, bin, root, projectId, query: 'return', budget: DEFAULT_BUDGET });
  assert.equal(out.resolution, 'fts');
  assert.ok(out.seeds.length > 0);
});

test('seeds carry depth-1 neighbours', () => {
  const { root, db, projectId, bin } = indexed();
  const out = contextCommand({ db, bin, root, projectId, query: 'helper', budget: DEFAULT_BUDGET });
  const names = out.seeds[0].neighbors.map((n) => n.symbol_name);
  assert.ok(names.includes('alpha'));
});

test('AMBIGUOUS neighbours are dropped unless explicitly requested', () => {
  const { root, db, projectId, bin } = indexed({
    'src/a.ts': 'export function dup() { return 1; }\n',
    'src/b.ts': 'export function dup() { return 2; }\n',
    'src/c.ts': 'export function caller() { return dup(); }\n',
  });
  const strict = contextCommand({ db, bin, root, projectId, query: 'caller', budget: DEFAULT_BUDGET });
  assert.equal(strict.seeds[0].neighbors.filter((n) => n.confidence === 'AMBIGUOUS').length, 0);
  const loose = contextCommand({
    db, bin, root, projectId, query: 'caller', budget: DEFAULT_BUDGET, includeAmbiguous: true,
  });
  assert.ok(loose.seeds[0].neighbors.filter((n) => n.confidence === 'AMBIGUOUS').length > 0);
});

test('an unresolvable reference is inlined on the fly and never persisted', () => {
  const { root, db, projectId, bin } = indexed({
    'src/only.ts': 'export function solo() { return externalThing(1); }\n',
  });
  const out = contextCommand({ db, bin, root, projectId, query: 'solo', budget: DEFAULT_BUDGET });
  const u = out.seeds[0].unresolved;
  assert.ok(Array.isArray(u));
  assert.equal(u.length, 1);
  assert.equal(u[0].confidence_reasoning, 'unresolved: externalThing');
  assert.equal(u[0].confidence_score, 0.5);
  assert.ok(!('target_chunk_id' in u[0]));
  assert.equal(db.prepare('SELECT COUNT(*) c FROM relationships').get().c, 0);
});

test('the emitted JSON actually fits the budget and reports truncation', () => {
  const files = { 'src/hub.ts': 'export function hub() { return 1; }\n' };
  for (let i = 0; i < 40; i += 1) {
    files[`src/c${i}.ts`] = `import { hub } from './hub';\nexport function caller${i}() { return hub(); }\n`;
  }
  const { root, db, projectId, bin } = indexed(files);
  const out = contextCommand({ db, bin, root, projectId, query: 'hub', budget: 400 });
  assert.ok(estimateTokens(JSON.stringify(out)) <= 400 * 1.15,
    'the budget is measured on real output, with only packet overhead allowed on top');
  assert.equal(out.truncated, true);
});

test('an unknown query returns an empty packet rather than throwing', () => {
  const { root, db, projectId, bin } = indexed();
  const out = contextCommand({ db, bin, root, projectId, query: 'nothingmatchesthis', budget: DEFAULT_BUDGET });
  assert.deepEqual(out.seeds, []);
  assert.equal(out.resolution, 'none');
});

test('context never invokes struct', async () => {
  const mod = await import('../src/context.js');
  const src = await import('node:fs').then((fs) => fs.readFileSync(new URL('../src/context.js', import.meta.url), 'utf8'));
  assert.ok(!src.includes("from './struct.js'"), 'stage 3 must not depend on stage 2');
  assert.equal(typeof mod.contextCommand, 'function');
});
