#!/usr/bin/env node
// Tests for the eval harness itself. The Rust cutover deleted the JS product *and* its tests,
// which is how `cort_calls` ended up matching a filename that no longer exists and how three
// rounds of null metrics went unnoticed. The harness decides whether the product gets built
// further, so it is not allowed to be the untested part of the repo.
//
//   node --test evals/
import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { estimateTokens, parseStream } from './agent-stream.mjs';
import { gradeAnswer, GATE, ANSWER_CONTRACT } from './grade.mjs';
import { CORT_BIN, AGENT_ARMS, REQUIRED_FIELDS, buildArgs, buildPrompt, buildRow, isCortCommand } from './run-agents.mjs';
import { summarize } from './run-eval.mjs';

const TASK = {
  id: 't1',
  prompt: 'Which functions reach leaf within 3 hops?',
  expected_symbols: ['mid', 'top', 'entry'],
  by_hop: { 1: ['mid'], 2: ['top'], 3: ['entry'] },
};

function stream({ toolCommands = [], results = [], usage = {}, subtype = 'success', resultText = '' }) {
  const events = [];
  for (const command of toolCommands) {
    events.push({ type: 'assistant', message: { content: [{ type: 'tool_use', name: 'Bash', input: { command } }] } });
  }
  for (const text of results) {
    events.push({ type: 'user', message: { content: [{ type: 'tool_result', content: text }] } });
  }
  events.push({ type: 'assistant', message: { content: [{ type: 'tool_use', name: 'Read', input: { file_path: 'src/a.ts' } }] } });
  events.push({
    type: 'result', subtype, num_turns: 4, result: resultText, total_cost_usd: 0.1, session_id: 's',
    permission_denials: [],
    usage: {
      input_tokens: 100, cache_creation_input_tokens: 10, cache_read_input_tokens: 20, output_tokens: 5,
      ...usage,
    },
  });
  return events.map((e) => JSON.stringify(e)).join('\n');
}

test('estimateTokens prices CJK as one token, not a quarter of one', () => {
  assert.equal(estimateTokens('abcd'), 1);
  assert.equal(estimateTokens('abcde'), 2);
  assert.equal(estimateTokens('查询使用者'), 5);
  // '查' + the space + 'abcd': the wide char is its own token and the five ASCII bytes round
  // up to two. Dividing everything by 4 would price this at 2 and flatter whichever arm reads
  // source with Chinese comments in it.
  assert.equal(estimateTokens('查 abcd'), 3);
});

test('parseStream measures per-tool payload instead of leaving it null', () => {
  const parsed = parseStream(stream({ toolCommands: [`${CORT_BIN} impact --symbol leaf -f lean`], results: ['h1\tsrc/c.ts\tmid\t2\n'] }));
  assert.ok(parsed.tool_return_bytes > 0, 'bytes must be measured');
  assert.ok(Number.isInteger(parsed.tool_return_tokens) && parsed.tool_return_tokens > 0);
  assert.equal(parsed.read_calls, 1);
  assert.equal(parsed.total_tokens, 100 + 10 + 20 + 5);
  assert.equal(parsed.turns, 4);
  assert.equal(parsed.hit_turn_cap, false);
});

test('parseStream flags a cell that hit the turn cap without failing', () => {
  const parsed = parseStream(stream({ subtype: 'error_max_turns' }));
  assert.equal(parsed.hit_turn_cap, true);
});

test('parseStream refuses to produce a null metric', () => {
  assert.throws(() => parseStream('{"type":"assistant","message":{"content":[]}}'), /no result event/);
  const noUsage = JSON.stringify({ type: 'result', num_turns: 1 });
  assert.throws(() => parseStream(noUsage), /no usage/);
  const nullish = JSON.stringify({
    type: 'result', num_turns: 1, usage: { input_tokens: null, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 0 },
  });
  assert.throws(() => parseStream(nullish), /not a number/);
});

test('isCortCommand counts the Rust binary, not the deleted JS entry point', () => {
  assert.equal(isCortCommand(`${CORT_BIN} impact --symbol leaf`), true);
  assert.equal(isCortCommand('/usr/home/x/.cargo/bin/cort read src/a.ts'), true);
  assert.equal(isCortCommand('cort status .'), true);
  assert.equal(isCortCommand('rg -n leaf src'), false);
  assert.equal(isCortCommand(''), false);
  // The regression this pins: the old filter looked for 'cort.js' and counted 0 for every cell.
  assert.equal(`${CORT_BIN} impact`.includes('cort.js'), false);
});

