#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import { projectIdFor, dbPathFor } from '../src/db.js';

const REPO = fileURLToPath(new URL('..', import.meta.url));
export const tok = (bytes) => Math.ceil(bytes / 4);

function hopsFor(db, projectId, names, depth) {
  const seeds = db.prepare(
    `SELECT chunk_id, symbol_name, file_path FROM chunks
      WHERE project_id = ? AND symbol_name IN (${names.map(() => '?').join(',')})`,
  ).all(projectId, ...names);
  if (seeds.length === 0) return { seeds: [], levels: [] };
  const q = seeds.map(() => '?').join(',');
  const rows = db.prepare(`
    WITH RECURSIVE dep(chunk_id, hop) AS (
      SELECT r.source_chunk_id, 1 FROM relationships r WHERE r.target_chunk_id IN (${q})
      UNION
      SELECT r.source_chunk_id, d.hop + 1
        FROM relationships r JOIN dep d ON r.target_chunk_id = d.chunk_id
       WHERE d.hop < ?
    )
    SELECT c.chunk_id, c.symbol_name, c.file_path, c.start_line, MIN(d.hop) AS hop
      FROM dep d JOIN chunks c ON c.chunk_id = d.chunk_id
     WHERE c.chunk_id NOT IN (${q})
     GROUP BY c.chunk_id`).all(...seeds.map((s) => s.chunk_id), depth, ...seeds.map((s) => s.chunk_id));
  const levels = [];
  for (let h = 1; h <= depth; h += 1) levels.push(rows.filter((r) => r.hop === h));
  return { seeds, levels };
}

function rgOut(repo, names, globs) {
  const args = ['-n', '--no-heading', '--no-messages', '.'];
  for (const g of globs) args.push('-g', g);
  for (const n of names) args.push('-e', `\\b${n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`);
  try {
    return execFileSync('rg', args, { cwd: repo, encoding: 'utf8', maxBuffer: 1 << 28 });
  } catch (err) {
    if (err.status === 1) return '';
    throw err;
  }
}

function enclosingChunk(db, projectId, filePath, line) {
  return db.prepare(
    `SELECT chunk_id, symbol_name, content, start_line FROM chunks
      WHERE project_id = ? AND file_path = ? AND start_line <= ? AND end_line >= ?
      ORDER BY start_line DESC LIMIT 1`,
  ).get(projectId, filePath, line, line);
}

export function measureOne({ db, projectId, repo, symbol, depth, globs }) {
  const { seeds, levels } = hopsFor(db, projectId, [symbol], depth);
  if (seeds.length === 0) return null;

  const cli = path.join(REPO, 'bin/cort.js');
  const runCli = (fmt) => execFileSync('node',
    [cli, 'impact', '--symbol', symbol, '--depth', String(depth), '-f', fmt],
    { cwd: repo, encoding: 'utf8', maxBuffer: 1 << 28 });
  const cortTokens = tok(Buffer.byteLength(runCli('json')));
  const leanTokens = tok(Buffer.byteLength(runCli('lean')));

  const onPathIds = new Set(levels.flat().map((r) => r.chunk_id));
  let hits = 0;
  let onPathHits = 0;
  let rgSearch = 0;
  let rgRead = 0;
  const perHop = [];

  for (let h = 1; h <= depth; h += 1) {
    const names = h === 1 ? [symbol]
      : [...new Set((levels[h - 1] || []).map((r) => r.symbol_name).filter(Boolean))];
    if (names.length === 0) { perHop.push({ hop: h, names: 0, grep_tokens: 0, read_tokens: 0, hit_lines: 0 }); continue; }
    const out = rgOut(repo, names, globs);
    const lines = out.split('\n').filter(Boolean);
    const grepTok = tok(Buffer.byteLength(out));
    const windows = new Map();
    for (const line of lines) {
      const m = /^(.+?):(\d+):/.exec(line);
      if (!m) continue;
      const rel = path.relative(repo, path.resolve(repo, m[1])).replaceAll(path.sep, '/');
      const ln = Number(m[2]);
      hits += 1;
      const enc = enclosingChunk(db, projectId, rel, ln);
      if (!enc) continue;
      if (onPathIds.has(enc.chunk_id)) onPathHits += 1;
      const prev = windows.get(enc.chunk_id);
      if (!prev || ln > prev.line) windows.set(enc.chunk_id, { line: ln, content: enc.content, start: enc.start_line });
    }
    let readTok = 0;
    for (const w of windows.values()) {
      const body = w.content.split('\n');
      const take = Math.min(body.length, Math.max(1, w.line - w.start + 1));
      readTok += tok(Buffer.byteLength(body.slice(0, take).join('\n')));
    }
    rgSearch += grepTok;
    rgRead += readTok;
    perHop.push({ hop: h, names: names.length, grep_tokens: grepTok, read_tokens: readTok, hit_lines: lines.length });
  }

  const rgTotal = rgSearch + rgRead;
  return {
    symbol,
    depth,
    answer_symbols: levels.flat().length,
    answer_files: new Set(levels.flat().map((r) => r.file_path)).size,
    cort_tokens: cortTokens,
    lean_tokens: leanTokens,
    rg_tokens: rgTotal,
    rg_search_tokens: rgSearch,
    rg_read_tokens: rgRead,
    rg_hits: hits,
    rg_precision: hits ? Number((onPathHits / hits).toFixed(3)) : null,
    ratio_rg_over_cort: Number((rgTotal / cortTokens).toFixed(2)),
    ratio_rg_over_lean: Number((rgTotal / Math.max(1, leanTokens)).toFixed(2)),
    ratio_lean_over_cort: Number((cortTokens / Math.max(1, leanTokens)).toFixed(2)),
    per_hop: perHop,
  };
}

export function topSymbols(db, projectId, depth, pick) {
  const all = db.prepare(
    'SELECT DISTINCT symbol_name AS s FROM chunks WHERE project_id = ? AND symbol_name IS NOT NULL',
  ).all(projectId).map((r) => r.s);
  const scored = [];
  for (const s of all) {
    const { levels } = hopsFor(db, projectId, [s], depth);
    const flat = levels.flat();
    const files = new Set(flat.map((r) => r.file_path)).size;
    const deep = levels[depth - 1] ? levels[depth - 1].length : 0;
    if (flat.length >= 4 && files >= 3 && deep >= 2) scored.push({ s, size: flat.length, files, deep });
  }
  scored.sort((a, b) => b.files - a.files || b.size - a.size);
  return scored.slice(0, pick).map((r) => r.s);
}

export function measureRepo({ repo, depth = 3, symbols, pick = 6, globs = ['*.ts', '!*.test.ts', '!dist/**'] }) {
  const real = fs.realpathSync(repo);
  const projectId = projectIdFor(real);
  const db = new Database(dbPathFor(real), { readonly: true });
  const chosen = symbols ?? topSymbols(db, projectId, depth, pick);
  const results = chosen
    .map((s) => measureOne({ db, projectId, repo: real, symbol: s, depth, globs }))
    .filter(Boolean);
  db.close();
  return { repo: real, depth, globs, results };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const argv = process.argv.slice(2);
  const flag = (name, dflt) => {
    const i = argv.indexOf(`--${name}`);
    return i === -1 ? dflt : argv[i + 1];
  };
  const symbols = flag('symbols', '') ? flag('symbols', '').split(',').map((s) => s.trim()) : undefined;
  const report = measureRepo({
    repo: flag('repo', process.cwd()),
    depth: Number(flag('depth', 3)),
    symbols,
    pick: Number(flag('pick', 6)),
  });
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
}
