import test from 'node:test';
import assert from 'node:assert/strict';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import {
  CONFIDENCE_SCORE, buildImportMap, resolveTargets,
  unresolvedInline, getNeighbors, getTransitiveDependents,
} from '../src/graph.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

function indexed(files = SAMPLE) {
  const root = makeProject(files);
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  const stats = fullIndex({ db, bin: resolveAstGrepBin(), root, projectId });
  return { root, db, projectId, stats };
}

test('confidence constants match the spec exactly', () => {
  assert.deepEqual(CONFIDENCE_SCORE, { EXTRACTED: 1.0, INFERRED: 0.7, AMBIGUOUS: 0.5 });
});

test('a single-hit call resolves to one INFERRED row', () => {
  const { db, projectId, stats } = indexed();
  assert.ok(stats.relationships > 0);
  const row = db.prepare(`SELECT r.* FROM relationships r
    JOIN chunks s ON s.chunk_id = r.source_chunk_id
    JOIN chunks t ON t.chunk_id = r.target_chunk_id
    WHERE s.project_id = ? AND s.symbol_name = 'alpha' AND t.symbol_name = 'helper'`).get(projectId);
  assert.ok(row, 'alpha -> helper must be a stored relationship');
  assert.equal(row.rel_type, 'calls');
  assert.equal(row.confidence, 'INFERRED');
  assert.equal(row.confidence_score, 0.7);
});

test('an ambiguous call writes one row per target with score 0.5/N', () => {
  const { db, projectId } = indexed({
    'src/a.ts': 'export function dup() { return 1; }\n',
    'src/b.ts': 'export function dup() { return 2; }\n',
    'src/c.ts': 'export function caller() { return dup(); }\n',
  });
  const rows = db.prepare(`SELECT r.* FROM relationships r
    JOIN chunks s ON s.chunk_id = r.source_chunk_id
    WHERE s.project_id = ? AND s.symbol_name = 'caller'`).all(projectId);
  assert.equal(rows.length, 2);
  assert.ok(rows.every((r) => r.confidence === 'AMBIGUOUS'));
  assert.ok(rows.every((r) => Math.abs(r.confidence_score - 0.25) < 1e-9), 'expected 0.5 * 1/2');
});

test('a call with no resolvable target writes no row at all', () => {
  const { db, projectId } = indexed({
    'src/only.ts': 'export function solo() { return externalThing(1); }\n',
  });
  const rows = db.prepare(`SELECT r.* FROM relationships r
    JOIN chunks s ON s.chunk_id = r.source_chunk_id WHERE s.project_id = ?`).all(projectId);
  assert.equal(rows.length, 0, 'zero-target edges must never be persisted');
  assert.equal(db.prepare("SELECT COUNT(*) c FROM sqlite_master WHERE name = 'unresolved_refs'").get().c, 0);
});

test('unresolvedInline is the on-the-fly shape and carries no chunk id', () => {
  const u = unresolvedInline('externalThing');
  assert.deepEqual(u, {
    confidence: 'AMBIGUOUS',
    confidence_score: 0.5,
    confidence_reasoning: 'unresolved: externalThing',
  });
  assert.ok(!('target_chunk_id' in u));
});

test('a symbol never calls itself', () => {
  const { db, projectId } = indexed({
    'src/rec.ts': 'export function loop(n: number) { return n > 0 ? loop(n - 1) : 0; }\n',
  });
  const rows = db.prepare(`SELECT r.* FROM relationships r
    JOIN chunks s ON s.chunk_id = r.source_chunk_id WHERE s.project_id = ?`).all(projectId);
  assert.equal(rows.filter((r) => r.source_chunk_id === r.target_chunk_id).length, 0);
});

test('getNeighbors returns depth-1 edges in both directions, capped', () => {
  const { db, projectId } = indexed();
  const helper = db.prepare("SELECT chunk_id FROM chunks WHERE project_id = ? AND symbol_name = 'helper'").get(projectId);
  const n = getNeighbors(db, helper.chunk_id, 3);
  assert.ok(n.length >= 1);
  assert.ok(n.length <= 3);
  assert.ok(n.some((x) => x.symbol_name === 'alpha' && x.direction === 'incoming'));
});

test('getTransitiveDependents walks the reverse edge up to depth', () => {
  const { db, projectId } = indexed();
  const helper = db.prepare("SELECT chunk_id FROM chunks WHERE project_id = ? AND symbol_name = 'helper'").get(projectId);
  const deps = getTransitiveDependents(db, helper.chunk_id, 3).map((d) => d.symbol_name).sort();
  assert.deepEqual(deps, ['alpha', 'go'], 'go -> alpha -> helper is a 2-hop reverse chain');
});

test('buildImportMap keys only the module specifiers of import edges', () => {
  const map = buildImportMap([
    { rel_type: 'imports', source_symbol: null, raw_target: './helper' },
    { rel_type: 'calls', source_symbol: 'alpha', raw_target: 'helper' },
  ]);
  assert.ok(map.has('./helper'));
  assert.equal(map.size, 1);
});

test('resolveTargets prefers files reachable through the import map', () => {
  const { db, projectId } = indexed({
    'src/helper.ts': 'export function dup() { return 1; }\n',
    'src/far.ts': 'export function dup() { return 2; }\n',
    'src/alpha.ts': "import { dup } from './helper';\nexport function alpha() { return dup(); }\n",
  });
  const map = buildImportMap([{ rel_type: 'imports', source_symbol: null, raw_target: './helper' }]);
  const ids = resolveTargets({ db, projectId, filePath: 'src/alpha.ts', importMap: map, symbol: 'dup' });
  assert.equal(ids.length, 1);
  assert.ok(ids[0].includes('src/helper.ts'));
});
