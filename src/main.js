import { invoke, Channel } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { reorderable } from './reorder.js';
import { pagesFromDocuments, insertPages, insertionAtPoint } from './organize-import.js';
import { icon } from './icons.js';
import { Previews } from './previews.js';
import { splitPlan, parseRange, pageList, movePagesTo, movePagesBeside, bookmarkGroups, pageKey } from './model.js';
import { UpdateChecks } from './updates.js';
import './styles.css';
import { loadPreferences, savePreferences } from './preferences.js';
import { setupAppearance } from './appearance.js';

setupAppearance();

const $ = (id) => document.getElementById(id);
const bytes = (value) => value >= 1000000 ? `${(value / 1000000).toFixed(2)} MB` : `${(value / 1000).toFixed(1)} KB`;
const tools = {
  merge: { heading: 'Merge PDFs', action: 'Merge PDFs' },
  compress: { heading: 'Compress PDF', action: 'Compress PDF' },
  split: { heading: 'Split PDF', action: 'Split PDF' },
  organize: { heading: 'Organize pages', action: 'Save PDF' },
  compare: { heading: 'Compare PDFs', action: 'Compare' },
};
const state = Object.fromEntries(Object.keys(tools).map((tool) => [tool, {
  documents: [], importErrors: [], pages: [], selected: new Set(), history: [], future: [], pageWindow: 0,
  outputLabels: {}, review: null, comparison: null, mergeBookmarks: false,
  compressionPreview: null, measurementKey: null, measurementFailed: false, compression: 'balanced', targetSize: '', splitMode: 'selected', splitSize: '10', groupSize: '5', ranges: '', splitGroups: [''], splitNames: [''], activeGroup: 0, splitSeparate: false, splitCombine: false, splitCuts: '', flattenForms: false, acknowledgeChanges: false, result: null,
}]));
let mode = 'merge'; let busy = false; let ready = false; let jobId = null; let cancelAvailable = false; let lastSelected = null;
let compressionTimer = null;
let previewSession = null;
let previewRenderSequence = 0;
let pendingUnlock = null;
let duplicateKey = 0;
let pendingUpdate = null; let updateChecking = false; let availableUpdateVersion = null;
let updateStorage; try { updateStorage = window.localStorage; } catch { /* Storage may be unavailable. */ }
const updateChecks = new UpdateChecks(updateStorage);
const preferences = loadPreferences(updateStorage);
state.compress.compression = preferences.compression; state.compress.targetSize = preferences.target;
let inspectorPage = null, inspectorIndex = 0, inspectorSequence = 0;
let localCancel = null;
const PAGE_WINDOW = 48;
const current = () => state[mode];
const previews = new Previews();
for (const element of document.querySelectorAll('[data-icon]')) element.replaceWith(icon(element.dataset.icon));
function message(text, type = '') { $('status').textContent = text; $('status').className = type; }
function outputDocuments(view) {
  if (mode !== 'organize') return view.documents;
  const used = new Set(view.pages.map((page) => page.documentId || view.documents[0]?.id));
  return view.documents.filter((doc) => used.has(doc.id));
}
function changesFor(view) {
  if (mode === 'compare') return { blocked: null, flatten: false, acknowledge: false, notes: [] };
  const pageOperation = mode !== 'compress';
  const docs = outputDocuments(view).filter((doc) => !doc.locked);
  const any = (test) => docs.some((doc) => test(doc.features));
  const blocked = any((features) => features.unsupportedForms) ? 'XFA forms are unsupported. Print or export a flattened copy, then add it again.'
    : pageOperation && any((features) => features.needsAppearances) ? 'A filled form must be saved in its original application before it can be changed.' : null;
  const flatten = !blocked && pageOperation && any((features) => features.forms);
  const acknowledge = !blocked && (any((features) => features.signatures) || (pageOperation && any((features) => features.tags)));
  const notes = [];
  if (flatten) notes.push('Forms and annotations will become static.');
  if (any((features) => features.signatures)) notes.push('Signatures will be removed in the saved copy.');
  if (pageOperation && any((features) => features.tags)) notes.push('Page changes may remove accessibility tags.');
  return { blocked, flatten, acknowledge, notes };
}
function targetBytes(view) {
  const mb = Number(view.targetSize);
  if (!Number.isFinite(mb) || view.compression === 'lossless') return null;
  const limit = Math.floor(mb * 1000000);
  return limit >= 100000 && limit <= 250000000 ? limit : null;
}
function validation() {
  const view = current();
  if (outputDocuments(view).some((doc) => doc.locked)) return false;
  const changes = changesFor(view);
  if (changes.blocked) return false;
  if (changes.flatten && !view.flattenForms) return false;
  if (changes.acknowledge && !view.acknowledgeChanges) return false;
  if (mode === 'compare') return view.documents.length === 2 && new Set(view.documents.map((doc) => doc.compareRole)).size === 2;
  if (mode === 'merge') return view.documents.length >= 2 && view.documents.reduce((sum, doc) => sum + doc.pages, 0) <= 20000;
  if (mode === 'compress') return view.documents.length > 0 && !(view.targetSize.trim() && targetBytes(view) === null);
  if (mode === 'organize') return view.pages.length > 0 && view.pages.length <= 20000;
  if (view.documents.length !== 1) return false;
  if (mode === 'split') { try { splitPlanFor(view); } catch { return false; } }
  return true;
}
function controls() {
  const view = current();
  $('save').disabled = busy || !ready || !validation() || (mode === 'compress' && !view.compressionPreview && !view.measurementFailed); $('clear').disabled = busy;
  $('app-update').disabled = busy || updateChecking;
  $('app-update').classList.toggle('update-available', Boolean(availableUpdateVersion));
  $('app-update').textContent = availableUpdateVersion ? 'Update available' : 'Check for updates';
  $('app-update').title = availableUpdateVersion ? `Install version ${availableUpdateVersion}` : '';
  $('app-update').setAttribute('aria-label', availableUpdateVersion ? `Update available. Install version ${availableUpdateVersion}` : 'Check for updates');
  $('release-notes').disabled = busy;
  $('add').disabled = busy || !ready;
  $('toggle-inspector').hidden = !view.documents.length;
  $('toggle-inspector').disabled = busy;
  $('check-pages').hidden = mode !== 'organize'; $('check-pages').disabled = busy || !view.pages.length;
  $('merge-bookmarks').disabled = busy;
  const canSaveLimit = Boolean(targetBytes(state.compress)) && preferences.presets.length < 12;
  $('save-limit').hidden = !canSaveLimit; $('save-limit').disabled = busy || !canSaveLimit;
  $('limit-presets').hidden = !preferences.presets.length && !canSaveLimit;
  $('limit-preset').hidden = !preferences.presets.length;
  $('limit-preset').disabled = busy || state.compress.compression === 'lossless';
  $('delete-limit').disabled = busy;
  $('swap-documents').disabled = busy || state.compare.documents.length !== 2;
  for (const button of $('compare-slots').querySelectorAll('button')) button.disabled = busy;
  $('continue-output').hidden = busy || !view.result;
  $('organizer-edits').hidden = mode !== 'organize' || !view.selected.size;
  $('organizer-selection').hidden = mode !== 'organize' || !view.selected.size;
  $('page-selection').hidden = mode === 'organize';
  document.querySelector('.page-more').hidden = mode !== 'organize';
  $('selected-page-actions').hidden = !view.selected.size;
  $('merge-options-summary').textContent = view.mergeBookmarks ? 'Options: bookmarks on' : 'Options';
  for (const button of $('page-review').querySelectorAll('button')) button.disabled = busy; $('choose').disabled = busy || !ready;
  $('busy-indicator').hidden = !busy; $('loaded').setAttribute('aria-busy', String(busy));
  $('cancel').hidden = !busy || !cancelAvailable; $('reveal').hidden = busy || !view.result;
  document.querySelectorAll('.tool, #compression-options input, #split-options input, #split-options select, #split-options button').forEach((control) => { control.disabled = busy; });
  $('invert-selection').disabled = busy || !view.pages.length;
  $('add-split-group').disabled = busy || view.splitGroups.length >= 500;
  $('rotate-left').disabled = busy || !view.selected.size;
  $('move-page-position').disabled = busy || !view.selected.size; $('move-pages-to').disabled = busy || !view.selected.size;
  $('move-page-position').max = String(Math.max(1, view.pages.length - view.selected.size + 1));
  $('rotate').disabled = busy || !view.selected.size;
  $('duplicate-pages').disabled = busy || !view.selected.size;
  $('delete-pages').disabled = busy || !view.selected.size || view.selected.size === view.pages.length;
  $('move-left').disabled = busy || !view.pages.some((page, index) => index > 0 && view.selected.has(pageKey(page)) && !view.selected.has(pageKey(view.pages[index - 1])));
  $('move-right').disabled = busy || !view.pages.some((page, index) => index < view.pages.length - 1 && view.selected.has(pageKey(page)) && !view.selected.has(pageKey(view.pages[index + 1])));
  $('undo').disabled = busy || !view.history.length; $('redo').disabled = busy || !view.future.length; $('select-all').disabled = busy || !view.pages.length;
  $('previous-pages').disabled = busy || view.pageWindow === 0; $('next-pages').disabled = busy || (view.pageWindow + 1) * PAGE_WINDOW >= view.pages.length;
  $('jump-page').disabled = busy || !view.pages.length; $('thumbnail-size').disabled = busy;
  $('compare-output').disabled = busy || !state.compress.compressionPreview || state.compress.documents.length !== 1;
  $('target-size').disabled = busy || view.compression === 'lossless';
  if (view.targetSize.trim() && targetBytes(view) === null) $('target-size').setAttribute('aria-invalid', 'true'); else $('target-size').removeAttribute('aria-invalid');
  $('password-unlock').disabled = busy; $('password-skip').disabled = busy; $('pdf-password').disabled = busy;
  $('flatten-forms').disabled = busy; $('acknowledge-changes').disabled = busy;
  for (const control of document.querySelectorAll('.file-actions button, .page-actions button')) control.disabled = busy || control.dataset.boundary === 'true';
  $('save-label').textContent = mode === 'split' && view.splitMode === 'selected' ? 'Extract pages' : mode === 'compress' && view.compressionPreview ? (view.documents.length > 1 ? 'Save PDFs' : 'Save PDF') : mode === 'compress' && view.documents.length > 1 ? 'Compress PDFs' : tools[mode].action;
  compressionSummary();
  $('add-label').textContent = mode === 'compare' ? 'Choose PDFs' : ['merge', 'organize'].includes(mode) ? 'Add files' : mode === 'compress' ? 'Add PDFs' : view.documents.length ? 'Replace PDF' : 'Add PDF';
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
    const cover = document.createElement('div'); cover.className = 'cover'; cover.append(icon('document'));
    if (!doc.locked) previews.watch(cover, doc.id, 1, 0, doc.previewBytes);
    const info = document.createElement('div'); info.className = 'file-info';
    const name = document.createElement('div'); name.className = 'file-name'; name.textContent = doc.name;
    const meta = document.createElement('div'); meta.className = 'file-meta';
    meta.textContent = doc.locked ? `${bytes(doc.bytes)}, password protected` : `${doc.pages} ${doc.pages === 1 ? 'page' : 'pages'}, ${bytes(doc.bytes)}`;
    const measured = mode === 'compress' && !doc.locked ? current().compressionPreview?.fileBytes[index] : null;
    if (measured != null) meta.textContent += `, output ${bytes(measured)}`;
    info.append(name, meta);
    const actions = document.createElement('div'); actions.className = 'file-actions';
    if (mode === 'merge') {
      const position = document.createElement('span'); position.className = 'position'; position.textContent = String(index + 1); row.append(position);
      actions.append(actionButton(`Move ${doc.name} earlier`, 'up', () => changeOrder(index, index - 1), index === 0));
      actions.append(actionButton(`Move ${doc.name} later`, 'down', () => changeOrder(index, index + 1), index === current().documents.length - 1));
    }
    if (doc.locked) {
      const unlock = document.createElement('button'); unlock.className = 'secondary'; unlock.textContent = 'Unlock';
      unlock.setAttribute('aria-label', `Unlock ${doc.name}`); unlock.addEventListener('click', () => promptPassword(doc));
      actions.append(unlock);
    } else {
      const preview = document.createElement('button'); preview.className = 'secondary';
      preview.append(icon('preview'), document.createTextNode('Preview'));
      preview.setAttribute('aria-label', `Preview ${doc.name}`); preview.addEventListener('click', () => openDocumentPreview(doc));
      actions.append(preview);
    }
    actions.append(actionButton(`Remove ${doc.name}`, 'trash', () => removeDocument(doc.id))); row.append(cover, info, actions); $('documents').append(row);
  });
}
function rememberEdit() {
  const view = current(); view.review = null; view.history.push(view.pages.map((page) => ({ ...page }))); view.future = [];
  if (view.history.length > 50) view.history.shift(); view.result = null;
}
function updateSelection() {
  for (const card of $('page-grid').children) {
    const selected = current().selected.has(Number(card.dataset.page)); card.classList.toggle('selected', selected); card.setAttribute('aria-pressed', String(selected));
    card.querySelector('.page-check').replaceChildren(...(selected ? [icon('check')] : []));
  }
  $('organizer-selection').textContent = mode === 'organize' ? `${current().selected.size} selected` : '';
  $('page-selection').textContent = mode === 'split' && current().splitMode === 'ranges' ? `Group ${current().activeGroup + 1}: ${current().selected.size} pages selected` : mode === 'split' && current().splitMode === 'after' ? `${current().selected.size} split points` : mode === 'split' && current().splitMode === 'bookmarks' ? '' : `${current().selected.size} selected`;
  renderInspector();
  $('select-all').textContent = current().selected.size === current().pages.length ? 'Deselect all' : 'Select all'; controls();
}
function renderPages() {
  $('page-grid').replaceChildren(); const view = current();
  view.pageWindow = Math.min(view.pageWindow, Math.max(0, Math.ceil(view.pages.length / PAGE_WINDOW) - 1));
  const start = view.pageWindow * PAGE_WINDOW;
  view.pages.slice(start, start + PAGE_WINDOW).forEach((page, offset) => {
    const position = start + offset; const card = document.createElement('div'); card.className = 'page-card'; card.tabIndex = 0;
    card.setAttribute('role', 'button'); card.setAttribute('aria-label', mode === 'split' ? `Page ${page.page}. ${view.splitMode === 'after' ? 'Toggle split point' : 'Select'}. Press Enter to preview.` : `Output page ${position + 1}, original page ${page.page}. Select, or press Enter to preview.`);
    card.dataset.page = String(pageKey(page));
    if (mode === 'organize') card.title = `${view.documents.find((doc) => doc.id === page.documentId)?.name || ''}, page ${page.page}`;
    if (mode === 'organize') reorderable(card, position, card, () => busy, (from, to, after) => {
      const pages = movePagesBeside(view.pages, view.selected, pageKey(view.pages[to]), after);
      if (pages.map(pageKey).join(',') === view.pages.map(pageKey).join(',')) return;
      rememberEdit(); view.pages = pages; render(); message('Page order updated.');
    }, { onStart: () => {
      if (!view.selected.has(pageKey(page))) view.selected = new Set([pageKey(page)]);
      updateSelection(); return view.selected.size;
    } });
    const source = view.documents.find((doc) => doc.id === page.documentId) || view.documents[0];
    const paper = document.createElement('div'); paper.className = 'page-paper'; previews.watch(paper, source.id, page.page, page.rotation, source.previewBytes);
    const check = document.createElement('span'); check.className = 'page-check'; check.setAttribute('aria-hidden', 'true');
    const caption = document.createElement('span'); caption.className = 'page-caption'; caption.textContent = `Page ${mode === 'organize' ? position + 1 : page.page}${page.rotation ? `, rotated ${page.rotation} degrees` : ''}`; card.append(paper, check, caption);
    if (mode === 'organize' && view.documents.length > 1) { const label = document.createElement('span'); label.className = 'page-source'; label.textContent = source.name; label.title = source.name; card.append(label); }
    {
      const actions = document.createElement('div'); actions.className = 'page-actions';
      const preview = actionButton(`Preview page ${page.page}`, 'preview', () => openPreview(page));
      const rotate = actionButton(`Rotate page ${page.page}`, 'rotate', () => { if (busy) return; rememberEdit(); page.rotation = (page.rotation + 90) % 360; render(); message('Unsaved changes'); });
      for (const button of [preview, rotate]) button.addEventListener('click', (event) => event.stopPropagation());
      actions.addEventListener('dblclick', (event) => event.stopPropagation()); actions.append(preview); if (mode === 'organize') actions.append(rotate); card.append(actions);
    }
    const select = (event) => {
      if (busy) return;
      const key = pageKey(page);
      if (event.shiftKey && lastSelected !== null) {
        const previous = view.pages.findIndex((item) => pageKey(item) === lastSelected);
        if (previous >= 0) for (let index = Math.min(previous, position); index <= Math.max(previous, position); index += 1) view.selected.add(pageKey(view.pages[index]));
      } else if (view.selected.has(key)) view.selected.delete(key); else view.selected.add(key);
      inspectorPage = { ...page, documentId: source.id };
      lastSelected = key; if (mode === 'split') writeSplitSelection(); updateSelection();
    };
    card.addEventListener('click', select); card.addEventListener('dblclick', () => openPreview(page));
    card.addEventListener('keydown', (event) => {
      if (event.target !== card) return;
      const offset = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -getComputedStyle($('page-grid')).gridTemplateColumns.split(' ').length, ArrowDown: getComputedStyle($('page-grid')).gridTemplateColumns.split(' ').length }[event.key];
      if (offset && !busy) {
        const nextPosition = Math.max(0, Math.min(view.pages.length - 1, position + offset)); const next = view.pages[nextPosition]; event.preventDefault();
        if (event.shiftKey) { if (lastSelected === null) lastSelected = pageKey(page); const found = view.pages.findIndex((item) => pageKey(item) === lastSelected); const anchor = found < 0 ? position : found; for (let index = Math.min(anchor, nextPosition); index <= Math.max(anchor, nextPosition); index++) view.selected.add(pageKey(view.pages[index])); if (mode === 'split') writeSplitSelection(); updateSelection(); }
        if (Math.floor(nextPosition / PAGE_WINDOW) !== view.pageWindow) { view.pageWindow = Math.floor(nextPosition / PAGE_WINDOW); render(); }
        $('page-grid').querySelector(`[data-page="${pageKey(next)}"]`)?.focus(); return;
      }
      if (event.key === ' ') { event.preventDefault(); select(event); } if (event.key === 'Enter') { event.preventDefault(); openPreview(page); } });
    $('page-grid').append(card);
  });
  $('page-window').textContent = view.pages.length ? `${start + 1}-${Math.min(start + PAGE_WINDOW, view.pages.length)} of ${view.pages.length}` : 'No pages';
  $('jump-page').max = String(view.pages.length); updateSelection();
}
function splitSizeBytes(view) {
  const value = Number(view.splitSize), bytes = Math.floor(value * 1000000);
  if (!Number.isFinite(value) || bytes < 100000 || bytes > 250000000) throw new Error('Choose a limit between 0.1 and 250 MB.');
  return bytes;
}
function splitPlanFor(view) {
  if (view.splitMode === 'file_size') { splitSizeBytes(view); return [Array.from({ length: view.documents[0].pages }, (_, index) => index + 1)]; }
  if (view.splitMode === 'bookmarks') {
    const plan = bookmarkGroups(view.documents[0].features.bookmarks, view.documents[0].pages).map((group) => group.pages);
    if (plan.length > 500 || plan.some((group) => group.length > 10000) || plan.flat().length > 20000) throw new Error('Choose at most 20,000 output pages in total.');
    return plan;
  }
  const ranges = view.splitMode === 'ranges' ? view.splitGroups.join(';') : view.splitMode === 'after' ? view.splitCuts : view.ranges;
  const plan = splitPlan(view.splitMode, view.groupSize, ranges, view.documents[0].pages, { separate: view.splitSeparate, combine: view.splitCombine });
  if (plan.map(pageList).join(';').length > 20000) throw new Error('The page range list is too long.');
  return plan;
}
function defaultSplitNames(view) {
  if (view.splitMode === 'ranges' && !view.splitCombine) return view.splitGroups.map((_, i) => view.splitNames[i]?.trim() || `Group ${i + 1}`);
  if (view.splitMode !== 'bookmarks') return [];
  try { return bookmarkGroups(view.documents[0].features.bookmarks, view.documents[0].pages).map((group) => group.name); } catch { return []; }
}
function splitOutputNames(view) {
  const defaults = defaultSplitNames(view);
  if (view.splitMode === 'file_size') return [];
  return splitPlanFor(view).map((pages, index) => view.outputLabels[`${view.splitMode}:${pages.join(',')}`] ?? defaults[index] ?? `part-${String(index + 1).padStart(3, '0')}`);
}
function renderSplitOutputs(plan, names) {
  const view = current(), doc = view.documents[0];
  plan.forEach((pages, index) => {
    const item = document.createElement('li'); item.className = 'split-output';
    const cover = document.createElement('button'); cover.className = 'output-cover'; cover.setAttribute('aria-label', `Preview output ${index + 1}`);
    previews.watch(cover, doc.id, pages[0], 0, doc.previewBytes); cover.addEventListener('click', () => openDocumentPreview(doc, pages[0]));
    const info = document.createElement('div'), label = document.createElement('label'); label.className = 'sr-only'; label.htmlFor = `output-name-${index}`; label.textContent = `Output ${index + 1} filename`;
    const name = document.createElement('input'); name.id = label.htmlFor; name.type = 'text'; name.maxLength = 80; name.value = names[index];
    const filename = document.createElement('p'); filename.className = 'output-filename';
    const update = () => { const cleaned = [...name.value.trim()].filter((c) => !/\p{Cc}/u.test(c) && !'/\\:*?"<>|'.includes(c)).slice(0, 80).join('').replace(/^[ .]+|[ .]+$/g, '') || `part-${String(index + 1).padStart(3, '0')}`; filename.textContent = `${String(index + 1).padStart(3, '0')}-${cleaned}.pdf`; };
    update(); name.addEventListener('input', () => { view.outputLabels[`${view.splitMode}:${pages.join(',')}`] = name.value; view.result = null; update(); controls(); });
    const meta = document.createElement('p'); meta.className = 'context-note'; meta.textContent = `${pages.length} ${pages.length === 1 ? 'page' : 'pages'}: ${pageList(pages)}`;
    info.append(label, name, filename, meta); item.append(cover, info); $('split-output-list').append(item);
  });
}
function readSplitSelection() {
  const view = current();
  if (view.splitMode === 'bookmarks') { view.selected.clear(); return; }
  const text = view.splitMode === 'ranges' ? view.splitGroups[view.activeGroup] : view.splitMode === 'after' ? view.splitCuts : view.ranges;
  try { view.selected = new Set(parseRange(text, view.documents[0].pages)); } catch { view.selected.clear(); }
}
function writeSplitSelection() {
  const view = current(); if (view.splitMode === 'bookmarks') return;
  const text = pageList([...view.selected].sort((a, b) => a - b)); view.result = null;
  if (view.splitMode === 'ranges') {
    view.splitGroups[view.activeGroup] = text;
    $(`split-group-${view.activeGroup}`).value = text;
  } else if (view.splitMode === 'after') { view.splitCuts = text; $('split-cuts').value = text; }
  else { view.ranges = text; $('split-ranges').value = text; }
  splitSummary();
}
function renderSplitGroups() {
  const view = current(); $('split-group-list').replaceChildren();
  view.splitGroups.forEach((value, index) => {
    const row = document.createElement('div'); row.className = 'split-group'; row.classList.toggle('active', index === view.activeGroup);
    const label = document.createElement('input'); label.type = 'text'; label.className = 'group-name'; label.placeholder = `Group ${index + 1}`; label.value = view.splitNames[index] || ''; label.setAttribute('aria-label', `Group ${index + 1} name`); label.maxLength = 80; label.addEventListener('input', () => { view.splitNames[index] = label.value; view.result = null; splitSummary(); });
    const input = document.createElement('input'); input.type = 'text'; input.id = `split-group-${index}`; input.setAttribute('aria-label', `Group ${index + 1} pages`); input.value = value; input.placeholder = '1-3, 5, 8'; input.autocomplete = 'off'; input.spellcheck = false;
    const activate = () => {
      if (busy) return;
      view.activeGroup = index; lastSelected = null;
      for (const [position, item] of [...$('split-group-list').children].entries()) item.classList.toggle('active', position === index);
      readSplitSelection(); updateSelection();
    };
    input.addEventListener('focus', activate);
    input.addEventListener('input', () => { view.splitGroups[index] = input.value; view.result = null; readSplitSelection(); splitSummary(); updateSelection(); });
    const select = document.createElement('button'); select.className = 'secondary'; select.textContent = 'Choose pages'; select.addEventListener('click', activate);
    const actions = document.createElement('div'); actions.className = 'file-actions';
    const move = (offset) => { if (busy) return; view.splitGroups.splice(index + offset, 0, view.splitGroups.splice(index, 1)[0]); view.splitNames.splice(index + offset, 0, view.splitNames.splice(index, 1)[0]); view.activeGroup = index + offset; view.result = null; lastSelected = null; render(); };
    actions.append(actionButton(`Move group ${index + 1} earlier`, 'up', () => move(-1), index === 0), actionButton(`Move group ${index + 1} later`, 'down', () => move(1), index === view.splitGroups.length - 1));
    actions.append(actionButton(`Remove group ${index + 1}`, 'trash', () => { if (busy) return; view.splitGroups.splice(index, 1); view.splitNames.splice(index, 1); view.activeGroup = Math.min(view.activeGroup, view.splitGroups.length - 1); view.result = null; lastSelected = null; render(); }, view.splitGroups.length === 1));
    row.append(label, input, select, actions); $('split-group-list').append(row);
  });
}
function splitSummary() {
  if (!current().documents.length) return;
  const view = current(); $('split-output-list').replaceChildren(); $('split-summary').hidden = false;
  if (view.splitMode === 'selected' && !view.ranges.trim()) { $('split-summary').hidden = true; $('split-output-details').hidden = true; controls(); return; }
  for (const input of $('split-options').querySelectorAll('input')) input.removeAttribute('aria-invalid');
  try {
    if (view.splitMode === 'file_size') {
      splitSizeBytes(view); $('split-summary').textContent = ''; $('split-summary').hidden = true;
      $('split-summary').className = 'context-note'; $('split-output-details').hidden = true; controls(); return;
    }
    const plan = splitPlanFor(view); const used = new Set(plan.flat()).size; const names = splitOutputNames(view);
    $('split-summary').textContent = `${plan.length} ${plan.length === 1 ? 'PDF' : 'PDFs'}, ${plan.flat().length} output pages${used < view.documents[0].pages ? `, ${view.documents[0].pages - used} pages excluded` : ''}.`;
    $('split-summary').className = 'context-note'; $('split-output-details').hidden = false;
    renderSplitOutputs(plan, names);
  } catch (error) {
    $('split-summary').textContent = error.message; $('split-summary').className = 'context-note invalid'; $('split-output-details').hidden = true;
    if (view.splitMode === 'ranges') view.splitGroups.forEach((range, index) => { try { parseRange(range, view.documents[0].pages); } catch { $(`split-group-${index}`).setAttribute('aria-invalid', 'true'); } });
    else if (view.splitMode !== 'bookmarks') $(view.splitMode === 'file_size' ? 'split-size' : view.splitMode === 'after' ? 'split-cuts' : view.splitMode === 'groups' ? 'group-size' : 'split-ranges').setAttribute('aria-invalid', 'true');
  }
  controls();
}
function renderChanges() {
  const view = current(); const section = $('document-changes');
  const info = view.documents.length ? changesFor(view) : null;
  const active = Boolean(info) && (Boolean(info.blocked) || info.flatten || info.acknowledge || info.notes.length > 0);
  section.hidden = !active;
  if (!active) return;
  $('changes-message').textContent = info.blocked || info.notes.join(' ');
  $('changes-message').className = info.blocked ? 'invalid' : '';
  $('flatten-row').hidden = !info.flatten; $('acknowledge-row').hidden = !info.acknowledge;
  $('flatten-forms').checked = view.flattenForms; $('acknowledge-changes').checked = view.acknowledgeChanges;
}
function render() {
  document.querySelector('.drop-label').textContent = ['merge', 'organize'].includes(mode) ? 'Drop PDFs or images here' : 'Drop PDFs here';
  $('choose-label').textContent = ['merge', 'organize'].includes(mode) ? 'Choose files' : mode === 'compress' ? 'Choose PDFs' : 'Choose a PDF';
  const previousCards = mode === 'organize' ? new Map([...$('page-grid').children].map((card) => [card.dataset.page, card.getBoundingClientRect()])) : new Map();
  previews.resetView(); const view = current(); const hasDocuments = view.documents.length > 0;
  $('empty').hidden = hasDocuments; $('loaded').hidden = !hasDocuments;
  $('compression-options').hidden = mode !== 'compress'; $('split-options').hidden = mode !== 'split'; $('organizer').hidden = mode !== 'organize' && !(mode === 'split' && ['selected', 'ranges', 'after'].includes(view.splitMode));
  $('organizer-edits').hidden = mode !== 'organize'; $('move-pages-controls').hidden = mode !== 'organize'; $('organizer-selection').hidden = mode !== 'organize'; $('undo').hidden = mode !== 'organize'; $('redo').hidden = mode !== 'organize'; $('invert-selection').hidden = mode !== 'split';
  $('page-grid').classList.toggle('split-pages', mode === 'split'); $('page-grid').setAttribute('aria-label', mode === 'split' ? 'Source pages' : 'Pages in output order');
  $('organizer-tools').parentElement.setAttribute('aria-label', mode === 'split' ? 'Select pages for splitting' : 'Organize selected pages');
  $('size-row').hidden = view.splitMode !== 'file_size'; $('group-row').hidden = view.splitMode !== 'groups'; $('range-row').hidden = view.splitMode !== 'ranges'; $('selection-row').hidden = view.splitMode !== 'selected'; $('cut-row').hidden = view.splitMode !== 'after'; renderDocuments();
  if (mode === 'split' && hasDocuments) { renderSplitGroups(); readSplitSelection(); }
  if (!$('organizer').hidden && hasDocuments) renderPages(); if (mode === 'split' && hasDocuments) splitSummary();
  if (mode === 'organize' && !window.matchMedia('(prefers-reduced-motion: reduce)').matches) for (const card of $('page-grid').children) {
    const before = previousCards.get(card.dataset.page); if (!before) continue; const after = card.getBoundingClientRect();
    if (before.x !== after.x || before.y !== after.y) card.animate([{ transform: `translate(${before.x - after.x}px, ${before.y - after.y}px)` }, { transform: 'translate(0, 0)' }], { duration: 200, easing: 'cubic-bezier(.2,.8,.2,1)' });
  }
  const total = view.documents.reduce((sum, doc) => sum + doc.bytes, 0);
  const pages = mode === 'organize' ? view.pages.length : view.documents.reduce((sum, doc) => sum + doc.pages, 0);
  $('summary').textContent = hasDocuments ? `${view.documents.length} ${view.documents.length === 1 ? 'document' : 'documents'}, ${pages} pages, ${bytes(total)}` : 'No PDFs selected';
  $('merge-options').hidden = mode !== 'merge'; $('merge-bookmarks').checked = view.mergeBookmarks;
  $('comparison').hidden = mode !== 'compare';
  if (mode === 'compare') { $('empty').hidden = true; $('loaded').hidden = false; $('documents').hidden = true; document.querySelector('.section-heading').hidden = true; renderCompareSlots(); if (view.comparison && !$('compare-changes').children.length) renderComparisonChanges(); }
  else { $('documents').hidden = false; document.querySelector('.section-heading').hidden = false; }
  renderPageReview(); renderContinuation(); renderInspector();
  renderChanges(); renderImportErrors();
  controls(); scheduleCompression();
}
async function cleanRegistry() {
  const ids = [...new Set(Object.values(state).flatMap((view) => view.documents.map((doc) => doc.id)))];
  await invoke('retain_documents', { ids });
  if (!state.compress.compressionPreview) await invoke('discard_compression_preview');
  await previews.releaseUnused(ids);
}
function resetEdits(view) {
  view.flattenForms = false; view.acknowledgeChanges = false; view.future = [];
}
async function removeDocument(id) {
  if (busy) return; busy = true;
  invalidateCompression(); current().documents = current().documents.filter((doc) => doc.id !== id); current().pages = mode === 'organize' ? current().pages.filter((page) => page.documentId !== id) : []; current().selected.clear(); current().history = []; current().result = null; current().review = null; if (mode === 'compare') clearComparison(); resetEdits(current()); render();
  try { await cleanRegistry(); message('Document removed.'); }
  catch (error) { message(String(error), 'error'); }
  finally { busy = false; render(); }
}
async function acceptSelection(selected, insertionIndex) {
  invalidateCompression();
  const view = current();
  if (mode === 'compare') {
    if (selected.length > 2) throw new Error('Choose two PDFs to compare.');
    if (selected.length === 2) view.documents = selected.map((doc, index) => ({ ...doc, compareRole: index }));
    else {
      if (view.documents.length === 2) throw new Error('Use Replace under Original PDF or Revised PDF.');
      const role = view.documents.some((doc) => doc.compareRole === 0) ? 1 : 0;
      view.documents = [...view.documents, { ...selected[0], compareRole: role }].sort((a,b) => a.compareRole - b.compareRole);
    }
    clearComparison();
  } else if (['merge', 'compress'].includes(mode)) {
    let position = insertionIndex ?? view.documents.length;
    for (const doc of selected) {
      const index = view.documents.findIndex((old) => old.id === doc.id);
      if (index === -1) view.documents.splice(position++, 0, doc); else view.documents[index] = doc;
    }
  } else if (mode === 'organize') {
    if (view.pages.length + selected.reduce((sum, doc) => sum + doc.pages, 0) > 20000) throw new Error('Choose at most 20,000 pages.');
    const added = pagesFromDocuments(selected, () => --duplicateKey);
    const wasEmpty = !view.pages.length;
    rememberEdit();
    const position = insertionIndex ?? view.pages.length;
    let pendingPosition = position;
    for (const doc of selected) {
      const existing = view.documents.findIndex((item) => item.id === doc.id);
      if (doc.locked) doc.insertionIndex = pendingPosition;
      pendingPosition += doc.pages;
      if (existing < 0) view.documents.push(doc); else view.documents[existing] = doc;
    }
    view.pages = insertPages(view.pages, added, position);
    view.selected = wasEmpty ? new Set() : new Set(added.map(pageKey));
    view.pageWindow = Math.floor(position / PAGE_WINDOW); lastSelected = null;
  } else {
    view.documents = selected;
    view.pages = ['organize', 'split'].includes(mode) ? Array.from({ length: selected[0].pages }, (_, index) => ({ page: index + 1, rotation: 0 })) : [];
    view.selected.clear(); view.history = []; view.pageWindow = 0; lastSelected = null;
    if (mode === 'split') { $('split-output-details').open = false; view.outputLabels = {}; view.ranges = ''; view.splitGroups = ['']; view.splitNames = ['']; view.activeGroup = 0; view.splitCuts = ''; $('split-ranges').value = ''; $('split-cuts').value = ''; }
  }
  resetEdits(view); view.result = null; view.review = null;
  for (const doc of selected) await previews.forget(doc.id);
  await cleanRegistry();
}
function promptPassword(doc) {
  if (busy || $('password-dialog').open) return;
  pendingUnlock = doc;
  $('password-name').textContent = doc.name;
  $('password-error').hidden = true; $('password-error').textContent = '';
  $('pdf-password').value = '';
  $('password-dialog').showModal();
  $('pdf-password').focus();
}
function promptNextLocked() {
  const locked = current().documents.find((doc) => doc.locked);
  if (locked) promptPassword(locked);
}
function showImportResult(result) {
  const added = result.documents.length, skipped = result.rejected.length;
  const parts = [];
  if (added) parts.push(`${added} ${added === 1 ? 'file added' : 'files added'}`);
  if (skipped) parts.push(`${skipped} ${skipped === 1 ? 'file skipped' : 'files skipped'}`);
  message(parts.join('. ') || 'Selection cancelled.', skipped && !added ? 'error' : '');
}
function renderImportErrors() {
  const report = $('import-errors'); const errors = current().importErrors;
  report.hidden = !errors.length;
  $('import-errors-summary').textContent = `${errors.length} ${errors.length === 1 ? 'file skipped' : 'files skipped'}`;
  $('import-errors-list').replaceChildren(...errors.map((error) => {
    const item = document.createElement('li');
    const name = document.createElement('strong'); name.textContent = error.name;
    item.append(name, document.createTextNode(`: ${error.reason}`)); return item;
  }));
}
async function choose() {
  if (busy || !ready) return; busy = true; controls(); message(['merge', 'organize'].includes(mode) ? 'Select files' : 'Select PDFs');
  try {
    const result = await invoke('choose_documents', { multiple: ['merge', 'compress', 'organize', 'compare'].includes(mode), allowImages: ['merge', 'organize'].includes(mode) });
    if (!result.documents.length && !result.rejected.length) { message('Selection cancelled.'); return; }
    current().importErrors = result.rejected; $('import-errors').open = !result.documents.length;
    if (result.documents.length) await acceptSelection(result.documents);
    showImportResult(result);
  } catch (error) {
    try { await cleanRegistry(); } catch { /* Keep the original import error visible. */ }
    message(String(error), 'error');
  } finally { busy = false; render(); }
  promptNextLocked();
}
async function importDrop(paths, position) {
  if ($('preview-dialog').open || $('password-dialog').open) { clearDropIndicator(); return; }
  if (busy) { message('Wait until processing finishes.'); return; }
  if (!ready) { message('PDF engine unavailable.', 'error'); return; }
  if (mode === 'split' && paths.length > 1) { message('This tool uses one PDF at a time.', 'error'); return; }
  const row = position ? document.elementFromPoint(position.x / window.devicePixelRatio, position.y / window.devicePixelRatio)?.closest('.document') : null;
  const insertionIndex = mode === 'organize' ? organizeDropTarget(position).index : mode === 'merge' && row ? Number(row.dataset.reorderIndex) : undefined;
  clearDropIndicator();
  busy = true; controls(); message(['merge', 'organize'].includes(mode) ? 'Adding files...' : 'Adding PDFs...');
  try {
    const result = await invoke('import_dropped_documents', { paths, allowImages: ['merge', 'organize'].includes(mode) });
    current().importErrors = result.rejected; $('import-errors').open = !result.documents.length;
    if (result.documents.length) await acceptSelection(result.documents, insertionIndex);
    showImportResult(result);
  } catch (error) {
    try { await cleanRegistry(); } catch { /* Keep the original import error visible. */ }
    message(String(error), 'error');
  } finally { busy = false; render(); }
  promptNextLocked();
}
const dropIndicator = document.createElement('div');
dropIndicator.className = 'file-drop-indicator'; dropIndicator.hidden = true;
dropIndicator.setAttribute('role', 'status'); dropIndicator.setAttribute('aria-live', 'polite'); document.body.append(dropIndicator);
let dropCard = null;
function organizeDropTarget(position) {
  if (!position) return { index: current().pages.length, card: null, after: true };
  const x = position.x / window.devicePixelRatio, y = position.y / window.devicePixelRatio;
  const grid = $('page-grid').getBoundingClientRect();
  if ($('organizer').hidden || x < grid.left || x > grid.right || y < grid.top || y > grid.bottom) return { index: current().pages.length, card: null, after: true };
  const cards = [...$('page-grid').children].map((element) => ({ element, index: Number(element.dataset.reorderIndex), rect: element.getBoundingClientRect() }));
  return insertionAtPoint(cards, x, y, current().pages.length);
}
function clearDropIndicator() {
  dropCard?.classList.remove('drag-target', 'drag-after'); dropCard = null;
  dropIndicator.hidden = true; $('content-scroll').classList.remove('file-drop-active');
}
function showDropIndicator(position) {
  clearDropIndicator();
  if (busy || !ready || $('preview-dialog').open || $('password-dialog').open) return;
  $('content-scroll').classList.add('file-drop-active');
  if (mode !== 'organize') return;
  if (position) {
    const scroll = $('content-scroll'); const rect = scroll.getBoundingClientRect();
    const y = position.y / window.devicePixelRatio;
    if (y > rect.bottom - 48 && y < rect.bottom) scroll.scrollTop += 16;
    else if (y < rect.top + 48 && y > rect.top) scroll.scrollTop -= 16;
  }
  const target = organizeDropTarget(position);
  dropCard = target.card?.element || null;
  dropCard?.classList.add('drag-target'); dropCard?.classList.toggle('drag-after', target.after);
  dropIndicator.textContent = target.index === current().pages.length ? 'Add pages at end' : `Insert before page ${target.index + 1}`;
  dropIndicator.hidden = false;
}
// Stop the browser's default file navigation even outside the document list.
for (const type of ['dragover', 'drop']) window.addEventListener(type, (event) => event.preventDefault(), true);
try {
  const unlisten = await getCurrentWebview().onDragDropEvent(({ payload }) => {
    if (payload.type === 'enter' || payload.type === 'over') showDropIndicator(payload.position);
    if (payload.type === 'leave') clearDropIndicator();
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
    host.classList.remove('invalid'); host.textContent = !validation() ? '' : view.measurementFailed ? 'Size unavailable' : 'Calculating output size...';
    return;
  }
  const saved = measured.inputBytes - measured.outputBytes;
  host.textContent = saved > 0 ? `Expected output: ${bytes(measured.outputBytes)}, saves ${bytes(saved)}` : `Expected output: ${bytes(measured.outputBytes)}, already optimized`;
  const limit = targetBytes(view);
  if (limit && measured.fileBytes.some((size) => size > limit)) host.textContent += ' — Over limit';
  host.classList.toggle('invalid', Boolean(limit && measured.fileBytes.some((size) => size > limit)));
  host.title = `${measured.outputBytes.toLocaleString()} bytes${measured.files > 1 ? ' total' : ''}`;
}
function scheduleCompression() {
  clearTimeout(compressionTimer);
  if (mode !== 'compress' || busy || !ready || !current().documents.length || !validation()) return;
  const view = current();
  const key = JSON.stringify([view.compression, view.targetSize, view.acknowledgeChanges, view.documents.map((doc) => doc.id)]);
  if (view.measurementKey === key) return;
  compressionTimer = setTimeout(() => measureCompression(key), 250);
}
async function measureCompression(key) {
  if (mode !== 'compress' || busy || !ready || !current().documents.length || !validation()) return;
  const view = current();
  if (key !== JSON.stringify([view.compression, view.targetSize, view.acknowledgeChanges, view.documents.map((doc) => doc.id)])) return;
  view.measurementKey = key; view.measurementFailed = false; view.compressionPreview = null;
  busy = true; cancelAvailable = false; jobId = crypto.randomUUID(); controls();
  const onProgress = new Channel();
  onProgress.onmessage = (progress) => { message(progress.message); cancelAvailable = progress.cancellable; controls(); };
  try {
    view.compressionPreview = await invoke('process_documents', {
      request: { jobId, operation: 'compress', ids: view.documents.map((doc) => doc.id), compression: view.compression, targetBytes: targetBytes(view), acknowledgeChanges: view.acknowledgeChanges, measureOnly: true }, onProgress,
    });
    message(view.compressionPreview.recovered ? 'Ready to save. PDF data recovered; review the saved copy.' : 'Ready to save');
  } catch (error) {
    view.measurementFailed = true;
    message(String(error), String(error).startsWith('Cancelled') ? '' : 'error');
  } finally {
    busy = false; jobId = null; cancelAvailable = false; render();
  }
}
async function save() {
  if (mode === 'compare') { await compareDocuments(); return; }
  if (busy || !ready || !validation() || (mode === 'compress' && !current().compressionPreview && !current().measurementFailed)) return;
  busy = true; cancelAvailable = false; const activeMode = mode; const view = current(); jobId = crypto.randomUUID();
  const onProgress = new Channel(); onProgress.onmessage = (progress) => { message(progress.message); cancelAvailable = progress.cancellable; controls(); };
  controls(); message(mode === 'split' || (mode === 'compress' && view.documents.length > 1) ? 'Select output folder' : 'Select save location');
  try {
    const splitRanges = mode === 'split' ? splitPlanFor(view).map(pageList).join(';') : view.ranges;
    const result = await invoke('process_documents', { request: { jobId, operation: mode, ids: outputDocuments(view).map((doc) => doc.id), compression: view.compression, previewId: view.compressionPreview?.id ?? null, splitMode: mode === 'split' ? (view.splitMode === 'file_size' ? 'file_size' : 'ranges') : 'every_page', pagesPerFile: Number(view.groupSize), ranges: splitRanges, pages: view.pages.map((page) => ({ documentId: page.documentId || '', page: page.page, rotation: page.rotation })), targetBytes: mode === 'compress' ? targetBytes(view) : null, maxOutputBytes: mode === 'split' && view.splitMode === 'file_size' ? splitSizeBytes(view) : null, outputNames: mode === 'split' ? splitOutputNames(view) : [], flattenForms: view.flattenForms, acknowledgeChanges: view.acknowledgeChanges, mergeBookmarks: view.mergeBookmarks }, onProgress });
    if (!result) { message('Save cancelled.'); return; } view.result = result;
    let text = activeMode !== 'split' && result.files === 1 ? `Saved ${result.name}. ${bytes(result.outputBytes)}.` : `Saved ${result.files} ${result.files === 1 ? 'PDF' : 'PDFs'} in ${result.name}.`;
    if (activeMode === 'compress') text += result.outputBytes < result.inputBytes ? ` ${Math.round((1 - result.outputBytes / result.inputBytes) * 100)}% smaller.` : result.recovered ? ' Saved a repaired copy.' : ' Already optimized. Saved an unchanged copy.';
    if (result.recovered) text += ' PDF data recovered; review the saved copy.';
    message(text, 'success');
  } catch (error) { message(String(error), String(error).startsWith('Cancelled') ? '' : 'error'); }
  finally { busy = false; jobId = null; cancelAvailable = false; renderContinuation(); controls(); }
}
function editPages(action) {
  if (busy || !current().selected.size || (action === 'remove' && current().selected.size === current().pages.length)) return;
  const view = current(); rememberEdit();
  if (action === 'rotate' || action === 'rotate-left') view.pages = view.pages.map((page) => view.selected.has(pageKey(page)) ? { ...page, rotation: (page.rotation + (action === 'rotate-left' ? 270 : 90)) % 360 } : page);
  if (action === 'remove') { view.pages = view.pages.filter((page) => !view.selected.has(pageKey(page))); view.selected.clear(); lastSelected = null; }
  if (action === 'earlier') for (let index = 1; index < view.pages.length; index += 1) { if (view.selected.has(pageKey(view.pages[index])) && !view.selected.has(pageKey(view.pages[index - 1]))) [view.pages[index - 1], view.pages[index]] = [view.pages[index], view.pages[index - 1]]; }
  if (action === 'later') for (let index = view.pages.length - 2; index >= 0; index -= 1) { if (view.selected.has(pageKey(view.pages[index])) && !view.selected.has(pageKey(view.pages[index + 1]))) [view.pages[index + 1], view.pages[index]] = [view.pages[index], view.pages[index + 1]]; }
  document.querySelector('.page-more')?.removeAttribute('open');
  render(); message('Unsaved changes');
}
function duplicateSelected() {
  const view = current();
  if (busy || !view.selected.size) return;
  if (view.pages.length + view.selected.size > 20000) { message('Keep at most 20,000 pages.', 'error'); return; }
  rememberEdit();
  const copies = [];
  view.pages = view.pages.flatMap((page) => {
    if (!view.selected.has(pageKey(page))) return [page];
    const copy = { ...page, key: --duplicateKey }; copies.push(copy);
    return [page, copy];
  });
  view.selected = new Set(copies.map(pageKey)); lastSelected = null;
  render(); message('Unsaved changes');
}
function openDocumentPreview(doc, pageNumber = null) {
  if (busy || doc.locked || $('preview-dialog').open) return;
  const edited = mode === 'organize' && current().pages.length > 0;
  previewSession = {
    doc, pages: edited ? current().pages.map((page) => ({ ...page })) : null,
    index: edited && pageNumber !== null ? Math.max(0, current().pages.findIndex((page) => page.page === pageNumber)) : edited ? Math.max(0, current().pages.findIndex((page) => page.documentId === doc.id)) : (pageNumber ?? 1) - 1,
    count: edited ? current().pages.length : doc.pages,
    compare: null,
  };
  $('preview-title').textContent = doc.name;
  $('preview-zoom').value = '1';
  $('preview-dialog').showModal();
  renderPreview();
}
function openCompare() {
  const view = state.compress;
  if (busy || !view.compressionPreview || view.documents.length !== 1 || $('preview-dialog').open) return;
  const doc = view.documents[0];
  previewSession = { doc, pages: null, index: 0, count: doc.pages, compare: { previewId: view.compressionPreview.id, outputIndex: 0, bytes: view.compressionPreview.fileBytes[0] } };
  $('preview-title').textContent = 'Compare';
  $('preview-zoom').value = '1';
  $('preview-dialog').showModal();
  renderPreview();
}
function openPreview(page) {
  const doc = current().documents.find((item) => item.id === page.documentId) || current().documents[0];
  openDocumentPreview(doc, page.page);
  if (mode === 'organize' && previewSession) { previewSession.index = current().pages.findIndex((item) => pageKey(item) === pageKey(page)); renderPreview(); }
}
async function renderPreview() {
  if (!previewSession || !$('preview-dialog').open) return;
  const sequence = ++previewRenderSequence;
  const { doc, pages, index, count, compare } = previewSession;
  $('compressed-pane').hidden = !compare; $('original-label').hidden = !compare; $('preview-panes').classList.toggle('comparing', Boolean(compare));
  const page = pages ? pages[index] : { page: index + 1, rotation: 0 };
  const source = pages ? state.organize.documents.find((item) => item.id === page.documentId) || doc : doc;
  if (pages) $('preview-title').textContent = source.name;
  $('preview-page').value = String(index + 1); $('preview-page').max = String(count);
  $('preview-total').textContent = `of ${count}`;
  $('preview-previous').disabled = index === 0; $('preview-next').disabled = index === count - 1;
  const zoom = Number($('preview-zoom').value);
  const host = $('large-preview'); host.textContent = 'Preparing preview...'; host.scrollTop = 0; host.scrollLeft = 0;
  const width = Math.max(1, host.clientWidth - 44) * zoom;
  const height = Math.max(1, host.clientHeight - 44) * zoom;
  await previews.draw({ host, id: source.id, page: page.page, rotation: page.rotation, width, height, bytes: source.previewBytes });
  if (sequence === previewRenderSequence && compare && previewSession && previewSession.compare === compare && $('preview-dialog').open) {
    const compressedHost = $('compressed-preview'); compressedHost.textContent = 'Preparing preview...'; compressedHost.scrollTop = 0; compressedHost.scrollLeft = 0;
    const compressedWidth = Math.max(1, compressedHost.clientWidth - 44) * zoom;
    const compressedHeight = Math.max(1, compressedHost.clientHeight - 44) * zoom;
    await previews.draw({ host: compressedHost, id: doc.id, previewId: compare.previewId, outputIndex: compare.outputIndex, page: page.page, rotation: page.rotation, width: compressedWidth, height: compressedHeight, bytes: compare.bytes });
  }
}
function resizePreview(width, height) {
  const dialog = $('preview-dialog');
  dialog.style.width = `${Math.max(Math.min(480, innerWidth - 32), Math.min(width, innerWidth - 32))}px`;
  dialog.style.height = `${Math.max(Math.min(420, innerHeight - 32), Math.min(height, innerHeight - 32))}px`;
}
$('preview-resize').addEventListener('pointerdown', (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  const grip = event.currentTarget;
  const rect = $('preview-dialog').getBoundingClientRect();
  const startX = event.clientX, startY = event.clientY;
  grip.setPointerCapture(event.pointerId);
  const move = (next) => resizePreview(rect.width + (next.clientX - startX) * 2, rect.height + (next.clientY - startY) * 2);
  const stop = () => { grip.removeEventListener('pointermove', move); grip.removeEventListener('pointerup', stop); grip.removeEventListener('pointercancel', stop); };
  grip.addEventListener('pointermove', move);
  grip.addEventListener('pointerup', stop);
  grip.addEventListener('pointercancel', stop);
});
$('preview-resize').addEventListener('keydown', (event) => {
  const directions = { ArrowLeft: [-20, 0], ArrowRight: [20, 0], ArrowUp: [0, -20], ArrowDown: [0, 20] };
  if (!directions[event.key]) return;
  event.preventDefault();
  const rect = $('preview-dialog').getBoundingClientRect();
  const [width, height] = directions[event.key];
  resizePreview(rect.width + width, rect.height + height);
});
let previewResizeTimer = null;
let previewSize = '';
new ResizeObserver(([entry]) => {
  const size = `${Math.round(entry.contentRect.width)}:${Math.round(entry.contentRect.height)}`;
  if (size === previewSize) return;
  previewSize = size;
  if (!previewSession || !$('preview-dialog').open) return;
  clearTimeout(previewResizeTimer);
  previewResizeTimer = setTimeout(() => { previewResizeTimer = null; renderPreview(); }, 120);
}).observe($('preview-dialog'));
function movePreview(offset) {
  if (!previewSession) return;
  previewSession.index = Math.max(0, Math.min(previewSession.count - 1, previewSession.index + offset));
  renderPreview();
}
function inspectorPages() {
  const view = current();
  const documents = mode === 'compare' ? view.documents.filter((doc) => doc.id === (inspectorPage?.documentId || view.documents[0]?.id)) : view.documents;
  return mode === 'organize' ? view.pages : documents.filter((doc) => !doc.locked).flatMap((doc) => Array.from({ length: doc.pages }, (_, index) => ({ documentId: doc.id, page: index + 1, rotation: 0 })));
}
function setInspector(open) {
  $('inspector').hidden = !open; $('editor-body').classList.toggle('inspecting', open);
  $('toggle-inspector').setAttribute('aria-pressed', String(open));
  $('toggle-inspector').setAttribute('aria-label', open ? 'Hide preview' : 'Show preview');
  $('toggle-inspector').title = open ? 'Hide preview' : 'Show preview';
  if (!open) { inspectorSequence++; $('inspector-canvas').previewTicket = null; $('inspector-canvas').replaceChildren(); }
  else renderInspector();
}
function inspectPage(page) { inspectorPage = page; setInspector(true); }
async function renderInspector() {
  if ($('inspector').hidden) return;
  const pages = inspectorPages();
  if (!pages.length) { setInspector(false); return; }
  let index = inspectorPage ? pages.findIndex((page) => mode === 'organize' ? pageKey(page) === pageKey(inspectorPage) : page.documentId === inspectorPage.documentId && page.page === inspectorPage.page) : -1;
  if (index < 0) index = Math.min(inspectorIndex, pages.length - 1);
  inspectorIndex = index; inspectorPage = { ...pages[index] };
  const page = pages[index], doc = current().documents.find((item) => item.id === page.documentId) || current().documents[0];
  const host = $('inspector-canvas'), sequence = ++inspectorSequence;
  $('inspector-title').textContent = mode === 'organize' ? `Output page ${index + 1}` : `Page ${page.page}`;
  $('inspector-source').textContent = `${doc.name}, page ${page.page}`;
  $('inspector-position').textContent = `${index + 1} of ${pages.length}`;
  $('inspector-previous').disabled = index === 0; $('inspector-next').disabled = index === pages.length - 1;
  const spec = `${doc.id}:${page.page}:${page.rotation}:${host.clientWidth}:${host.clientHeight}`;
  if (host.dataset.spec === spec && host.querySelector('canvas')) return;
  host.dataset.spec = spec;
  await previews.draw({ host, id: doc.id, bytes: doc.previewBytes, page: page.page, rotation: page.rotation,
    width: Math.max(1, host.clientWidth - 32), height: Math.max(1, host.clientHeight - 32) });
  if (sequence !== inspectorSequence) return;
}
function renderContinuation() {
  const view = current(), select = $('continue-tool'), previous = select.value;
  const choices = view.result ? ['compress', 'organize', 'split', 'merge'].filter((tool) => tool !== mode && (tool !== 'split' || view.result.files === 1)) : [];
  select.replaceChildren(...choices.map((tool) => { const option = document.createElement('option'); option.value = tool; option.textContent = state[tool].documents.length && tool !== 'split' ? `Add to ${tool === 'organize' ? 'Organize' : tool === 'compress' ? 'Compress' : 'Merge'}` : tools[tool].heading; return option; }));
  if (choices.includes(previous)) select.value = previous;
  $('continue-output').hidden = busy || !choices.length;
}
async function continueOutput() {
  if (busy || !current().result) return;
  const next = $('continue-tool').value, id = current().result.id;
  busy = true; controls();
  try {
    const result = await invoke('import_result_documents', { resultId: id });
    if (!result.documents.length) throw new Error(result.rejected[0]?.reason || 'The saved output is unavailable.');
    busy = false; setMode(next); busy = true;
    await acceptSelection(result.documents); current().importErrors = result.rejected; showImportResult(result);
  } catch (error) { message(String(error), 'error'); }
  finally { busy = false; render(); }
}
function persistCompression() {
  preferences.compression = state.compress.compression;
  preferences.target = state.compress.targetSize;
  savePreferences(updateStorage, preferences);
}
function renderLimitPresets() {
  const select = $('limit-preset');
  const empty = document.createElement('option'); empty.value = ''; empty.textContent = 'Upload limits';
  select.replaceChildren(empty, ...preferences.presets.map((preset, index) => {
    const option = document.createElement('option'); option.value = String(index); option.textContent = `${preset.name} (${preset.mb} MB)`; return option;
  }));
  const match = preferences.presets.findIndex((preset) => preset.mb === Number(state.compress.targetSize));
  select.value = match < 0 ? '' : String(match); $('delete-limit').hidden = match < 0;
}
function clearComparison() {
  state.compare.comparison = null; $('compare-changes').replaceChildren(); $('compare-summary').textContent = ''; $('compare-warning').hidden = true;
}
function renderCompareSlots() {
  const view = state.compare;
  $('compare-slots').replaceChildren(...['Original PDF', 'Revised PDF'].map((label, index) => {
    const doc = view.documents.find((item) => (item.compareRole ?? view.documents.indexOf(item)) === index), slot = document.createElement('section'); slot.className = 'compare-slot';
    const heading = document.createElement('h3'); heading.textContent = label;
    const name = document.createElement('p'); name.className = 'file-name'; name.textContent = doc?.name || 'No PDF selected';
    const actions = document.createElement('div'); actions.className = 'file-actions';
    const choose = document.createElement('button'); choose.className = 'secondary'; choose.dataset.slot = String(index); choose.textContent = doc ? 'Replace' : 'Choose PDF'; choose.addEventListener('click', () => chooseCompareSlot(index)); actions.append(choose);
    if (doc) {
      const button = document.createElement('button'); button.className = 'secondary'; button.textContent = doc.locked ? 'Unlock' : 'Preview';
      button.addEventListener('click', () => doc.locked ? promptPassword(doc) : openDocumentPreview(doc)); actions.append(button);
    }
    slot.append(heading, name, actions); return slot;
  }));
}
async function chooseCompareSlot(index) {
  if (busy) return; busy = true; controls();
  try {
    const result = await invoke('choose_documents', { multiple: false, allowImages: false });
    if (result.documents.length) {
      const doc = { ...result.documents[0], compareRole: index };
      state.compare.documents = [...state.compare.documents.filter((item) => item.compareRole !== index), doc].sort((a,b) => a.compareRole - b.compareRole);
      clearComparison(); await cleanRegistry();
    }
    state.compare.importErrors = result.rejected;
  } catch (error) { message(String(error), 'error'); }
  finally { busy = false; render(); promptNextLocked(); }
}
async function compareDocuments() {
  if (busy || !validation()) return;
  busy = true; cancelAvailable = true; clearComparison(); controls();
  let cancelled = false, worker, rejectWorker;
  localCancel = () => { cancelled = true; worker?.terminate(); rejectWorker?.(new Error('Comparison cancelled.')); message('Cancelling comparison...'); };
  try {
    const documents = state.compare.documents, texts = [];
    for (let index = 0; index < 2; index++) {
      const doc = documents[index];
      texts.push(await previews.extractText({ id: doc.id, bytes: doc.previewBytes }, (page, total) => message(`Reading ${index === 0 ? 'original' : 'revised'}: page ${page} of ${total}`), () => cancelled));
    }
    if (cancelled) throw new Error('Comparison cancelled.');
    if (texts.some((pages) => !pages.some((page) => page.trim()))) throw new Error('A PDF has no readable page text. Scans need OCR before comparison.');
    message('Comparing page text...');
    worker = new Worker(new URL('./compare-worker.js', import.meta.url), { type: 'module' });
    const result = await new Promise((resolve, reject) => {
      rejectWorker = reject;
      worker.onmessage = ({ data }) => data.error ? reject(new Error(data.error)) : resolve(data.result);
      worker.onerror = () => reject(new Error('These documents could not be compared.'));
      worker.postMessage({ original: texts[0], revised: texts[1] });
    });
    const missing = texts.map((pages) => pages.filter((page) => !page.trim()).length);
    state.compare.comparison = result;
    $('compare-summary').textContent = result.changes.length ? `${result.changes.length} changed ${result.changes.length === 1 ? 'passage' : 'passages'}` : 'No wording changes found';
    const warnings = [];
    if (missing.some(Boolean)) warnings.push(`${missing[0]} original and ${missing[1]} revised pages have no readable text.`);
    if (result.coarse) warnings.push('Large differences are grouped together.');
    $('compare-warning').textContent = warnings.join(' '); $('compare-warning').hidden = !warnings.length;
    renderComparisonChanges(); message('Comparison ready');
  } catch (error) { message(error.message, cancelled ? '' : 'error'); }
  finally { worker?.terminate(); localCancel = null; busy = false; cancelAvailable = false; controls(); }
}
function renderComparisonChanges(limit = 30) {
  const changes = state.compare.comparison?.changes || [], list = $('compare-changes'); list.replaceChildren();
  for (const [index, change] of changes.slice(0, limit).entries()) {
    const row = document.createElement('li');
    const context = document.createElement('p'); context.className = 'change-context'; context.textContent = change.before;
    const columns = document.createElement('div'); columns.className = 'compare-columns';
    for (const [side, label, page, tokens] of [[0, 'Original', change.oldPage, change.removed], [1, 'Revised', change.newPage, change.added]]) {
      const section = document.createElement('section');
      const jump = document.createElement('button'); jump.className = 'text-button'; jump.textContent = `${label}, page ${page}`;
      jump.addEventListener('click', () => inspectPage({ documentId: state.compare.documents[side].id, page, rotation: 0 }));
      const text = document.createElement(side === 0 ? 'del' : 'ins'); const full = tokens.join(' '); text.textContent = full.slice(0, 4000) || (side === 0 ? 'Nothing removed' : 'Nothing added');
      section.append(jump, text);
      if (full.length > 4000) { const more = document.createElement('button'); more.className = 'text-button'; more.textContent = 'Show full passage'; more.addEventListener('click', () => { text.textContent = full; more.remove(); }); section.append(more); }
      columns.append(section);
    }
    row.setAttribute('aria-label', `Change ${index + 1}`); row.append(context, columns); list.append(row);
  }
  if (changes.length > limit) { const more = document.createElement('button'); more.className = 'secondary'; more.textContent = 'Show more changes'; more.addEventListener('click', () => renderComparisonChanges(limit + 30)); const row = document.createElement('li'); row.append(more); list.append(row); }
}
async function checkPages() {
  const view = current();
  if (busy || mode !== 'organize') return;
  document.querySelector('.page-more').open = false;
  if (view.pages.length > 500) { message('Check up to 500 pages at a time. Split a section first.', 'error'); return; }
  busy = true; cancelAvailable = true; let cancelled = false; localCancel = () => { cancelled = true; };
  controls(); const findings = [], seen = new Map();
  try {
    for (const [index, page] of view.pages.entries()) {
      if (cancelled) throw new Error('Page check cancelled.');
      message(`Checking page ${index + 1} of ${view.pages.length}`);
      const doc = view.documents.find((item) => item.id === page.documentId);
      const result = await previews.reviewPage({ id: doc.id, bytes: doc.previewBytes, page: page.page, rotation: page.rotation });
      if (result.blank) findings.push({ key: pageKey(page), label: `Page ${index + 1}: possibly blank` });
      else if (seen.has(result.hash)) findings.push({ key: pageKey(page), label: `Page ${index + 1}: possible duplicate of page ${seen.get(result.hash)}` });
      else seen.set(result.hash, index + 1);
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    if (cancelled) throw new Error('Page check cancelled.');
    view.review = findings; renderPageReview(); message(findings.length ? 'Review flagged pages before removing anything.' : 'No possible blank or duplicate pages found.');
  } catch (error) { message(error.message, cancelled ? '' : 'error'); }
  finally { busy = false; localCancel = null; cancelAvailable = false; controls(); }
}
function renderPageReview() {
  const host = $('page-review'), review = mode === 'organize' ? current().review : null;
  host.hidden = !review?.length; host.replaceChildren();
  if (!review?.length) return;
  const details = document.createElement('details'), summary = document.createElement('summary'); summary.textContent = `${review.length} pages to review`; details.append(summary);
  for (const finding of review) { const button = document.createElement('button'); button.className = 'text-button'; button.textContent = finding.label; button.addEventListener('click', () => { const page = current().pages.find((item) => pageKey(item) === finding.key); if (page) inspectPage(page); }); details.append(button); }
  host.append(details);
}

async function checkEngine() {
  if (busy) return; busy = true; controls(); $('retry-engine').hidden = true;
  try { await invoke('engine_status'); ready = true; message('Ready'); }
  catch (error) { ready = false; message(String(error), 'error'); $('retry-engine').hidden = false; }
  finally { busy = false; render(); }
}
function setMode(next) {
  if (busy || next === mode) return; inspectorPage = null; inspectorIndex = 0; mode = next; lastSelected = null; const tool = tools[mode];
  document.title = tool.heading; invoke('set_view_title', { tool: mode }).catch(() => {}); $('heading').textContent = tool.heading; $('save-label').textContent = tool.action;
  $('choose-label').textContent = ['merge', 'organize'].includes(mode) ? 'Choose files' : mode === 'compress' ? 'Choose PDFs' : 'Choose a PDF';
  $('list-label').textContent = ['merge', 'compress', 'organize'].includes(mode) ? 'Documents' : 'Document';
  $('split-size').value = current().splitSize; $('split-mode').value = current().splitMode; $('group-size').value = current().groupSize; $('split-ranges').value = current().ranges; $('split-cuts').value = current().splitCuts; $('split-separate').checked = current().splitSeparate; $('split-combine').checked = current().splitCombine;
  $('target-size').value = current().targetSize;
  for (const input of document.querySelectorAll('input[name="compression"]')) input.checked = input.value === current().compression;
  for (const button of document.querySelectorAll('.tool')) { button.classList.toggle('active', button.dataset.mode === mode); button.setAttribute('aria-pressed', String(button.dataset.mode === mode)); }
  $('content-scroll').scrollTop = 0; message(ready ? 'Ready' : 'The local PDF engine is unavailable.', ready ? '' : 'error'); render();
}
for (const button of document.querySelectorAll('.tool')) button.addEventListener('click', () => setMode(button.dataset.mode));
$('add').addEventListener('click', choose); $('choose').addEventListener('click', choose); $('save').addEventListener('click', save);
$('clear').addEventListener('click', async () => {
  if (busy) return; busy = true; invalidateCompression(); current().importErrors = []; current().documents = []; current().pages = []; current().history = []; current().selected.clear(); current().result = null; current().comparison = null; current().review = null; $('compare-changes').replaceChildren(); $('compare-summary').textContent = ''; resetEdits(current()); render();
  try { await cleanRegistry(); message('Documents cleared.'); }
  catch (error) { message(String(error), 'error'); }
  finally { busy = false; controls(); }
});
$('password-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (!pendingUnlock || busy) return;
  const doc = pendingUnlock;
  busy = true; controls();
  let info;
  try {
    const password = $('pdf-password').value; $('pdf-password').value = '';
    info = await invoke('unlock_document', { id: doc.id, password });
  } catch (error) {
    $('password-error').textContent = String(error); $('password-error').hidden = false;
    busy = false; controls(); return;
  }
  for (const [tool, view] of Object.entries(state)) {
    if (tool === 'compare') { view.documents = view.documents.map((doc) => doc.id === info.id ? { ...info, compareRole: doc.compareRole } : doc); clearComparison(); continue; }
    const index = view.documents.findIndex((item) => item.id === info.id);
    if (index === -1) continue;
    const previous = view.documents[index];
    view.documents[index] = tool === 'compare' ? { ...info, compareRole: previous.compareRole } : info;
    if (tool === 'organize') {
      const wasEmpty = !view.pages.length;
      view.history.push(view.pages.map((page) => ({ ...page }))); view.future = [];
      const added = pagesFromDocuments([info], () => --duplicateKey);
      const position = Math.min(previous.insertionIndex ?? view.pages.length, view.pages.length);
      view.pages = insertPages(view.pages, added, position);
      for (const pending of view.documents) if (pending.locked && pending.insertionIndex >= position) pending.insertionIndex += added.length;
      view.selected = wasEmpty ? new Set() : new Set(added.map(pageKey)); view.result = null;
    } else if (tool === 'split') {
      view.pages = Array.from({ length: info.pages }, (_, page) => ({ page: page + 1, rotation: 0 }));
      view.selected.clear(); view.history = []; view.future = []; view.pageWindow = 0; view.result = null;
    }
  }
  await previews.forget(info.id);
  $('password-dialog').close();
  message('PDF unlocked.');
  busy = false; render();
  promptNextLocked();
});
$('password-skip').addEventListener('click', () => $('password-dialog').close());
$('password-dialog').addEventListener('close', () => { pendingUnlock = null; $('pdf-password').value = ''; });
$('password-dialog').addEventListener('cancel', (event) => { if (busy) event.preventDefault(); });
async function fetchUpdate() {
  const update = await invoke('check_app_update');
  pendingUpdate = update; availableUpdateVersion = update?.version || null; updateChecks.remember(availableUpdateVersion); controls(); return update;
}
async function automaticUpdateCheck(installedVersion) {
  availableUpdateVersion = updateChecks.available(installedVersion); controls();
  if (!updateChecks.claim()) return;
  updateChecking = true; controls();
  try { await fetchUpdate(); } catch { /* Offline launches remain quiet. Try again manually or on a later launch. */ }
  finally { updateChecking = false; controls(); }
}
$('app-update').addEventListener('click', async () => {
  if (busy || updateChecking) return;
  clearTimeout(compressionTimer); busy = true; cancelAvailable = false; controls();
  try {
    if (availableUpdateVersion) {
      if (!pendingUpdate) await fetchUpdate();
      if (!pendingUpdate) { message('You have the latest version.'); return; }
      const approved = await invoke('confirm_app_update', { version: pendingUpdate.version });
      if (!approved) return;
      const onProgress = new Channel(); onProgress.onmessage = (progress) => message(progress.message);
      await invoke('install_app_update', { updateId: pendingUpdate.id, onProgress });
    } else {
      message('Checking for updates...');
      await fetchUpdate();
      message(availableUpdateVersion ? `Version ${availableUpdateVersion} is available.` : 'You have the latest version.');
    }
  } catch (error) {
    pendingUpdate = null; message(String(error), String(error).startsWith('No update') ? '' : 'error');
  } finally { busy = false; controls(); scheduleCompression(); }
});
$('release-notes').addEventListener('click', async () => {
  if (busy) return;
  try { await invoke('open_release_notes'); } catch (error) { message(String(error), 'error'); }
});
$('retry-engine').addEventListener('click', checkEngine);
$('cancel').addEventListener('click', async () => {
  if (localCancel) { localCancel(); return; }
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
for (const input of document.querySelectorAll('input[name="compression"]')) input.addEventListener('change', () => { invalidateCompression(); current().compression = input.value; current().result = null; render(); });
$('target-size').addEventListener('input', () => { invalidateCompression(); current().targetSize = $('target-size').value; current().result = null; render(); });
$('compare-output').addEventListener('click', openCompare);
$('flatten-forms').addEventListener('change', () => { current().flattenForms = $('flatten-forms').checked; current().result = null; controls(); });
$('acknowledge-changes').addEventListener('change', () => { invalidateCompression(); current().acknowledgeChanges = $('acknowledge-changes').checked; current().result = null; render(); });
$('split-mode').addEventListener('change', () => { current().splitMode = $('split-mode').value; current().result = null; lastSelected = null; render(); });
$('group-size').addEventListener('input', () => { current().groupSize = $('group-size').value; current().result = null; splitSummary(); });
$('split-size').addEventListener('input', () => { current().splitSize = $('split-size').value; current().result = null; splitSummary(); });
$('split-ranges').addEventListener('input', () => { current().ranges = $('split-ranges').value; current().result = null; readSplitSelection(); splitSummary(); updateSelection(); });
$('split-cuts').addEventListener('input', () => { current().splitCuts = $('split-cuts').value; current().result = null; readSplitSelection(); splitSummary(); updateSelection(); });
$('split-separate').addEventListener('change', () => { current().splitSeparate = $('split-separate').checked; current().result = null; splitSummary(); });
$('split-combine').addEventListener('change', () => { current().splitCombine = $('split-combine').checked; current().result = null; splitSummary(); });
$('add-split-group').addEventListener('click', () => { if (busy || current().splitGroups.length >= 500) return; const view = current(); view.splitGroups.push(''); view.splitNames.push(''); view.activeGroup = view.splitGroups.length - 1; view.result = null; lastSelected = null; render(); $(`split-group-${view.activeGroup}`).focus(); });
$('invert-selection').addEventListener('click', () => { if (busy) return; const view = current(); view.selected = new Set(view.pages.filter((page) => !view.selected.has(pageKey(page))).map(pageKey)); lastSelected = null; writeSplitSelection(); updateSelection(); });
$('select-all').addEventListener('click', () => { if (busy) return; const view = current(); view.selected = view.selected.size === view.pages.length ? new Set() : new Set(view.pages.map(pageKey)); lastSelected = null; if (mode === 'split') writeSplitSelection(); updateSelection(); });
$('rotate-left').addEventListener('click', () => editPages('rotate-left'));
$('duplicate-pages').addEventListener('click', duplicateSelected);
$('move-pages-to').addEventListener('click', () => {
  if (busy || !current().selected.size) return;
  try { const pages = movePagesTo(current().pages, current().selected, Number($('move-page-position').value)); rememberEdit(); current().pages = pages; current().pageWindow = Math.floor((Number($('move-page-position').value) - 1) / PAGE_WINDOW); document.querySelector('.page-more')?.removeAttribute('open'); render(); $('page-grid').querySelector('.selected')?.focus(); message('Page order updated.'); } catch (error) { message(error.message, 'error'); }
});
$('rotate').addEventListener('click', () => editPages('rotate')); $('delete-pages').addEventListener('click', () => editPages('remove'));
$('move-left').addEventListener('click', () => editPages('earlier')); $('move-right').addEventListener('click', () => editPages('later'));
$('undo').addEventListener('click', () => { const view = current(); if (busy || !view.history.length) return; view.review = null; view.future.push(view.pages.map((page) => ({ ...page }))); view.pages = view.history.pop(); view.selected.clear(); lastSelected = null; view.result = null; render(); message('Page edit undone.'); });
$('redo').addEventListener('click', () => { const view = current(); if (busy || !view.future.length) return; view.review = null; view.history.push(view.pages.map((page) => ({ ...page }))); view.pages = view.future.pop(); view.selected.clear(); lastSelected = null; view.result = null; render(); message('Page edit redone.'); });
$('previous-pages').addEventListener('click', () => { if (busy || !current().pageWindow) return; current().pageWindow -= 1; render(); });
$('next-pages').addEventListener('click', () => { if (busy || (current().pageWindow + 1) * PAGE_WINDOW >= current().pages.length) return; current().pageWindow += 1; render(); });
$('jump-page').addEventListener('change', () => {
  const view = current(); const value = Number($('jump-page').value);
  if (!busy && Number.isInteger(value) && value >= 1 && value <= view.pages.length) {
    view.pageWindow = Math.floor((value - 1) / PAGE_WINDOW); render();
    $('page-grid').children[(value - 1) % PAGE_WINDOW]?.focus();
  }
  $('jump-page').value = '';
});
$('thumbnail-size').addEventListener('input', () => { $('page-grid').style.setProperty('--thumb', `${$('thumbnail-size').value}px`); preferences.thumbnail = Number($('thumbnail-size').value); savePreferences(updateStorage, preferences); });
$('preview-previous').addEventListener('click', () => movePreview(-1));
$('preview-next').addEventListener('click', () => movePreview(1));
$('preview-page').addEventListener('change', () => {
  if (!previewSession) return;
  const value = Number($('preview-page').value);
  if (Number.isInteger(value) && value >= 1 && value <= previewSession.count) previewSession.index = value - 1;
  renderPreview();
});
$('preview-zoom').addEventListener('change', renderPreview);
$('preview-dialog').addEventListener('keydown', (event) => {
  if (event.target.closest('input, select') || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
  event.preventDefault(); movePreview(event.key === 'ArrowLeft' ? -1 : 1);
});
$('close-preview').addEventListener('click', () => $('preview-dialog').close()); $('preview-dialog').addEventListener('close', () => {
  previewSession = null;
  for (const host of [$('large-preview'), $('compressed-preview')]) { host.previewTicket = null; host.replaceChildren(); }
  $('compressed-pane').hidden = true; $('original-label').hidden = true;
});
document.addEventListener('keydown', (event) => {
  const editing = ['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName);
  if (!busy && mode === 'organize' && !$('preview-dialog').open && !$('password-dialog').open && !editing && event.target.closest('#organizer')) {
    if (['Backspace', 'Delete'].includes(event.key)) { event.preventDefault(); if (!$('delete-pages').disabled) editPages('remove'); return; }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a') { event.preventDefault(); current().selected = new Set(current().pages.map(pageKey)); updateSelection(); return; }
  }
  if (!(event.metaKey || event.ctrlKey) || busy || $('preview-dialog').open || $('password-dialog').open) return;
  if (event.key.toLowerCase() === 'o') { event.preventDefault(); choose(); }
  if (event.key.toLowerCase() === 's') { event.preventDefault(); save(); }
  if (event.key.toLowerCase() === 'z' && mode === 'organize' && !['INPUT', 'TEXTAREA'].includes(event.target.tagName)) { event.preventDefault(); (event.shiftKey ? $('redo') : $('undo')).click(); }
});
$('toggle-inspector').addEventListener('click', () => setInspector($('inspector').hidden));
$('close-inspector').addEventListener('click', () => setInspector(false));
for (const [id, offset] of [['inspector-previous', -1], ['inspector-next', 1]]) $(id).addEventListener('click', () => {
  const pages = inspectorPages(); inspectorIndex = Math.max(0, Math.min(pages.length - 1, inspectorIndex + offset)); inspectorPage = pages[inspectorIndex]; renderInspector();
});
$('expand-inspector').addEventListener('click', () => { if (inspectorPage) { const doc = current().documents.find((item) => item.id === inspectorPage.documentId) || current().documents[0]; openDocumentPreview(doc, inspectorPage.page); if (mode === 'organize' && previewSession) { previewSession.index = inspectorIndex; renderPreview(); } } });
let inspectorResizeTimer;
new ResizeObserver(() => { clearTimeout(inspectorResizeTimer); inspectorResizeTimer = setTimeout(renderInspector, 100); }).observe($('inspector-canvas'));
$('continue-button').addEventListener('click', continueOutput);
$('merge-bookmarks').addEventListener('change', () => { current().mergeBookmarks = $('merge-bookmarks').checked; current().result = null; controls(); });
$('check-pages').addEventListener('click', checkPages);
$('swap-documents').addEventListener('click', () => { if (busy) return; for (const doc of state.compare.documents) doc.compareRole = 1 - doc.compareRole; state.compare.documents.sort((a,b) => a.compareRole - b.compareRole); clearComparison(); render(); });
$('limit-preset').addEventListener('change', () => {
  const preset = preferences.presets[Number($('limit-preset').value)];
  if ($('limit-preset').value === '' || !preset) return;
  invalidateCompression(); state.compress.targetSize = String(preset.mb); state.compress.result = null; $('target-size').value = state.compress.targetSize;
  persistCompression(); renderLimitPresets(); render();
});
$('save-limit').addEventListener('click', () => { if (busy || !targetBytes(state.compress)) return; $('limit-name').value = ''; $('limit-dialog').showModal(); $('limit-name').focus(); });
$('cancel-limit').addEventListener('click', () => $('limit-dialog').close());
$('limit-form').addEventListener('submit', (event) => {
  event.preventDefault(); const name = $('limit-name').value.trim();
  if (!name || !targetBytes(state.compress)) return;
  const existing = preferences.presets.findIndex((preset) => preset.name.toLowerCase() === name.toLowerCase());
  const preset = { name, mb: Number(state.compress.targetSize) };
  if (existing >= 0) preferences.presets[existing] = preset; else if (preferences.presets.length < 12) preferences.presets.push(preset);
  savePreferences(updateStorage, preferences); renderLimitPresets(); $('limit-dialog').close(); controls();
});
$('delete-limit').addEventListener('click', () => { if (busy || $('limit-preset').value === '') return; preferences.presets.splice(Number($('limit-preset').value), 1); savePreferences(updateStorage, preferences); renderLimitPresets(); controls(); });
for (const input of document.querySelectorAll('input[name="compression"]')) input.addEventListener('change', persistCompression);
$('target-size').addEventListener('input', () => { persistCompression(); renderLimitPresets(); });
// Keep corresponding locations aligned while reviewing image compression.
let syncingPreview = false;
for (const [sourceId, targetId] of [['large-preview', 'compressed-preview'], ['compressed-preview', 'large-preview']]) $(sourceId).addEventListener('scroll', () => {
  if (!previewSession?.compare || syncingPreview) return;
  syncingPreview = true; const source = $(sourceId), target = $(targetId);
  target.scrollTop = source.scrollTop / Math.max(1, source.scrollHeight - source.clientHeight) * (target.scrollHeight - target.clientHeight);
  target.scrollLeft = source.scrollLeft / Math.max(1, source.scrollWidth - source.clientWidth) * (target.scrollWidth - target.clientWidth);
  requestAnimationFrame(() => { syncingPreview = false; });
});

$('thumbnail-size').value = String(preferences.thumbnail); $('target-size').value = state.compress.targetSize;
for (const input of document.querySelectorAll('input[name="compression"]')) input.checked = input.value === state.compress.compression;
$('page-grid').style.setProperty('--thumb', `${preferences.thumbnail}px`);
renderLimitPresets();
const installedVersion = getVersion().then((version) => { $('app-version').textContent = `Version ${version}`; $('app-version').hidden = false; return version; }).catch(() => null);
render();
Promise.all([checkEngine(), installedVersion]).then(([, version]) => { if (version) automaticUpdateCheck(version); });
