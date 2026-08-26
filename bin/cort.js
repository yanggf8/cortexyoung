#!/usr/bin/env node
import fs from 'node:fs';
import { CortError } from '../src/errors.js';
import { resolveAstGrepBin, assertAstGrepVersion } from '../src/ast-grep.js';
import { openDb, ensureSchema, projectIdFor, dbPathFor, listProjects, deleteProject, withBusyRetry } from '../src/db.js';
import { fullIndex, statusOf } from '../src/indexer.js';
import { incrementalIndex } from '../src/incremental.js';
import { computeStale } from '../src/staleness.js';
import { structCommand } from '../src/struct.js';
import { contextCommand } from '../src/context.js';
import { impactCommand } from '../src/impact.js';

export function parseArgs(argv) {
  const out = { _: [], flags: {} };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (!a.startsWith('-')) { out._.push(a); continue; }
    const name = a.replace(/^--?/, '');
    const next = argv[i + 1];
    if (next === undefined || next.startsWith('-')) out.flags[name] = true;
    else { out.flags[name] = next; i += 1; }
  }
  return out;
}

function emit(value) { process.stdout.write(`${JSON.stringify(value, null, 2)}\n`); }

function openProject(root) {
  const real = fs.realpathSync(root);
  const projectId = projectIdFor(real);
  const db = openDb(dbPathFor(real));
  ensureSchema(db);
  return { real, projectId, db };
}

async function main() {
  const { _: positional, flags } = parseArgs(process.argv.slice(2));
  const command = positional[0];

  if (command === 'index') {
    const bin = resolveAstGrepBin();
    assertAstGrepVersion(bin);
    const { real, projectId, db } = openProject(positional[1] ?? process.cwd());
    emit(withBusyRetry(() => flags.incremental
      ? incrementalIndex({ db, bin, root: real, projectId })
      : fullIndex({ db, bin, root: real, projectId })));
    return;
  }

  if (command === 'status') {
    const { real, projectId, db } = openProject(positional[1] ?? process.cwd());
    const base = statusOf({ db, root: real, projectId });
    if (!base.indexed) { emit(base); return; }
    const bin = resolveAstGrepBin();
    assertAstGrepVersion(bin);
    emit({ ...base, ...computeStale({ db, bin, root: real, projectId }) });
    return;
  }

  if (command === 'projects') { emit(listProjects()); return; }

  if (command === 'delete') {
    emit(deleteProject(fs.realpathSync(positional[1] ?? process.cwd())));
    return;
  }

  if (command === 'struct') {
    const pattern = flags.p ?? flags.pattern;
    const lang = flags.lang;
    if (typeof pattern !== 'string') throw new CortError('missing_pattern', { hint: "cort struct -p '<pattern>' --lang ts" });
    if (typeof lang !== 'string') throw new CortError('missing_lang', { hint: 'pre-flight pattern validation requires --lang' });
    const bin = resolveAstGrepBin();
    assertAstGrepVersion(bin);
    const { real, projectId, db } = openProject(process.cwd());
    const globs = typeof flags.g === 'string' ? [flags.g] : [];
    emit(structCommand({
      db, bin, root: real, projectId, pattern, lang, globs,
      budget: Number(flags.budget ?? 1500),
    }));
    return;
  }

  if (command === 'context') {
    const query = positional[1];
    if (typeof query !== 'string') throw new CortError('missing_query', { hint: 'cort context <symbol|query>' });
    const bin = resolveAstGrepBin();
    assertAstGrepVersion(bin);
    const { real, projectId, db } = openProject(process.cwd());
    emit(contextCommand({
      db, bin, root: real, projectId, query,
      budget: Number(flags.budget ?? 1500),
      includeAmbiguous: flags['include-ambiguous'] === true,
    }));
    return;
  }

  if (command === 'impact') {
    const symbol = flags.symbol;
    if (typeof symbol !== 'string') throw new CortError('missing_symbol', { hint: 'cort impact --symbol <name>' });
    const bin = resolveAstGrepBin();
    assertAstGrepVersion(bin);
    const { real, projectId, db } = openProject(process.cwd());
    emit(impactCommand({ db, bin, root: real, projectId, symbol, depth: Number(flags.depth ?? 3) }));
    return;
  }

  throw new CortError('unknown_command', { command: command ?? null, known: ['index', 'status', 'projects', 'delete', 'struct', 'context', 'impact'] });
}

main().catch((err) => {
  if (err instanceof CortError) { emit(err.toJSON()); process.exit(1); }
  throw err;
});
