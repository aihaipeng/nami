import { useEffect, useEffectEvent } from 'react';
import type { AppSnapshot } from './contracts';
import { clipboardKeyAction, clipboardPanelShortcuts, clipboardShortcutAvailable, sharedRecordActions } from './clipboardShortcuts';

const BLOCKING_OVERLAY_SELECTOR = '.clipboard-editor-layer, .clipboard-context-menu, .history-shortcuts-panel, .clipboard-navigation-confirm, .clipboard-delete-confirm';

export function isPreviewShortcutEvent(
  event: KeyboardEvent,
  snapshot: AppSnapshot,
  native: boolean,
  options: { blocked?: boolean; includedContainer?: string; excludedContainers?: string } = {},
) {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.key === 'Dead' || options.blocked) return false;
  if (document.querySelector(BLOCKING_OVERLAY_SELECTOR)) return false;
  const target = event.target;
  if (options.includedContainer && !(target instanceof Element && target.closest(options.includedContainer))) return false;
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
  includedContainer,
  excludedContainers,
  snapshot,
  native,
  onPreview,
}: {
  active: boolean;
  blocked?: boolean;
  canTrigger?: boolean;
  includedContainer?: string;
  excludedContainers?: string;
  snapshot: AppSnapshot;
  native: boolean;
  onPreview: (rowId: string | undefined, target: EventTarget | null) => void;
}) {
  const handle = useEffectEvent((event: KeyboardEvent) => {
    if (!canTrigger || !isPreviewShortcutEvent(event, snapshot, native, { blocked, includedContainer, excludedContainers })) return;
    event.preventDefault();
    if (event.repeat) return;
    const rowId = event.target instanceof Element
      ? event.target.closest<HTMLElement>('[data-record-id]')?.dataset.recordId
      : undefined;
    onPreview(rowId, event.target);
  });

  useEffect(() => {
    if (!active) return;
    const keydown = (event: KeyboardEvent) => handle(event);
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [active]);
}
