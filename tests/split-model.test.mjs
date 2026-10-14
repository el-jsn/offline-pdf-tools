import test from 'node:test';
import assert from 'node:assert/strict';
import { splitPlan, pageList } from '../src/model.js';

test('extracts nonconsecutive pages together in the requested order', () => {
  assert.deepEqual(splitPlan('selected', 5, '7, 1-3, 5', 9), [[7, 1, 2, 3, 5]]);
  assert.deepEqual(splitPlan('selected', 5, '1, 4, 7', 9, { separate: true }), [[1], [4], [7]]);
});
test('custom groups can overlap and combine without silently removing pages', () => {
  assert.deepEqual(splitPlan('ranges', 5, '1-3;3,7', 9), [[1, 2, 3], [3, 7]]);
  assert.deepEqual(splitPlan('ranges', 5, '1-3;3,7', 9, { combine: true }), [[1, 2, 3, 3, 7]]);
});
test('split points are sorted, unique, and retain the remainder', () => {
  assert.deepEqual(splitPlan('after', 5, '7,3,3', 9), [[1, 2, 3], [4, 5, 6, 7], [8, 9]]);
  assert.deepEqual(splitPlan('after', 5, '3,9', 9), [[1, 2, 3], [4, 5, 6, 7, 8, 9]]);
});
test('fixed groups retain a short final group and support one-page documents', () => {
  assert.deepEqual(splitPlan('groups', 4, '', 9), [[1, 2, 3, 4], [5, 6, 7, 8], [9]]);
  assert.deepEqual(splitPlan('every_page', 5, '', 1), [[1]]);
});
test('invalid or empty selections and output limits fail before saving', () => {
  for (const value of ['', '0', '10', '3-1', '1,,3', '1;3']) assert.throws(() => splitPlan('selected', 5, value, 9));
  assert.throws(() => splitPlan('ranges', 5, '1-3;', 9));
  assert.throws(() => splitPlan('selected', 5, '1-501', 501, { separate: true }), /500/);
  assert.throws(() => splitPlan('ranges', 5, '1-6000;1-6000', 6000, { combine: true }), /10,000/);
  assert.throws(() => splitPlan('ranges', 5, '1-8000;1-8000;1-8000', 8000), /20,000/);
});
test('all UI plans serialize into the backend range format without changing page order', () => {
  for (const mode of ['selected', 'ranges', 'after', 'groups', 'every_page']) {
    const input = mode === 'ranges' ? '7,1-3;3,9' : mode === 'after' ? '3,7' : '7,1-3';
    const plan = splitPlan(mode, 4, input, 9);
    assert.deepEqual(splitPlan('ranges', 4, plan.map(pageList).join(';'), 9), plan);
  }
});
