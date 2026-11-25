// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
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
const column = (index: number) => screen.getByRole('region', { name: index ? 'Secondary language' : 'Primary language' });
const partialResult = (failed: number, id = 'partial-test', error = 'Target unavailable') => ({ partial: {
  id, sourceLanguage: 'ja', translations: failed === 0 ? ['', 'Partial second'] : ['Partial first', ''],
  errors: failed === 0 ? [error, null] : [null, error],
} });
function deferred<T = TranslationOutput>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('translation reliability', () => {
  it('ignores unrelated preference broadcasts and applies only changed defaults', async () => {
    await mount();
    submit('Preference broadcast test');
    await within(column(0)).findByText('Synthetic translation 1');
    const preferences = bridge.snapshot.translationPreferences;
    const cancellations = calls('translation_cancel').length;
    const next = { ...preferences, disabledShortcuts: ['translate' as const] };
    await emit('translation_preferences_changed', next);
    expect(calls('translation_run')).toHaveLength(1);
    expect(calls('translation_cancel')).toHaveLength(cancellations);

    const changed = { ...next, secondaryLanguage: 'fr' };
    await emit('translation_preferences_changed', changed);
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]).toMatchObject({ text: 'Preference broadcast test', source: 'auto', targets: ['en', 'fr'], scope: 'standalone', reuseTargetIndex: 1 });
    await emit('translation_preferences_changed', changed);
    expect(calls('translation_run')).toHaveLength(2);
  });

  it('reports one task error for a failed dual-target request', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? Promise.reject('Service unavailable') : invoke(name, args));
    await mount(); submit('Fail both targets');
    await waitFor(() => expect(screen.getAllByRole('alert').map((value) => value.textContent)).toEqual(['Service unavailable']));
    expect(calls('translation_run')).toHaveLength(1);
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  });

  it.each([false, true])('explicitly retries with the alternate engine (partial: %s)', async (partial) => {
    const invoke = bridge.invoke.getMockImplementation()!;
    const pending = deferred();
    const run = vi.fn().mockImplementationOnce(() => partial ? Promise.resolve(partialResult(1)) : Promise.reject('Engine unavailable'))
      .mockReturnValueOnce(pending.promise);
    bridge.invoke.mockImplementation((name, args) => name === 'translation_run' ? run(args) : invoke(name, args));
    await mount(); submit('Keep this input');
    const button = await screen.findByRole('button', { name: 'Retry with Google' });
    expect(run).toHaveBeenCalledTimes(1);
    fireEvent.click(button);
    expect(run.mock.calls[1]?.[0]).toEqual({ text: 'Keep this input', source: 'auto', targets: ['en', 'zh-CN'], scope: 'standalone', retryEngine: 'google', ...(partial ? { retryId: 'partial-test' } : {}) });
    if (partial) expect(column(0).textContent).toBe('Partial first');
    await act(async () => pending.resolve(result('Keep this input')));
    expect(column(0).textContent).toBe('Translated Keep this input');
    expect(calls('translation_preferences_set')).toHaveLength(0);
  });

  it.each([0, 1])('isolates failed column %i and copies only the successful column', async (failed) => {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? Promise.resolve(partialResult(failed)) : invoke(name, args));
    await mount(); submit('Partial input');
    await within(column(failed)).findByRole('button', { name: 'Retry' });
    expect(within(column(failed)).getByRole('alert').textContent).toBe('Target unavailable');
    expect(column(1 - failed).textContent).toBe(failed ? 'Partial first' : 'Partial second');
    expect(within(column(1 - failed)).queryByRole('alert')).toBeNull();
    expect(document.querySelector('.source-bar')?.getAttribute('data-detected-language')).toBe('ja');
    expect(screen.queryByRole('progressbar')).toBeNull();
    await act(async () => {
      fireEvent.keyDown(source(), { key: String(failed + 1), metaKey: true });
      fireEvent.keyDown(source(), { key: String(2 - failed), metaKey: true });
    });
    expect(calls('translation_copy')).toEqual([['translation_copy', { scope: 'standalone', part: failed ? 'primary' : 'secondary' }]]);
  });

  it('retries the original captured request once at a time and accepts a refreshed partial ticket before completion', async () => {
    const pending = deferred<ReturnType<typeof partialResult>>();
    const run = vi.fn().mockResolvedValueOnce(partialResult(1)).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(result('Captured input'));
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => {
      if (name === 'translation_run') return run(args);
      if (name === 'selection_take' && calls('selection_take').length === 2) return Promise.resolve({ text: 'Captured input', error: null });
      return invoke(name, args);
    });
    await mount(); await emit('selection_ready', null);
    const retry = await within(column(1)).findByRole('button', { name: 'Retry' });
    const cancellations = calls('translation_cancel').length;
    fireEvent.click(retry); fireEvent.click(retry);
    expect(run.mock.calls).toEqual([
      [{ text: 'Captured input', source: 'auto', targets: ['en', 'zh-CN'], scope: 'standalone' }],
      [{ text: 'Captured input', source: 'auto', targets: ['en', 'zh-CN'], scope: 'standalone', retryId: 'partial-test' }],
    ]);
    expect((retry as HTMLButtonElement).disabled).toBe(true);
    expect(column(0).textContent).toBe('Partial first');
    await act(async () => pending.resolve(partialResult(1, 'partial-next', 'Still unavailable')));
    expect(within(column(1)).getByRole('alert').textContent).toBe('Still unavailable');
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(calls('translation_copy').at(-1)?.[1]).toEqual({ scope: 'standalone', part: 'primary' });
    fireEvent.click(within(column(1)).getByRole('button', { name: 'Retry' }));
    await within(column(1)).findByText('Second result');
    expect(run.mock.calls[2]).toEqual([{ ...run.mock.calls[0][0], retryId: 'partial-next' }]);
    expect(calls('translation_cancel')).toHaveLength(cancellations);
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
    fireEvent.keyDown(source(), { key: '2', metaKey: true });
    expect(calls('translation_copy').at(-1)?.[1]).toEqual({ scope: 'standalone', part: 'secondary' });
  });

  it('replaces only current results when changing a cached target, and omits reuse on retry', async () => {
    await mount(); submit('Change cached target');
    await within(column(1)).findByText('Synthetic translation 2');
    const pending = deferred<ReturnType<typeof partialResult>>();
    const invoke = bridge.invoke.getMockImplementation()!;
    const run = vi.fn().mockReturnValueOnce(pending.promise).mockRejectedValueOnce('Retry request failed');
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? run(args) : invoke(name, args));
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Secondary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    expect(column(0).textContent).toBe('Synthetic translation 1');
    expect(column(1).textContent).toBe('');
    expect(run.mock.calls[0][0]).toMatchObject({ scope: 'standalone', reuseTargetIndex: 1 });
    await act(async () => pending.resolve({ partial: { ...partialResult(1).partial, translations: ['Synthetic translation 1', ''] } }));
    const retry = within(column(1)).getByRole('button', { name: 'Retry' });
    await user.click(retry);
    await screen.findByText('Retry request failed');
    expect(run.mock.calls[1]).toEqual([{ text: 'Change cached target', source: 'auto', targets: ['en', 'fr'], scope: 'standalone', retryId: 'partial-test' }]);
    expect((retry as HTMLButtonElement).disabled).toBe(false);
    expect(column(0).textContent).toBe('Synthetic translation 1');
    expect(screen.queryByText('Synthetic translation 2')).toBeNull();
  });

  it.each(['edit', 'engine', 'language'] as const)('clears the partial and ignores a late retry after %s', async (change) => {
    const pending = deferred<TranslationOutput | ReturnType<typeof partialResult>>();
    const invoke = bridge.invoke.getMockImplementation()!;
    const run = vi.fn().mockResolvedValueOnce(partialResult(1)).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(result('New language result'));
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? run(args) : invoke(name, args));
    await mount(); submit('Expired input');
    fireEvent.click(await within(column(1)).findByRole('button', { name: 'Retry' }));
    const cancellations = calls('translation_cancel').length;
    if (change === 'edit') fireEvent.change(source(), { target: { value: 'New text' } });
    if (change === 'engine') await emit('translation_engine_changed', 'google');
    if (change === 'language') {
      await emit('translation_preferences_changed', { ...bridge.snapshot.translationPreferences, secondaryLanguage: 'fr' });
      expect(run.mock.calls[2]).toEqual([{ text: 'Expired input', source: 'auto', targets: ['en', 'fr'], scope: 'standalone' }]);
    }
    await waitFor(() => expect(calls('translation_cancel')).toHaveLength(cancellations + 1));
    await act(async () => pending.resolve(partialResult(0, 'stale-ticket')));
    expect(screen.queryByRole('button', { name: /Retry/ })).toBeNull();
    expect(column(0).textContent).toBe(change === 'language' ? 'Translated New language result' : '');
    expect(screen.queryByText('Partial second')).toBeNull();
  });

  it('copies without a success message and only reports failures for the current draft', async () => {
    await mount(); submit('Text for copy');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    await act(async () => { fireEvent.keyDown(source(), { key: '1', metaKey: true }); });
    expect(calls('translation_copy').at(-1)?.[1]).toEqual({ scope: 'standalone', part: 'primary' });
    expect(screen.queryByText(/translation copied/)).toBeNull();
    expect(screen.queryByRole('progressbar')).toBeNull();
    bridge.invoke.mockRejectedValueOnce('clipboard_write');
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    await screen.findByText('Content was not copied. Try again.');
    const invoke = bridge.invoke.getMockImplementation()!;
    let fail!: (reason: string) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_copy' ? new Promise<void>((_, reject) => { fail = reject; }) : invoke(name, args));
    fireEvent.keyDown(source(), { key: '2', metaKey: true });
    fireEvent.change(source(), { target: { value: 'Different draft' } });
    await act(async () => fail('clipboard_write'));
    expect(screen.queryByText('Content was not copied. Try again.')).toBeNull();
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

  it('keeps the unchanged result when the changed target fails', async () => {
    const user = userEvent.setup(); await mount(); submit('Failure switch');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? Promise.reject('Service unavailable') : invoke(name, args));
    await user.click(screen.getByRole('button', { name: 'Secondary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('Service unavailable'));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1');
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('keeps the app available and blocks translation when storage cannot open', async () => {
    bridge.snapshot.storageError = 'storage_version';
    await mount(); submit('Do not send when storage is unavailable');
    expect(screen.getByRole('alert').textContent).toContain('Existing files are preserved');
    expect(calls('translation_run')).toHaveLength(0);
  });

  it('blocks new translations when storage corruption is reported after startup', async () => {
    await mount();
    await emit('storage_error', 'storage_corrupt');
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

  it('preserves unsupported selections on an engine switch and blocks shortcut submission', async () => {
    bridge.snapshot.translationEngine = 'google';
    bridge.snapshot.translationPreferences.engine = 'google';
    bridge.snapshot.translationPreferences.primaryLanguage = 'eo';
    await mount();
    fireEvent.change(source(), { target: { value: 'Keep my requested target' } });
    bridge.snapshot.translationEngine = 'edge'; bridge.snapshot.translationPreferences.engine = 'edge';
    await emit('translation_engine_changed', 'edge');
    expect(screen.getByRole('button', { name: 'Primary language: Esperanto (unavailable)' })).toBeTruthy();
    expect(screen.getByRole('status').textContent).toContain('not supported');
    fireEvent.keyDown(source(), { key: 'Enter', metaKey: true });
    expect(calls('translation_run')).toHaveLength(0);
    expect(calls('translation_preferences_set')).toHaveLength(0);
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
