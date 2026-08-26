import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import {
  AST_GREP_PINNED, resolveAstGrepBin, astGrepVersion,
  assertAstGrepVersion, execAstGrep,
} from '../src/ast-grep.js';
import { CortError } from '../src/errors.js';

const FAKE = fileURLToPath(new URL('./fixtures/fake-ast-grep.js', import.meta.url));

test('resolves the real ast-grep and it matches the pin', () => {
  const bin = resolveAstGrepBin();
  assert.equal(astGrepVersion(bin), AST_GREP_PINNED);
  assert.doesNotThrow(() => assertAstGrepVersion(bin));
});

test('missing binary is fail-closed', () => {
  const prevBin = process.env.CORT_AST_GREP_BIN;
  const prevPath = process.env.PATH;
  process.env.CORT_AST_GREP_BIN = '/nonexistent/ast-grep';
  try {
    process.env.PATH = '';
    assert.throws(() => resolveAstGrepBin(), (e) => e instanceof CortError && e.code === 'ast_grep_missing');
  } finally {
    if (prevBin === undefined) delete process.env.CORT_AST_GREP_BIN;
    else process.env.CORT_AST_GREP_BIN = prevBin;
    process.env.PATH = prevPath;
  }
});

test('wrong version is fail-closed with found/expected detail', () => {
  process.env.FAKE_AG_MODE = 'version:0.44.9';
  try {
    assert.throws(() => assertAstGrepVersion(FAKE), (err) => {
      assert.equal(err.code, 'ast_grep_version_mismatch');
      assert.deepEqual(err.detail, { found: '0.44.9', expected: AST_GREP_PINNED });
      assert.deepEqual(err.toJSON(), {
        error: 'ast_grep_version_mismatch',
        detail: { found: '0.44.9', expected: AST_GREP_PINNED },
      });
      return true;
    });
  } finally { delete process.env.FAKE_AG_MODE; }
});

test('a hung subprocess raises ast_grep_timeout', () => {
  process.env.FAKE_AG_MODE = 'hang';
  try {
    assert.throws(() => execAstGrep(FAKE, ['run'], { timeoutMs: 150 }), (err) => {
      assert.equal(err.code, 'ast_grep_timeout');
      return true;
    });
  } finally { delete process.env.FAKE_AG_MODE; }
});

test('execAstGrep returns code, stdout and stderr separately', () => {
  process.env.FAKE_AG_MODE = 'streams';
  try {
    const r = execAstGrep(FAKE, ['run']);
    assert.equal(r.code, 1);
    assert.equal(r.stdout, 'OUT\n');
    assert.equal(r.stderr, 'ERR\n');
  } finally { delete process.env.FAKE_AG_MODE; }
});
