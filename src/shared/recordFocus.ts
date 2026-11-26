import { useLayoutEffect, useRef, type MouseEvent, type RefObject } from 'react';

const rowSelector = '.history-row:not([hidden]) .history-row-select';
// Pinned records render above the ordinary list; opening starts at the first ordinary record.
const unpinnedRowSelector = `.history-unpinned ${rowSelector}`;

export function focusRecordList(panel: HTMLElement | null | undefined) {
  const button = panel?.querySelector<HTMLButtonElement>('.history-row[data-keyboard-current]:not([hidden]) .history-row-select')
    ?? panel?.querySelector<HTMLButtonElement>(rowSelector);
  button?.focus();
  return Boolean(button);
}

export function useRecordPointer() {
  const position = useRef<{ x: number; y: number } | null>(null);
  return (event: MouseEvent<HTMLElement>, onTarget: () => void) => {
    const { clientX: x, clientY: y, buttons, currentTarget } = event;
    // A list update may retarget a mouse event without moving the pointer.
    if (buttons || (position.current?.x === x && position.current?.y === y)) return false;
    position.current = { x, y };
    const focusedRow = document.activeElement?.closest('.history-row');
    if (focusedRow && focusedRow !== currentTarget && focusedRow.closest('.history-panel') === currentTarget.closest('.history-panel')) {
      const button = currentTarget.querySelector<HTMLButtonElement>('.history-row-select');
      if (button) {
        let focused = false;
        const mark = () => { focused = true; };
        button.addEventListener('focus', mark, { once: true });
        button.focus({ preventScroll: true });
        button.removeEventListener('focus', mark);
        if (focused) return true;
      }
    }
    onTarget();
    return true;
  };
}

export function useRecordsOpenFocus(search: RefObject<HTMLInputElement | null>, active: boolean, visible: boolean, request: number | undefined, preferList: boolean, canFocusList: boolean, settled: boolean, onTarget: (id: string | null) => void) {
  const pending = useRef<{ origin: Element | null; recordId: string | null } | null>(null);
  const options = useRef({ preferList, onTarget });
  // Entering a view while the records stay visible returns to its last keyboard record; a hidden workspace opens on the first record.
  const shown = useRef(false);
  const lastRecord = useRef<string | null>(null);
  useLayoutEffect(() => { options.current = { preferList, onTarget }; }, [preferList, onTarget]);
  useLayoutEffect(() => {
    const switching = shown.current;
    shown.current = visible;
    pending.current = null;
    if (!active || request === undefined) return;
    const recordId = switching ? lastRecord.current : null;
    options.current.onTarget(options.current.preferList ? recordId : null);
    if (options.current.preferList) pending.current = { origin: document.activeElement, recordId };
    else search.current?.focus({ preventScroll: true });
  }, [active, request, search]);
  useLayoutEffect(() => {
    if (!active || pending.current) return;
    const current = search.current?.closest('.history-panel')?.querySelector<HTMLElement>('.history-row[data-keyboard-current]')?.dataset.recordId;
    if (current) lastRecord.current = current;
  });
  useLayoutEffect(() => {
    if (!pending.current) return;
    const { origin, recordId } = pending.current;
    if (document.activeElement !== origin && document.activeElement !== document.body) {
      pending.current = null;
      return;
    }
    // A copy that arrived while hidden may still be loading; focusing the cached page would miss it.
    const panel = canFocusList && settled ? search.current?.closest('.history-panel') : null;
    const resumed = panel && recordId
      ? [...panel.querySelectorAll<HTMLElement>('.history-row:not([hidden])')].find((row) => row.dataset.recordId === recordId) : null;
    const button = resumed?.querySelector<HTMLButtonElement>('.history-row-select')
      ?? panel?.querySelector<HTMLButtonElement>(unpinnedRowSelector) ?? panel?.querySelector<HTMLButtonElement>(rowSelector);
    if (button) {
      pending.current = null;
      options.current.onTarget(button.closest<HTMLElement>('[data-record-id]')?.dataset.recordId ?? null);
      button.focus();
      return;
    }
    if (!settled) return;
    pending.current = null;
    options.current.onTarget(null);
    search.current?.focus({ preventScroll: true });
  });
}
