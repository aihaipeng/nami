import { translationShortcutEnabled, type AppSnapshot } from './contracts';
import type { ClipboardSearchOptions } from './clipboardSearch';
import type { MessageKey } from './i18n';
import { bindingFromEvent, editsText, globalTranslationActions, recordWindowActions, sameBinding, type KeyBinding } from './shortcuts';

export const recordNavigationActions = ['navigateUp', 'navigateDown', 'previousCategory', 'nextCategory'] as const;
export const sharedRecordActions = [...recordNavigationActions, 'switchView', 'preview', 'note', 'favorite', 'pin', 'copyRecord', 'delete'] as const;
export const clipboardMenuOnlyActions = ['pasteRecord', 'pastePlain', 'saveImage'] as const;
export const clipboardOnlyActions = ['openTarget', 'ocr', 'translate', ...clipboardMenuOnlyActions] as const;
export const clipboardRecordActions = [...sharedRecordActions, ...clipboardOnlyActions] as const;
export type ClipboardRecordAction = typeof clipboardRecordActions[number];
type RecordShortcut = { binding: KeyBinding | null; enabled: boolean };
export type RecordsOpenFocus = 'search' | 'list';
export type ClipboardControls = { search?: ClipboardSearchOptions; recordsOpenFocus?: RecordsOpenFocus; previewLines?: Record<'clipboard' | 'translation', number>; activation: 'copy' | 'paste'; shortcuts: Record<ClipboardRecordAction, RecordShortcut> };

export function defaultClipboardShortcuts(primary: 'meta' | 'control') {
  const shortcut = (code: string, modifiers: KeyBinding['modifiers'] = ['alt']) => ({ enabled: true, binding: { code, modifiers } });
  return {
    openTarget: shortcut('KeyO', [primary]),
    pasteRecord: shortcut('Enter', []), pastePlain: shortcut('Enter', [primary]), saveImage: shortcut('KeyS', [primary]),
    preview: shortcut('KeyY', [primary]),
    note: shortcut('KeyN', [primary, 'shift']), ocr: shortcut('KeyO', [primary, 'shift']), translate: shortcut('KeyT', primary === 'meta' ? [primary] : [primary, 'shift']),
    navigateUp: shortcut('ArrowUp', []), navigateDown: shortcut('ArrowDown', []),
    previousCategory: shortcut('ArrowLeft', []), nextCategory: shortcut('ArrowRight', []),
    switchView: shortcut('Tab', []), copyRecord: shortcut('KeyC', [primary, 'shift']),
    favorite: shortcut('KeyF'), pin: shortcut('KeyP'), delete: shortcut('Backspace', [primary]),
  };
}

export function sameClipboardBinding(left: KeyBinding, right: KeyBinding) {
  return left.code === right.code && left.modifiers.length === right.modifiers.length && left.modifiers.every((key) => right.modifiers.includes(key));
}

export const clipboardShortcutLabels = {
  pasteRecord: 'clipboardShortcutPasteRecord', pastePlain: 'clipboardShortcutPastePlain', saveImage: 'clipboardShortcutSaveImage',
  openTarget: 'clipboardShortcutOpenTarget', preview: 'clipboardShortcutPreview', note: 'toggleRecordNote', ocr: 'clipboardShortcutOcr', translate: 'translation',
  navigateUp: 'navigateUp', navigateDown: 'navigateDown', previousCategory: 'previousCategory', nextCategory: 'nextCategory',
  switchView: 'switchRecordView', copyRecord: 'copy',
  favorite: 'clipboardShortcutFavorite', pin: 'clipboardShortcutPin', delete: 'clipboardShortcutDelete',
} as const satisfies Record<ClipboardRecordAction, MessageKey>;

export const isClipboardTranslateLaunchKey = (binding: KeyBinding) => binding.code === 'KeyT' && binding.modifiers.length === 1 && binding.modifiers[0] === 'meta';

export function clipboardGlobalBindings(snapshot: AppSnapshot, includeTranslationLaunch = true) {
  const { translationPreferences: preferences } = snapshot;
  return [
    snapshot.clipboardShortcutEnabled ? snapshot.clipboardBinding : null,
    ...([...(includeTranslationLaunch ? globalTranslationActions : []), 'toggleHistory', 'openSettings'] as const)
      .filter((action) => translationShortcutEnabled(preferences, action)).map((action) => preferences.shortcuts[action]),
  ];
}

export function clipboardPanelShortcuts(snapshot: AppSnapshot, actions: readonly ClipboardRecordAction[] = clipboardRecordActions) {
  const globals = clipboardGlobalBindings(snapshot);
  return actions.map((action) => {
    const shortcut = snapshot.clipboardControls.shortcuts[action] ?? defaultClipboardShortcuts(snapshot.capabilities.primaryModifier)[action];
    const { binding } = shortcut;
    const contextual = action === 'translate' && snapshot.capabilities.platform === 'macos' && binding && isClipboardTranslateLaunchKey(binding);
    const globalConflict = !!binding && [
      ...(contextual ? clipboardGlobalBindings(snapshot, false) : globals),
      ...recordWindowActions.filter((candidate) => candidate !== 'openSettings' && action === 'copyRecord' && translationShortcutEnabled(snapshot.translationPreferences, candidate)).map((candidate) => snapshot.translationPreferences.shortcuts[candidate]),
    ].some((global) => global && sameBinding(global, binding));
    return { action, ...shortcut, globalConflict };
  });
}

export function clipboardShortcutAvailable(snapshot: AppSnapshot, native: boolean, action: ClipboardRecordAction) {
  return action === 'preview' || action === 'switchView' || recordNavigationActions.some((navigation) => navigation === action) || (native && !snapshot.storageError
    && (action !== 'ocr' || snapshot.capabilities.imageOcr && snapshot.capabilities.clipboardWrite)
    && (action !== 'openTarget' || snapshot.capabilities.clipboardOpenTarget)
    && (action !== 'copyRecord' || snapshot.capabilities.clipboardWrite)
    && (action !== 'pasteRecord' && action !== 'pastePlain' || snapshot.capabilities.autoPaste)
    && (action !== 'saveImage' || snapshot.capabilities.clipboardImageSave));
}

export function clipboardKeyAction(shortcuts: ReturnType<typeof clipboardPanelShortcuts>, event: KeyboardEvent) {
  const key = bindingFromEvent(event);
  const action = key && shortcuts.find(({ binding, enabled, globalConflict }) => enabled && !globalConflict && binding && sameClipboardBinding(binding, key))?.action;
  if (action === 'pasteRecord' || action === 'pastePlain' || action === 'saveImage') return;
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
