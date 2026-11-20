import { translationShortcutEnabled, type AppSnapshot } from './contracts';
import type { MessageKey } from './i18n';
import { bindingFromEvent, editsText, globalTranslationActions, recordWindowActions, sameBinding, type KeyBinding } from './shortcuts';

export const recordNavigationActions = ['navigateUp', 'navigateDown', 'previousCategory', 'nextCategory'] as const;
export const sharedRecordActions = [...recordNavigationActions, 'switchView', 'focusRecords', 'preview', 'note', 'favorite', 'pin', 'copyRecord', 'delete'] as const;
export const clipboardOnlyActions = ['openTarget', 'ocr', 'translate'] as const;
export const clipboardRecordActions = [...sharedRecordActions, ...clipboardOnlyActions] as const;
export type ClipboardRecordAction = typeof clipboardRecordActions[number];
type RecordShortcut = { binding: KeyBinding | null; enabled: boolean };
export type RecordsOpenFocus = 'search' | 'list';
export type ClipboardControls = { autoImageIndex?: boolean; recordsOpenFocus?: RecordsOpenFocus; previewLines?: Record<'clipboard' | 'translation', number>; activation: 'copy' | 'paste'; shortcuts: Record<ClipboardRecordAction, RecordShortcut> };

export function defaultClipboardShortcuts(primary: 'meta' | 'control') {
  const shortcut = (code: string, modifiers: KeyBinding['modifiers'] = ['alt']) => ({ enabled: true, binding: { code, modifiers } });
  return {
    openTarget: shortcut('KeyO', [primary]),
    preview: shortcut('KeyY', [primary]),
    note: shortcut('KeyN', [primary, 'shift']), ocr: shortcut('KeyO', [primary, 'shift']), translate: shortcut('KeyT', [primary, 'shift']),
    navigateUp: shortcut('ArrowUp', []), navigateDown: shortcut('ArrowDown', []),
    previousCategory: shortcut('ArrowLeft', []), nextCategory: shortcut('ArrowRight', []),
    switchView: shortcut('Tab', []), focusRecords: { binding: null, enabled: true }, copyRecord: shortcut('KeyC', [primary, 'shift']),
    favorite: shortcut('KeyF'), pin: shortcut('KeyP'), delete: shortcut('Backspace', [primary]),
  };
}

export function sameClipboardBinding(left: KeyBinding, right: KeyBinding) {
  return left.code === right.code && left.modifiers.length === right.modifiers.length && left.modifiers.every((key) => right.modifiers.includes(key));
}

export const clipboardShortcutLabels = {
  openTarget: 'clipboardShortcutOpenTarget', preview: 'clipboardShortcutPreview', note: 'toggleRecordNote', ocr: 'clipboardShortcutOcr', translate: 'translation',
  navigateUp: 'navigateUp', navigateDown: 'navigateDown', previousCategory: 'previousCategory', nextCategory: 'nextCategory',
  switchView: 'switchRecordView', focusRecords: 'focusRecords', copyRecord: 'copy',
  favorite: 'clipboardShortcutFavorite', pin: 'clipboardShortcutPin', delete: 'clipboardShortcutDelete',
} as const satisfies Record<ClipboardRecordAction, MessageKey>;

export function clipboardPanelShortcuts(snapshot: AppSnapshot, actions: readonly ClipboardRecordAction[] = clipboardRecordActions) {
  return actions.map((action) => {
    const shortcut = snapshot.clipboardControls.shortcuts[action] ?? defaultClipboardShortcuts(snapshot.capabilities.primaryModifier)[action];
    const { binding } = shortcut;
    const globalConflict = !!binding && [
      snapshot.clipboardShortcutEnabled ? snapshot.clipboardBinding : null,
      ...globalTranslationActions.filter((action) => translationShortcutEnabled(snapshot.translationPreferences, action)).map((action) => snapshot.translationPreferences.shortcuts[action]),
      translationShortcutEnabled(snapshot.translationPreferences, 'toggleHistory') ? snapshot.translationPreferences.shortcuts.toggleHistory : null,
      ...recordWindowActions.filter((candidate) => (candidate === 'openSettings' || action === 'copyRecord' || action === 'focusRecords') && translationShortcutEnabled(snapshot.translationPreferences, candidate)).map((candidate) => snapshot.translationPreferences.shortcuts[candidate]),
    ].some((global) => global && sameBinding(global, binding));
    return { action, ...shortcut, globalConflict };
  });
}

export function clipboardShortcutAvailable(snapshot: AppSnapshot, native: boolean, action: ClipboardRecordAction) {
  return action === 'preview' || action === 'switchView' || action === 'focusRecords' || recordNavigationActions.some((navigation) => navigation === action) || (native && !snapshot.storageError
    && (action !== 'ocr' || snapshot.capabilities.imageOcr && snapshot.capabilities.clipboardWrite)
    && (action !== 'openTarget' || snapshot.capabilities.clipboardOpenTarget)
    && (action !== 'copyRecord' || snapshot.capabilities.clipboardWrite));
}

export function clipboardKeyAction(shortcuts: ReturnType<typeof clipboardPanelShortcuts>, event: KeyboardEvent) {
  const key = bindingFromEvent(event);
  const action = key && shortcuts.find(({ binding, enabled, globalConflict }) => enabled && !globalConflict && binding && sameClipboardBinding(binding, key))?.action;
  if (!editsText(event)) return action;
  const search = event.target instanceof HTMLInputElement && event.target.type === 'search' ? event.target : null;
  if (!search) return;
  if (action === 'openTarget' && key?.code === 'KeyO' && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) return action;
  if ((action === 'previousCategory' || action === 'nextCategory') && !search.value && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')
    && !event.altKey && !event.shiftKey) return action;
  // Keep the established Option+F/P record actions and let Option+Delete edit a non-empty query.
  if (event.altKey && !event.shiftKey && ((action === 'favorite' && key?.code === 'KeyF') || (action === 'pin' && key?.code === 'KeyP')
    || (action === 'delete' && key?.code === 'Backspace' && !search.value))) return action;
}
