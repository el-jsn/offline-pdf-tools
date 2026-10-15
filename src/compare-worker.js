import { words, compareWords } from './compare-model.js';
self.onmessage = ({ data }) => {
  try { self.postMessage({ result: compareWords(words(data.original), words(data.revised)) }); }
  catch (error) { self.postMessage({ error: error.message || 'These documents could not be compared.' }); }
};
