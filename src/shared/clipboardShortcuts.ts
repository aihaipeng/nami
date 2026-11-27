import { translationShortcutEnabled, type AppSnapshot } from './contracts';
import type { ClipboardSearchOptions } from './clipboardSearch';
import type { MessageKey } from './i18n';
import { bindingFromEvent, editsText, globalTranslationActions, recordWindowActions, sameBinding, type KeyBinding } from './shortcuts';

export const sharedRecordActions = ['preview', 'note', 'favorite', 'pin', 'delete'] as const;
export const clipboardOnlyActions = ['openTarget', 'ocr', 'translate', 'pastePlain'] as const;
export const clipboardRecordActions = [...sharedRecordActions, ...clipboardOnlyActions] as const;
export type ClipboardRecordAction = typeof clipboardRecordActions[number];
type RecordShortcut = { binding: KeyBinding | null; enabled: boolean };
export type RecordsOpenFocus = 'search' | 'list';
export const DEFAULT_HOVER_PREVIEW_DELAY_MS = 1000;
export const MAX_HOVER_PREVIEW_DELAY_MS = 5000;
export type ClipboardControls = {
  search?: ClipboardSearchOptions;
  recordsOpenFocus?: RecordsOpenFocus;
  previewLines?: { clipboard: number };
  hoverPreviewEnabled?: boolean;
  hoverPreviewDelayMs?: number;
  shortcuts: Record<ClipboardRecordAction, RecordShortcut>;
};

export function defaultClipboardShortcuts(primary: 'meta' | 'control') {
  const shortcut = (code: string, modifiers: KeyBinding['modifiers'] = [primary]) => ({ enabled: true, binding: { code, modifiers } });
  return {
    openTarget: shortcut('KeyO', [primary]),
    pastePlain: shortcut('Enter', [primary]),
    preview: shortcut('Space', []),
    note: shortcut('KeyN'), ocr: shortcut('KeyO', [primary, 'shift']), translate: shortcut('KeyT', primary === 'meta' ? [primary] : [primary, 'shift']),
    favorite: shortcut('KeyD'), pin: shortcut('KeyP', primary === 'meta' ? ['alt', 'meta'] : ['control', 'alt']), delete: shortcut('Backspace', [primary]),
  };
}

export function sameClipboardBinding(left: KeyBinding, right: KeyBinding) {
  return left.code === right.code && left.modifiers.length === right.modifiers.length && left.modifiers.every((key) => right.modifiers.includes(key));
}

export const clipboardShortcutLabels = {
  pastePlain: 'clipboardShortcutPastePlain',
  openTarget: 'clipboardShortcutOpenTarget', preview: 'clipboardShortcutPreview', note: 'toggleRecordNote', ocr: 'clipboardShortcutOcr', translate: 'translation',
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
    const globalConflict = !!binding && (contextual ? clipboardGlobalBindings(snapshot, false) : globals)
      .some((global) => global && sameBinding(global, binding));
    return { action, ...shortcut, globalConflict };
  });
}

export function clipboardCategoryArrowDirection(
  shortcuts: ReturnType<typeof clipboardPanelShortcuts>,
  snapshot: AppSnapshot,
  event: KeyboardEvent,
  searchInput: HTMLInputElement | null,
) {
  if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return 0;
  const direction = event.key === 'ArrowLeft' || event.code === 'ArrowLeft'
    ? -1
    : event.key === 'ArrowRight' || event.code === 'ArrowRight'
      ? 1
      : 0;
  if (!direction) return 0;
  const target = event.target;
  if (target instanceof HTMLElement && target.closest('input, textarea, [contenteditable=true]')) {
    if (target !== searchInput || searchInput.value.length > 0) return 0;
  }
  const pressed = bindingFromEvent(event);
  if (pressed) {
    if (shortcuts.some((other) => other.enabled && !other.globalConflict && other.binding && sameClipboardBinding(other.binding, pressed))) return 0;
    if (recordWindowActions.some((action) => translationShortcutEnabled(snapshot.translationPreferences, action)
      && snapshot.translationPreferences.shortcuts[action] && sameBinding(snapshot.translationPreferences.shortcuts[action]!, pressed))) return 0;
  }
  return direction;
}

const blockedBrowserPrimaryCodes = new Set([
  'KeyA', 'KeyB', 'KeyD', 'KeyE', 'KeyF', 'KeyG', 'KeyI', 'KeyJ', 'KeyK', 'KeyL',
  'KeyN', 'KeyO', 'KeyP', 'KeyR', 'KeyS', 'KeyT', 'KeyU', 'KeyV', 'KeyX', 'KeyZ',
  'Equal', 'Minus', 'Digit0', 'NumpadAdd', 'NumpadSubtract', 'Numpad0',
  'BracketLeft', 'BracketRight', 'ArrowLeft', 'ArrowRight',
]);

export function isRecordBrowserDefaultNavigation(event: KeyboardEvent) {
  return !event.metaKey && !event.ctrlKey && !event.altKey
    && (event.key === 'Tab' || (event.shiftKey && event.key === 'F10'));
}

export function isNonNamiClipboardBrowserKey(
  event: KeyboardEvent,
  primaryModifier: 'meta' | 'control',
  inSearchInput: boolean,
) {
  if (isRecordBrowserDefaultNavigation(event) || event.key === 'F5' || event.code === 'F5') return true;
  if (!inSearchInput && (event.key === ' ' || event.code === 'Space')) return true;
  if (!inSearchInput && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
    && (event.key === 'Backspace' || event.key === 'Delete')) return true;
  const primary = primaryModifier === 'meta' ? event.metaKey : event.ctrlKey;
  if (!primary) return false;
  const code = bindingFromEvent(event)?.code ?? event.code;
  if (inSearchInput && (editsText(event) || (!event.altKey && !(event.metaKey && event.ctrlKey)
    && (code === 'KeyV' || primaryModifier === 'meta' && (code === 'ArrowLeft' || code === 'ArrowRight'))))) return false;
  if (code === 'KeyQ' || code === 'KeyH' || code === 'KeyM' || code === 'KeyW' || code === 'Comma') return false;
  return blockedBrowserPrimaryCodes.has(code);
}

export function clipboardShortcutAvailable(snapshot: AppSnapshot, native: boolean, action: ClipboardRecordAction) {
  return action === 'preview' || (native && !snapshot.storageError
    && (action !== 'ocr' || snapshot.capabilities.imageOcr)
    && (action !== 'openTarget' || snapshot.capabilities.clipboardOpenTarget)
    && (action !== 'pastePlain' || snapshot.capabilities.autoPaste));
}

export function clipboardKeyAction(shortcuts: ReturnType<typeof clipboardPanelShortcuts>, event: KeyboardEvent) {
  const key = bindingFromEvent(event);
  const action = key && shortcuts.find(({ binding, enabled, globalConflict }) => enabled && !globalConflict && binding && sameClipboardBinding(binding, key))?.action;
  if (!editsText(event)) return action;
  const search = event.target instanceof HTMLInputElement && event.target.type === 'search' ? event.target : null;
  if (!search) return;
  if (action === 'openTarget' && key?.code === 'KeyO' && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) return action;
  // Keep the established Option+F/P record actions and let Option+Delete edit a non-empty query.
  if (event.altKey && !event.shiftKey && ((action === 'favorite' && key?.code === 'KeyF') || (action === 'pin' && key?.code === 'KeyP')
    || (action === 'delete' && key?.code === 'Backspace' && !search.value))) return action;
}
