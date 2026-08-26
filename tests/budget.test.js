import test from 'node:test';
import assert from 'node:assert/strict';
import { estimateTokens, applyBudget } from '../src/budget.js';

test('token estimate is four characters per token, rounded up', () => {
  assert.equal(estimateTokens(''), 0);
  assert.equal(estimateTokens('abcd'), 1);
  assert.equal(estimateTokens('abcde'), 2);
});

test('applyBudget keeps items while the cumulative rendered size fits', () => {
  const items = [{ n: 1 }, { n: 2 }, { n: 3 }];
  const render = () => 'x'.repeat(40);            // 10 tokens each
  const r = applyBudget(items, 25, render);
  assert.equal(r.kept.length, 2);
  assert.equal(r.truncated, true);
});

test('applyBudget reports no truncation when everything fits', () => {
  const r = applyBudget([{ n: 1 }], 1000, () => 'short');
  assert.equal(r.kept.length, 1);
  assert.equal(r.truncated, false);
});

test('applyBudget always keeps at least one item so the answer is never empty', () => {
  const r = applyBudget([{ n: 1 }, { n: 2 }], 1, () => 'x'.repeat(400));
  assert.equal(r.kept.length, 1);
  assert.equal(r.truncated, true);
});
