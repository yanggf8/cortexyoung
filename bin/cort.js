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
import { parseFormat, render, FORMAT } from '../src/render.js';

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

const KNOWN_COMMANDS = ['index', 'status', 'projects', 'delete', 'struct', 'context', 'impact'];

// `--help` parses into a flag like any other, so without this every command would run its own
// side effects and ignore it — `cort index --help` indexed the cwd instead of explaining itself.
const USAGE = {
  usage: 'cort <command> [options]',
  commands: {
    index: 'cort index [root] [--incremental]',
    status: 'cort status [root]',
    projects: 'cort projects',
    delete: 'cort delete [root]',
    struct: "cort struct -p '<pattern>' --lang <lang> [-g <glob>] [--budget <n>] [-f json|lean]",
    context: 'cort context <symbol|query> [--budget <n>] [--include-ambiguous] [--content full] [-f json|lean]',
    impact: 'cort impact --symbol <name> [--depth <n>] [-f json|lean]',
  },
  env: {
    CORT_CACHE_DIR: 'where indexes live (default ~/.cache/cortex-ng)',
  },
  note: 'Commands read the project at the cwd unless they take a root argument.',
};

function wantsHelp(positional, flags) {
  return positional[0] === 'help' || 'help' in flags || 'h' in flags;
}

function resolveFormat(flags) {
  const format = parseFormat(flags.f ?? flags.format);
  if (format === null) throw new CortError('unknown_format', { hint: '--format json|lean' });
  return format;
}

function emit(value, format = FORMAT.JSON, command = null) {
  process.stdout.write(render(command, format, value));
}

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

  if (wantsHelp(positional, flags)) { emit(USAGE); return; }

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
    const format = resolveFormat(flags);
    emit(structCommand({
      db, bin, root: real, projectId, pattern, lang, globs,
      budget: Number(flags.budget ?? 1500),
    }), format, 'struct');
    return;
  }

  if (command === 'context') {
    const query = positional[1];
    if (typeof query !== 'string') throw new CortError('missing_query', { hint: 'cort context <symbol|query>' });
    const bin = resolveAstGrepBin();
    assertAstGrepVersion(bin);
    const { real, projectId, db } = openProject(process.cwd());
    const format = resolveFormat(flags);
    emit(contextCommand({
      db, bin, root: real, projectId, query,
      budget: Number(flags.budget ?? 1500),
      includeAmbiguous: flags['include-ambiguous'] === true,
      fullContent: flags.content === 'full',
    }), format, 'context');
    return;
  }

  if (command === 'impact') {
    const symbol = flags.symbol;
    if (typeof symbol !== 'string') throw new CortError('missing_symbol', { hint: 'cort impact --symbol <name>' });
    const bin = resolveAstGrepBin();
    assertAstGrepVersion(bin);
    const { real, projectId, db } = openProject(process.cwd());
    const format = resolveFormat(flags);
    emit(impactCommand({ db, bin, root: real, projectId, symbol, depth: Number(flags.depth ?? 3) }), format, 'impact');
    return;
  }

  throw new CortError('unknown_command', { command: command ?? null, known: KNOWN_COMMANDS });
}

main().catch((err) => {
  if (err instanceof CortError) { emit(err.toJSON()); process.exit(1); }
  throw err;
});
