export const recordWindowActions = ['pin', 'showShortcuts', 'focusSearch'] as const;
export const generalShortcutActions = ['selection', ...recordWindowActions] as const;
export const translationWindowActions = ['translate', 'copyPrimary', 'copySecondary', 'actions'] as const;
export const shortcutActions = [...generalShortcutActions, ...translationWindowActions] as const;

export const shortcutTitles = {
  selection: 'openTranslation', pin: 'pinWindow', showShortcuts: 'showShortcuts', focusSearch: 'focusSearch',
  translate: 'translation', copyPrimary: 'copyPrimary', copySecondary: 'copySecondary', actions: 'actions',
} as const;
export type ShortcutAction = typeof shortcutActions[number];
export type ShortcutModifier = 'control' | 'alt' | 'shift' | 'meta';

export interface KeyBinding {
  code: string;
  modifiers: ShortcutModifier[];
}

export type TranslationShortcuts = Record<ShortcutAction, KeyBinding | null>;

export const defaultClipboardBinding: KeyBinding = { code: 'Backquote', modifiers: ['control'] };

const modifierOrder: ShortcutModifier[] = ['control', 'alt', 'shift', 'meta'];

export function defaultTranslationShortcuts(primary: 'meta' | 'control'): TranslationShortcuts {
  const key = (code: string, ...extra: ShortcutModifier[]): KeyBinding => ({ code, modifiers: normalizeModifiers([primary, ...extra]) });
  return {
    selection: key('KeyT'),
    translate: key('Enter'),
    copyPrimary: key('Digit1'),
    copySecondary: key('Digit2'),
    pin: key('KeyP', 'shift'),
    showShortcuts: key('Slash'),
    focusSearch: key('KeyF'),
    actions: key('KeyK'),
  };
}

export function normalizeBinding(binding: KeyBinding): KeyBinding | null {
  const code = binding.code === 'NumpadEnter' ? 'Enter' : binding.code;
  if (!supportedCode(code)) return null;
  const modifiers = normalizeModifiers(binding.modifiers);
  if (modifiers.length !== binding.modifiers.length || !modifiers.some((modifier) => modifier === 'meta' || modifier === 'control' || modifier === 'alt')) return null;
  return { code, modifiers };
}

export function sameBinding(left: KeyBinding, right: KeyBinding) {
  const a = normalizeBinding(left);
  const b = normalizeBinding(right);
  return Boolean(a && b && a.code === b.code && a.modifiers.join('+') === b.modifiers.join('+'));
}

export function shortcutError(action: ShortcutAction | 'clipboard', binding: KeyBinding | null, current: TranslationShortcuts, clipboard: KeyBinding | null = defaultClipboardBinding) {
  if (!binding) return null;
  const value = normalizeBinding(binding);
  if (!value) return binding.modifiers.some((modifier) => modifier === 'meta' || modifier === 'control' || modifier === 'alt') ? 'shortcut_key' : 'shortcut_modifier';
  if (reserved(value)) return 'shortcut_reserved';
  if (clipboard && sameBinding(value, clipboard)) return 'shortcut_clipboard';
  const conflict = shortcutActions.find((other) => other !== action && current[other] && sameBinding(current[other], value));
  return conflict ? `shortcut_conflict:${conflict}` : null;
}

export function bindingFromEvent(event: { code?: string; key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }): KeyBinding | null {
  let code = event.code && event.code !== 'Unidentified' ? event.code : codeFromKey(event.key);
  if (code === 'NumpadEnter') code = 'Enter';
  if (!code || ['MetaLeft', 'MetaRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight'].includes(code)) return null;
  const modifiers = modifierOrder.filter((modifier) => ({ control: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey })[modifier]);
  return { code, modifiers };
}

export function matchesBinding(binding: KeyBinding | null, event: { code?: string; key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }) {
  const pressed = bindingFromEvent(event);
  return Boolean(binding && pressed && sameBinding(binding, pressed));
}

export function keycapLabels(binding: KeyBinding, macos: boolean) {
  const symbols = macos
    ? { control: '⌃', alt: '⌥', shift: '⇧', meta: '⌘' }
    : { control: 'Ctrl', alt: 'Alt', shift: 'Shift', meta: 'Ctrl' };
  const names: Record<string, string> = { Enter: '↵', Backspace: macos ? '⌫' : 'Backspace', Space: 'Space', ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\', Semicolon: ';', Quote: '\'', Comma: ',', Period: '.', Slash: '/' };
  const key = names[binding.code] ?? (binding.code.startsWith('Key') ? binding.code.slice(3) : binding.code.startsWith('Digit') ? binding.code.slice(5) : binding.code);
  return [...normalizeModifiers(binding.modifiers).map((modifier) => symbols[modifier]), key];
}

function normalizeModifiers(modifiers: readonly string[]): ShortcutModifier[] {
  return modifierOrder.filter((modifier) => modifiers.includes(modifier));
}

function supportedCode(code: string) {
  if (/^Key[A-Z]$/.test(code) || /^Digit[0-9]$/.test(code)) return true;
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return true;
  return ['Enter', 'Space', 'Minus', 'Equal', 'BracketLeft', 'BracketRight', 'Backslash', 'Semicolon', 'Quote', 'Comma', 'Period', 'Slash', 'Backquote', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(code);
}

function reserved(binding: KeyBinding) {
  if (binding.code === 'Tab' || binding.code === 'Escape') return true;
  const modifiers = binding.modifiers.join('+');
  const editing = ['KeyA', 'KeyZ', 'KeyX', 'KeyC', 'KeyV', 'KeyQ', 'KeyW', 'Comma'].includes(binding.code);
  return (editing && (modifiers === 'meta' || modifiers === 'control')) || ((modifiers === 'shift+meta' || modifiers === 'control+shift') && binding.code === 'KeyZ');
}

function codeFromKey(key: string) {
  if (key === 'Enter' || key === 'Backspace') return key;
  if (key === ' ') return 'Space';
  if (key === '`') return 'Backquote';
  if (/^[a-zA-Z]$/.test(key)) return `Key${key.toUpperCase()}`;
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  return '';
}
