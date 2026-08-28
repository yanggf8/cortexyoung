#!/usr/bin/env node
// Independent check of an `impact` answer: the graph supplies the hypothesis, the file text decides.
// For every dependent at hop k, its own body (start_line..end_line, straight off disk) must contain
// a word-boundary reference to at least one symbol from hop k-1.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const CORT = fileURLToPath(new URL('../bin/cort.js', import.meta.url));

function esc(name) { return String(name).replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }

export function verifyImpact({ repo, symbol, depth = 3 }) {
  const raw = execFileSync('node', [CORT, 'impact', '--symbol', symbol, '--depth', String(depth)],
    { cwd: repo, encoding: 'utf8', maxBuffer: 1 << 28 });
  const payload = JSON.parse(raw);
  const parents = { 1: [symbol] };
  for (let h = 2; h <= depth; h += 1) {
    parents[h] = [...new Set(payload.dependents.filter((d) => d.hop === h - 1).map((d) => d.symbol_name))];
  }
  const bodyCache = new Map();
  const rows = payload.dependents.map((d) => {
    if (!bodyCache.has(d.file_path)) bodyCache.set(d.file_path, fs.readFileSync(path.join(repo, d.file_path), 'utf8').split('\n'));
    const body = bodyCache.get(d.file_path).slice(d.start_line - 1, d.end_line).join('\n');
    const names = (parents[d.hop] ?? []).filter(Boolean);
    const matched = names.find((n) => new RegExp(`\\b${esc(n)}\\b`).test(body)) ?? null;
    return { hop: d.hop, file: d.file_path, symbol: d.symbol_name, confirmed: matched !== null, via: matched };
  });
  const byHop = {};
  for (const r of rows) {
    const b = (byHop[r.hop] ||= { total: 0, confirmed: 0 });
    b.total += 1;
    if (r.confirmed) b.confirmed += 1;
  }
  return {
    symbol,
    depth,
    seed_count: payload.seeds.length,
    dependents: rows.length,
    by_hop: byHop,
    confirmed: rows.filter((r) => r.confirmed).length,
    precision: rows.length ? Number((rows.filter((r) => r.confirmed).length / rows.length).toFixed(3)) : null,
    unconfirmed: rows.filter((r) => !r.confirmed).map((r) => `h${r.hop}:${r.symbol ?? r.file}`),
  };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const argv = process.argv.slice(2);
  const at = (n, d) => (argv.includes(`--${n}`) ? argv[argv.indexOf(`--${n}`) + 1] : d);
  const repo = at('repo', process.cwd());
  const depth = Number(at('depth', 3));
  const symbols = String(at('symbols', '')).split(',').map((s) => s.trim()).filter(Boolean);
  const report = symbols.map((s) => verifyImpact({ repo, symbol: s, depth }));
  process.stdout.write(`${JSON.stringify({ repo, depth, report }, null, 2)}\n`);
}
