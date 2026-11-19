import type { AppSnapshot } from './contracts';
import type { MessageKey } from './i18n';
import { bindingFromEvent, sameBinding, type KeyBinding } from './shortcuts';

export const clipboardRecordActions = ['preview', 'note', 'ocr', 'translate', 'favorite', 'pin', 'delete'] as const;
export type ClipboardRecordAction = typeof clipboardRecordActions[number];
type RecordShortcut = { binding: KeyBinding | null; enabled: boolean };
export type ClipboardControls = { activation: 'copy' | 'paste'; shortcuts: Record<ClipboardRecordAction, RecordShortcut> };

export function defaultClipboardShortcuts(primary: 'meta' | 'control') {
  const shortcut = (code: string, modifiers: KeyBinding['modifiers'] = ['alt']) => ({ enabled: true, binding: { code, modifiers } });
  return {
    preview: shortcut('KeyY', [primary]),
    note: shortcut('KeyN', [primary, 'shift']), ocr: shortcut('KeyO', [primary, 'shift']), translate: shortcut('KeyT', [primary, 'shift']),
    favorite: shortcut('KeyF'), pin: shortcut('KeyP'), delete: shortcut('Backspace', [primary]),
  };
}

export function sameClipboardBinding(left: KeyBinding, right: KeyBinding) {
  return left.code === right.code && left.modifiers.length === right.modifiers.length && left.modifiers.every((key) => right.modifiers.includes(key));
}

export const clipboardShortcutLabels = {
  preview: 'clipboardShortcutPreview', note: 'clipboardShortcutNote', ocr: 'clipboardShortcutOcr', translate: 'translation',
  favorite: 'clipboardShortcutFavorite', pin: 'clipboardShortcutPin', delete: 'clipboardShortcutDelete',
} as const satisfies Record<ClipboardRecordAction, MessageKey>;

export function clipboardPanelShortcuts(snapshot: AppSnapshot) {
  return clipboardRecordActions.map((action) => {
    const shortcut = snapshot.clipboardControls.shortcuts[action];
    const { binding } = shortcut;
    const globalConflict = !!binding && [
      snapshot.clipboardShortcutEnabled ? snapshot.clipboardBinding : null,
      snapshot.translationPreferences.selectionEnabled ? snapshot.translationPreferences.shortcuts.selection : null,
    ].some((global) => global && sameBinding(global, binding));
    return { action, ...shortcut, globalConflict };
  });
}

export function clipboardShortcutAvailable(snapshot: AppSnapshot, native: boolean, action: ClipboardRecordAction) {
  return action === 'preview' || (native && !snapshot.storageError && (action !== 'ocr' || snapshot.capabilities.imageOcr && snapshot.capabilities.clipboardWrite));
}

export function clipboardKeyAction(shortcuts: ReturnType<typeof clipboardPanelShortcuts>, event: KeyboardEvent) {
  const key = bindingFromEvent(event);
  return key && shortcuts.find(({ binding, enabled, globalConflict }) => enabled && !globalConflict && binding && sameClipboardBinding(binding, key))?.action;
}
