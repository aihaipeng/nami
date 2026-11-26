import type { Translate } from './i18n';

export const recordWindowActions = ['pin', 'showShortcuts', 'focusSearch', 'openSettings'] as const;
export const globalTranslationActions = ['openTranslation', 'selection'] as const;
export const generalShortcutActions = [...globalTranslationActions, ...recordWindowActions] as const;
export const translationWindowActions = ['translate', 'copyPrimary', 'copySecondary'] as const;
export const shortcutActions = [...globalTranslationActions, ...recordWindowActions, ...translationWindowActions] as const;

export const shortcutTitles = {
  openTranslation: 'openTranslation', selection: 'autofillInput', pin: 'pinWindow', showShortcuts: 'showShortcuts', focusSearch: 'focusSearch', openSettings: 'openSettings',
  translate: 'translation', copyPrimary: 'copyPrimary', copySecondary: 'copySecondary',
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
    openTranslation: key('KeyT'), selection: key('KeyT'),
    translate: key('Enter'),
    copyPrimary: key('Digit1'),
    copySecondary: key('Digit2'),
    pin: key('KeyP'),
    showShortcuts: key('KeyK'),
    focusSearch: key('KeyF'),
    openSettings: key('Comma'),
  };
}

export function normalizeBinding(binding: KeyBinding): KeyBinding | null {
  const code = binding.code === 'NumpadEnter' ? 'Enter' : binding.code;
  if (!/^[A-Z][A-Za-z0-9]+$/.test(code) || code === 'Unidentified') return null;
  const modifiers = normalizeModifiers(binding.modifiers);
  if (modifiers.length !== binding.modifiers.length) return null;
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
  if (!value) return 'shortcut_key';
  if (clipboard && sameBinding(value, clipboard)) return 'shortcut_clipboard';
  const globalTranslation = (candidate: string) => globalTranslationActions.some((entry) => entry === candidate);
  const conflict = shortcutActions.find((other) => other !== action && !(globalTranslation(other) && globalTranslation(action))
    && current[other] && sameBinding(current[other], value));
  return conflict ? `shortcut_conflict:${conflict}` : null;
}

export function bindingFromEvent(event: { code?: string; key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }): KeyBinding | null {
  let code = event.code && event.code !== 'Unidentified' ? event.code : codeFromKey(event.key);
  if (code === 'NumpadEnter') code = 'Enter';
  if (!code || modifierKey(code) && (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey)) return null;
  const modifiers = modifierOrder.filter((modifier) => ({ control: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey })[modifier]);
  return { code, modifiers };
}

export function matchesBinding(binding: KeyBinding | null, event: { code?: string; key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }) {
  const pressed = bindingFromEvent(event);
  return Boolean(binding && pressed && sameBinding(binding, pressed));
}

export function commandKeyLabel(binding: KeyBinding | null, macos: boolean, t: Translate) {
  const value = binding && normalizeBinding(binding);
  if (!value || value.modifiers.length !== 1 || value.modifiers[0] !== 'meta') return;
  const key = keycapLabels(value, macos, t).at(-1);
  return key && [...key].length === 1 ? key : undefined;
}

export function keycapLabels(binding: KeyBinding, macos: boolean, t: Translate) {
  const symbols = macos
    ? { control: '⌃', alt: '⌥', shift: '⇧', meta: '⌘' }
    : { control: 'Ctrl', alt: 'Alt', shift: 'Shift', meta: 'Ctrl' };
  const names: Record<string, string> = { Enter: '↵', Tab: 'Tab', Backspace: macos ? '⌫' : 'Backspace', Space: 'Space', ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\', Semicolon: ';', Quote: '\'', Comma: ',', Period: '.', Slash: '/' };
  const modifier = modifierKey(binding.code);
  const key = (modifier ? `${t(binding.code.endsWith('Left') ? 'shortcutLeft' : 'shortcutRight')} ${symbols[modifier]}` : names[binding.code]) ?? (binding.code.startsWith('Key') ? binding.code.slice(3) : binding.code.startsWith('Digit') ? binding.code.slice(5) : binding.code);
  return [...normalizeModifiers(binding.modifiers).map((modifier) => symbols[modifier]), key];
}

function normalizeModifiers(modifiers: readonly string[]): ShortcutModifier[] {
  return modifierOrder.filter((modifier) => modifiers.includes(modifier));
}

function codeFromKey(key: string) {
  if (/^(Enter|Backspace|Delete|Escape|Tab|Home|End|PageUp|PageDown|Arrow(Up|Down|Left|Right)|F[0-9]+)$/.test(key)) return key;
  if (key === ' ') return 'Space';
  if (key === ',') return 'Comma';
  if (key === '`') return 'Backquote';
  if (/^[a-zA-Z]$/.test(key)) return `Key${key.toUpperCase()}`;
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  return '';
}

export function editsText(event: KeyboardEvent) {
  if (!(event.target instanceof HTMLElement) || !event.target.closest('input, textarea, [contenteditable=true]')) return false;
  if (event.metaKey || event.ctrlKey) {
    return ['KeyA', 'KeyC', 'KeyV', 'KeyX', 'KeyZ'].includes(event.code) && !(event.metaKey && event.ctrlKey) && !event.altKey
      && (!event.shiftKey || event.code === 'KeyZ');
  }
  return event.key.length === 1 || ['Backspace', 'Delete', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'Dead'].includes(event.key);
}

export function modifierKey(code: string): ShortcutModifier | undefined {
  if (!/^(Meta|Control|Alt|Shift)(Left|Right)$/.test(code)) return;
  return code.replace(/Left|Right/, '').toLowerCase() as ShortcutModifier;
}

export function listenForModifierTaps() {
  let pending: { code: string; target: EventTarget | null; started: number } | undefined;
  const down = (event: KeyboardEvent) => {
    if (!event.isTrusted) return;
    const modifier = modifierKey(event.code);
    const held = Number(event.metaKey) + Number(event.ctrlKey) + Number(event.altKey) + Number(event.shiftKey);
    pending = modifier && held === 1 && !event.repeat ? { code: event.code, target: event.target, started: event.timeStamp } : undefined;
  };
  const up = (event: KeyboardEvent) => {
    const tap = pending;
    pending = undefined;
    if (!event.isTrusted || tap?.code !== event.code || tap.target !== event.target || event.timeStamp - tap.started > 500) return;
    event.target?.dispatchEvent(new KeyboardEvent('keydown', { code: event.code, key: event.key, bubbles: true, cancelable: true }));
  };
  const cancel = () => { pending = undefined; };
  window.addEventListener('keydown', down, true);
  window.addEventListener('keyup', up, true);
  window.addEventListener('blur', cancel);
  const interruptions = ['pointerdown', 'wheel', 'copy', 'cut', 'paste', 'beforeinput', 'selectionchange'];
  for (const type of interruptions) window.addEventListener(type, cancel, true);
  return () => {
    window.removeEventListener('keydown', down, true);
    window.removeEventListener('keyup', up, true);
    window.removeEventListener('blur', cancel);
    for (const type of interruptions) window.removeEventListener(type, cancel, true);
  };
}
