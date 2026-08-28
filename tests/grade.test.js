import test from 'node:test';
import assert from 'node:assert/strict';
import { ANSWER_CONTRACT, GATE, gradeAnswer } from '../evals/grade.mjs';

const TASK = {
  id: 'toy',
  expected_symbols: ['alpha', 'beta', 'gamma', 'delta'],
  by_hop: { 1: ['alpha', 'beta'], 2: ['gamma'], 3: ['delta'] },
};

function block(lines) {
  return `Here is what I found.\n\n\`\`\`answer\n${lines.join('\n')}\n\`\`\`\n`;
}

test('the answer contract is one text, so both arms are asked for the same shape', () => {
  assert.match(ANSWER_CONTRACT, /```answer/);
  assert.match(ANSWER_CONTRACT, /hop/i);
});

test('a complete answer scores one on both axes', () => {
  const g = gradeAnswer(block(['alpha\t1', 'beta\t1', 'gamma\t2', 'delta\t3']), TASK);
  assert.equal(g.coverage, 1);
  assert.equal(g.precision, 1);
  assert.equal(g.success, true);
});

test('a missing symbol costs coverage but not precision', () => {
  const g = gradeAnswer(block(['alpha\t1', 'beta\t1', 'gamma\t2']), TASK);
  assert.equal(g.coverage, 0.75);
  assert.equal(g.precision, 1);
});

test('an invented symbol costs precision but not coverage', () => {
  const g = gradeAnswer(block(['alpha\t1', 'beta\t1', 'gamma\t2', 'delta\t3', 'epsilon\t2']), TASK);
  assert.equal(g.coverage, 1);
  assert.equal(g.precision, 0.8);
  assert.deepEqual(g.spurious_symbols, ['epsilon']);
});

test('naming a symbol twice neither helps coverage nor hurts precision', () => {
  const g = gradeAnswer(block(['alpha\t1', 'alpha\t1', 'beta\t1', 'gamma\t2', 'delta\t3']), TASK);
  assert.equal(g.answered_symbols.length, 4);
  assert.equal(g.precision, 1);
});

test('the wrong distance is recorded without being confused with a wrong symbol', () => {
  const g = gradeAnswer(block(['alpha\t1', 'beta\t2', 'gamma\t2', 'delta\t3']), TASK);
  assert.equal(g.coverage, 1);
  assert.equal(g.precision, 1);
  assert.equal(g.hop_accuracy, 0.75);
  assert.deepEqual(g.wrong_hop, [{ symbol: 'beta', said: 2, actual: 1 }]);
});

test('an answer with no block at all is a failed cell, not a null metric', () => {
  const g = gradeAnswer('I could not work it out.', TASK);
  assert.equal(g.answer_block, false);
  assert.equal(g.coverage, 0);
  assert.equal(g.precision, 0);
  assert.equal(g.success, false);
});

test('only the last block counts, so a quoted example cannot pad the answer', () => {
  const text = `${block(['alpha\t1'])}\nOn reflection:\n${block(['alpha\t1', 'beta\t1', 'gamma\t2', 'delta\t3'])}`;
  const g = gradeAnswer(text, TASK);
  assert.equal(g.coverage, 1);
});

test('spacing and stray bullets in the block do not change the answer', () => {
  const g = gradeAnswer(block(['- alpha   1', '  beta\t1 ', '* gamma 2', 'delta 3']), TASK);
  assert.equal(g.coverage, 1);
  assert.equal(g.precision, 1);
});

test('a line without a distance still names a symbol, and is marked as unplaced', () => {
  const g = gradeAnswer(block(['alpha', 'beta\t1', 'gamma\t2', 'delta\t3']), TASK);
  assert.equal(g.coverage, 1);
  assert.deepEqual(g.wrong_hop, [{ symbol: 'alpha', said: null, actual: 1 }]);
});

test('the gate is the one the plan fixed in advance, not one tuned to a result', () => {
  assert.deepEqual(GATE, { coverage: 0.9, precision: 0.7 });
  const nearMiss = gradeAnswer(block(['alpha\t1', 'beta\t1', 'gamma\t2']), TASK);
  assert.equal(nearMiss.coverage < GATE.coverage, true);
  assert.equal(nearMiss.success, false);
});

test('a cell that hit the turn cap can still be graded on what it answered', () => {
  const g = gradeAnswer(block(['alpha\t1', 'beta\t1']), TASK);
  assert.equal(g.coverage, 0.5);
  assert.equal(g.precision, 1);
});
