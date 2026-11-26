// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, flushFrames, mount, windowState } from './harness';
// @ts-expect-error Vitest runs in Node; the app's TypeScript configuration includes browser types only.
import { readFileSync } from 'node:fs';

const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;

// These tests exercise DOM behavior and the native bridge contract. AppKit geometry,
// glass rendering and frame pacing are covered by the separate native checks.
describe('translation input and shortcuts', () => {
  it('uses the shared focus binding in the translation window without changing the draft', async () => {
    const preferences = bridge.snapshot.translationPreferences;
    preferences.shortcuts.focusSearch = { code: 'F6', modifiers: [] };
    await mount();
    fireEvent.change(source(), { target: { value: 'Keep this draft' } });
    const result = screen.getByRole('region', { name: 'Primary language' });
    act(() => result.focus());
    expect(fireEvent.keyDown(result, { key: 'f', code: 'KeyF', metaKey: true })).toBe(true);
    expect(fireEvent.keyDown(result, { key: 'F6', code: 'F6', isComposing: true })).toBe(true);
    expect(fireEvent.keyDown(result, { key: 'F6', code: 'F6' })).toBe(false);
    expect(document.activeElement).toBe(source());
    expect(source().value).toBe('Keep this draft');
    await emit('translation_preferences_changed', { ...preferences, disabledShortcuts: ['focusSearch'] });
    act(() => result.focus());
    expect(fireEvent.keyDown(result, { key: 'F6', code: 'F6' })).toBe(true);
    expect(document.activeElement).toBe(result);
  });

  it('does not render the removed history clock trigger in standalone translation', async () => {
    await mount();
    expect(screen.queryByRole('button', { name: 'Translation history' })).toBeNull();
    expect(document.querySelector('.history-trigger')).toBeNull();
  });

  it('dismisses the language picker on hide and preserves the translation draft', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: 'Preserved draft' } });
    fireEvent.click(screen.getByRole('button', { name: 'Source language: Auto-Detect' }));
    expect(screen.getByRole('dialog', { name: 'Source language' })).toBeTruthy();
    await windowState({ standaloneTranslationVisible: false, rightExtent: 0 });
    expect(screen.queryByRole('dialog', { name: 'Source language' })).toBeNull();
    await windowState({ standaloneTranslationVisible: true, rightExtent: 920 });
    expect(source().value).toBe('Preserved draft');
  });

  it('makes panel whitespace draggable while preserving result text selection', async () => {
    await mount();
    expect(document.querySelector('.translation-panel')?.getAttribute('data-tauri-drag-region')).toBe('deep');
    for (const result of document.querySelectorAll('.result-column')) expect(result.getAttribute('data-tauri-drag-region')).toBe('deep');
    expect(source().hasAttribute('data-tauri-drag-region')).toBe(false);
    fireEvent.change(source(), { target: { value: 'Draggable test' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(document.querySelectorAll('.result-column > span')).toHaveLength(2));
    for (const text of document.querySelectorAll('.result-column > span')) expect(text.getAttribute('data-tauri-drag-region')).toBe('false');
  });

  it('scrolls hidden source text while dragging a selection past either edge', async () => {
    await mount();
    const text = 'a'.repeat(80);
    fireEvent.change(source(), { target: { value: text, selectionStart: text.length, selectionEnd: text.length } });
    const field = source();
    field.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: 120, bottom: 32, width: 120, height: 32, toJSON() { return {}; } });
    Object.defineProperty(field, 'clientWidth', { configurable: true, value: 120 });
    Object.defineProperty(field, 'scrollWidth', { configurable: true, value: text.length * 6 });
    field.setSelectionRange(8, 8);
    fireEvent.pointerDown(field, { button: 0, clientX: 60, pointerId: 1, detail: 1 });
    await flushFrames();
    expect(field.scrollLeft).toBe(0);
    expect([field.selectionStart, field.selectionEnd]).toEqual([8, 8]);
    fireEvent.pointerMove(field, { clientX: 200, pointerId: 1 });
    await flushFrames();
    expect(field.scrollLeft).toBeGreaterThan(0);
    expect(field.selectionEnd).toBeGreaterThan(8);
    const forward = field.scrollLeft;
    fireEvent.pointerMove(field, { clientX: -40, pointerId: 1 });
    await flushFrames();
    expect(field.scrollLeft).toBeLessThan(forward);
    expect(field.selectionStart).toBeLessThan(8);
    fireEvent.pointerUp(field, { pointerId: 1, button: 0 });
    const stopped = field.scrollLeft;
    await flushFrames();
    expect(field.scrollLeft).toBe(stopped);
  });

  it('does not rewind the caret after typing or overwrite a newer selection', async () => {
    const user = userEvent.setup(); await mount();
    await user.type(source(), 'abc');
    source().setSelectionRange(0, 0);
    await flushFrames();
    expect(source().selectionStart).toBe(0);
    await user.keyboard('x');
    expect(source().value).toBe('xabc');
    source().setSelectionRange(1, 3);
    await flushFrames();
    expect([source().selectionStart, source().selectionEnd]).toEqual([1, 3]);
  });

  it('normalizes pasted line breaks synchronously and retains the original translation text', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: 'first\nsecond', selectionStart: 8, selectionEnd: 8 } });
    expect(source().value).toBe('first second');
    expect(source().selectionStart).toBe(8);
    source().setSelectionRange(2, 2);
    await flushFrames();
    expect(source().selectionStart).toBe(2);
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(calls('translation_run')[0][1]?.text).toBe('first\nsecond');
  });

  it('does not collapse the active IME selection on the next frame', async () => {
    await mount();
    fireEvent.compositionStart(source());
    fireEvent.change(source(), { target: { value: '中文', selectionStart: 2, selectionEnd: 2 } });
    source().setSelectionRange(0, 2);
    await flushFrames();
    expect([source().selectionStart, source().selectionEnd]).toEqual([0, 2]);
    fireEvent.compositionEnd(source());
  });

  it('displays the detected language while preserving automatic requests for new input', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      const result = await original(name, args);
      if (name !== 'translation_run' || !result || typeof result !== 'object') return result;
      return { ...result, sourceLanguage: 'ja' };
    });
    await mount();
    fireEvent.change(source(), { target: { value: 'こんにちは' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Source language: Japanese' }).getAttribute('title')).toBe('Detected: Japanese'));
    expect(calls('translation_chrome_update').at(-1)?.[1]?.state.sourceLabel).toBe('Japanese');
    fireEvent.change(source(), { target: { value: 'Bonjour le monde' } });
    expect(screen.getByRole('button', { name: 'Source language: Auto-Detect' }).getAttribute('title')).toBeNull();
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]?.source).toBe('auto');
  });

  it('keeps an explicit source even if the service responds with a different detection', async () => {
    bridge.snapshot.translationPreferences.sourceLanguage = 'fr';
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      const result = await original(name, args);
      return name === 'translation_run' ? { ...result, sourceLanguage: 'en' } : result;
    });
    await mount();
    fireEvent.change(source(), { target: { value: 'Bonjour' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(screen.getByRole('button', { name: 'Source language: French' })).toBeTruthy();
  });

  it('keeps the DOM source picker until native installation is acknowledged, with translation available by shortcut only', async () => {
    let installed!: (ready: boolean) => void;
    const ready = new Promise<boolean>((resolve) => { installed = resolve; });
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_chrome_update' ? ready : invoke(name, args));
    await mount();
    expect(screen.getByRole('button', { name: 'Source language: Auto-Detect' })).toBeTruthy();
    expect(calls('translation_chrome_update').at(-1)?.[1]).toEqual({ state: { sourceLabel: 'Auto-Detect', sourceWidth: 140 } });
    installed(true);
    await waitFor(() => expect(document.querySelector('.translation-panel')?.getAttribute('data-native-chrome')).toBe('true'));
    expect(screen.queryByRole('button', { name: 'Source language: Auto-Detect' })).toBeNull();
    expect(screen.queryByRole('button', { name: /^Actions/ })).toBeNull();
    fireEvent.change(source(), { target: { value: 'Native control test' } });
    expect(screen.queryByRole('button', { name: 'Translate' })).toBeNull();
    await emit('translation_chrome_action', 'translate');
    expect(calls('translation_run')).toHaveLength(0);
    await emit('translation_chrome_action', 'source');
    expect(screen.getByRole('dialog', { name: 'Source language' })).toBeTruthy();
    fireEvent.keyDown(source(), { key: 'Escape' });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(source().value).toBe('Native control test');
  });

  it('starts empty without preset text or placeholder and cannot submit an empty draft', async () => {
    await mount();
    expect(source().value).toBe('');
    expect(source().hasAttribute('placeholder')).toBe(false);
    expect(screen.queryByRole('button', { name: 'Translate' })).toBeNull();
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByRole('complementary', { name: 'Translation history' })).toBeNull();
  });

  it('keeps plain Return in the single-line editor from submitting', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: 'A deliberate draft' } });
    expect(fireEvent.keyDown(source(), { key: 'Enter' })).toBe(false);
    expect(source().value).toBe('A deliberate draft');
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('submits a nonempty draft with Command-Return and preserves its text', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: 'A deliberate draft' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(source().value).toBe('A deliberate draft');
    expect(document.activeElement).toBe(source());
  });

  it('ignores submit shortcuts during IME composition and accepts them after composition ends', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: '中文草稿' } });
    fireEvent.compositionStart(source());
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true, isComposing: true });
    expect(screen.queryByRole('status')).toBeNull();
    fireEvent.compositionEnd(source());
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
  });

  it('ignores the keyCode 229 IME fallback', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: 'Draft' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true, keyCode: 229 });
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('reports an over-limit source instead of treating it as a submitted translation', async () => {
    await mount(); fireEvent.change(source(), { target: { value: 'x'.repeat(5001) } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    expect(screen.getAllByRole('status')[0].textContent).toContain('Input limit:');
    expect(source().value).toHaveLength(5001);
  });

  it('dismisses the language picker before hiding the translation and restores trigger focus', async () => {
    await mount();
    fireEvent.click(screen.getByRole('button', { name: 'Source language: Auto-Detect' }));
    fireEvent.keyDown(screen.getByRole('dialog', { name: 'Source language' }), { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(calls('window_hide')).toHaveLength(0);
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Source language: Auto-Detect' }));
    fireEvent.keyDown(source(), { key: 'Escape' });
    expect(calls('window_hide').at(-1)).toEqual(['window_hide', { region: 'translation' }]);
  });

  it('supports Command-W dismissal and the translation pin shortcut', async () => {
    await mount();
    fireEvent.keyDown(source(), { key: 'p', metaKey: true });
    expect(calls('window_pin').at(-1)).toEqual(['window_pin', { region: 'translation', pinned: true }]);
    await windowState({ translationPinned: true });
    fireEvent.keyDown(source(), { key: 'p', metaKey: true });
    expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'translation', pinned: false });
    fireEvent.keyDown(source(), { key: 'w', metaKey: true });
    expect(calls('window_hide').at(-1)).toEqual(['window_hide', { region: 'translation' }]);
  });

  it('copies primary and secondary results via standalone translation_copy', async () => {
    await mount();
    fireEvent.change(source(), { target: { value: 'Copy shortcuts test' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    fireEvent.keyDown(source(), { key: '2', metaKey: true });
    expect(calls('translation_copy')).toEqual([
      ['translation_copy', { scope: 'standalone', part: 'primary' }],
      ['translation_copy', { scope: 'standalone', part: 'secondary' }],
    ]);
  });

  it('copies the primary result with the secondary shortcut for a single-target translation', async () => {
    bridge.snapshot.translationPreferences.secondaryLanguage = 'none';
    await mount();
    fireEvent.change(source(), { target: { value: 'Single target copy' } });
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    fireEvent.keyDown(source(), { key: '2', code: 'Digit2', metaKey: true });
    expect(calls('translation_copy').at(-1)?.[1]).toEqual({ scope: 'standalone', part: 'primary' });
  });
});

describe('language dropdown and menu keyboard behavior', () => {
  it.each(['choose', 'Escape', 'Tab'])('restores source input after %s from the native source language picker', async (action) => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'translation_chrome_update' ? true : original(name, args));
    const user = userEvent.setup(); await mount();
    await waitFor(() => expect(document.querySelector('.translation-panel')?.getAttribute('data-native-chrome')).toBe('true'));
    await emit('translation_chrome_action', 'source');
    expect(document.activeElement).toBe(screen.getByRole('combobox'));
    if (action === 'choose') {
      await user.type(screen.getByRole('combobox'), 'French');
      await user.keyboard('{Enter}');
    } else await user.keyboard(`{${action}}`);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(source());
  });

  it('does not restore the language trigger over an outside button receiving focus', async () => {
    const user = userEvent.setup(); await mount();
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    const outside = document.createElement('button');
    outside.textContent = 'External target'; document.body.append(outside);
    try {
      await user.click(outside); await flushFrames();
      expect(screen.queryByRole('dialog')).toBeNull();
      expect(document.activeElement).toBe(outside);
    } finally { outside.remove(); }
  });

  it('allows long language names to wrap without losing the checkmark or viewport bounds', async () => {
    const dropdownStyles = readFileSync('src/styles.css', 'utf8') as string;
    const rules = ['.language-options button {', '.language-options button span {', '.language-options button svg {'];
    const style = document.createElement('style');
    style.textContent = rules.map((selector) => selector + dropdownStyles.split(selector)[1].split('}')[0] + '}').join('\n');
    document.head.append(style);
    try {
      const user = userEvent.setup(); await mount();
      await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
      const longName = screen.getByRole('option', { name: 'Chinese (Traditional)' });
      const row = getComputedStyle(longName), text = getComputedStyle(longName.querySelector('span')!);
      expect(row.minHeight).toBe('40px');
      expect(row.height).not.toBe('40px');
      expect(text.whiteSpace).toBe('normal');
      expect(text.overflowWrap).toBe('anywhere');
      expect(text.textOverflow).not.toBe('ellipsis');
      expect(screen.getByRole('option', { name: 'English' }).querySelector('svg')).not.toBeNull();
      const panel = screen.getByRole('dialog');
      expect(Number.parseFloat(panel.style.maxWidth)).toBeLessThanOrEqual(innerWidth - 16);
    } finally { style.remove(); }
  });

  it('searches languages, moves with arrows and chooses the active option with Return', async () => {
    const user = userEvent.setup(); await mount();
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    const search = screen.getByRole('combobox', { name: 'Search languages or codes…' });
    expect(document.activeElement).toBe(search);
    await user.type(search, 'Chinese');
    expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual(['Chinese (Literary)', 'Chinese (Simplified)', 'Chinese (Traditional)']);
    await user.keyboard('{ArrowDown}{ArrowDown}{Enter}');
    const trigger = screen.getByRole('button', { name: 'Primary language: Chinese (Traditional)' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it('Escape restores the dropdown trigger and a fresh opening clears the search', async () => {
    const user = userEvent.setup(); await mount();
    const trigger = screen.getByRole('button', { name: 'Source language: Auto-Detect' });
    await user.click(trigger); await user.type(screen.getByRole('combobox'), 'French');
    await user.keyboard('{Escape}');
    expect(document.activeElement).toBe(trigger);
    expect(screen.queryByRole('dialog')).toBeNull();
    await user.click(trigger);
    expect((screen.getByRole('combobox') as HTMLInputElement).value).toBe('');
    expect(screen.getAllByRole('option').length).toBeGreaterThan(10);
  });

  it('does not select a language while composing or when search has no matches', async () => {
    const user = userEvent.setup(); await mount();
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    const search = screen.getByRole('combobox');
    fireEvent.keyDown(search, { key: 'Enter', isComposing: true });
    expect(screen.getByRole('dialog')).toBeTruthy();
    await user.type(search, 'not-a-language'); await user.keyboard('{Enter}');
    expect(screen.getByText('No matching records')).toBeTruthy();
    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Primary language: English' })).toBeTruthy();
  });

  it('Command-Return in a dropdown submits the draft without selecting its active option', async () => {
    const user = userEvent.setup(); await mount();
    fireEvent.change(source(), { target: { value: 'Ready to submit' } });
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    await user.type(screen.getByRole('combobox'), 'French');
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter', metaKey: true });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button', { name: 'Primary language: English' })).toBeTruthy();
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
  });
});
