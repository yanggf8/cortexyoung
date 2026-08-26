#!/usr/bin/env node
import fs from 'node:fs';
import { CortError } from '../src/errors.js';
import { resolveAstGrepBin, assertAstGrepVersion } from '../src/ast-grep.js';
import { openDb, ensureSchema, projectIdFor, dbPathFor, listProjects, deleteProject, withBusyRetry } from '../src/db.js';
import { fullIndex, statusOf } from '../src/indexer.js';

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
    if (flags.incremental) {
      try {
        const { incrementalIndex } = await import('../src/incremental.js');
        emit(withBusyRetry(() => incrementalIndex({ db, bin, root: real, projectId })));
      } catch (err) {
        if (err && err.code === 'ERR_MODULE_NOT_FOUND') {
          emit(withBusyRetry(() => fullIndex({ db, bin, root: real, projectId })));
        } else throw err;
      }
    } else {
      emit(withBusyRetry(() => fullIndex({ db, bin, root: real, projectId })));
    }
    return;
  }

  if (command === 'status') {
    const { real, projectId, db } = openProject(positional[1] ?? process.cwd());
    emit(statusOf({ db, root: real, projectId }));
    return;
  }

  if (command === 'projects') { emit(listProjects()); return; }

  if (command === 'delete') {
    emit(deleteProject(fs.realpathSync(positional[1] ?? process.cwd())));
    return;
  }

  throw new CortError('unknown_command', { command: command ?? null, known: ['index', 'status', 'projects', 'delete'] });
}

main().catch((err) => {
  if (err instanceof CortError) { emit(err.toJSON()); process.exit(1); }
  throw err;
});
