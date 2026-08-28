import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  parseScanStream, edgeString, fileContentHash, chunkIdFor, extractFile,
} from '../src/chunker.js';
import { resolveAstGrepBin } from '../src/ast-grep.js';

function tmpFile(name, body) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cort-chunk-'));
  const p = path.join(dir, name);
  fs.writeFileSync(p, body);
  return p;
}

const TS = [
  "import { helper } from './helper';",
  'export function alpha(a: number) { return helper(a) + 1; }',
  'export class Beta {',
  '  go() { return alpha(2); }',
  '}',
].join('\n');

test('malformed lines are skipped and counted, valid ones survive', () => {
  const r = parseScanStream('{"a":1}\nnot json\n{"b":2}\n');
  assert.equal(r.total, 3);
  assert.equal(r.malformed, 1);
  assert.deepEqual(r.records, [{ a: 1 }, { b: 2 }]);
});

test('edge strings use the tab-separated pre-resolution form', () => {
  assert.equal(edgeString({ rel_type: 'calls', source_symbol: 'go', raw_target: 'alpha' }), 'calls\tgo\talpha');
  assert.equal(edgeString({ rel_type: 'imports', source_symbol: null, raw_target: './helper' }), 'imports\t\t./helper');
});

test('file_content_hash covers both chunk contents and edge strings', () => {
  const chunks = [{ start_line: 5, content: 'B' }, { start_line: 1, content: 'A' }];
  const edges = [{ rel_type: 'calls', source_symbol: 'x', raw_target: 'z' }];
  const base = fileContentHash(chunks, edges);
  assert.equal(base, fileContentHash([...chunks].reverse(), edges), 'chunk order must not matter');
  const edgeChanged = fileContentHash(chunks, [{ rel_type: 'calls', source_symbol: 'x', raw_target: 'w' }]);
  assert.notEqual(base, edgeChanged, 'an edge-only change must move the hash');
  const chunkChanged = fileContentHash([{ start_line: 1, content: 'A2' }, { start_line: 5, content: 'B' }], edges);
  assert.notEqual(base, chunkChanged, 'a chunk-only change must move the hash');
});

test('extractFile produces 1-indexed lines and V6-shaped chunk ids', () => {
  const abs = tmpFile('k.ts', TS);
  const out = extractFile({
    bin: resolveAstGrepBin(), projectId: 'p', filePath: 'k.ts', absPath: abs, source: TS,
  });
  assert.equal(out.unparsed, false);
  const alpha = out.chunks.find((c) => c.symbol_name === 'alpha');
  assert.equal(alpha.start_line, 2, 'ast-grep reports line 1 (0-indexed); we store 2');
  assert.equal(alpha.chunk_id, chunkIdFor('p', 'k.ts', 2));
  assert.equal(alpha.chunk_id, 'p:k.ts:2');
  assert.equal(alpha.chunk_type, 'function');
  assert.equal(alpha.chunk_source, 'ast');
  assert.equal(alpha.language, 'TypeScript');
  assert.ok(out.chunks.every((c) => c.start_line >= 1));
});

test('Rust functions and impl methods are symbol-scoped AST chunks', () => {
  const body = [
    'fn alpha(x: i32) -> i32 {',
    '    x + 1',
    '}',
    '',
    'struct Worker;',
    'impl Worker {',
    '    pub async fn work(&self) -> i32 {',
    '        alpha(1)',
    '    }',
    '}',
    '',
  ].join('\n');
  const abs = tmpFile('main.rs', body);
  const out = extractFile({
    bin: resolveAstGrepBin(), projectId: 'p', filePath: 'main.rs', absPath: abs, source: body,
  });
  assert.equal(out.unparsed, false);
  const alpha = out.chunks.find((c) => c.symbol_name === 'alpha');
  const work = out.chunks.find((c) => c.symbol_name === 'work');
  assert.equal(alpha.start_line, 1);
  assert.equal(alpha.end_line, 3);
  assert.equal(alpha.language, 'Rust');
  assert.equal(alpha.content, 'fn alpha(x: i32) -> i32 {\n    x + 1\n}');
  assert.equal(work.start_line, 7);
  assert.equal(work.end_line, 9);
  assert.ok(work.content.includes('async fn work'));
  assert.ok(out.chunks.every((c) => c.chunk_source === 'ast'));
});

