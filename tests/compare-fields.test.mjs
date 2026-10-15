import test from 'node:test';
import assert from 'node:assert/strict';
import { comparisonFields, compareFields } from '../src/compare-fields.js';

const widget = (fieldName, fieldValue, extra = {}) => ({
  subtype: 'Widget', fieldType: 'Tx', fieldName, fieldValue, ...extra,
});
const fields = (items, page = 1) => comparisonFields(items, page);

test('saved text, checkbox and choice changes are compared independently of page text', () => {
  const original = fields([widget('Income', ''), widget('Consent', 'Off', { fieldType: 'Btn', checkBox: true }), widget('Sources', ['Employment'], { fieldType: 'Ch' })]);
  const revised = fields([widget('Income', '50000'), widget('Consent', 'Yes', { fieldType: 'Btn', checkBox: true }), widget('Sources', ['Employment', 'Other'], { fieldType: 'Ch' })]);
  const changes = compareFields(original, revised);
  assert.equal(changes.length, 3);
  assert.deepEqual(changes[0].removed, ['Empty']);
  assert.deepEqual(changes[0].added, ['50000']);
  assert.deepEqual(changes[1].removed, ['Unchecked']);
  assert.deepEqual(changes[1].added, ['Checked']);
  assert.deepEqual(changes[2].added, ['Employment\nOther']);
});

test('logical names match despite object ID, widget order and page changes', () => {
  const original = fields([widget('Income', '50000', { id: '1R' }), widget('Name', 'Jane')]);
  const revised = fields([widget('Name', 'Jane'), widget('Income', '50000', { id: '99R' })], 2);
  assert.deepEqual(compareFields(original, revised), []);
  revised[1].value = '60000';
  const [change] = compareFields(original, revised);
  assert.equal(change.oldPage, 1);
  assert.equal(change.newPage, 2);
});

test('repeated widgets report one logical change and missing fields remain distinct from empty fields', () => {
  const original = fields([widget('Income', ''), widget('Income', ''), widget('Removed', '')]);
  const revised = fields([widget('Income', '50000'), widget('Income', '50000'), widget('Added', '')]);
  const changes = compareFields(original, revised);
  assert.equal(changes.length, 3);
  assert.equal(changes[1].newPage, null);
  assert.deepEqual(changes[1].added, ['Field not present']);
  assert.equal(changes[2].oldPage, null);
  assert.deepEqual(changes[2].added, ['Empty']);
});

test('radio selections and cleared values are detected, while signatures and pushbuttons are excluded', () => {
  const extra = { fieldType: 'Btn', radioButton: true };
  const original = fields([widget('Choice', 'A', extra), widget('Name', 'Jane')]);
  const revised = fields([widget('Choice', 'B', extra), widget('Name', ''), widget('Sign', '', { fieldType: 'Sig' }), widget('Submit', '', { fieldType: 'Btn', pushButton: true })]);
  const changes = compareFields(original, revised);
  assert.equal(changes.length, 2);
  assert.deepEqual(changes[0].added, ['B']);
  assert.deepEqual(changes[1].added, ['Empty']);
});