test('the cort arm is whitelisted for exactly the command its prompt tells it to run', () => {
  // allowedTools entries are Bash(<literal prefix>:*) — the agent's command must start with
  // exactly the prefix, or the cell is measuring a denied tool call instead of the tool.
  const entry = AGENT_ARMS.cort.allowedTools.find((t) => t.startsWith('Bash('));
  assert.ok(entry.startsWith('Bash(') && entry.endsWith(':*)'), `unexpected matcher shape: ${entry}`);
  const allowed = entry.slice(5, -3).replace(/:$/, '');
  assert.equal(allowed, CORT_BIN, 'the whitelist prefix must be the binary the prompt hands over');
  const command = buildPrompt(TASK, 'cort').split('\n').find((l) => l.startsWith(CORT_BIN));
  assert.ok(command, 'guidance must give a copy-able command');
  assert.ok(command.startsWith(allowed), `${command} is outside the whitelist ${allowed}`);
  for (const arm of Object.keys(AGENT_ARMS)) {
    const tools = AGENT_ARMS[arm].allowedTools.join(' ');
    assert.ok(!tools.includes('cort.js'), `${arm} still whitelists the deleted JS entry point`);
  }
  assert.ok(AGENT_ARMS['rg+Read'].allowedTools.includes('Bash(rg:*)'));
  assert.ok(!AGENT_ARMS['rg+Read'].allowedTools.some((t) => t.includes('cort')), 'baseline arm must not hold the tool under test');
});

test('buildArgs runs in the venue, because projectId comes from cwd', () => {
  const { cwd, args } = buildArgs({ ...TASK, venue: '/tmp/venue-under-test' }, 'cort', { maxTurns: 40 });
  assert.equal(cwd, '/tmp/venue-under-test');
  assert.ok(args.includes('--strict-mcp-config'));
  assert.equal(args[args.indexOf('--max-turns') + 1], '40');
});

test('gradeAnswer scores coverage, precision and hop distance from the answer block', () => {
  const good = 'prose...\n\n```answer\nmid\t1\ntop\t2\nentry\t3\n```';
  const g = gradeAnswer(good, TASK);
  assert.equal(g.coverage, 1);
  assert.equal(g.precision, 1);
  assert.equal(g.hop_accuracy, 1);
  assert.equal(g.success, true);

  const noisy = gradeAnswer('```answer\nmid\t1\nunrelated\t9\n```', TASK);
  assert.equal(noisy.coverage, 0.333);
  assert.equal(noisy.precision, 0.5);
  assert.equal(noisy.success, false);

  const wrongHop = gradeAnswer('```answer\nmid\t2\ntop\t2\nentry\t3\n```', TASK);
  assert.equal(wrongHop.success, true, 'gate is coverage+precision only');
  assert.equal(wrongHop.hop_accuracy, 0.667);
  assert.deepEqual(wrongHop.wrong_hop.map((w) => w.symbol), ['mid']);

  assert.equal(gradeAnswer('no block at all', TASK).answer_block, false);
  assert.equal(gradeAnswer('no block at all', TASK).success, false);
  assert.deepEqual(GATE, { coverage: 0.9, precision: 0.7 });
  assert.match(ANSWER_CONTRACT, /```answer/);
});

test('buildRow emits every required field and rejects unmeasured ones', () => {
  const parsed = parseStream(stream({
    toolCommands: [`${CORT_BIN} impact --symbol leaf --depth 3 -f lean`],
    results: ['h1\tsrc/c.ts\tmid\t2\n'],
    resultText: '```answer\nmid\t1\ntop\t2\nentry\t3\n```',
  }));
  const row = buildRow({ arm: 'cort', task: TASK, parsed, venueHead: 'deadbee' });
  for (const f of REQUIRED_FIELDS) {
    assert.ok(f in row, `row is missing required field ${f}`);
    assert.notEqual(row[f], null, `${f} must never be null`);
  }
  assert.equal(row.cort_calls, 1);
  assert.equal(row.rg_calls, 0);
  assert.equal(row.read_calls, 1);
  assert.equal(row.tool_return_tokens > 0, true);
  assert.equal(row.success, true);
  assert.equal(row.guidance_given, true);
  assert.equal(row.venue_head, 'deadbee');
  assert.equal(row.estimator, 'ascii/4 + non-ascii*1 (v1)');

  const missing = { ...parsed, tool_return_tokens: null };
  assert.throws(() => buildRow({ arm: 'cort', task: TASK, parsed: missing, venueHead: 'x' }), /tool_return_tokens/);
});

test('summarize never averages nulls into a verdict, and fails closed', () => {
  const hist = [
    { arm: 'cort', success: true, total_tokens: 388000, tool_return_tokens: null, turns: 20, read_calls: null },
    { arm: 'ast-grep+Read', success: true, total_tokens: 200000, tool_return_tokens: null, turns: 9, read_calls: null },
  ];
  const s = summarize(hist);
  assert.equal(s.by_arm.cort.mean_tool_return_tokens, null, 'unmeasured stays null, never 0');
  assert.equal(s.by_arm.cort.stale_reads, null);
  assert.ok(s.by_arm.cort.metrics_missing.tool_return_tokens === 1);
  assert.equal(Number.isNaN(s.by_arm.cort.mean_tool_return_tokens), false, 'NaN would serialise to null unnoticed');

  assert.throws(() => summarize(hist, { strict: true }), /refusing to summarise/);

  const measured = [
    { arm: 'cort', success: true, total_tokens: 50, tool_return_tokens: 5, turns: 2, read_calls: 0, stale_reads: 0 },
    { arm: 'ast-grep+Read', success: true, total_tokens: 500, tool_return_tokens: 50, turns: 8, read_calls: 5, stale_reads: 1 },
  ];
  const win = summarize(measured, { strict: true });
  assert.equal(win.verdict.cort_beats_ast_grep, true);
  assert.equal(win.verdict.next_action, 'continue to deferred features');
});
