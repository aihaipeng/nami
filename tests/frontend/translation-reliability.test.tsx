// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, mount } from './harness';
import { historySamples } from '../../src/translation/history';

const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
const result = (text: string, id = 'translation-runtime') => ({ ...historySamples[0], id, text, translations: [`Translated ${text}`, 'Second result'] });
const submit = (text: string) => {
  fireEvent.change(source(), { target: { value: text } });
  fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
};
function deferred() {
  let resolve!: (value: ReturnType<typeof result>) => void;
  const promise = new Promise<ReturnType<typeof result>>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('translation reliability', () => {
  it('reports one task error for a failed dual-target request', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? Promise.reject('Service unavailable') : invoke(name, args));
    await mount(); submit('Fail both targets');
    await waitFor(() => expect(screen.getAllByRole('alert').map((value) => value.textContent)).toEqual(['Service unavailable']));
    expect(calls('translation_run')).toHaveLength(1);
  });

  it('confirms copying a result locally and ignores completion after the draft changes', async () => {
    await mount(); submit('Text for copy');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      await act(async () => { fireEvent.keyDown(source(), { key: '1', metaKey: true }); });
      expect(calls('translation_sample_copy').at(-1)?.[1]).toEqual({ sampleId: 'translation-test', part: 'primary' });
      expect(screen.getByText('Primary translation copied').getAttribute('role')).toBe('status');
      expect(screen.queryByRole('progressbar')).toBeNull();
      act(() => vi.advanceTimersByTime(1000));
      expect(screen.queryByText(/translation copied/)).toBeNull();
      const invoke = bridge.invoke.getMockImplementation()!;
      let finish!: () => void;
      bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_sample_copy' ? new Promise<void>((resolve) => { finish = resolve; }) : invoke(name, args));
      fireEvent.keyDown(source(), { key: '2', metaKey: true });
      fireEvent.change(source(), { target: { value: 'Different draft' } });
      await act(async () => finish());
      expect(screen.queryByText(/translation copied/)).toBeNull();
    } finally { vi.useRealTimers(); }
  });

  it('keeps an active translation running while browsing history and does not replace the browsed record on completion', async () => {
    const pending = deferred(); const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? pending.promise : original(name, args));
    await mount(); submit('Background image text');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    const cancellations = calls('translation_cancel').length;
    await emit('history_sample_selected', { sampleId: historySamples[1].id, record: historySamples[1], retranslate: false });
    expect(source().value).toBe(historySamples[1].text);
    expect(calls('translation_cancel')).toHaveLength(cancellations);
    await act(async () => pending.resolve(result('Background image text')));
    expect(source().value).toBe(historySamples[1].text);
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe(historySamples[1].translations[0]);
  });

  it('uses the latest languages after rapid switches and ignores a late response', async () => {
    const user = userEvent.setup(); await mount(); submit('Rapid switches');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    const old = deferred(); const latest = deferred(); const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: { targets: string[] }) => name === 'translation_run' ? (args?.targets[0] === 'de' ? old.promise : latest.promise) : invoke(name, args));
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    await user.click(screen.getByRole('button', { name: 'Primary language: German' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(3));
    await act(async () => latest.resolve(result('Newest')));
    await act(async () => old.resolve(result('Outdated')));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Translated Newest');
    expect(calls('translation_run')[2][1]?.targets).toEqual(['fr', 'zh-CN']);
  });

  it('keeps the unchanged result and creates no record when the changed target fails', async () => {
    const user = userEvent.setup(); await mount(); submit('Failure switch');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    const count = bridge.history.records.length; const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? Promise.reject('Service unavailable') : invoke(name, args));
    await user.click(screen.getByRole('button', { name: 'Secondary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('Service unavailable'));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1');
    expect(bridge.history.records).toHaveLength(count);
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('keeps the app available and blocks translation when history storage cannot open', async () => {
    bridge.snapshot.storageError = 'storage_version';
    await mount(); submit('Do not send when storage is unavailable');
    expect(screen.getByRole('alert').textContent).toContain('Existing files are preserved');
    expect((screen.getByRole('button', { name: /^Translate/ }) as HTMLButtonElement).disabled).toBe(true);
    expect(calls('translation_run')).toHaveLength(0);
    await emit('translation_chrome_action', 'translate');
    expect(calls('translation_run')).toHaveLength(0);
  });

  it('blocks new translations when storage corruption is reported after startup', async () => {
    await mount();
    await emit('storage_error','storage_corrupt');
    submit('Do not send after corruption');
    expect(screen.getByRole('alert').textContent).toContain('Existing files are preserved');
    expect(calls('translation_run')).toHaveLength(0);
  });

  it('uses animation while waiting and discards results after the draft changes', async () => {
    const pending = deferred(); const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? pending.promise : invoke(name, args));
    await mount(); submit('Old input');
    await waitFor(() => expect(screen.getByRole('progressbar')).toBeTruthy());
    expect(screen.queryByRole('status')).toBeNull();
    fireEvent.change(source(), { target: { value: 'New draft' } });
    await act(async () => pending.resolve(result('Old input')));
    expect(source().value).toBe('New draft');
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('');
    expect(screen.queryByRole('progressbar')).toBeNull();
    expect(calls('translation_cancel').length).toBeGreaterThan(0);
  });

  it('keeps the latest result when network responses finish in reverse order', async () => {
    const old = deferred(); const latest = deferred(); const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: { text: string }) => name === 'translation_run' ? (args?.text === 'Old' ? old.promise : latest.promise) : invoke(name, args));
    await mount(); submit('Old'); await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    submit('Latest'); await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    await act(async () => latest.resolve(result('Latest')));
    await act(async () => old.resolve(result('Old')));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Translated Latest');
  });

  it('cancels pending output when the engine changes', async () => {
    const pending = deferred(); const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? pending.promise : invoke(name, args));
    await mount(); submit('Old engine'); await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await emit('translation_engine_changed', 'google');
    await act(async () => pending.resolve(result('Old engine')));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('');
  });

  it('preserves unsupported selections on an engine switch and blocks both DOM and native submission', async () => {
    bridge.snapshot.translationEngine = 'google';
    bridge.snapshot.translationPreferences.engine = 'google';
    bridge.snapshot.translationPreferences.primaryLanguage = 'eo';
    await mount();
    fireEvent.change(source(), { target: { value: 'Keep my requested target' } });
    bridge.snapshot.translationEngine = 'edge'; bridge.snapshot.translationPreferences.engine = 'edge';
    await emit('translation_engine_changed', 'edge');
    expect(screen.getByRole('button', { name: 'Primary language: Esperanto (unavailable)' })).toBeTruthy();
    expect(screen.getByRole('status').textContent).toContain('not supported');
    expect(screen.getByRole('button', { name: /^Translate/ })).toHaveProperty('disabled', true);
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    await emit('translation_chrome_action', 'translate');
    expect(calls('translation_run')).toHaveLength(0);
    expect(calls('translation_preferences_set')).toHaveLength(0);
    expect(calls('translation_chrome_update').at(-1)?.[1]?.state.canTranslate).toBe(false);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Primary language: Esperanto (unavailable)' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(calls('translation_run')[0][1]?.targets).toEqual(['de', 'zh-CN']);
  });

  it('rejects a stale language summary after editing and after a newer manual source choice', async () => {
    const pending = deferred(); const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? pending.promise : invoke(name, args));
    await mount(); submit('Outdated language');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    fireEvent.change(source(), { target: { value: '' } });
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Source language: Auto-Detect' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await act(async () => pending.resolve({ ...result('Outdated language'), sourceLanguage: 'ja' }));
    expect(screen.getByRole('button', { name: 'Source language: French' })).toBeTruthy();
    expect(document.querySelector('.source-bar')?.getAttribute('data-detected-language')).toBeNull();
  });

  it('shows service errors and removes the loading animation', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? Promise.reject('The translation request timed out. Try again.') : invoke(name, args));
    await mount(); submit('Synthetic failure');
    await waitFor(() => expect(screen.getAllByRole('alert')[0].textContent).toContain('timed out'));
    expect(screen.queryByRole('progressbar')).toBeNull();
  });
});
