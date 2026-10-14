import test from 'node:test';
import assert from 'node:assert/strict';
import { UpdateChecks, UPDATE_CHECK_INTERVAL, newerVersion } from '../src/updates.js';
const storage = (initial) => { let value = initial; return {getItem: () => value, setItem: (_, next) => { value = next; }}; };
test('automatic attempts are limited across repeated launches, including failures', () => {
  let now = 100000; const data = storage();
  assert.equal(new UpdateChecks(data, () => now).claim(), true);
  assert.equal(new UpdateChecks(data, () => now).claim(), false);
  now += UPDATE_CHECK_INTERVAL - 1;
  assert.equal(new UpdateChecks(data, () => now).claim(), false);
  now += 1;
  assert.equal(new UpdateChecks(data, () => now).claim(), true);
});
test('available updates survive restart and disappear after installation', () => {
  const data = storage(); const checks = new UpdateChecks(data, () => 100000);
  checks.remember('0.1.9');
  const restarted = new UpdateChecks(data, () => 100001);
  assert.equal(restarted.available('0.1.8'), '0.1.9');
  assert.equal(restarted.available('0.1.9'), null);
  assert.equal(restarted.available('0.2.0'), null);
  restarted.remember(null);
  assert.equal(new UpdateChecks(data).available('0.1.8'), null);
});
test('malformed or unavailable storage does not prevent checking', () => {
  for (const value of ['broken', 'null', '"text"', '[]']) assert.equal(new UpdateChecks(storage(value)).claim(), true);
  const broken = {getItem: () => {throw Error();},setItem: () => {throw Error();}};
  const checks = new UpdateChecks(broken); assert.equal(checks.claim(), true); assert.equal(checks.claim(), false);
});
test('version comparison handles multi-digit releases and invalid data', () => {
  assert.equal(newerVersion('0.1.10', '0.1.9'), true);
  assert.equal(newerVersion('0.2.0', '0.1.9'), true);
  assert.equal(newerVersion('0.1.8', '0.1.8'), false);
  assert.equal(newerVersion('invalid', '0.1.8'), false);
});
