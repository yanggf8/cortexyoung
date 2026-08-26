import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { PACK_DIR, SGCONFIG, packFiles, extractorVersion } from '../src/pack.js';
import { resolveAstGrepBin, execAstGrep } from '../src/ast-grep.js';

test('pack files are enumerated in sorted order and hash deterministically', () => {
  const files = packFiles();
  assert.ok(files.length >= 5);
  assert.deepEqual(files, [...files].sort());
  assert.ok(files.every((f) => path.isAbsolute(f)));
  const v = extractorVersion();
  assert.match(v, /^[0-9a-f]{64}$/);
  assert.equal(v, extractorVersion());
});

test('extractor_version changes when any pack file changes', () => {
  const target = packFiles().find((f) => f.endsWith('typescript.yml'));
  const before = fs.readFileSync(target, 'utf8');
  const v1 = extractorVersion();
  fs.writeFileSync(target, `${before}\n# probe\n`);
  try { assert.notEqual(extractorVersion(), v1); }
  finally { fs.writeFileSync(target, before); }
  assert.equal(extractorVersion(), v1);
});

test('the pack extracts chunks and edges from TypeScript with the expected tags', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cort-pack-'));
  const file = path.join(dir, 'k.ts');
  fs.writeFileSync(file, [
    "import { helper } from './helper';",
    'export function alpha(a: number) { return helper(a) + 1; }',
    'export class Beta {',
    '  go() { return alpha(2); }',
    '}',
  ].join('\n'));
  const bin = resolveAstGrepBin();
  const r = execAstGrep(bin, ['scan', '--json=stream', '--config', SGCONFIG, file]);
  assert.equal(r.code, 0);
  const recs = r.stdout.trim().split('\n').filter(Boolean).map((l) => JSON.parse(l));
  const tags = recs.map((x) => x.message).sort();
  assert.deepEqual(tags, ['chunk:class', 'chunk:function', 'chunk:method', 'edge:calls', 'edge:calls', 'edge:imports']);
  const fn = recs.find((x) => x.message === 'chunk:function');
  assert.equal(fn.metaVariables.single.NAME.text, 'alpha');
  const imp = recs.find((x) => x.message === 'edge:imports');
  assert.equal(imp.metaVariables.single.SRC.text, "'./helper'");
});

test('the pack extracts chunks and edges from Python', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cort-pack-py-'));
  const file = path.join(dir, 'k.py');
  fs.writeFileSync(file, [
    'import os',
    'from helper import assist',
    'def alpha(a):',
    '    return assist(a) + 1',
    'class Beta:',
    '    def go(self):',
    '        return alpha(2)',
  ].join('\n'));
  const r = execAstGrep(resolveAstGrepBin(), ['scan', '--json=stream', '--config', SGCONFIG, file]);
  const recs = r.stdout.trim().split('\n').filter(Boolean).map((l) => JSON.parse(l));
  const imports = recs.filter((x) => x.message === 'edge:imports').map((x) => x.metaVariables.single.SRC.text).sort();
  assert.deepEqual(imports, ['helper', 'os']);
  assert.equal(recs.filter((x) => x.message === 'chunk:class').length, 1);
  assert.equal(recs.filter((x) => x.message === 'chunk:function').length, 2);
});

test('PACK_DIR points at a real directory containing sgconfig.yml', () => {
  assert.ok(fs.statSync(PACK_DIR).isDirectory());
  assert.ok(fs.statSync(SGCONFIG).isFile());
});
