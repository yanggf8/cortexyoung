import test from 'node:test';
import assert from 'node:assert/strict';
import { AGENT_ARMS, CORT_BIN, buildArgs, buildEnv, buildPrompt, buildRow, REQUIRED_FIELDS } from '../evals/run-agents.mjs';

const TASK = {
  id: 'toy',
  prompt: 'Who reaches alpha within 3 hops?',
  venue: '/home/yanggf/a/cct',
  seed_symbol: 'alpha',
  min_hops_required: 3,
  expected_symbols: ['alpha', 'beta'],
  by_hop: { 1: ['alpha'], 2: ['beta'] },
};

const OPTS = { maxTurns: 40, configDir: '/tmp/cc-eval', cacheDir: '/tmp/cort-exp' };

test('the two arms are the experiment: identical prompt, different tools', () => {
  const rg = buildPrompt(TASK, 'rg+Read');
  const cort = buildPrompt(TASK, 'cort');
  assert.ok(rg.includes(TASK.prompt));
  assert.ok(cort.includes(TASK.prompt));
  assert.ok(rg.includes('```answer'));
  assert.ok(cort.includes('```answer'));
});

test('neither arm is handed the tool that defines the other arm', () => {
  const rg = AGENT_ARMS['rg+Read'].allowedTools.join(' ');
  const cort = AGENT_ARMS.cort.allowedTools.join(' ');
  assert.ok(rg.includes('Bash(rg:*)'));
  assert.ok(!rg.includes('cort.js'));
  assert.ok(cort.includes('cort.js'));
  assert.ok(!cort.includes('rg:'));
});

test('the command the prompt tells the cort arm to run is one the whitelist accepts', () => {
  // allowedTools is a literal prefix match. A prompt that shows `CORT_CACHE_DIR=... node cort.js`
  // would be denied, the agent would fall back to Read, and the cell would be worthless.
  const prefix = AGENT_ARMS.cort.allowedTools.find((t) => t.includes('cort.js'))
    .replace(/^Bash\(/, '').replace(/:\*\)$/, '');
  const shown = buildPrompt(TASK, 'cort').split('\n').find((l) => l.includes('cort.js'));
  assert.ok(shown, 'the cort arm must be shown a command to copy');
  assert.ok(shown.trim().startsWith(prefix), `${shown.trim()} must start with ${prefix}`);
  assert.ok(!shown.includes('CORT_CACHE_DIR='), 'the cache dir comes from the parent, not the prompt');
});

test('the cell runs in the venue, because projectId is derived from the cwd', () => {
  const { cwd } = buildArgs(TASK, 'cort', OPTS);
  assert.equal(cwd, TASK.venue);
});

test('the transcript flags the earlier rounds lacked are all requested', () => {
  const { args } = buildArgs(TASK, 'cort', OPTS);
  assert.ok(args.includes('--output-format'));
  assert.ok(args.includes('stream-json'));
  assert.ok(args.includes('--verbose'));
  assert.ok(args.includes('--strict-mcp-config'));
  assert.deepEqual(args.slice(args.indexOf('--max-turns'), args.indexOf('--max-turns') + 2), ['--max-turns', '40']);
});

test('every tool the arm may use is passed, and nothing else', () => {
  const { args } = buildArgs(TASK, 'rg+Read', OPTS);
  const allowed = args[args.indexOf('--allowedTools') + 1];
  assert.equal(allowed, AGENT_ARMS['rg+Read'].allowedTools.join(','));
});

test('the environment is the isolated one, not the user configuration', () => {
  const env = buildEnv(OPTS);
  assert.equal(env.CLAUDE_CONFIG_DIR, '/tmp/cc-eval');
  assert.equal(env.CORT_CACHE_DIR, '/tmp/cort-exp');
});

test('a row carries every metric the gate reads, none of them null', () => {
  const parsed = {
    turns: 7, hit_turn_cap: false, tool_calls: [{ name: 'Read', input: {} }], read_calls: 1,
    tool_return_tokens: 120, tool_return_bytes: 480, input_tokens: 1, cache_creation: 2,
    cache_read: 3, output_tokens: 4, total_tokens: 10, permission_denials: [], cost_usd: 0.1,
    session_id: 's', answer_text: '```answer\nalpha\t1\nbeta\t2\n```',
  };
  const row = buildRow({ arm: 'cort', task: TASK, parsed, venueHead: 'abc1234' });
  for (const f of REQUIRED_FIELDS) {
    assert.notEqual(row[f], null, `${f} is null`);
    assert.notEqual(row[f], undefined, `${f} is missing`);
  }
  assert.equal(row.coverage, 1);
  assert.equal(row.venue_head, 'abc1234');
});

test('a row refuses to be built from a transcript missing a metric', () => {
  const parsed = { turns: 7, read_calls: 1, tool_return_tokens: null, answer_text: '' };
  assert.throws(() => buildRow({ arm: 'cort', task: TASK, parsed, venueHead: 'abc1234' }), /tool_return_tokens/);
});

test('a denied tool call is carried onto the row, where a reader will see it', () => {
  const parsed = {
    turns: 2, hit_turn_cap: false, tool_calls: [], read_calls: 0, tool_return_tokens: 0,
    tool_return_bytes: 0, input_tokens: 1, cache_creation: 1, cache_read: 1, output_tokens: 1,
    total_tokens: 4, permission_denials: [{ tool_name: 'Bash' }], cost_usd: 0.01,
    session_id: 's', answer_text: '',
  };
  const row = buildRow({ arm: 'cort', task: TASK, parsed, venueHead: 'abc1234' });
  assert.equal(row.permission_denials, 1);
  assert.equal(row.success, false);
});