test('edges are attributed to the innermost containing chunk', () => {
  const abs = tmpFile('k.ts', TS);
  const out = extractFile({
    bin: resolveAstGrepBin(), projectId: 'p', filePath: 'k.ts', absPath: abs, source: TS,
  });
  const imp = out.edges.find((e) => e.rel_type === 'imports');
  assert.equal(imp.source_symbol, null);
  assert.equal(imp.raw_target, './helper', 'quotes are stripped from the module specifier');
  const callInGo = out.edges.find((e) => e.rel_type === 'calls' && e.raw_target === 'alpha');
  assert.equal(callInGo.source_symbol, 'go');
  const callInAlpha = out.edges.find((e) => e.rel_type === 'calls' && e.raw_target === 'helper');
  assert.equal(callInAlpha.source_symbol, 'alpha');
});

test('a file ast-grep cannot parse becomes a single unparsed FTS-only chunk', () => {
  const body = 'function (((\n';
  const abs = tmpFile('broken.ts', body);
  const out = extractFile({
    bin: resolveAstGrepBin(), projectId: 'p', filePath: 'broken.ts', absPath: abs, source: body,
  });
  assert.equal(out.unparsed, true);
  assert.equal(out.chunks.length, 1);
  assert.equal(out.chunks[0].chunk_source, 'unparsed');
  assert.equal(out.chunks[0].chunk_type, 'unparsed');
  assert.equal(out.chunks[0].symbol_name, null);
  assert.equal(out.chunks[0].start_line, 1);
  assert.equal(out.chunks[0].content, body);
  assert.deepEqual(out.edges, []);
  assert.ok(out.file_content_hash.length === 64);
});

test('an all-malformed scan stream degrades that file to unparsed and never throws', () => {
  const body = 'export function ok() {}\n';
  const abs = tmpFile('m.ts', body);
  const prev = process.env.FAKE_AG_MODE;
  process.env.FAKE_AG_MODE = 'emit:' + Buffer.from('garbage\nalso garbage\n').toString('base64');
  try {
    const fake = path.join(process.cwd(), 'tests/fixtures/fake-ast-grep.js');
    const out = extractFile({ bin: fake, projectId: 'p', filePath: 'm.ts', absPath: abs, source: body });
    assert.equal(out.unparsed, true);
    assert.equal(out.malformed, 2);
    assert.equal(out.chunks[0].chunk_source, 'unparsed');
  } finally {
    if (prev === undefined) delete process.env.FAKE_AG_MODE; else process.env.FAKE_AG_MODE = prev;
  }
});

test('a 90%-malformed scan stream still indexes the surviving record — scan never aborts', () => {
  const body = 'export function ok() {}\n';
  const abs = tmpFile('p.ts', body);
  const good = JSON.stringify({
    text: 'export function ok() {}', message: 'chunk:function', language: 'TypeScript',
    range: { start: { line: 0, column: 0 }, end: { line: 0, column: 23 } },
    metaVariables: { single: { NAME: { text: 'ok' } } },
  });
  const stream = `${'junk\n'.repeat(19)}${good}\n`;
  const prev = process.env.FAKE_AG_MODE;
  process.env.FAKE_AG_MODE = 'emit:' + Buffer.from(stream).toString('base64');
  try {
    const fake = path.join(process.cwd(), 'tests/fixtures/fake-ast-grep.js');
    const out = extractFile({ bin: fake, projectId: 'p', filePath: 'p.ts', absPath: abs, source: body });
    assert.equal(out.unparsed, false, '95% malformed must NOT abort the index — that rule is run-only');
    assert.equal(out.malformed, 19);
    assert.equal(out.chunks.length, 1);
    assert.equal(out.chunks[0].symbol_name, 'ok');
  } finally {
    if (prev === undefined) delete process.env.FAKE_AG_MODE; else process.env.FAKE_AG_MODE = prev;
  }
});

