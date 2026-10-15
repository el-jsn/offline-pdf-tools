import test from 'node:test';
import assert from 'node:assert/strict';
import { words, compareWords } from '../src/compare-model.js';
import { pageText } from '../src/pdf-text.js';
const compare = (a, b, distance) => compareWords(words(a), words(b), distance);
test('whitespace and moved page breaks do not create wording changes', () => {
  assert.deepEqual(compare(['Income  amount', 'is 50,000.'], ['Income amount is', '50,000.']).changes, []);
});
test('amount changes preserve page locations and context', () => {
  const result = compare(['Applicant Jane', 'Income 50,000 annually'], ['Applicant Jane', 'Income 60,000 annually']);
  assert.equal(result.changes.length, 1);
  assert.deepEqual(result.changes[0].removed, ['50,000']); assert.deepEqual(result.changes[0].added, ['60,000']);
  assert.equal(result.changes[0].oldPage, 2); assert.equal(result.changes[0].newPage, 2);
});
test('insertions, deletions, and repeated wording reconstruct both documents', () => {
  const vocabulary = ['income', 'borrower', 'rate', '5.2', '5.3'];
  let seed = 52;
  const next = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed; };
  for (let i = 0; i < 100; i++) {
    const a = Array.from({ length: next() % 20 }, () => vocabulary[next() % 5]);
    const b = Array.from({ length: next() % 20 }, () => vocabulary[next() % 5]);
    const result = compare([a.join(' ')], [b.join(' ')]);
    assert.equal(result.coarse, false);
    const removed = result.changes.flatMap((change) => change.removed), added = result.changes.flatMap((change) => change.added);
    const counts = (tokens) => tokens.reduce((map, token) => (map[token] = (map[token] || 0) + 1, map), {});
    assert.deepEqual(counts([...a, ...added]), counts([...b, ...removed]));
    const distance = Array.from({ length: a.length + 1 }, () => Array(b.length + 1).fill(0));
    for (let row = 0; row <= a.length; row++) for (let col = 0; col <= b.length; col++) {
      distance[row][col] = row === 0 ? col : col === 0 ? row : a[row - 1] === b[col - 1] ? distance[row - 1][col - 1] : 1 + Math.min(distance[row - 1][col], distance[row][col - 1]);
    }
    assert.equal(removed.length + added.length, distance[a.length][b.length]);
  }
});
test('large differences are grouped without reporting false equality', () => {
  const result = compare(['Start one two three end'], ['Start four five six end'], 1);
  assert.equal(result.coarse, true); assert.equal(result.changes.length, 1);
  assert.deepEqual(result.changes[0].removed, ['one', 'two', 'three']);
});
test('font runs within a word do not invent wording changes', () => {
  const item = (str, x, width, hasEOL = false) => ({ str, transform: [12,0,0,12,x,100], width, dir: 'ltr', hasEOL });
  assert.equal(pageText([item('In',0,10), item('come',10,28), item('amount',42,40)]), 'Income amount');
  assert.equal(pageText([item('Income',0,38,true), item('amount',0,40)]), 'Income amount');
});
