import { invoke, Channel } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { reorderable } from './reorder.js';
import { icon } from './icons.js';
import { Previews } from './previews.js';
import { splitPlan } from './model.js';
import './styles.css';

const $ = (id) => document.getElementById(id);
const bytes = (value) => value >= 1048576 ? `${(value / 1048576).toFixed(2)} MB` : `${(value / 1024).toFixed(1)} KB`;
const tools = {
  merge: { heading: 'Merge PDFs', action: 'Merge PDFs' },
  compress: { heading: 'Compress PDF', action: 'Compress PDF' },
  split: { heading: 'Split PDF', action: 'Split PDF' },
  organize: { heading: 'Organize pages', action: 'Save PDF' },
};
const state = Object.fromEntries(Object.keys(tools).map((tool) => [tool, {
  documents: [], pages: [], selected: new Set(), history: [], pageWindow: 0,
  compressionPreview: null, measurementKey: null, measurementFailed: false, compression: 'lossless', splitMode: 'every_page', groupSize: '5', ranges: '', result: null,
}]));
let mode = 'merge'; let busy = false; let ready = false; let jobId = null; let cancelAvailable = false; let lastSelected = null;
let compressionTimer = null;
const PAGE_WINDOW = 48;
const current = () => state[mode];
const previews = new Previews();
for (const element of document.querySelectorAll('[data-icon]')) element.replaceWith(icon(element.dataset.icon));
function message(text, type = '') { $('status').textContent = text; $('status').className = type; }
function validation() {
  const view = current();
  if (mode === 'merge') return view.documents.length >= 2 && view.documents.reduce((sum, doc) => sum + doc.pages, 0) <= 20000;
  if (mode === 'compress') return view.documents.length > 0;
  if (view.documents.length !== 1) return false;
  if (mode === 'organize') return view.pages.length > 0;
  if (mode === 'split') { try { splitPlan(view.splitMode, view.groupSize, view.ranges, view.documents[0].pages); } catch { return false; } }
  return true;
}
function controls() {
  const view = current();
  $('save').disabled = busy || !ready || !validation() || (mode === 'compress' && !view.compressionPreview && !view.measurementFailed); $('clear').disabled = busy;
  $('add').disabled = busy || !ready; $('choose').disabled = busy || !ready;
  $('busy-indicator').hidden = !busy; $('loaded').setAttribute('aria-busy', String(busy));
  $('cancel').hidden = !busy || !cancelAvailable; $('reveal').hidden = busy || !view.result;
  document.querySelectorAll('.tool, #compression-options input, #split-options input, #split-options select').forEach((control) => { control.disabled = busy; });
  $('rotate').disabled = busy || !view.selected.size;
  $('delete-pages').disabled = busy || !view.selected.size || view.selected.size === view.pages.length;
  $('move-left').disabled = busy || !view.pages.some((page, index) => index > 0 && view.selected.has(page.page) && !view.selected.has(view.pages[index - 1].page));
  $('move-right').disabled = busy || !view.pages.some((page, index) => index < view.pages.length - 1 && view.selected.has(page.page) && !view.selected.has(view.pages[index + 1].page));
  $('undo').disabled = busy || !view.history.length; $('select-all').disabled = busy || !view.pages.length;
  $('previous-pages').disabled = busy || view.pageWindow === 0; $('next-pages').disabled = busy || (view.pageWindow + 1) * PAGE_WINDOW >= view.pages.length;
  for (const control of document.querySelectorAll('.file-actions button')) control.disabled = busy || control.dataset.boundary === 'true';
  $('save-label').textContent = mode === 'compress' && view.compressionPreview ? (view.documents.length > 1 ? 'Save PDFs' : 'Save PDF') : mode === 'compress' && view.documents.length > 1 ? 'Compress PDFs' : tools[mode].action;
  compressionSummary();
  $('add-label').textContent = ['merge', 'compress'].includes(mode) ? 'Add PDFs' : view.documents.length ? 'Replace PDF' : 'Add PDF';
}
function actionButton(label, symbol, action, disabled = false) {
  const element = document.createElement('button'); element.className = 'icon-button'; element.title = label;
  element.setAttribute('aria-label', label); element.dataset.boundary = String(disabled); element.disabled = busy || disabled;
  element.append(icon(symbol)); element.addEventListener('click', action); return element;
}
function changeOrder(from, to) {
  if (busy || from === to || to < 0 || to >= current().documents.length) return;
  current().documents.splice(to, 0, current().documents.splice(from, 1)[0]); current().result = null; render(); message('Document order updated.');
}
function renderDocuments() {
  $('documents').replaceChildren();
  current().documents.forEach((doc, index) => {
    const row = document.createElement('li'); row.className = 'document';
    if (mode === 'merge') {
      const grip = icon('grip'); row.append(grip);
      reorderable(row, index, grip, () => busy, changeOrder);
    }
    const cover = document.createElement('div'); cover.className = 'cover'; cover.append(icon('document')); previews.watch(cover, doc.id);
    const info = document.createElement('div'); info.className = 'file-info';
    const name = document.createElement('div'); name.className = 'file-name'; name.textContent = doc.name;
    const meta = document.createElement('div'); meta.className = 'file-meta'; meta.textContent = `${doc.pages} ${doc.pages === 1 ? 'page' : 'pages'}, ${bytes(doc.bytes)}`;
    const measured = mode === 'compress' ? current().compressionPreview?.fileBytes[index] : null;
    if (measured != null) meta.textContent += `, output ${bytes(measured)}`;
    info.append(name, meta);
    const actions = document.createElement('div'); actions.className = 'file-actions';
    if (mode === 'merge') {
      const position = document.createElement('span'); position.className = 'position'; position.textContent = String(index + 1); row.append(position);
      actions.append(actionButton(`Move ${doc.name} earlier`, 'up', () => changeOrder(index, index - 1), index === 0));
      actions.append(actionButton(`Move ${doc.name} later`, 'down', () => changeOrder(index, index + 1), index === current().documents.length - 1));
    }
    actions.append(actionButton(`Remove ${doc.name}`, 'trash', () => removeDocument(doc.id))); row.append(cover, info, actions); $('documents').append(row);
  });
}
function rememberEdit() {
  const view = current(); view.history.push(view.pages.map((page) => ({ ...page })));
  if (view.history.length > 50) view.history.shift(); view.result = null;
}
function updateSelection() {
  for (const card of $('page-grid').children) {
    const selected = current().selected.has(Number(card.dataset.page)); card.classList.toggle('selected', selected); card.setAttribute('aria-pressed', String(selected));
    card.querySelector('.page-check').replaceChildren(...(selected ? [icon('check')] : []));
  }
  $('page-selection').textContent = `${current().selected.size} selected`;
  $('select-all').textContent = current().selected.size === current().pages.length ? 'Deselect all' : 'Select all'; controls();
}
function renderPages() {
  $('page-grid').replaceChildren(); const view = current();
  view.pageWindow = Math.min(view.pageWindow, Math.max(0, Math.ceil(view.pages.length / PAGE_WINDOW) - 1));
  const start = view.pageWindow * PAGE_WINDOW;
  view.pages.slice(start, start + PAGE_WINDOW).forEach((page, offset) => {
    const position = start + offset; const card = document.createElement('div'); card.className = 'page-card'; card.tabIndex = 0;
    card.setAttribute('role', 'button'); card.setAttribute('aria-label', `Output page ${position + 1}, original page ${page.page}. Select, or press Enter to preview.`);
    card.dataset.page = String(page.page);
    reorderable(card, position, card, () => busy, (from, to) => {
      rememberEdit(); view.pages.splice(to, 0, view.pages.splice(from, 1)[0]); render(); message('Page order updated.');
    });
    const paper = document.createElement('div'); paper.className = 'page-paper'; previews.watch(paper, view.documents[0].id, page.page, page.rotation);
    const check = document.createElement('span'); check.className = 'page-check'; check.setAttribute('aria-hidden', 'true');
    const caption = document.createElement('span'); caption.className = 'page-caption'; caption.textContent = `Page ${page.page}${page.rotation ? `, rotated ${page.rotation} degrees` : ''}`; card.append(paper, check, caption);
    const select = (event) => {
      if (busy) return;
      if (event.shiftKey && lastSelected !== null) {
        const previous = view.pages.findIndex((item) => item.page === lastSelected);
        if (previous >= 0) for (let index = Math.min(previous, position); index <= Math.max(previous, position); index += 1) view.selected.add(view.pages[index].page);
      } else if (view.selected.has(page.page)) view.selected.delete(page.page); else view.selected.add(page.page);
      lastSelected = page.page; updateSelection();
    };
    card.addEventListener('click', select); card.addEventListener('dblclick', () => openPreview(page));
    card.addEventListener('keydown', (event) => { if (event.key === ' ') { event.preventDefault(); select(event); } if (event.key === 'Enter') { event.preventDefault(); openPreview(page); } });
    $('page-grid').append(card);
  });
  $('page-window').textContent = view.pages.length ? `${start + 1}-${Math.min(start + PAGE_WINDOW, view.pages.length)} of ${view.pages.length}` : 'No pages'; updateSelection();
}
function splitSummary() {
  if (!current().documents.length) return;
  try {
    const plan = splitPlan(current().splitMode, current().groupSize, current().ranges, current().documents[0].pages);
    $('split-summary').textContent = `${plan.length} ${plan.length === 1 ? 'PDF' : 'PDFs'} in a new folder.`;
    $('split-summary').className = 'context-note'; $('split-ranges').removeAttribute('aria-invalid');
  } catch (error) { $('split-summary').textContent = error.message; $('split-summary').className = 'context-note invalid'; $('split-ranges').setAttribute('aria-invalid', 'true'); }
  controls();
}
function render() {
  previews.resetView(); const view = current(); const hasDocuments = view.documents.length > 0;
  $('empty').hidden = hasDocuments; $('loaded').hidden = !hasDocuments;
  $('compression-options').hidden = mode !== 'compress'; $('split-options').hidden = mode !== 'split'; $('organizer').hidden = mode !== 'organize';
  $('group-row').hidden = view.splitMode !== 'groups'; $('range-row').hidden = view.splitMode !== 'ranges'; renderDocuments();
  if (mode === 'organize' && hasDocuments) renderPages(); if (mode === 'split' && hasDocuments) splitSummary();
  const total = view.documents.reduce((sum, doc) => sum + doc.bytes, 0);
  const pages = mode === 'organize' ? view.pages.length : view.documents.reduce((sum, doc) => sum + doc.pages, 0);
  $('summary').textContent = hasDocuments ? `${view.documents.length} ${view.documents.length === 1 ? 'document' : 'documents'}, ${pages} pages, ${bytes(total)}` : 'No PDFs selected';
  $('document-note').textContent = mode === 'compress' ? 'Encrypted PDFs and actual fillable fields are unsupported.' : 'Bookmarks, attachments, and accessibility tags are not copied. Actual fillable fields require a flattened copy.';
  controls(); scheduleCompression();
}
async function cleanRegistry() {
  const ids = [...new Set(Object.values(state).flatMap((view) => view.documents.map((doc) => doc.id)))];
  await invoke('retain_documents', { ids });
  if (!state.compress.compressionPreview) await invoke('discard_compression_preview');
  await previews.releaseUnused(ids);
}
async function removeDocument(id) {
  if (busy) return; busy = true;
  invalidateCompression(); current().documents = current().documents.filter((doc) => doc.id !== id); current().pages = []; current().selected.clear(); current().history = []; current().result = null; render();
  try { await cleanRegistry(); message('Document removed.'); }
  catch (error) { message(String(error), 'error'); }
  finally { busy = false; render(); }
}
async function acceptSelection(selected, insertionIndex) {
  invalidateCompression();
  const view = current();
  if (['merge', 'compress'].includes(mode)) {
    let position = insertionIndex ?? view.documents.length;
    for (const doc of selected) {
      const index = view.documents.findIndex((old) => old.id === doc.id);
      if (index === -1) view.documents.splice(position++, 0, doc); else view.documents[index] = doc;
    }
  } else {
    view.documents = selected;
    view.pages = mode === 'organize' ? Array.from({ length: selected[0].pages }, (_, index) => ({ page: index + 1, rotation: 0 })) : [];
    view.selected.clear(); view.history = []; view.pageWindow = 0; lastSelected = null;
  }
  view.result = null;
  for (const doc of selected) await previews.forget(doc.id);
  await cleanRegistry();
}
async function choose() {
  if (busy || !ready) return; busy = true; controls(); message('Select PDFs');
  try {
    const selected = await invoke('choose_documents', { multiple: ['merge', 'compress'].includes(mode) });
    if (!selected.length) { message('Selection cancelled.'); return; }
    await acceptSelection(selected); message(`${selected.length} ${selected.length === 1 ? 'PDF added' : 'PDFs added'}`);
  } catch (error) { message(String(error), 'error'); } finally { busy = false; render(); }
}
async function importDrop(paths, position) {
  $('content-scroll').classList.remove('file-drop-active');
  if (busy) { message('Wait until processing finishes.'); return; }
  if (!ready) { message('PDF engine unavailable.', 'error'); return; }
  if (!['merge', 'compress'].includes(mode) && paths.length > 1) { message('This tool uses one PDF at a time.', 'error'); return; }
  const row = position ? document.elementFromPoint(position.x / window.devicePixelRatio, position.y / window.devicePixelRatio)?.closest('.document') : null;
  const insertionIndex = mode === 'merge' && row ? Number(row.dataset.reorderIndex) : undefined;
  busy = true; controls(); message('Adding PDFs...');
  try {
    const selected = await invoke('import_dropped_documents', { paths });
    await acceptSelection(selected, insertionIndex);
    message(`${selected.length} ${selected.length === 1 ? 'PDF added' : 'PDFs added'}`);
  } catch (error) { message(String(error), 'error'); } finally { busy = false; render(); }
}
// Stop the browser's default file navigation even outside the document list.
for (const type of ['dragover', 'drop']) window.addEventListener(type, (event) => event.preventDefault(), true);
try {
  const unlisten = await getCurrentWebview().onDragDropEvent(({ payload }) => {
    if (payload.type === 'enter' || payload.type === 'over') $('content-scroll').classList.toggle('file-drop-active', !busy && ready);
    if (payload.type === 'leave') $('content-scroll').classList.remove('file-drop-active');
    if (payload.type === 'drop') importDrop(payload.paths, payload.position);
  });
  window.addEventListener('beforeunload', () => { unlisten(); }, { once: true });
} catch { message('File drop could not start.', 'error'); }
function invalidateCompression() {
  if (mode !== 'compress') return;
  const view = current();
  view.compressionPreview = null; view.measurementKey = null; view.measurementFailed = false;
}
function compressionSummary() {
  const view = state.compress; const host = $('compression-size');
  host.hidden = mode !== 'compress' || !view.documents.length;
  const measured = view.compressionPreview;
  if (!measured) {
    host.removeAttribute('title');
    host.textContent = view.measurementFailed ? 'Size unavailable' : 'Calculating output size...';
    return;
  }
  const saved = measured.inputBytes - measured.outputBytes;
  host.textContent = saved > 0 ? `Output ${bytes(measured.outputBytes)}, saves ${bytes(saved)}` : `Output ${bytes(measured.outputBytes)}, already optimized`;
  host.title = `${measured.outputBytes.toLocaleString()} bytes${measured.files > 1 ? ' total' : ''}`;
}
function scheduleCompression() {
  clearTimeout(compressionTimer);
  if (mode !== 'compress' || busy || !ready || !current().documents.length) return;
  const view = current();
  const key = JSON.stringify([view.compression, view.documents.map((doc) => doc.id)]);
  if (view.measurementKey === key) return;
  compressionTimer = setTimeout(() => measureCompression(key), 250);
}
async function measureCompression(key) {
  if (mode !== 'compress' || busy || !ready || !current().documents.length) return;
  const view = current();
  if (key !== JSON.stringify([view.compression, view.documents.map((doc) => doc.id)])) return;
  view.measurementKey = key; view.measurementFailed = false; view.compressionPreview = null;
  busy = true; cancelAvailable = false; jobId = crypto.randomUUID(); controls();
  const onProgress = new Channel();
  onProgress.onmessage = (progress) => { message(progress.message); cancelAvailable = progress.cancellable; controls(); };
  try {
    view.compressionPreview = await invoke('process_documents', {
      request: { jobId, operation: 'compress', ids: view.documents.map((doc) => doc.id), compression: view.compression, measureOnly: true }, onProgress,
    });
    message('Ready to save');
  } catch (error) {
    view.measurementFailed = true;
    message(String(error), String(error).startsWith('Cancelled') ? '' : 'error');
  } finally {
    busy = false; jobId = null; cancelAvailable = false; render();
  }
}
async function save() {
  if (busy || !ready || !validation() || (mode === 'compress' && !current().compressionPreview && !current().measurementFailed)) return;
  busy = true; cancelAvailable = false; const activeMode = mode; const view = current(); jobId = crypto.randomUUID();
  const onProgress = new Channel(); onProgress.onmessage = (progress) => { message(progress.message); cancelAvailable = progress.cancellable; controls(); };
  controls(); message(mode === 'split' || (mode === 'compress' && view.documents.length > 1) ? 'Select output folder' : 'Select save location');
  try {
    const result = await invoke('process_documents', { request: { jobId, operation: mode, ids: view.documents.map((doc) => doc.id), compression: view.compression, previewId: view.compressionPreview?.id ?? null, splitMode: view.splitMode, pagesPerFile: Number(view.groupSize), ranges: view.ranges, pages: view.pages }, onProgress });
    if (!result) { message('Save cancelled.'); return; } view.result = result;
    let text = activeMode !== 'split' && result.files === 1 ? `Saved ${result.name}. ${bytes(result.outputBytes)}.` : `Saved ${result.files} PDFs in ${result.name}.`;
    if (activeMode === 'compress') text += result.outputBytes < result.inputBytes ? ` ${Math.round((1 - result.outputBytes / result.inputBytes) * 100)}% smaller.` : ' Already optimized. Saved an unchanged copy.';
    message(text, 'success');
  } catch (error) { message(String(error), String(error).startsWith('Cancelled') ? '' : 'error'); }
  finally { busy = false; jobId = null; cancelAvailable = false; controls(); }
}
function editPages(action) {
  if (busy || !current().selected.size || (action === 'remove' && current().selected.size === current().pages.length)) return;
  const view = current(); rememberEdit();
  if (action === 'rotate') view.pages = view.pages.map((page) => view.selected.has(page.page) ? { ...page, rotation: (page.rotation + 90) % 360 } : page);
  if (action === 'remove') { view.pages = view.pages.filter((page) => !view.selected.has(page.page)); view.selected.clear(); }
  if (action === 'earlier') for (let index = 1; index < view.pages.length; index += 1) { if (view.selected.has(view.pages[index].page) && !view.selected.has(view.pages[index - 1].page)) [view.pages[index - 1], view.pages[index]] = [view.pages[index], view.pages[index - 1]]; }
  if (action === 'later') for (let index = view.pages.length - 2; index >= 0; index -= 1) { if (view.selected.has(view.pages[index].page) && !view.selected.has(view.pages[index + 1].page)) [view.pages[index + 1], view.pages[index]] = [view.pages[index], view.pages[index + 1]]; }
  render(); message('Unsaved changes');
}
async function openPreview(page) {
  if (busy || $('preview-dialog').open) return;
  $('preview-title').textContent = `Original page ${page.page}`; $('large-preview').textContent = 'Preparing preview...'; $('preview-dialog').showModal();
  await previews.draw({ host: $('large-preview'), id: current().documents[0].id, page: page.page, rotation: page.rotation, width: 700 });
}
async function checkEngine() {
  if (busy) return; busy = true; controls(); $('retry-engine').hidden = true;
  try { await invoke('engine_status'); ready = true; message('Ready'); }
  catch (error) { ready = false; message(String(error), 'error'); $('retry-engine').hidden = false; }
  finally { busy = false; render(); }
}
function setMode(next) {
  if (busy || next === mode) return; mode = next; lastSelected = null; const tool = tools[mode];
  document.title = tool.heading; invoke('set_view_title', { tool: mode }).catch(() => {}); $('heading').textContent = tool.heading; $('save-label').textContent = tool.action;
  $('choose-label').textContent = ['merge', 'compress'].includes(mode) ? 'Choose PDFs' : 'Choose a PDF'; 
  $('list-label').textContent = ['merge', 'compress'].includes(mode) ? 'Documents' : 'Document';
  $('split-mode').value = current().splitMode; $('group-size').value = current().groupSize; $('split-ranges').value = current().ranges;
  for (const input of document.querySelectorAll('input[name="compression"]')) input.checked = input.value === current().compression;
  $('quality-note').hidden = current().compression === 'lossless';
  for (const button of document.querySelectorAll('.tool')) { button.classList.toggle('active', button.dataset.mode === mode); button.setAttribute('aria-pressed', String(button.dataset.mode === mode)); }
  $('content-scroll').scrollTop = 0; message(ready ? 'Ready' : 'The local PDF engine is unavailable.', ready ? '' : 'error'); render();
}
for (const button of document.querySelectorAll('.tool')) button.addEventListener('click', () => setMode(button.dataset.mode));
$('add').addEventListener('click', choose); $('choose').addEventListener('click', choose); $('save').addEventListener('click', save);
$('clear').addEventListener('click', async () => {
  if (busy) return; busy = true; invalidateCompression(); current().documents = []; current().pages = []; current().history = []; current().selected.clear(); current().result = null; render();
  try { await cleanRegistry(); message('Documents cleared.'); }
  catch (error) { message(String(error), 'error'); }
  finally { busy = false; controls(); }
});
$('retry-engine').addEventListener('click', checkEngine);
$('cancel').addEventListener('click', async () => {
  if (!jobId) return; const cancelledId = jobId; cancelAvailable = false; controls();
  try { await invoke('cancel_job', { jobId: cancelledId }); if (busy && jobId === cancelledId) message('Cancelling local processing...'); }
  catch (error) { if (busy && jobId === cancelledId) message(String(error), 'error'); }
});
$('reveal').addEventListener('click', async () => { if (!current().result) return; try { await invoke('reveal_result', { resultId: current().result.id }); } catch (error) { message(String(error), 'error'); } });
$('toggle-sidebar').addEventListener('click', () => {
  const narrow = window.matchMedia('(max-width: 680px)').matches;
  const hidden = narrow ? !$('workspace').classList.toggle('sidebar-visible') : $('workspace').classList.toggle('sidebar-hidden');
  $('toggle-sidebar').setAttribute('aria-label', hidden ? 'Show sidebar' : 'Hide sidebar'); $('toggle-sidebar').title = hidden ? 'Show sidebar' : 'Hide sidebar';
});
for (const input of document.querySelectorAll('input[name="compression"]')) input.addEventListener('change', () => { invalidateCompression(); current().compression = input.value; $('quality-note').hidden = input.value === 'lossless'; current().result = null; render(); });
$('split-mode').addEventListener('change', () => { current().splitMode = $('split-mode').value; current().result = null; render(); });
$('group-size').addEventListener('input', () => { current().groupSize = $('group-size').value; current().result = null; splitSummary(); });
$('split-ranges').addEventListener('input', () => { current().ranges = $('split-ranges').value; current().result = null; splitSummary(); });
$('select-all').addEventListener('click', () => { if (busy) return; const view = current(); view.selected = view.selected.size === view.pages.length ? new Set() : new Set(view.pages.map((page) => page.page)); updateSelection(); });
$('rotate').addEventListener('click', () => editPages('rotate')); $('delete-pages').addEventListener('click', () => editPages('remove'));
$('move-left').addEventListener('click', () => editPages('earlier')); $('move-right').addEventListener('click', () => editPages('later'));
$('undo').addEventListener('click', () => { if (busy || !current().history.length) return; current().pages = current().history.pop(); current().selected.clear(); current().result = null; render(); message('Page edit undone.'); });
$('previous-pages').addEventListener('click', () => { if (busy || !current().pageWindow) return; current().pageWindow -= 1; render(); });
$('next-pages').addEventListener('click', () => { if (busy || (current().pageWindow + 1) * PAGE_WINDOW >= current().pages.length) return; current().pageWindow += 1; render(); });
$('close-preview').addEventListener('click', () => $('preview-dialog').close()); $('preview-dialog').addEventListener('close', () => { $('large-preview').previewTicket = null; $('large-preview').replaceChildren(); });
document.addEventListener('keydown', (event) => {
  if (!(event.metaKey || event.ctrlKey) || busy || $('preview-dialog').open) return;
  if (event.key.toLowerCase() === 'o') { event.preventDefault(); choose(); }
  if (event.key.toLowerCase() === 's') { event.preventDefault(); save(); }
  if (event.key.toLowerCase() === 'z' && mode === 'organize' && !['INPUT', 'TEXTAREA'].includes(event.target.tagName)) { event.preventDefault(); $('undo').click(); }
});
render(); checkEngine();
