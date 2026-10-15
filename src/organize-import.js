export function pagesFromDocuments(documents, nextKey) {
  return documents.flatMap((doc) => Array.from({ length: doc.pages }, (_, index) => ({ documentId: doc.id, page: index + 1, rotation: 0, key: nextKey() })));
}
export function insertPages(pages, added, position = pages.length) {
  const index = Math.max(0, Math.min(pages.length, position));
  return [...pages.slice(0, index), ...added, ...pages.slice(index)];
}
export function insertionAtPoint(cards, x, y, total) {
  for (const card of cards) {
    const { left, right, top, bottom } = card.rect;
    if (y < top) return { index: card.index, card, after: false };
    if (y <= bottom) {
      if (x < left) return { index: card.index, card, after: false };
      if (x <= right) { const after = x > (left + right) / 2; return { index: card.index + Number(after), card, after }; }
    }
  }
  return { index: total, card: null, after: true };
}
