// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount } from './harness';
import type { TranslationOutput } from '../../src/shared/contracts';

const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
const result = (text: string, id = 'translation-runtime'): TranslationOutput => ({
  id,
  text,
  sourceLanguage: 'auto',
  translation: { requestedSource: 'auto', engine: 'edge', strategyVersion: 1, detections: [] },
  targetLanguages: ['en', 'zh-CN'],
  translations: [`Translated ${text}`, 'Second result'],
});
const submit = (text: string) => {
  fireEvent.change(source(), { target: { value: text } });
  fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
};
function deferred() {
  let resolve!: (value: TranslationOutput) => void;
  const promise = new Promise<TranslationOutput>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('live translation bridge', () => {
  it('renders translated output and copies it through standalone scope', async () => {
    await mount(); submit('Synthetic input');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(calls('translation_copy').at(-1)?.[1]).toEqual({ scope: 'standalone', part: 'primary' });
    expect(calls('translation_run').at(-1)?.[1]).toEqual({ text: 'Synthetic input', source: 'auto', targets: ['en', 'zh-CN'], scope: 'standalone' });
  });

  it('stops responding to a disabled translation shortcut and resumes with the same keys', async () => {
    await mount();
    const preferences = bridge.snapshot.translationPreferences;
    await emit('translation_preferences_changed', { ...preferences, disabledShortcuts: ['translate'] });
    submit('Synthetic input');
    expect(calls('translation_run')).toHaveLength(0);
    await emit('translation_preferences_changed', preferences);
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
  });

  it('immediately retranslates only the changed target with reuseTargetIndex', async () => {
    const user = userEvent.setup(); await mount(); submit('Switch language');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]).toEqual({ text: 'Switch language', source: 'auto', targets: ['de', 'zh-CN'], scope: 'standalone', reuseTargetIndex: 0 });
  });

  it('keeps the other column visible while a target switch is pending', async () => {
    const user = userEvent.setup(); await mount(); submit('Keep other column');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    const pending = deferred(); const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? pending.promise : invoke(name, args));
    await user.click(screen.getByRole('button', { name: 'Secondary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1');
    expect(screen.getByRole('region', { name: 'Secondary language' }).textContent).toBe('');
    expect(screen.getByRole('progressbar')).toBeTruthy();
    await act(async () => pending.resolve({ ...result('Keep other column'), targetLanguages: ['en', 'fr'], translations: ['Synthetic translation 1', 'French result'] }));
    expect(screen.getByRole('region', { name: 'Secondary language' }).textContent).toBe('French result');
  });

  it('retranslates both columns when the source language changes', async () => {
    const user = userEvent.setup(); await mount(); submit('Source switch');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Source language: Auto-Detect' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]?.source).toBe('de');
    expect(calls('translation_run')[1][1]).not.toHaveProperty('reuseTargetIndex');
  });

  it('does not request for an empty input or the same chosen language', async () => {
    const user = userEvent.setup(); await mount();
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    expect(calls('translation_run')).toHaveLength(0);
    submit('Same language'); await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Primary language: German' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    expect(calls('translation_run')).toHaveLength(1);
  });

  it('handles disabling and enabling the secondary column without losing the primary result', async () => {
    const user = userEvent.setup(); await mount(); submit('Column changes');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Secondary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'None' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]?.targets).toEqual(['en']);
    await user.click(screen.getByRole('button', { name: 'Secondary language: None' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(3));
    expect(calls('translation_run')[2][1]?.targets).toEqual(['en', 'fr']);
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1');
  });

  it('consumes selected text on first connection and translates automatically', async () => {
    let context: unknown = { text: 'Synthetic selection', error: null };
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => {
      if (name === 'selection_take') { const next = context; context = null; return Promise.resolve(next); }
      return invoke(name, args);
    });
    await mount();
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(source().value).toBe('Synthetic selection');
    expect(calls('translation_run')[0][1]).toEqual({ text: 'Synthetic selection', source: 'auto', targets: ['en', 'zh-CN'], scope: 'standalone' });
    await emit('selection_ready', null);
    expect(calls('translation_run')).toHaveLength(1);
  });

  it('opens a blank draft without a network request when there is no selection', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    await mount(); fireEvent.change(source(), { target: { value: 'Prior draft' } });
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'selection_take' ? Promise.resolve({ text: '', error: null }) : invoke(name, args));
    await emit('selection_ready', null);
    expect(source().value).toBe(''); expect(calls('translation_run')).toHaveLength(0);
  });

  it('commits a blank input and cleared results before presenting a new selection summon', async () => {
    await mount(); submit('Previous selected text');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => {
      if (name === 'selection_take') return Promise.resolve({ text: '', error: null, presentationId: 7 });
      if (name === 'selection_present') {
        expect(source().value).toBe('');
        expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('');
        expect(screen.getByRole('region', { name: 'Secondary language' }).textContent).toBe('');
      }
      return invoke(name, args);
    });
    await emit('selection_ready', null);
    expect(calls('selection_present').at(-1)?.[1]).toEqual({ presentationId: 7 });
    expect(calls('translation_run')).toHaveLength(1);
  });

  it('ignores an older selection reply that arrives after the newer summon', async () => {
    await mount();
    let resolveOld!: (value: unknown) => void;
    const old = new Promise((resolve) => { resolveOld = resolve; });
    const invoke = bridge.invoke.getMockImplementation()!;
    let takes = 0;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => {
      if (name === 'selection_take') return ++takes === 1 ? old : Promise.resolve({ text: 'New selection', error: null, presentationId: 9 });
      return invoke(name, args);
    });
    await emit('selection_ready', null);
    await emit('selection_ready', null);
    await waitFor(() => expect(calls('selection_present')).toHaveLength(1));
    await act(async () => resolveOld({ text: 'Old selection', error: null, presentationId: 8 }));
    expect(source().value).toBe('New selection');
    expect(calls('selection_present')[0][1]).toEqual({ presentationId: 9 });
    expect(calls('translation_run')).toHaveLength(1);
  });
});

