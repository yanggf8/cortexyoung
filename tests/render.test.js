import test from 'node:test';
import assert from 'node:assert/strict';
import { openDb, ensureSchema, projectIdFor } from '../src/db.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';
import { fullIndex } from '../src/indexer.js';
import { impactCommand } from '../src/impact.js';
import { contextCommand } from '../src/context.js';
import { structCommand } from '../src/struct.js';
import { parseFormat, render, renderImpact, FORMAT } from '../src/render.js';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

const CHAIN = {
  'src/d.ts': 'export function d() { return 1; }\n',
  'src/c.ts': "import { d } from './d';\nexport function c() { return d(); }\n",
  'src/b.ts': "import { c } from './c';\nexport function b() { return c(); }\n",
  'src/a.ts': "import { b } from './b';\nexport function a() { return b(); }\n",
};

function indexed(files = SAMPLE) {
  const root = makeProject(files);
  const db = openDb(':memory:');
  ensureSchema(db);
  const projectId = projectIdFor(root);
  const bin = resolveAstGrepBin();
  fullIndex({ db, bin, root, projectId });
  return { root, db, projectId, bin };
}

function chain() {
  const { root, db, projectId, bin } = indexed(CHAIN);
  return {
    db,
    projectId,
    impact: impactCommand({ db, bin, root, projectId, symbol: 'd', depth: 3 }),
    context: contextCommand({ db, bin, root, projectId, query: 'c' }),
    struct: structCommand({ db, bin, root, projectId, pattern: 'd()', lang: 'ts', globs: [`${root}/src/c.ts`], budget: 1500 }),
    root,
  };
}

test('parseFormat accepts json and lean case-insensitively and rejects anything else', () => {
  assert.equal(parseFormat(undefined), FORMAT.JSON);
  assert.equal(parseFormat('json'), FORMAT.JSON);
  assert.equal(parseFormat('lean'), FORMAT.LEAN);
  assert.equal(parseFormat('LEAN'), FORMAT.LEAN);
  assert.equal(parseFormat('yaml'), null);
});

test('lean impact output lists every dependent with its hop and drops the stored chunk_id', () => {
  const { impact } = chain();
  const out = renderImpact(impact);
  assert.match(out, /^# impact d depth=3 seeds=1 dependents=3 stale=false$/m);
  assert.match(out, /^h1\tsrc\/c\.ts\tc\t2$/m);
  assert.match(out, /^h3\tsrc\/a\.ts\ta\t2$/m);
  assert.ok(!out.includes(impact.dependents[0].chunk_id), 'lean must not repeat the chunk_id');
  assert.ok(!/^[0-9a-f]{64}/m.test(out), 'no 64-char project hash in lean output');
});

test('lean is smaller than json for the same payload on all three verbs', () => {
  const { impact, context, struct } = chain();
  for (const [command, payload] of [['impact', impact], ['context', context], ['struct', struct]]) {
    const json = render(command, FORMAT.JSON, payload);
    const lean = render(command, FORMAT.LEAN, payload);
    assert.ok(lean.length < json.length, `${command}: lean ${lean.length} should be < json ${json.length}`);
    assert.equal(lean.endsWith('\n'), true);
  }
});

test('lean context keeps neighbours and unresolved refs one per line', () => {
  const { context } = chain();
  const out = render('context', FORMAT.LEAN, context);
  assert.match(out, /^# context c resolution=exact_symbol/s);
  assert.match(out, /^src\/c\.ts:2\tc\tfunction$/m);
  for (const neighbor of context.seeds[0].neighbors) {
    assert.ok(out.includes(neighbor.file_path), `neighbour ${neighbor.symbol_name} missing`);
  }
});

test('lean struct emits one row per match with the enclosing symbol', () => {
  const { struct } = chain();
  const out = render('struct', FORMAT.LEAN, struct);
  assert.match(out, /^# struct d\(\) lang=ts matches=1 shown=1 truncated=false stale=false$/m);
  const rows = out.split('\n').slice(1).filter(Boolean);
  assert.equal(rows.length, struct.matches.length);
  assert.match(rows[0], /^src\/c\.ts:\d+\tc\t/);
});

test('unknown commands and json format fall through to the JSON contract', () => {
  const payload = { ok: true };
  assert.equal(render('status', FORMAT.LEAN, payload), `${JSON.stringify(payload, null, 2)}\n`);
  assert.equal(render('impact', FORMAT.JSON, payload), `${JSON.stringify(payload, null, 2)}\n`);
});

test('lean reading output identifies cache provenance and keeps stored content', () => {
  const reading = {
    file_path: 'src/main.rs', start_line: 10, end_line: 12,
    source: 'store', read_count: 2, content: 'fn work() {\n}',
  };
  const out = render('read', FORMAT.LEAN, reading);
  assert.match(out, /^# read src\/main\.rs:10-12 source=store reads=2$/m);
  assert.ok(out.includes('fn work()'));

  const recall = render('recall', FORMAT.LEAN, {
    query: 'work', reading_count: 1, truncated_query: false,
    readings: [{ ...reading, content_truncated: false, last_read_at: 1 }],
  });
  assert.match(recall, /^# recall work readings=1 truncated_query=false$/m);
  assert.match(recall, /^src\/main\.rs:10-12\treads=2$/m);
});
