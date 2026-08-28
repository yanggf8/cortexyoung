import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { makeProject, SAMPLE } from './helpers/tmp-project.js';

const CORT = fileURLToPath(new URL('../bin/cort.js', import.meta.url));

function runCort(args, { cwd, cache }) {
  try {
    const stdout = execFileSync('node', [CORT, ...args], {
      cwd, encoding: 'utf8', env: { ...process.env, CORT_CACHE_DIR: cache },
    });
    return { code: 0, payload: JSON.parse(stdout) };
  } catch (err) {
    return { code: err.status, payload: JSON.parse(err.stdout) };
  }
}

function sandbox() {
  const cache = fs.mkdtempSync(path.join(os.tmpdir(), 'cort-cache-'));
  return { cwd: makeProject(SAMPLE), cache };
}

test('asking a command for help explains it instead of running it', () => {
  const { cwd, cache } = sandbox();
  const { code, payload } = runCort(['index', '--help'], { cwd, cache });
  assert.equal(code, 0);
  assert.ok(payload.commands.index.startsWith('cort index'));
  // The regression: help used to fall through to the command, indexing the cwd as a side effect.
  assert.deepEqual(fs.readdirSync(cache), []);
});

test('every spelling of help reaches the same usage, and none of them is an error', () => {
  const { cwd, cache } = sandbox();
  for (const args of [['help'], ['--help'], ['-h'], ['impact', '-h'], ['delete', '--help']]) {
    const { code, payload } = runCort(args, { cwd, cache });
    assert.equal(code, 0, `${args.join(' ')} should exit 0`);
    assert.equal(payload.usage, 'cort <command> [options]');
    assert.deepEqual(fs.readdirSync(cache), [], `${args.join(' ')} must not touch the cache`);
  }
});

test('usage documents every command the dispatcher actually knows', () => {
  const { cwd, cache } = sandbox();
  const usage = runCort(['--help'], { cwd, cache }).payload;
  const known = runCort(['nope'], { cwd, cache }).payload.detail.known;
  assert.deepEqual(Object.keys(usage.commands).sort(), [...known].sort());
});

test('an unknown command is still a failure, not usage', () => {
  const { cwd, cache } = sandbox();
  const { code, payload } = runCort(['nope'], { cwd, cache });
  assert.equal(code, 1);
  assert.equal(payload.error, 'unknown_command');
  assert.equal(payload.detail.command, 'nope');
});

test('index without --help still indexes, so the guard did not swallow the command', () => {
  const { cwd, cache } = sandbox();
  const { code, payload } = runCort(['index'], { cwd, cache });
  assert.equal(code, 0);
  assert.ok(payload.chunks > 0);
  assert.equal(fs.readdirSync(cache).length, 1);
});
