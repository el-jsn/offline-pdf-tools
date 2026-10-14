// Pointer-based reordering remains available when native file drops are enabled.
export function reorderable(element, index, handle, isBusy, move) {
  element.dataset.reorderIndex = String(index);
  handle.classList.add('reorder-handle');
  let suppressClick = false;
  element.addEventListener('click', (event) => {
    if (suppressClick) { event.preventDefault(); event.stopImmediatePropagation(); suppressClick = false; }
  }, true);
  handle.addEventListener('pointerdown', (start) => {
    if (start.button !== 0 || isBusy() || start.target.closest('button,input')) return;
    let moving = false;
    let target = null;
    const group = element.parentElement;
    const clean = () => {
      target?.classList.remove('drag-target');
      document.body.classList.remove('reordering');
      document.removeEventListener('pointermove', pointerMove, true);
      document.removeEventListener('pointerup', pointerUp, true);
      document.removeEventListener('pointercancel', pointerCancel, true);
    };
    const pointerMove = (event) => {
      if (event.pointerId !== start.pointerId) return;
      if (!moving && Math.hypot(event.clientX - start.clientX, event.clientY - start.clientY) < 6) return;
      moving = true; event.preventDefault(); document.body.classList.add('reordering');
      target?.classList.remove('drag-target');
      const candidate = document.elementFromPoint(event.clientX, event.clientY)?.closest('[data-reorder-index]');
      target = candidate?.parentElement === group ? candidate : null;
      if (target !== element) target?.classList.add('drag-target');
    };
    const pointerUp = (event) => {
      if (event.pointerId !== start.pointerId) return;
      const destination = target ? Number(target.dataset.reorderIndex) : index;
      clean();
      if (moving) {
        suppressClick = true;
        setTimeout(() => { suppressClick = false; }, 0);
        if (!isBusy() && destination !== index) move(index, destination);
      }
    };
    const pointerCancel = () => clean();
    document.addEventListener('pointermove', pointerMove, { capture: true, passive: false });
    document.addEventListener('pointerup', pointerUp, true);
    document.addEventListener('pointercancel', pointerCancel, true);
  });
}
