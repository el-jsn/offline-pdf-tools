import { words, compareWords } from './compare-model.js';
import { compareFields } from './compare-fields.js';
self.onmessage = ({ data }) => {
  try {
    const result = compareWords(words(data.original.pages), words(data.revised.pages));
    result.changes.push(...compareFields(data.original.fields, data.revised.fields));
    self.postMessage({ result });
  }
  catch (error) { self.postMessage({ error: error.message || 'These documents could not be compared.' }); }
};
