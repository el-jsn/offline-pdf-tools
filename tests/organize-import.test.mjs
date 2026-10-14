import test from 'node:test';
import assert from 'node:assert/strict';
import { pagesFromDocuments, insertPages, insertionAtPoint } from '../src/organize-import.js';
test('insert pages from multiple documents without confusing identical source page numbers', () => {
  let key = 0;
  const original = pagesFromDocuments([{id:'pdf-a',pages:2}], () => --key);
  const added = pagesFromDocuments([{id:'image',pages:1},{id:'pdf-b',pages:2}], () => --key);
  const result = insertPages(original, added, 1);
  assert.deepEqual(result.map(p => [p.documentId,p.page]), [['pdf-a',1],['image',1],['pdf-b',1],['pdf-b',2],['pdf-a',2]]);
  assert.equal(new Set(result.map(p=>p.key)).size, 5);
  assert.deepEqual(original.map(p=>p.page), [1,2]);
  assert.deepEqual(insertPages(original,added).slice(-3),added);
});
test('drop positions choose either half of a card, gaps, and the end', () => {
  const cards = [0,1,2].map(index=>({index,rect:{left:index*120,right:index*120+100,top:0,bottom:150}}));
  assert.equal(insertionAtPoint(cards,140,50,3).index,1);
  assert.equal(insertionAtPoint(cards,200,50,3).index,2);
  assert.equal(insertionAtPoint(cards,110,50,3).index,1);
  assert.equal(insertionAtPoint(cards,100,200,3).index,3);
  assert.equal(insertionAtPoint([],100,200,0).index,0);
});
