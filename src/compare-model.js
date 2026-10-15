// Word comparison ignores whitespace and page breaks. PDF extraction and drawing
// stay in the UI; this bounded calculation can run in a cancellable local worker.
export function words(pages) {
  const result = [];
  for (const [index, text] of pages.entries()) for (const match of text.normalize('NFC').matchAll(/[\p{L}\p{N}]+(?:[.,'’/-][\p{L}\p{N}]+)*|[^\s]/gu)) {
    if (result.length >= 200000) throw new Error('This PDF has too many words to compare at once. Split a section first.');
    result.push({ text: match[0], page: index + 1 });
  }
  return result;
}

export function compareWords(a, b, maxDistance = 500) {
  let start = 0;
  while (start < a.length && start < b.length && a[start].text === b[start].text) start++;
  let endA = a.length, endB = b.length;
  while (endA > start && endB > start && a[endA - 1].text === b[endB - 1].text) { endA--; endB--; }
  if (start === endA && start === endB) return { changes: [], coarse: false };
  const left = a.slice(start, endA), right = b.slice(start, endB);
  const traces = []; let distance = -1;
  for (let d = 0; d <= Math.min(maxDistance, left.length + right.length); d++) {
    const line = new Int32Array(2 * d + 3); line.fill(-1);
    const previous = traces[d - 1];
    for (let k = -d; k <= d; k += 2) {
      let x = d === 0 ? 0 : k === -d || (k !== d && previous[k - 1 + d] < previous[k + 1 + d])
        ? previous[k + 1 + d] : previous[k - 1 + d] + 1;
      let y = x - k;
      while (x < left.length && y < right.length && left[x].text === right[y].text) { x++; y++; }
      line[k + d + 1] = x;
      if (x >= left.length && y >= right.length) { distance = d; break; }
    }
    traces.push(line);
    if (distance >= 0) break;
  }
  let operations = [], coarse = distance < 0;
  if (coarse) operations = [...left.map((token) => ({ kind: 'remove', token })), ...right.map((token) => ({ kind: 'add', token }))];
  else {
    let x = left.length, y = right.length;
    for (let d = distance; d > 0; d--) {
      const k = x - y, previous = traces[d - 1];
      const previousK = k === -d || (k !== d && previous[k - 1 + d] < previous[k + 1 + d]) ? k + 1 : k - 1;
      const previousX = previous[previousK + d], previousY = previousX - previousK;
      while (x > previousX && y > previousY) operations.push({ kind: 'equal', token: left[--x] }), y--;
      if (x === previousX) operations.push({ kind: 'add', token: right[--y] });
      else operations.push({ kind: 'remove', token: left[--x] });
    }
    while (x > 0 && y > 0) operations.push({ kind: 'equal', token: left[--x] }), y--;
    operations.reverse();
  }
  const changes = []; let current = null, oldIndex = start, newIndex = start;
  for (const { kind, token } of operations) {
    if (kind === 'equal') { current = null; oldIndex++; newIndex++; continue; }
    if (!current) {
      current = { removed: [], added: [], oldPage: a[oldIndex]?.page ?? a.at(-1)?.page ?? 1, newPage: b[newIndex]?.page ?? b.at(-1)?.page ?? 1,
        before: a.slice(Math.max(0, oldIndex - 7), oldIndex).map((word) => word.text).join(' ') };
      changes.push(current);
    }
    current[kind === 'remove' ? 'removed' : 'added'].push(token.text);
    if (kind === 'remove') oldIndex++; else newIndex++;
  }
  return { changes, coarse };
}
