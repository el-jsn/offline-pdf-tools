export const MAX_OUTPUTS = 500;
export function parseRange(value, total) {
  if (!value.trim()) throw new Error('Enter a page range.');
  const pages = [];
  for (const segment of value.split(',')) {
    const match = segment.trim().match(/^(\d+)(?:\s*-\s*(\d+))?$/);
    if (!match) throw new Error('Use page numbers or ranges such as 1-3,5.');
    const first = Number(match[1]); const last = Number(match[2] || match[1]);
    if (!Number.isSafeInteger(first) || !Number.isSafeInteger(last) || first < 1 || last < first || last > total) throw new Error(`Use page numbers between 1 and ${total}, with ranges in ascending order.`);
    if (pages.length + last - first + 1 > 10000) throw new Error('Choose at most 10,000 pages per output.');
    for (let page = first; page <= last; page += 1) pages.push(page);
  }
  return pages;
}
export function splitPlan(mode, size, ranges, total) {
  let plan;
  if (mode === 'ranges') {
    const chunks = ranges.split(';');
    if (chunks.length > MAX_OUTPUTS) throw new Error(`Create at most ${MAX_OUTPUTS} PDFs at once.`);
    plan = chunks.map((range) => parseRange(range, total));
  } else {
    const count = mode === 'every_page' ? 1 : Number(size);
    if (!Number.isSafeInteger(count) || count < 1 || count > total) throw new Error(`Use a group size between 1 and ${total}.`);
    if (Math.ceil(total / count) > MAX_OUTPUTS) throw new Error(`Create at most ${MAX_OUTPUTS} PDFs at once. Choose a larger group size.`);
    plan = [];
    for (let start = 1; start <= total; start += count) plan.push(Array.from({ length: Math.min(count, total - start + 1) }, (_, index) => start + index));
  }
  if (plan.reduce((count, pages) => count + pages.length, 0) > 20000) throw new Error('Choose at most 20,000 output pages in total.');
  return plan;
}