describe('translation engine settings', () => {
  it('lists languages supported by the selected engine without silently replacing an unsupported target', async () => {
    bridge.snapshot.translationEngine = 'google';
    bridge.snapshot.translationPreferences.engine = 'google';
    bridge.snapshot.translationPreferences.primaryLanguage = 'eo';
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: /^Translation$/ }));
    expect(screen.getByRole('button', { name: 'Primary Language: Esperanto' })).toBeTruthy();
    await user.click(screen.getByRole('button', { name: 'Translation Service: Google Translate' }));
    await user.click(screen.getByRole('option', { name: 'Microsoft Edge Translate' }));
    expect(screen.getByRole('button', { name: 'Primary Language: Esperanto (unavailable)' })).toBeTruthy();
    expect(screen.getByRole('status').textContent).toContain('not supported');
    expect(calls('translation_preferences_set').at(-1)?.[1]).toEqual({ engine: 'edge' });
    expect(bridge.snapshot.translationPreferences.primaryLanguage).toBe('eo');
    await user.click(screen.getByRole('button', { name: 'Primary Language: Esperanto (unavailable)' }));
    expect(screen.queryByRole('option', { name: 'Esperanto' })).toBeNull();
    expect(screen.getByRole('option', { name: 'English' })).toBeTruthy();
  });

  it('offers only Edge and Google and sends the chosen engine to the backend', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: /^Translation$/ }));
    await user.click(screen.getByRole('button', { name: 'Translation Service: Microsoft Edge Translate' }));
    expect(screen.getAllByRole('option').map((item) => item.textContent)).toEqual(['Microsoft Edge Translate', 'Google Translate']);
    await user.click(screen.getByRole('option', { name: 'Google Translate' }));
    expect(calls('translation_preferences_set').at(-1)?.[1]).toEqual({ engine: 'google' });
  });

  it('shows independent open and autofill shortcuts with the same default', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const group = within(screen.getByRole('group', { name: 'Translation' }));
    const open = group.getByRole<HTMLInputElement>('checkbox', { name: 'Open Translation' });
    const autofill = group.getByRole<HTMLInputElement>('checkbox', { name: 'Autofill Input' });
    const openKey = group.getByRole('button', { name: /^Open Translation Shortcut/i });
    const autofillKey = group.getByRole('button', { name: /^Autofill Input Shortcut/i });
    expect(openKey.textContent).toBe('⌘T');
    expect(autofillKey.textContent).toBe('⌘T');
    await user.click(open);
    await waitFor(() => expect(open.checked).toBe(false));
    expect(calls('translation_shortcut_set').at(-1)?.[1]).toEqual({ action: 'openTranslation', enabled: false });
    expect(autofill.checked).toBe(true);
    await user.click(autofillKey);
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', metaKey: true });
    await waitFor(() => expect(autofillKey.textContent).toBe('⌘J'));
    expect(openKey.textContent).toBe('⌘T');
  });

  it('records a local shortcut without turning it on', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const toggle = within(screen.getByRole('group', { name: 'Translation' })).getByRole<HTMLInputElement>('checkbox', { name: 'Translate' });
    const recorder = within(screen.getByRole('group', { name: 'Translation' })).getByRole<HTMLButtonElement>('button', { name: 'Translate shortcut' });
    await user.click(toggle);
    expect(calls('translation_shortcut_set').at(-1)?.[1]).toEqual({ action: 'translate', enabled: false });
    expect(recorder.disabled).toBe(false);
    expect(recorder.textContent).toBe('⌘↵');
    await user.click(recorder);
    expect(recorder.textContent).toBe('Press shortcut');
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', metaKey: true });
    await waitFor(() => expect(recorder.textContent).toBe('⌘J'));
    expect(toggle.checked).toBe(false);
    await user.click(toggle);
    expect(toggle.checked).toBe(true);
    expect(recorder.textContent).toBe('⌘J');
  });

  it('records Tab and ends recording when the settings window loses focus', async () => {
    bridge.snapshot.translationPreferences.disabledShortcuts = ['copyPrimary'];
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    await user.click(within(screen.getByRole('group', { name: 'Translation' })).getByRole('button', { name: 'Translate shortcut' }));
    await user.tab();
    await waitFor(() => expect(bridge.snapshot.translationPreferences.shortcuts.translate).toEqual({ code: 'Tab', modifiers: [] }));
    bridge.invoke.mockClear();
    const copy = screen.getByRole('button', { name: 'Copy Primary Translation shortcut' });
    await user.click(copy);
    expect(copy.getAttribute('aria-pressed')).toBe('true');
    expect(calls('translation_shortcut_capture').map(([, args]) => args)).toEqual([{ active: true }]);
    fireEvent(window, new FocusEvent('blur'));
    expect(calls('translation_shortcut_capture').map(([, args]) => args)).toEqual([{ active: true }, { active: false }]);
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', metaKey: true });
    expect(calls('translation_shortcut_set')).toHaveLength(0);
  });

  it('validates the proxy on enable or save and keeps rejected drafts out of the active preferences', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: /^Translation$/ }));
    const input = screen.getByRole<HTMLInputElement>('textbox', { name: 'HTTP Proxy URL' });
    const toggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Use HTTP Proxy' });
    expect(input.closest('.settings-field')).toBe(toggle.closest('.settings-field'));
    await user.click(input);
    expect(input).toBe(document.activeElement);
    expect(calls('translation_preferences_set')).toHaveLength(0);
    fireEvent.change(input, { target: { value: 'http://user:pass@127.0.0.1:7890' } });
    expect((await screen.findByRole('alert')).textContent).toContain('Proxy URLs cannot include a username or password.');
    await user.click(toggle);
    expect(calls('translation_preferences_set')).toHaveLength(0);
    expect(toggle.checked).toBe(false);

    fireEvent.change(input, { target: { value: 'socks5://127.0.0.1:7890' } });
    expect(calls('translation_preferences_set')).toHaveLength(0);
    bridge.invoke.mockRejectedValueOnce('proxy_url');
    await user.click(toggle);
    expect((await screen.findByRole('alert')).textContent).toBe('Enter a valid HTTP or HTTPS proxy URL.');
    expect(toggle.checked).toBe(false);
    expect(input.value).toBe('socks5://127.0.0.1:7890');

    fireEvent.change(input, { target: { value: 'http://127.0.0.1:7890' } });
    expect(calls('translation_preferences_set')).toHaveLength(1);
    await user.click(toggle);
    await waitFor(() => expect(toggle.checked).toBe(true));
    expect(calls('translation_preferences_set').at(-1)?.[1]).toEqual({ proxyUrl: 'http://127.0.0.1:7890', proxyEnabled: true });
    fireEvent.change(input, { target: { value: 'https://example.test:8443' } });
    input.focus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(bridge.snapshot.translationPreferences.proxyUrl).toBe('https://example.test:8443'));
    expect(calls('translation_preferences_set').at(-1)?.[1]).toEqual({ proxyUrl: 'https://example.test:8443' });
    expect(document.activeElement).toBe(input);

    fireEvent.change(input, { target: { value: 'https://other.example.test:8443' } });
    bridge.invoke.mockRejectedValueOnce('storage_io');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect((await screen.findByRole('alert')).textContent).toContain('Could not save proxy settings.');
    expect(toggle.checked).toBe(true);
    expect(bridge.snapshot.translationPreferences.proxyUrl).toBe('https://example.test:8443');
    expect(input.value).toBe('https://other.example.test:8443');
    await user.keyboard('{Escape}');
    expect(input.value).toBe('https://example.test:8443');
    expect(screen.queryByRole('alert')).toBeNull();

    await user.click(screen.getByText('Use HTTP Proxy', { exact: true }));
    await waitFor(() => expect(toggle.checked).toBe(false));
    expect(calls('translation_preferences_set').at(-1)?.[1]).toEqual({ proxyEnabled: false });
    expect(input.value).toBe('https://example.test:8443');
    expect(input.readOnly).toBe(false);
  });

  it('rebinds a shortcut without taking the clipboard shortcut and hides translation history controls', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: /^Translation$/ }));
    expect(screen.getByRole('heading', { name: 'Translation' })).toBeTruthy();
    for (const name of ['Languages', 'Connection']) expect(screen.getByRole('region', { name })).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'History & Storage' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Actions shortcut' })).toBeNull();
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    expect(screen.queryByRole('button', { name: 'Open Translation History shortcut' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Switch Record View shortcut' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Copy Primary Translation shortcut' }));
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', metaKey: true });
    await waitFor(() => expect(calls('translation_shortcut_set').at(-1)?.[1]).toEqual({ action: 'copyPrimary', binding: { code: 'KeyJ', modifiers: ['meta'] } }));
    fireEvent.click(within(screen.getByRole('group', { name: 'Translation' })).getByRole('button', { name: 'Translate shortcut' }));
    fireEvent.keyDown(window, { key: '`', code: 'Backquote', ctrlKey: true });
    expect((await screen.findByRole('alertdialog')).textContent).toContain('Already assigned to Open Clipboard.');
    expect(document.activeElement?.textContent).toBe('OK');
    fireEvent.click(screen.getByRole('button', { name: 'OK' }));
    fireEvent.click(within(screen.getByRole('region', { name: 'Translation' })).getByRole('button', { name: 'Restore Default Shortcuts' }));
    await waitFor(() => expect(calls('translation_shortcuts_restore')).toHaveLength(1));
  });
});
