import { translationShortcutEnabled, type AppSnapshot } from './contracts';
import type { ClipboardSearchOptions } from './clipboardSearch';
import type { MessageKey } from './i18n';
import { bindingFromEvent, editsText, globalTranslationActions, recordWindowActions, sameBinding, type KeyBinding } from './shortcuts';

export const sharedRecordActions = ['preview', 'note', 'favorite', 'pin', 'copyRecord', 'delete'] as const;
export const clipboardOnlyActions = ['switchCategory', 'openTarget', 'ocr', 'translate', 'pastePlain', 'saveImage'] as const;
export const clipboardRecordActions = [...sharedRecordActions, ...clipboardOnlyActions] as const;
export type ClipboardRecordAction = typeof clipboardRecordActions[number];
type RecordShortcut = { binding: KeyBinding | null; enabled: boolean };
export type RecordsOpenFocus = 'search' | 'list';
export type ClipboardControls = { search?: ClipboardSearchOptions; recordsOpenFocus?: RecordsOpenFocus; previewLines?: { clipboard: number }; activation: 'copy' | 'paste'; shortcuts: Record<ClipboardRecordAction, RecordShortcut> };

export function defaultClipboardShortcuts(primary: 'meta' | 'control') {
  const shortcut = (code: string, modifiers: KeyBinding['modifiers'] = [primary]) => ({ enabled: true, binding: { code, modifiers } });
  return {
    switchCategory: shortcut('Tab', []),
    openTarget: shortcut('KeyO', [primary]),
    pastePlain: shortcut('Enter', [primary]), saveImage: shortcut('KeyS', [primary]),
    preview: shortcut('Space', []),
    note: shortcut('KeyN'), ocr: shortcut('KeyO', [primary, 'shift']), translate: shortcut('KeyT', primary === 'meta' ? [primary] : [primary, 'shift']),
    copyRecord: shortcut('KeyC'),
    favorite: shortcut('KeyD'), pin: shortcut('KeyP', primary === 'meta' ? ['alt', 'meta'] : ['control', 'alt']), delete: shortcut('Backspace', [primary]),
  };
}

export function sameClipboardBinding(left: KeyBinding, right: KeyBinding) {
  return left.code === right.code && left.modifiers.length === right.modifiers.length && left.modifiers.every((key) => right.modifiers.includes(key));
}

export const clipboardShortcutLabels = {
  switchCategory: 'switchCategory', pastePlain: 'clipboardShortcutPastePlain', saveImage: 'clipboardShortcutSaveImage',
  openTarget: 'clipboardShortcutOpenTarget', preview: 'clipboardShortcutPreview', note: 'toggleRecordNote', ocr: 'clipboardShortcutOcr', translate: 'translation',
  copyRecord: 'copy',
  favorite: 'clipboardShortcutFavorite', pin: 'clipboardShortcutPin', delete: 'clipboardShortcutDelete',
} as const satisfies Record<ClipboardRecordAction, MessageKey>;

export const isClipboardTranslateLaunchKey = (binding: KeyBinding) => binding.code === 'KeyT' && binding.modifiers.length === 1 && binding.modifiers[0] === 'meta';

export function clipboardGlobalBindings(snapshot: AppSnapshot, includeTranslationLaunch = true) {
  const { translationPreferences: preferences } = snapshot;
  return [
    snapshot.clipboardShortcutEnabled ? snapshot.clipboardBinding : null,
    ...([...(includeTranslationLaunch ? globalTranslationActions : []), 'openSettings'] as const)
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

export function clipboardCategoryDirection(shortcuts: ReturnType<typeof clipboardPanelShortcuts>, event: KeyboardEvent) {
  const shortcut = shortcuts.find(({ action }) => action === 'switchCategory');
  const binding = shortcut?.enabled && !shortcut.globalConflict ? shortcut.binding : null;
  const pressed = binding && bindingFromEvent(event);
  if (!binding || !pressed || editsText(event)) return 0;
  if (shortcuts.some((other) => other.action !== 'switchCategory' && other.enabled && !other.globalConflict && other.binding && sameClipboardBinding(other.binding, pressed))) return 0;
  if (sameClipboardBinding(pressed, binding)) return 1;
  return !binding.modifiers.includes('shift') && sameClipboardBinding(pressed, { code: binding.code, modifiers: [...binding.modifiers, 'shift'] }) ? -1 : 0;
}

export function clipboardShortcutAvailable(snapshot: AppSnapshot, native: boolean, action: ClipboardRecordAction) {
  return action === 'preview' || action === 'switchCategory' || (native && !snapshot.storageError
    && (action !== 'ocr' || snapshot.capabilities.imageOcr && snapshot.capabilities.clipboardWrite)
    && (action !== 'openTarget' || snapshot.capabilities.clipboardOpenTarget)
    && (action !== 'copyRecord' || snapshot.capabilities.clipboardWrite)
    && (action !== 'pastePlain' || snapshot.capabilities.autoPaste)
    && (action !== 'saveImage' || snapshot.capabilities.clipboardImageSave));
}

export function clipboardKeyAction(shortcuts: ReturnType<typeof clipboardPanelShortcuts>, event: KeyboardEvent) {
  const key = bindingFromEvent(event);
  const action = key && shortcuts.find(({ binding, enabled, globalConflict }) => enabled && !globalConflict && binding && sameClipboardBinding(binding, key))?.action;
  if (action === 'switchCategory') return;
  if (!editsText(event)) return action;
  const search = event.target instanceof HTMLInputElement && event.target.type === 'search' ? event.target : null;
  if (!search) return;
  if (action === 'openTarget' && key?.code === 'KeyO' && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) return action;
  // Keep the established Option+F/P record actions and let Option+Delete edit a non-empty query.
  if (event.altKey && !event.shiftKey && ((action === 'favorite' && key?.code === 'KeyF') || (action === 'pin' && key?.code === 'KeyP')
    || (action === 'delete' && key?.code === 'Backspace' && !search.value))) return action;
}
