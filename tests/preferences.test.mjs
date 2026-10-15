import test from 'node:test';
import assert from 'node:assert/strict';
import { loadPreferences, savePreferences } from '../src/preferences.js';
test('settings tolerate unavailable, damaged, and invalid stored values', () => {
  for (const raw of ['bad json', 'null', '{"thumbnail":900,"compression":"invalid","target":"999"}']) {
    const result = loadPreferences({ getItem: () => raw });
    assert.equal(result.thumbnail, 130); assert.equal(result.compression, 'balanced'); assert.equal(result.target, '');
  }
  assert.doesNotThrow(() => savePreferences({ setItem() { throw Error('full'); } }, {}));
});
test('only bounded valid upload presets are restored', () => {
  let raw;
  const storage = { setItem(key, value) { raw = value; }, getItem() { return raw; } };
  savePreferences(storage, { thumbnail: 160, compression: 'small', target: '5', presets: [{ name: 'Company portal', mb: 5 }, { name: 'Invalid', mb: -2 }, { name: '', mb: 4 }] });
  assert.deepEqual(loadPreferences(storage), { thumbnail: 160, compression: 'small', target: '5', presets: [{ name: 'Company portal', mb: 5 }] });
});
