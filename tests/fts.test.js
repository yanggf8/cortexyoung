import test from 'node:test';
import assert from 'node:assert/strict';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import { MAX_OR_TERMS, sanitizeFtsQuery, keywordSearch } from '../src/fts.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

function indexed(files = SAMPLE) {
  const root = makeProject(files);
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  fullIndex({ db, bin: resolveAstGrepBin(), root, projectId });
  return { db, projectId };
}

test('each term is quoted so FTS operators cannot leak through', () => {
  assert.equal(sanitizeFtsQuery('helper').query, '"helper"');
  assert.equal(sanitizeFtsQuery('foo(bar)').query, '"foo(bar)"');
  assert.equal(sanitizeFtsQuery('a - b').query, '"a" OR "-" OR "b"');
  assert.equal(sanitizeFtsQuery('src/alpha.ts').query, '"src/alpha.ts"');
  assert.equal(sanitizeFtsQuery('say "hi"').query, '"say" OR """hi"""');
});

test('more than MAX_OR_TERMS terms truncates and reports it', () => {
  const many = Array.from({ length: MAX_OR_TERMS + 5 }, (_, i) => `t${i}`).join(' ');
  const s = sanitizeFtsQuery(many);
  assert.equal(s.truncated_query, true);
  assert.equal(s.query.split(' OR ').length, MAX_OR_TERMS);
  assert.equal(sanitizeFtsQuery('one two').truncated_query, false);
});

test('an empty query is rejected loudly', () => {
  assert.throws(() => sanitizeFtsQuery('   '), (e) => e.code === 'empty_query');
});

test('keywordSearch finds a symbol by name', () => {
  const { db, projectId } = indexed();
  const { rows } = keywordSearch(db, projectId, 'helper', 10);
  assert.ok(rows.length > 0);
  assert.ok(rows.some((r) => r.symbol_name === 'helper'));
  assert.ok(rows.every((r) => typeof r.chunk_id === 'string'));
});

test('keywordSearch survives punctuation that would otherwise be FTS syntax', () => {
  const { db, projectId } = indexed();
  assert.doesNotThrow(() => keywordSearch(db, projectId, 'helper(a) - alpha', 10));
});

test('unicode61 tokenizing lets CJK identifiers through', () => {
  const { db, projectId } = indexed({
    'src/cjk.ts': 'export function 查詢使用者() { return 1; }\n',
  });
  const { rows } = keywordSearch(db, projectId, '查詢使用者', 10);
  assert.ok(rows.length > 0);
});

test('results are scoped to the project', () => {
  const { db, projectId } = indexed();
  const { rows } = keywordSearch(db, 'some-other-project-id', 'helper', 10);
  assert.equal(rows.length, 0);
});

test('the limit is honoured', () => {
  const { db, projectId } = indexed();
  const { rows } = keywordSearch(db, projectId, 'return', 1);
  assert.ok(rows.length <= 1);
});
