import { useEffect, useEffectEvent } from 'react';
import type { AppSnapshot } from './contracts';
import { clipboardKeyAction, clipboardPanelShortcuts, clipboardShortcutAvailable, sharedRecordActions } from './clipboardShortcuts';

const BLOCKING_OVERLAY_SELECTOR = '.clipboard-editor-layer, .clipboard-context-menu, .history-shortcuts-panel, .clipboard-navigation-confirm';

export function isPreviewShortcutEvent(
  event: KeyboardEvent,
  snapshot: AppSnapshot,
  native: boolean,
  options: { blocked?: boolean; excludedContainers?: string } = {},
) {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.key === 'Dead' || options.blocked) return false;
  if (document.querySelector(BLOCKING_OVERLAY_SELECTOR)) return false;
  const target = event.target;
  if (target instanceof Element) {
    if (target.closest('[role=dialog], [role=menu]')) return false;
    if (options.excludedContainers && target.closest(options.excludedContainers)) return false;
  }
  const action = clipboardKeyAction(clipboardPanelShortcuts(snapshot, sharedRecordActions), event);
  return action === 'preview' && clipboardShortcutAvailable(snapshot, native, 'preview');
}

export function useRecordsPreviewShortcut({
  active,
  blocked = false,
  canTrigger = true,
  excludedContainers,
  snapshot,
  native,
  onPreview,
}: {
  active: boolean;
  blocked?: boolean;
  canTrigger?: boolean;
  excludedContainers?: string;
  snapshot: AppSnapshot;
  native: boolean;
  onPreview: (rowId: string | undefined) => void;
}) {
  const handle = useEffectEvent((event: KeyboardEvent) => {
    if (!canTrigger || !isPreviewShortcutEvent(event, snapshot, native, { blocked, excludedContainers })) return;
    event.preventDefault();
    if (event.repeat) return;
    const rowId = event.target instanceof Element
      ? event.target.closest<HTMLElement>('[data-record-id]')?.dataset.recordId
      : undefined;
    onPreview(rowId);
  });

  useEffect(() => {
    if (!active) return;
    const keydown = (event: KeyboardEvent) => handle(event);
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [active]);
}
