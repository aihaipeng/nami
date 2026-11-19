import { useLayoutEffect, useRef, type MouseEvent, type RefObject } from 'react';

const rowSelector = '.history-row:not([hidden]) .history-row-select';

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
    if (buttons || (position.current?.x === x && position.current?.y === y)) return;
    position.current = { x, y };
    const focusedRow = document.activeElement?.closest('.history-row');
    if (focusedRow && focusedRow !== currentTarget && focusedRow.closest('.history-panel') === currentTarget.closest('.history-panel')) {
      const button = currentTarget.querySelector<HTMLButtonElement>('.history-row-select');
      button?.focus({ preventScroll: true });
      if (button && document.activeElement === button) return;
    }
    onTarget();
  };
}

export function useRecordsOpenFocus(search: RefObject<HTMLInputElement | null>, active: boolean, request: number | undefined, preferList: boolean, canFocusList: boolean, settled: boolean, onTarget: (id: string | null) => void) {
  const pending = useRef<{ origin: Element | null } | null>(null);
  const options = useRef({ preferList, onTarget });
  useLayoutEffect(() => { options.current = { preferList, onTarget }; }, [preferList, onTarget]);
  useLayoutEffect(() => {
    pending.current = null;
    if (!active || request === undefined) return;
    options.current.onTarget(null);
    if (options.current.preferList) pending.current = { origin: document.activeElement };
    else search.current?.focus({ preventScroll: true });
  }, [active, request, search]);
  useLayoutEffect(() => {
    if (!pending.current) return;
    const origin = pending.current.origin;
    if (document.activeElement !== origin && document.activeElement !== document.body) {
      pending.current = null;
      return;
    }
    const button = canFocusList ? search.current?.closest('.history-panel')?.querySelector<HTMLButtonElement>(rowSelector) : null;
    if (button) {
      pending.current = null;
      options.current.onTarget(button.closest<HTMLElement>('[data-record-id]')?.dataset.recordId ?? null);
      button.focus();
      return;
    }
    if (!settled) return;
    pending.current = null;
    search.current?.focus({ preventScroll: true });
  });
}
