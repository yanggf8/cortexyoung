#!/usr/bin/env node
// End-to-end, two-arm agent evaluation on evals/tasks-graph.json.
//
// Rounds 1-3 priced agent turns rather than tool payloads, and recorded tool_return_tokens and
// read_calls as null. This runner exists to answer one question honestly: does an agent holding
// `cort` reach the labelled answer for less than an agent holding `rg` + Read?
//
// The whitelist IS the experiment. Everything else — prompt, task, venue, model, config dir —
// is held identical across the two arms.
import fs from 'node:fs';
import path from 'node:path';
import { execFile, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { parseStream } from './agent-stream.mjs';
import { ANSWER_CONTRACT, GATE, gradeAnswer } from './grade.mjs';

// cort is the Rust binary since the cutover; JS bin/cort.js is gone.
export const CORT_BIN = process.env.CORT_BIN
  ?? fileURLToPath(new URL('../rust/target/release/cort', import.meta.url));

// Shipped guidance, not a hint invented for the eval: this is what skills/ast-grep/SKILL.md tells
// any agent that has cort. Withholding it would measure a tool nobody was told how to use — but
// results must record that the cort arm received it.
const CORT_GUIDANCE = [
  'You have an offline code-intelligence CLI. Run it exactly like this, copying the path verbatim:',
  '',
  `${CORT_BIN} impact --symbol <name> --depth <n> -f lean`,
  '',
  'It answers relationship questions — who reaches a symbol, and in how many hops — from a',
  'pre-built index, in one call per query. `-f lean` keeps the answer small. Its lean output',
  'reports stale=; if that is ever true, say so in your reply.',
].join('\n');

export const AGENT_ARMS = {
  'rg+Read': {
    allowedTools: ['Read', 'Bash(rg:*)'],
    guidance: null,
  },
  cort: {
    allowedTools: ['Read', `Bash(${CORT_BIN}:*)`],
    guidance: CORT_GUIDANCE,
  },
};

export const REQUIRED_FIELDS = [
  'arm', 'task', 'success', 'coverage', 'precision', 'answered_symbols', 'total_tokens',
  'tool_return_tokens', 'tool_return_bytes', 'read_calls', 'turns', 'hit_turn_cap',
  'permission_denials', 'estimator', 'venue_head', 'cort_calls',
];

/// Count the arm's own tool invocations. The JS `bin/cort.js` entry point is gone since the
/// Rust cutover, so matching on that filename silently reported 0 calls for a Rust arm that
/// called cort on every turn — the metric that proves the whitelist was actually exercised.
export function isCortCommand(command) {
  const raw = String(command ?? '').trim();
  if (raw === '') return false;
  const first = raw.split(/\s+/)[0];
  return first === CORT_BIN || path.basename(first) === 'cort' || raw.includes(CORT_BIN);
}

export function buildPrompt(task, arm) {
  const { guidance } = AGENT_ARMS[arm];
  return [task.prompt, guidance, ANSWER_CONTRACT].filter(Boolean).join('\n\n');
}

export function buildArgs(task, arm, { maxTurns }) {
  return {
    cmd: 'claude',
    // cwd is load-bearing: cort derives projectId from it, and from the wrong directory the same
    // query returns seeds=0 and stale=true — measuring a missing symbol instead of the tool.
    cwd: task.venue,
    args: [
      '-p', buildPrompt(task, arm),
      '--output-format', 'stream-json',
      '--verbose',
      '--strict-mcp-config',
      '--max-turns', String(maxTurns),
      '--allowedTools', AGENT_ARMS[arm].allowedTools.join(','),
    ],
  };
}

export function buildEnv({ configDir, cacheDir }) {
  // CORT_CACHE_DIR has to come from here: allowedTools is a literal prefix match, so an agent that
  // wrote the variable itself would be denied and quietly fall back to Read.
  return { ...process.env, CLAUDE_CONFIG_DIR: configDir, CORT_CACHE_DIR: cacheDir };
}

export function buildRow({ arm, task, parsed, venueHead }) {
  for (const f of ['turns', 'tool_return_tokens', 'tool_return_bytes', 'read_calls', 'total_tokens']) {
    if (typeof parsed[f] !== 'number') {
      throw new Error(`${task.id}/${arm}: ${f} is ${parsed[f]}; refusing to write a null metric`);
    }
  }
  const graded = gradeAnswer(parsed.answer_text, task);
  return {
    arm,
    task: task.id,
    success: graded.success,
    coverage: graded.coverage,
    precision: graded.precision,
    hop_accuracy: graded.hop_accuracy,
    answer_block: graded.answer_block,
    answered_symbols: graded.answered_symbols,
    covered_symbols: graded.covered_symbols,
    spurious_symbols: graded.spurious_symbols,
    wrong_hop: graded.wrong_hop,
    expected_symbols: task.expected_symbols,
    total_tokens: parsed.total_tokens,
    input_tokens: parsed.input_tokens,
    cache_creation: parsed.cache_creation,
    cache_read: parsed.cache_read,
    output_tokens: parsed.output_tokens,
    tool_return_tokens: parsed.tool_return_tokens,
    tool_return_bytes: parsed.tool_return_bytes,
    read_calls: parsed.read_calls,
    cort_calls: parsed.tool_calls.filter((c) => isCortCommand(c.input?.command)).length,
    rg_calls: parsed.tool_calls.filter((c) => /^\s*rg\b/.test(String(c.input?.command ?? ''))).length,
    turns: parsed.turns,
    hit_turn_cap: parsed.hit_turn_cap,
    permission_denials: parsed.permission_denials.length,
    guidance_given: AGENT_ARMS[arm].guidance !== null,
    cost_usd: parsed.cost_usd,
    session_id: parsed.session_id,
    estimator: 'ascii/4 + non-ascii*1 (v1)',
    venue_head: venueHead,
  };
}

function runCell(task, arm, opts) {
  const { cmd, args, cwd } = buildArgs(task, arm, opts);
  return new Promise((resolve, reject) => {
    execFile(cmd, args, { cwd, env: buildEnv(opts), maxBuffer: 1 << 30, encoding: 'utf8' },
      (err, stdout) => {
        // A non-zero exit still leaves a usable transcript when the cell merely hit its turn cap.
        if (err && !stdout) reject(err); else resolve(stdout);
      });
  });
}

async function pool(items, size, fn) {
  const out = [];
  let i = 0;
  await Promise.all(Array.from({ length: Math.min(size, items.length) }, async () => {
    while (i < items.length) {
      const at = i;
      i += 1;
      out[at] = await fn(items[at]);
    }
  }));
  return out;
}

async function main() {
  const argv = process.argv.slice(2);
  const at = (n, d) => (argv.includes(`--${n}`) ? argv[argv.indexOf(`--${n}`) + 1] : d);

  const doc = JSON.parse(fs.readFileSync(new URL('./tasks-graph.json', import.meta.url), 'utf8'));
  const only = at('only', null);
  const tasks = doc.tasks.filter((t) => only === null || t.id === only);
  if (tasks.length === 0) throw new Error(`no task matched --only ${only}`);

  const arms = String(at('arms', 'rg+Read,cort')).split(',').map((s) => s.trim());
  for (const arm of arms) if (!AGENT_ARMS[arm]) throw new Error(`unknown arm ${arm}`);

  const opts = {
    maxTurns: Number(at('max-turns', 40)),
    configDir: at('config-dir', '/tmp/cc-eval'),
    cacheDir: at('cache-dir', '/tmp/cort-exp'),
  };
  const outDir = at('out', path.join(path.dirname(fileURLToPath(import.meta.url)), 'runs', '2026-08-28-graph'));
  const concurrency = Number(at('concurrency', 2));

  if (!fs.existsSync(opts.configDir)) {
    throw new Error(`config dir ${opts.configDir} does not exist; the user settings would add ~16k tokens of noise per request`);
  }

  const venue = tasks[0].venue;
  const venueHead = execFileSync('git', ['rev-parse', '--short', 'HEAD'], { cwd: venue, encoding: 'utf8' }).trim();

  const cells = tasks.flatMap((task) => arms.map((arm) => ({ task, arm })));
  const rows = await pool(cells, concurrency, async ({ task, arm }) => {
    const dir = path.join(outDir, arm.replace(/[^\w.+-]/g, '_'));
    fs.mkdirSync(dir, { recursive: true });
    const stdout = await runCell(task, arm, opts);
    fs.writeFileSync(path.join(dir, `${task.id}.stream.jsonl`), stdout);
    const row = buildRow({ arm, task, parsed: parseStream(stdout), venueHead });
    fs.writeFileSync(path.join(dir, `${task.id}.json`), `${JSON.stringify(row, null, 2)}\n`);
    process.stderr.write(`${arm}/${task.id}: coverage=${row.coverage} precision=${row.precision} tokens=${row.total_tokens} tool_return=${row.tool_return_tokens}\n`);
    return row;
  });

  fs.mkdirSync(outDir, { recursive: true });
  fs.writeFileSync(path.join(outDir, 'rows.json'), `${JSON.stringify(rows, null, 2)}\n`);
  process.stdout.write(`${JSON.stringify({ gate: GATE, venue_head: venueHead, cells: rows.length, out: outDir }, null, 2)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((err) => { process.stderr.write(`${err.message}\n`); process.exit(1); });
}
