import test from 'node:test';
import assert from 'node:assert/strict';
import { estimateTokens, parseStream } from '../evals/agent-stream.mjs';

function ndjson(events) {
  return events.map((e) => JSON.stringify(e)).join('\n') + '\n';
}

const RESULT = {
  type: 'result',
  num_turns: 4,
  total_cost_usd: 0.12,
  session_id: 'sess-1',
  permission_denials: [],
  usage: {
    input_tokens: 100,
    cache_creation_input_tokens: 200,
    cache_read_input_tokens: 300,
    output_tokens: 50,
  },
  result: 'done',
};

function toolUse(name, input) {
  return { type: 'assistant', message: { content: [{ type: 'tool_use', name, input }] } };
}

function toolResult(text) {
  return { type: 'user', message: { content: [{ type: 'tool_result', content: text }] } };
}

test('an ASCII payload costs about a token every four characters', () => {
  assert.equal(estimateTokens('abcd'), 1);
  assert.equal(estimateTokens('a'.repeat(400)), 100);
});

test('a CJK character costs a whole token, so cct comments are not under-counted', () => {
  // Dividing everything by 4 would price these at 1 token and flatter the arm that reads source.
  assert.equal(estimateTokens('市場資料'), 4);
  assert.equal(estimateTokens('abcd市場資料'), 5);
});

test('the empty payload is zero tokens, not a fraction of one', () => {
  assert.equal(estimateTokens(''), 0);
});

test('tool results are measured, which is the metric three rounds recorded as null', () => {
  const parsed = parseStream(ndjson([
    toolUse('Read', { file_path: 'src/a.ts' }),
    toolResult('a'.repeat(40)),
    toolUse('Bash', { command: 'rg foo' }),
    toolResult('b'.repeat(80)),
    RESULT,
  ]));
  assert.equal(parsed.tool_return_tokens, 30);
  assert.equal(parsed.tool_return_bytes, 120);
});

test('Read calls are counted apart from the other tools', () => {
  const parsed = parseStream(ndjson([
    toolUse('Read', { file_path: 'src/a.ts' }),
    toolResult('x'),
    toolUse('Read', { file_path: 'src/b.ts' }),
    toolResult('x'),
    toolUse('Bash', { command: 'rg foo' }),
    toolResult('x'),
    RESULT,
  ]));
  assert.equal(parsed.read_calls, 2);
  assert.equal(parsed.tool_calls.length, 3);
});

test('every cort invocation is kept so the arm can be proved to have used its own tool', () => {
  const parsed = parseStream(ndjson([
    toolUse('Bash', { command: 'node /home/yanggf/a/cortexyoung/bin/cort.js impact --symbol foo' }),
    toolResult('x'),
    RESULT,
  ]));
  assert.deepEqual(parsed.tool_calls.map((c) => c.name), ['Bash']);
  assert.match(parsed.tool_calls[0].input.command, /cort\.js impact/);
});

test('usage is summed the way the earlier rounds summed it', () => {
  const parsed = parseStream(ndjson([RESULT]));
  assert.equal(parsed.total_tokens, 650);
  assert.equal(parsed.output_tokens, 50);
  assert.equal(parsed.turns, 4);
  assert.equal(parsed.cost_usd, 0.12);
  assert.equal(parsed.session_id, 'sess-1');
});

test('a denied tool call is surfaced, because a leaking whitelist invalidates the cell', () => {
  const denied = { ...RESULT, permission_denials: [{ tool_name: 'Bash', tool_input: { command: 'rg foo' } }] };
  const parsed = parseStream(ndjson([denied]));
  assert.equal(parsed.permission_denials.length, 1);
});

test('a stream that never produced a result throws instead of reporting nulls', () => {
  assert.throws(
    () => parseStream(ndjson([toolUse('Read', {}), toolResult('x')])),
    /no result event/,
  );
});

test('a result without usage throws rather than writing a null metric', () => {
  const { usage, ...noUsage } = RESULT;
  assert.throws(() => parseStream(ndjson([noUsage])), /usage/);
});

test('blank lines in the stream are skipped, not treated as corruption', () => {
  const parsed = parseStream(`\n${JSON.stringify(RESULT)}\n\n`);
  assert.equal(parsed.turns, 4);
});

test('the turn cap is reported as a fact about the cell, not as failure', () => {
  const capped = { ...RESULT, num_turns: 40, subtype: 'error_max_turns' };
  const parsed = parseStream(ndjson([capped]));
  assert.equal(parsed.hit_turn_cap, true);
  assert.equal(parseStream(ndjson([RESULT])).hit_turn_cap, false);
});

test('tool_result content arrives as blocks as well as a bare string', () => {
  const blocks = { type: 'user', message: { content: [{ type: 'tool_result', content: [{ type: 'text', text: 'a'.repeat(40) }] }] } };
  const parsed = parseStream(ndjson([toolUse('Read', {}), blocks, RESULT]));
  assert.equal(parsed.tool_return_tokens, 10);
});