test('a scan that times out degrades that file to unparsed instead of aborting', () => {
  const body = 'export function big() {}\n'.repeat(500);
  const abs = tmpFile('huge.ts', body);
  const prev = process.env.FAKE_AG_MODE;
  process.env.FAKE_AG_MODE = 'hang';
  try {
    const fake = path.join(process.cwd(), 'tests/fixtures/fake-ast-grep.js');
    const out = extractFile({
      bin: fake, projectId: 'p', filePath: 'huge.ts', absPath: abs, source: body, timeoutMs: 200,
    });
    assert.equal(out.unparsed, true, 'a timed-out scan must degrade, never abort the index');
    assert.equal(out.chunks.length, 1);
    assert.equal(out.chunks[0].chunk_source, 'unparsed');
    assert.equal(out.chunks[0].content, body);
    assert.deepEqual(out.edges, []);
    assert.match(out.file_content_hash, /^[0-9a-f]{64}$/);
  } finally {
    if (prev === undefined) delete process.env.FAKE_AG_MODE; else process.env.FAKE_AG_MODE = prev;
  }
});

test('a spawn failure still propagates — only timeout degrades to unparsed', () => {
  const body = 'export function x() {}\n';
  const abs = tmpFile('x.ts', body);
  assert.throws(() => extractFile({
    bin: '/nonexistent/ast-grep-binary', projectId: 'p', filePath: 'x.ts', absPath: abs, source: body,
  }), (e) => e.code === 'ast_grep_spawn_failed',
  'environment-wide failures must stay loud; per-file timeouts are the only silent degrade');
});

const CONST_FN = [
  "import { helper } from './helper';",
  'export const alpha = (a: number) => helper(a) + 1;',
  'const beta = function () { return helper(2); };',
  'const gamma = helper;                       // not a function value',
  'const rows = [1, 2, 3].map((n) => helper(n)); // data, not a named function',
  'export const handler = createHandler("x", async (req: Request) => { return helper(1); });',
].join('\n') + '\n';

function constFnChunks() {
  const abs = tmpFile('cf.ts', CONST_FN);
  return extractFile({
    bin: resolveAstGrepBin(), projectId: 'p', filePath: 'cf.ts', absPath: abs, source: CONST_FN,
  });
}

test('const-bound arrow and function expressions become function chunks', () => {
  const out = constFnChunks();
  assert.equal(out.unparsed, false);
  const names = out.chunks.map((c) => c.symbol_name);
  assert.deepEqual(names.slice().sort(), ['alpha', 'beta', 'handler']);
  for (const name of ['alpha', 'beta', 'handler']) {
    const c = out.chunks.find((x) => x.symbol_name === name);
    assert.equal(c.chunk_type, 'function');
    assert.equal(c.chunk_source, 'ast');
  }
});

test('collection transforms and bare aliases do not become chunks', () => {
  const names = constFnChunks().chunks.map((c) => c.symbol_name);
  assert.ok(!names.includes('rows'), 'x.map(n => ...) must not make `rows` a symbol');
  assert.ok(!names.includes('gamma'), 'an alias to a function must not make `gamma` a symbol');
});

test('calls inside a const-bound handler get the handler as their source symbol', () => {
  const { edges } = constFnChunks();
  const inside = edges.filter((e) => e.source_symbol === 'handler' && e.rel_type === 'calls');
  assert.ok(inside.some((e) => e.raw_target === 'helper'), 'handler body must resolve to its caller chunk');
});
