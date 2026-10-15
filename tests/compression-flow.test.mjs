import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../src/main.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('function invalidateCompression()'), source.indexOf('function editPages('));

function setup(responses) {
  const calls = [];
  const summary = { classList: { remove() {}, toggle() {} }, removeAttribute() {} };
  const view = { documents: [{ id: 'synthetic' }], pages: [], compression: 'balanced', compressionPreview: null, result: null };
  const context = vm.createContext({
    view, calls, summary,
    state: { compress: view }, mode: 'compress', busy: false, ready: true, jobId: null, cancelAvailable: false,
    current: () => view, validation: () => true, targetBytes: () => null,
    $: () => summary, bytes: (value) => `${value} bytes`,
    controls() {}, render() {}, renderContinuation() {}, message() {},
    processingError(_view, error) { view.errorDetails = String(error); },
    outputDocuments: () => view.documents,
    Channel: class {}, crypto: { randomUUID: () => 'test-job' },
    window: { matchMedia: () => ({ matches: true }) },
    async invoke(command, args) {
      calls.push({ command, request: args.request });
      const response = responses.shift();
      if (response instanceof Error) throw response;
      return response;
    },
  });
  vm.runInContext(`${functions}\nglobalThis.actions = { save, invalidateCompression, compressionSummary };`, context);
  return context;
}
const output = () => ({ id: 'cached-output', files: 1, inputBytes: 100, outputBytes: 60, fileBytes: [60], name: 'synthetic.pdf' });

test('first click compresses; second click saves the cached output', async () => {
  const app = setup([output(), output()]);
  app.actions.compressionSummary();
  assert.equal(app.summary.hidden, true);
  assert.equal(app.calls.length, 0);
  await app.actions.save();
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].request.measureOnly, true);
  assert.equal(app.view.result, null);
  assert.equal(app.summary.hidden, false);
  assert.match(app.summary.textContent, /^Output: 60 bytes/);
  await app.actions.save();
  assert.equal(app.calls.length, 2);
  assert.equal(app.calls[1].request.previewId, 'cached-output');
  assert.equal(app.calls[1].request.measureOnly, undefined);
  assert.equal(app.view.result.outputBytes, 60);
});

test('settings changes invalidate output without starting work', async () => {
  const app = setup([output(), output()]);
  await app.actions.save();
  app.view.compression = 'small';
  app.actions.invalidateCompression();
  app.actions.compressionSummary();
  assert.equal(app.view.compressionPreview, null);
  assert.equal(app.summary.hidden, true);
  assert.equal(app.calls.length, 1);
  await app.actions.save();
  assert.equal(app.calls[1].request.measureOnly, true);
  assert.equal(app.calls[1].request.compression, 'small');
});

for (const response of [null, new Error('Cancelled')]) {
  test(`compression ${response === null ? 'cancellation' : 'failure'} waits for another click to retry`, async () => {
    const app = setup([response, output()]);
    await app.actions.save();
    assert.equal(app.calls.length, 1);
    assert.equal(app.busy, false);
    assert.equal(app.view.compressionPreview, null);
    await app.actions.save();
    assert.equal(app.calls[1].request.measureOnly, true);
    assert.equal(app.view.compressionPreview.id, 'cached-output');
  });
}

test('cancelling save retains the compressed output', async () => {
  const app = setup([output(), null, output()]);
  await app.actions.save();
  await app.actions.save();
  assert.equal(app.view.compressionPreview.id, 'cached-output');
  await app.actions.save();
  assert.equal(app.calls[2].request.previewId, 'cached-output');
});

const policy = source.slice(source.indexOf('function changesFor('), source.indexOf('function targetBytes('));
test('signed compression is blocked without a removal checkbox; ordinary forms remain eligible', () => {
  const doc = { features: { signatures: true } };
  const context = vm.createContext({ mode: 'compress', outputDocuments: () => [doc] });
  vm.runInContext(`${policy}\nglobalThis.changes = () => changesFor({});`, context);
  let changes = context.changes();
  assert.match(changes.blocked, /invalidate its signature/);
  assert.equal(changes.acknowledge, false);
  assert.equal(changes.notes.length, 0);
  doc.features = { signatures: false, forms: true };
  changes = context.changes();
  assert.equal(changes.blocked, null);
  assert.equal(changes.acknowledge, false);
  assert.equal(changes.flatten, false);
});

test('busy or unavailable engine prevents processing', async () => {
  const app = setup([output()]);
  app.busy = true;
  await app.actions.save();
  app.busy = false;
  app.ready = false;
  await app.actions.save();
  assert.equal(app.calls.length, 0);
});

test('other tools still save in one action', async () => {
  const app = setup([output()]);
  app.mode = 'merge';
  await app.actions.save();
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].request.operation, 'merge');
  assert.equal(app.calls[0].request.measureOnly, undefined);
  assert.equal(app.view.result.outputBytes, 60);
});
