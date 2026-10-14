import { invoke } from '@tauri-apps/api/core';
import { getDocument, GlobalWorkerOptions } from 'pdfjs-dist/legacy/build/pdf.mjs';
import workerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url';
GlobalWorkerOptions.workerSrc = workerUrl;

export class Previews {
  constructor() {
    this.cache = new Map(); this.queue = []; this.active = 0; this.generation = 0; this.renders = new Set(); this.activeIds = new Map();
    this.observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        this.observer.unobserve(entry.target);
        this.queue.push({ ...entry.target.previewSpec, host: entry.target, generation: this.generation });
      }
      this.pump();
    }, { root: document.getElementById('content-scroll'), rootMargin: '100px' });
  }
  resetView() {
    this.generation += 1; this.observer.disconnect(); this.queue = [];
    for (const task of this.renders) task.cancel();
    this.renders.clear();
  }
  async forget(id) {
    const entry = this.cache.get(id); this.cache.delete(id);
    if (entry) {
      try { await entry.promise; } catch { /* Failed loading tasks can still be destroyed. */ }
      await entry.task?.destroy();
    }
  }
  watch(host, id, page = 1, rotation = 0) {
    host.previewSpec = { id, page, rotation, width: 150 }; this.observer.observe(host);
  }
  async pdf(id) {
    if (this.cache.has(id)) return this.cache.get(id).promise;
    // Bound decoded-document retention; canvases already drawn remain usable.
    if (this.cache.size >= 3) {
      const oldest = [...this.cache.keys()].find((key) => !this.activeIds.has(key));
      if (oldest !== undefined) await this.forget(oldest);
    }
    if (this.cache.has(id)) return this.cache.get(id).promise;
    const entry = {}; this.cache.set(id, entry);
    entry.promise = (async () => {
      const buffer = await invoke('read_document', { id });
      const assets = new URL('/pdfjs/', window.location.href).href;
      entry.task = getDocument({
        data: new Uint8Array(buffer), isEvalSupported: false, verbosity: 0,
        cMapUrl: `${assets}cmaps/`, cMapPacked: true, iccUrl: `${assets}iccs/`,
        standardFontDataUrl: `${assets}standard_fonts/`, wasmUrl: `${assets}wasm/`,
        useSystemFonts: true, enableXfa: false,
      });
      return entry.task.promise;
    })();
    try { return await entry.promise; } catch (error) { this.cache.delete(id); throw error; }
  }
  async draw({ host, id, page, rotation = 0, width = 150, generation = this.generation }) {
    const ticket = Symbol(); host.previewTicket = ticket;
    this.activeIds.set(id, (this.activeIds.get(id) || 0) + 1);
    try {
      const pdf = await this.pdf(id);
      if (!host.isConnected || generation !== this.generation || host.previewTicket !== ticket) return;
      const pdfPage = await pdf.getPage(page);
      if (!host.isConnected || generation !== this.generation || host.previewTicket !== ticket) return;
      const angle = (pdfPage.rotate + rotation) % 360;
      const initial = pdfPage.getViewport({ scale: 1, rotation: angle });
      const viewport = pdfPage.getViewport({ scale: width / initial.width, rotation: angle });
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const canvas = document.createElement('canvas');
      canvas.width = Math.ceil(viewport.width * ratio); canvas.height = Math.ceil(viewport.height * ratio);
      canvas.style.width = `${viewport.width}px`; canvas.style.height = `${viewport.height}px`;
      const task = pdfPage.render({ canvas, canvasContext: canvas.getContext('2d'), viewport,
        transform: ratio === 1 ? undefined : [ratio, 0, 0, ratio, 0, 0], annotationMode: 0 });
      this.renders.add(task);
      try { await task.promise; } finally { this.renders.delete(task); }
      if (host.isConnected && generation === this.generation && host.previewTicket === ticket) host.replaceChildren(canvas);
    } catch (error) {
      if (error?.name === 'RenderingCancelledException' || !host.isConnected || generation !== this.generation || host.previewTicket !== ticket) return;
      const note = document.createElement('span'); note.className = 'preview-unavailable'; note.textContent = 'Preview unavailable';
      host.replaceChildren(note); host.title = 'The preview could not be rendered. PDF processing is still available.';
    } finally {
      const remaining = this.activeIds.get(id) - 1;
      if (remaining) this.activeIds.set(id, remaining); else this.activeIds.delete(id);
    }
  }
  pump() {
    while (this.active < 2 && this.queue.length) {
      const item = this.queue.shift(); this.active += 1;
      this.draw(item).finally(() => { this.active -= 1; this.pump(); });
    }
  }
  async releaseUnused(ids) {
    const keep = new Set(ids);
    for (const id of [...this.cache.keys()]) if (!keep.has(id)) await this.forget(id);
  }
}
