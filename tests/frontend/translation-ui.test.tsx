// @vitest-environment jsdom
import { act, fireEvent, screen, within, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, flushFrames, mount, windowState } from './harness';
import { historySamples } from '../../src/translation/history';

const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
const sample = historySamples[0];
const select = (id = sample.id, retranslate = false) => act(async () => { await bridge.invoke('history_sample_select', { sampleId: id, retranslate }); });

// These tests exercise DOM behavior and the native bridge contract. AppKit geometry,
// glass rendering and frame pacing are covered by the separate native checks.
describe('translation input and shortcuts', () => {
  it('makes panel whitespace draggable while preserving result text selection', async () => {
    await mount();
    expect(document.querySelector('.translation-panel')?.getAttribute('data-tauri-drag-region')).toBe('deep');
    for (const result of document.querySelectorAll('.result-column')) expect(result.getAttribute('data-tauri-drag-region')).toBe('deep');
    expect(source().hasAttribute('data-tauri-drag-region')).toBe(false);
    expect(screen.getByRole('button', { name: 'Translation history' }).hasAttribute('data-tauri-drag-region')).toBe(false);
    await emit('history_sample_selected', { sampleId: sample.id, retranslate: false });
    for (const text of document.querySelectorAll('.result-column > span')) expect(text.getAttribute('data-tauri-drag-region')).toBe('false');
    expect(document.querySelectorAll('.result-column > span')).toHaveLength(2);
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

  it('keeps DOM controls available until native installation is acknowledged, then routes native actions', async () => {
    await mount();
    expect(screen.getByRole('button', { name: 'Source language: Auto-Detect' })).toBeTruthy();
    expect(calls('translation_chrome_update').at(-1)?.[1]).toEqual({ state: {
      sourceLabel: 'Auto-Detect', sourceWidth: 140, translateLabel: 'Translate', actionsLabel: 'Actions', canTranslate: false, actionsVisible: false,
    } });
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'translation_chrome_update' ? true : invoke(name, args));
    fireEvent.change(source(), { target: { value: 'Native control test' } });
    await waitFor(() => expect(document.querySelector('.translation-panel')?.getAttribute('data-native-chrome')).toBe('true'));
    expect(screen.queryByRole('button', { name: 'Source language: Auto-Detect' })).toBeNull();
    expect(screen.queryByRole('button', { name: /^Actions/ })).toBeNull();
    await emit('translation_chrome_action', 'source');
    expect(screen.getByRole('dialog', { name: 'Source language' })).toBeTruthy();
    await emit('translation_chrome_action', 'actions');
    expect(screen.getByRole('menu', { name: 'Actions' })).toBeTruthy();
    await emit('translation_chrome_action', 'translate');
    expect(screen.queryByRole('menu')).toBeNull();
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(source().value).toBe('Native control test');
    expect(calls('translation_chrome_update').at(-1)?.[1]?.state.canTranslate).toBe(true);
  });

  it('starts empty without preset text or placeholder and cannot submit an empty draft', async () => {
    await mount();
    expect(source().value).toBe('');
    expect(source().hasAttribute('placeholder')).toBe(false);
    expect((screen.getByRole('button', { name: /^Translate/ }) as HTMLButtonElement).disabled).toBe(true);
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

  it('toggles history through the native command without replacing the draft', async () => {
    const user = userEvent.setup(); await mount();
    await user.type(source(), 'A draft worth keeping');
    const trigger = screen.getByRole('button', { name: 'Translation history' });
    await user.click(trigger); await windowState({ historyVisible: false, historyExtent: 0 });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    await user.click(trigger); await windowState({ historyVisible: true, historyExtent: 360 });
    expect(calls('history_toggle')).toHaveLength(2);
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(source().value).toBe('A draft worth keeping');
  });

  it('dismisses an open menu before hiding the translation and retains source focus', async () => {
    await mount();
    fireEvent.keyDown(source(), { key: 'k', metaKey: true });
    expect(screen.getByRole('menu', { name: 'Actions' })).toBeTruthy();
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(calls('window_hide')).toHaveLength(0);
    expect(document.activeElement).toBe(screen.getByRole('button', { name: /^Actions/ }));
    fireEvent.keyDown(source(), { key: 'Escape' });
    expect(calls('window_hide').at(-1)).toEqual(['window_hide', { region: 'translation' }]);
  });

  it('supports Command-W dismissal and the translation pin shortcut', async () => {
    await mount();
    fireEvent.keyDown(source(), { key: 'p', metaKey: true, shiftKey: true });
    expect(calls('window_pin').at(-1)).toEqual(['window_pin', { region: 'translation', pinned: true }]);
    await windowState({ translationPinned: true });
    fireEvent.keyDown(source(), { key: 'p', metaKey: true, shiftKey: true });
    expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'translation', pinned: false });
    fireEvent.keyDown(source(), { key: 'w', metaKey: true });
    expect(calls('window_hide').at(-1)).toEqual(['window_hide', { region: 'translation' }]);
  });
});

describe('selection across native webviews', () => {
  it('restores the selected source, paired cached translations and language controls', async () => {
    await mount(); await select();
    expect(source().value).toBe(sample.text);
    expect(screen.getByRole('button', { name: 'Source language: English' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Primary language: Chinese (Simplified)' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Secondary language: French' })).toBeTruthy();
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe(sample.translations[0]);
    expect(screen.getByRole('region', { name: 'Secondary language' }).textContent).toBe(sample.translations[1]);
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    expect(calls('window_show')).toHaveLength(0);
  });

  it('selecting the same record again hides only translation while retaining history', async () => {
    await mount(); await select(); await select();
    expect(bridge.snapshot.window.translationVisible).toBe(false);
    expect(bridge.snapshot.window.historyVisible).toBe(true);
    expect(calls('window_hide')).toHaveLength(0);
    expect((document.querySelector('.source-input') as HTMLTextAreaElement).value).toBe(sample.text);
  });

  it('leaves collapse and restore authoritative in the backend across rapid selection commands', async () => {
    await mount();
    const visible: boolean[] = [];
    await act(async () => {
      for (let index = 0; index < 3; index += 1) {
        await bridge.invoke('history_sample_select', { sampleId: sample.id, retranslate: false });
        visible.push(bridge.snapshot.window.translationVisible);
      }
    });
    expect(visible).toEqual([true, false, true]);
    expect(source().value).toBe(sample.text);
    expect(calls('window_show')).toHaveLength(0);
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('restores data without toggling windows when selection events arrive before delayed window state', async () => {
    await mount();
    await act(async () => {
      await emit('history_sample_selected', { sampleId: sample.id, retranslate: false, record: sample });
      await emit('history_sample_selected', { sampleId: sample.id, retranslate: false, record: sample });
      await emit('history_sample_selected', { sampleId: sample.id, retranslate: false, record: sample });
    });
    expect(source().value).toBe(sample.text);
    expect(calls('window_show')).toHaveLength(0);
    expect(calls('window_hide')).toHaveLength(0);
    await windowState({ translationVisible: false, translationExtent: 0 });
    expect(calls('window_show')).toHaveLength(0);
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('clears backend selection without changing translation visibility', async () => {
    await mount(); await select(); await select();
    expect(bridge.snapshot.window.translationVisible).toBe(false);
    await act(async () => { await bridge.invoke('history_sample_select', { sampleId: null, retranslate: false }); });
    expect(bridge.history.selectedId).toBeNull();
    expect(bridge.snapshot.window.translationVisible).toBe(false);
    expect(calls('window_show')).toHaveLength(0);
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('keeps a pinned translation visible when selecting the same record again', async () => {
    bridge.snapshot.window.translationPinned = true;
    await mount(); await select(); await select();
    expect(calls('window_hide')).toHaveLength(0);
    expect(calls('window_show')).toHaveLength(0);
    expect(bridge.snapshot.window.translationVisible).toBe(true);
  });

  it('retranslation submits the record through the selected service', async () => {
    await mount(); await select(); await select(sample.id, true);
    expect(screen.queryByText(sample.translations[0])).toBeNull();
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('moves the only history selection to the new record after retranslation', async () => {
    await mount(); const history = await mount('history');
    await select();
    const original = within(history.container).getByRole('button', { name: `Translation: ${sample.text}` });
    expect(original.getAttribute('aria-pressed')).toBe('true');
    await select(sample.id, true);
    await waitFor(() => expect(history.container.querySelector('[data-record-id="translation-test"] .history-row-select')?.getAttribute('aria-pressed')).toBe('true'));
    expect(original.getAttribute('aria-pressed')).toBe('false');
    expect(history.container.querySelectorAll('.history-row.selected')).toHaveLength(1);
  });

  it('selecting a row in the history view restores translation through the shared event bus', async () => {
    const user = userEvent.setup(); const translation = await mount(); const history = await mount('history');
    const button = within(history.container).getByRole('button', { name: `Translation: ${sample.text}` });
    await user.click(button);
    expect((within(translation.container).getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement).value).toBe(sample.text);
    expect(within(translation.container).getByRole('region', { name: 'Primary language' }).textContent).toBe(sample.translations[0]);
    expect(button.getAttribute('aria-pressed')).toBe('true');
    await user.click(button);
    expect(bridge.snapshot.window.translationVisible).toBe(false);
    expect(bridge.snapshot.window.historyVisible).toBe(true);
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('submitting a selected sample clears history selection and cached output', async () => {
    await mount(); await select();
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    expect(calls('history_sample_select').at(-1)).toEqual(['history_sample_select', { sampleId: null, retranslate: false }]);
    expect(screen.queryByText(sample.translations[0])).toBeNull();
    expect(source().value).toBe(sample.text);
  });

  it('a one-target record turns off the second column and permits enabling it through Actions', async () => {
    const user = userEvent.setup(); await mount(); await select('sample-07');
    expect(screen.queryByRole('region', { name: 'Secondary language' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Secondary language: None' })).toBeTruthy();
    expect(calls('translation_columns_set').at(-1)?.[1]).toEqual({ secondaryEnabled: false });
    fireEvent.keyDown(source(), { key: 'k', metaKey: true });
    await user.click(screen.getByRole('menuitem', { name: 'Enable Second Language' }));
    expect(screen.getByRole('region', { name: 'Secondary language' })).toBeTruthy();
    expect(calls('translation_columns_set').at(-1)?.[1]).toEqual({ secondaryEnabled: true });
  });

  it('copies primary and secondary cached results by sample ID, never by arbitrary source text', async () => {
    await mount(); await select();
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    fireEvent.keyDown(source(), { key: '2', metaKey: true });
    expect(calls('translation_sample_copy')).toEqual([
      ['translation_sample_copy', { sampleId: sample.id, part: 'primary' }],
      ['translation_sample_copy', { sampleId: sample.id, part: 'secondary' }],
    ]);
  });

  it('editing source clears selection and cached results in both webviews', async () => {
    const translation = await mount();
    const history = await mount('history');
    await select();
    const selected = within(history.container).getByRole('button', { name: `Translation: ${sample.text}` });
    expect(selected.getAttribute('aria-pressed')).toBe('true');
    const input = within(translation.container).getByRole('textbox', { name: 'Source text' });
    fireEvent.change(input, { target: { value: 'Edited draft' } });
    await act(async () => {});
    expect(calls('history_sample_select').at(-1)).toEqual(['history_sample_select', { sampleId: null, retranslate: false }]);
    expect(selected.getAttribute('aria-pressed')).toBe('false');
    expect(within(translation.container).queryByText(sample.translations[0])).toBeNull();
  });

  it('changing a selected language clears the history selection and cached results', async () => {
    const user = userEvent.setup(); await mount(); await select();
    await user.click(screen.getByRole('button', { name: 'Primary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    expect(calls('history_sample_select').at(-1)).toEqual(['history_sample_select', { sampleId: null, retranslate: false }]);
    expect(screen.queryByText(sample.translations[0])).toBeNull();
  });

  it('clearing selection does not erase the current source or cached text', async () => {
    await mount(); await select(); await emit('history_sample_selected', { sampleId: null, retranslate: false });
    expect(source().value).toBe(sample.text);
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe(sample.translations[0]);
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(calls('translation_sample_copy')).toHaveLength(0);
  });

  it('ignores unknown record IDs instead of replacing the current draft', async () => {
    await mount(); fireEvent.change(source(), { target: { value: 'Private local draft' } });
    await emit('history_sample_selected', { sampleId: 'missing-record', retranslate: false });
    expect(source().value).toBe('Private local draft');
    expect(calls('window_show')).toHaveLength(0);
  });
});

describe('language dropdown and menu keyboard behavior', () => {
  it('searches languages, moves with arrows and chooses the active option with Return', async () => {
    const user = userEvent.setup(); await mount();
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    const search = screen.getByRole('combobox', { name: 'Search languages or codes…' });
    expect(document.activeElement).toBe(search);
    await user.type(search, 'Chinese');
    expect(screen.getAllByRole('option')).toHaveLength(2);
    await user.keyboard('{ArrowDown}{Enter}');
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

  it('moves through Actions and dismisses it with Tab without trapping focus', async () => {
    await mount(); await select();
    fireEvent.keyDown(source(), { key: 'k', metaKey: true });
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    expect(document.activeElement).toBe(screen.getByRole('menuitemcheckbox'));
    fireEvent.keyDown(document.activeElement!, { key: 'Home' });
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: /^Copy Primary Translation/ }));
    fireEvent.keyDown(document.activeElement!, { key: 'Tab' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: /^Actions/ }));
  });
});
