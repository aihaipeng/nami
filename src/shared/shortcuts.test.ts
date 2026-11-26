import { describe, expect, it } from 'vitest';
import { defaultTranslationShortcuts, matchesBinding, shortcutError } from './shortcuts';

describe('translation shortcuts', () => {
  const shortcuts = defaultTranslationShortcuts('meta');

  it('keeps the current translation bindings as defaults', () => {
    expect(shortcuts.openTranslation).toEqual(shortcuts.selection);
    expect(shortcutError('openTranslation', shortcuts.selection, shortcuts)).toBeNull();
    expect(shortcuts.selection).toEqual({ code: 'KeyT', modifiers: ['meta'] });
    expect(shortcuts.showShortcuts).toEqual({ code: 'KeyK', modifiers: ['meta'] });
    expect(shortcuts.translate).toEqual({ code: 'Enter', modifiers: ['meta'] });
    expect(shortcuts.pin).toEqual({ code: 'KeyP', modifiers: ['meta'] });
    expect(defaultTranslationShortcuts('control').copyPrimary).toEqual({ code: 'Digit1', modifiers: ['control'] });
  });

  it('matches a keyboard event and rejects reserved, clipboard, and duplicate bindings', () => {
    expect(matchesBinding(shortcuts.translate, { key: 'Enter', metaKey: true, ctrlKey: false, altKey: false, shiftKey: false })).toBe(true);
    expect(matchesBinding(shortcuts.pin, { key: 'p', code: 'KeyP', metaKey: true, ctrlKey: false, altKey: false, shiftKey: false })).toBe(true);
    expect(shortcutError('translate', { code: 'KeyW', modifiers: ['meta'] }, shortcuts)).toBeNull();
    expect(shortcutError('translate', { code: 'KeyZ', modifiers: ['meta', 'shift'] }, shortcuts)).toBeNull();
    expect(shortcutError('copyPrimary', { code: 'Backquote', modifiers: ['control'] }, shortcuts)).toBe('shortcut_clipboard');
    expect(shortcutError('pin', { code: 'Enter', modifiers: ['meta'] }, shortcuts)).toBe('shortcut_conflict:translate');
    expect(shortcutError('translate', { code: 'KeyP', modifiers: ['shift'] }, shortcuts)).toBeNull();
    expect(shortcutError('copyPrimary', null, shortcuts)).toBeNull();
  });
});
