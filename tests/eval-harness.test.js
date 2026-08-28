import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { summarize, ARMS, METRICS } from '../evals/run-eval.mjs';

test('the three arms are exactly the ones the spec names', () => {
  assert.deepEqual(ARMS, ['rg+Read', 'ast-grep+Read', 'cort']);
});

test('the metric set includes what the V6 eval plan was missing', () => {
  assert.ok(METRICS.includes('tool_return_tokens'));
  assert.ok(METRICS.includes('stale_reads'));
  assert.ok(METRICS.includes('total_tokens'));
  assert.ok(METRICS.includes('turns'));
  assert.ok(METRICS.includes('read_calls'));
});

test('every task declares a verifiable expected answer', () => {
  const tasks = JSON.parse(fs.readFileSync(new URL('../evals/tasks.json', import.meta.url), 'utf8'));
  assert.ok(tasks.length >= 5);
  for (const t of tasks) {
    assert.equal(typeof t.id, 'string');
    assert.equal(typeof t.prompt, 'string');
    assert.ok(Array.isArray(t.expected_symbols));
    assert.ok(t.expected_symbols.length > 0);
  }
});

test('summarize computes per-arm aggregates and the stop/go verdict', () => {
  const results = [
    { arm: 'ast-grep+Read', task: 't1', success: true, total_tokens: 1000, tool_return_tokens: 600, turns: 4, read_calls: 3, stale_reads: 0 },
    { arm: 'cort', task: 't1', success: true, total_tokens: 400, tool_return_tokens: 200, turns: 2, read_calls: 0, stale_reads: 0 },
  ];
  const s = summarize(results);
  assert.equal(s.by_arm['cort'].mean_total_tokens, 400);
  assert.equal(s.by_arm['ast-grep+Read'].success_rate, 1);
  assert.equal(s.verdict.cort_beats_ast_grep, true);
});

test('summarize returns a stop verdict when cort loses on tokens', () => {
  const results = [
    { arm: 'ast-grep+Read', task: 't1', success: true, total_tokens: 400, tool_return_tokens: 200, turns: 2, read_calls: 1, stale_reads: 0 },
    { arm: 'cort', task: 't1', success: true, total_tokens: 900, tool_return_tokens: 700, turns: 3, read_calls: 0, stale_reads: 0 },
  ];
  assert.equal(summarize(results).verdict.cort_beats_ast_grep, false);
});

test('every graph task labels each symbol exactly once, at one distance', () => {
  const doc = JSON.parse(fs.readFileSync(new URL('../evals/tasks-graph.json', import.meta.url), 'utf8'));
  assert.ok(doc.tasks.length >= 5);
  for (const t of doc.tasks) {
    const flat = Object.values(t.by_hop).flat();
    // by_hop was once built from raw impact rows, so a symbol reachable by several paths was
    // listed two or three times, at odds with expected_symbols and with itself.
    assert.equal(new Set(flat).size, flat.length, `${t.id}: by_hop repeats a symbol`);
    assert.equal(new Set(t.expected_symbols).size, t.expected_symbols.length, `${t.id}: expected_symbols repeats`);
    assert.deepEqual(new Set(flat), new Set(t.expected_symbols), `${t.id}: by_hop and expected_symbols disagree`);
    for (const hop of Object.keys(t.by_hop)) {
      assert.ok(Number(hop) <= t.min_hops_required, `${t.id}: labelled beyond the declared depth`);
    }
  }
});
