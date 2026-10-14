import test from 'node:test';
import assert from 'node:assert/strict';
import { movePagesTo, movePagesBeside } from '../src/model.js';
const pages = [1, 2, 3, 4, 5, 6].map((page) => ({ page, rotation: page === 3 ? 90 : 0 }));
const numbers = (value) => value.map((page) => page.page);
test('moves a disjoint selection together and preserves its internal order and rotations', () => {
  const output = movePagesTo(pages, new Set([1, 3]), 4);
  assert.deepEqual(numbers(output), [2, 4, 5, 1, 3, 6]);
  assert.equal(output[4].rotation, 90);
  assert.deepEqual(numbers(pages), [1, 2, 3, 4, 5, 6]);
});
test('drops a group before or after a target in either direction', () => {
  assert.deepEqual(numbers(movePagesBeside(pages, new Set([1, 2]), 5)), [3, 4, 1, 2, 5, 6]);
  assert.deepEqual(numbers(movePagesBeside(pages, new Set([1, 2]), 5, true)), [3, 4, 5, 1, 2, 6]);
  assert.deepEqual(numbers(movePagesBeside(pages, new Set([5, 6]), 2)), [1, 5, 6, 2, 3, 4]);
});
test('dropping inside the selection leaves it unchanged, and invalid positions fail', () => {
  assert.equal(movePagesBeside(pages, new Set([1, 2]), 2), pages);
  for (const position of [0, 6, 1.5, NaN]) assert.throws(() => movePagesTo(pages, new Set([1, 2]), position));
  assert.deepEqual(numbers(movePagesTo(pages, new Set([1, 2, 3, 4, 5, 6]), 1)), [1, 2, 3, 4, 5, 6]);
});
