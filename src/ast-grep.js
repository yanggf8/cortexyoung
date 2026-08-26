import { spawnSync } from 'node:child_process';
import { CortError } from './errors.js';

export const AST_GREP_PINNED = '0.45.2';
export const SUBPROCESS_TIMEOUT_MS = 30_000;

export function resolveAstGrepBin() {
  const override = process.env.CORT_AST_GREP_BIN;
  const candidate = override && override.length > 0 ? override : 'ast-grep';
  const probe = spawnSync(candidate, ['--version'], { encoding: 'utf8' });
  if (probe.error || probe.status !== 0) {
    throw new CortError('ast_grep_missing', { candidate });
  }
  return candidate;
}

export function astGrepVersion(bin) {
  const r = spawnSync(bin, ['--version'], { encoding: 'utf8' });
  if (r.error || r.status !== 0) throw new CortError('ast_grep_missing', { candidate: bin });
  const m = /(\d+\.\d+\.\d+)/.exec(r.stdout);
  if (!m) throw new CortError('ast_grep_version_unreadable', { stdout: r.stdout });
  return m[1];
}

export function assertAstGrepVersion(bin) {
  const found = astGrepVersion(bin);
  if (found !== AST_GREP_PINNED) {
    throw new CortError('ast_grep_version_mismatch', { found, expected: AST_GREP_PINNED });
  }
}

export function execAstGrep(bin, args, opts = {}) {
  const timeoutMs = opts.timeoutMs ?? SUBPROCESS_TIMEOUT_MS;
  const r = spawnSync(bin, args, {
    encoding: 'utf8', cwd: opts.cwd, timeout: timeoutMs, maxBuffer: 256 * 1024 * 1024,
  });
  if (r.error && r.error.code === 'ETIMEDOUT') throw new CortError('ast_grep_timeout', { args, timeoutMs });
  if (r.signal === 'SIGTERM') throw new CortError('ast_grep_timeout', { args, timeoutMs });
  if (r.error) throw new CortError('ast_grep_spawn_failed', { args, message: r.error.message });
  return { code: r.status ?? 0, stdout: r.stdout ?? '', stderr: r.stderr ?? '' };
}
