// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
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

describe('live translation bridge', () => {
  it('renders translated output and copies it through the runtime history id', async () => {
    await mount(); submit('Synthetic input');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    fireEvent.keyDown(source(), { key: '1', metaKey: true });
    expect(calls('translation_sample_copy').at(-1)?.[1]).toEqual({ sampleId: 'translation-test', part: 'primary' });
    expect(calls('translation_run').at(-1)?.[1]).toEqual({ text: 'Synthetic input', source: 'auto', targets: ['en', 'zh-CN'], fromSelection: false });
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
    expect(calls('translation_run')[0][1]).toEqual({ text: 'Synthetic selection', source: 'auto', targets: ['en', 'zh-CN'], fromSelection: true });
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

  it('clears old output and does not translate a non-text capture', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    await mount(); submit('Prior synthetic text');
    await waitFor(() => expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1'));
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'selection_take' ? Promise.resolve({ text: '', sourceApp: null, error: null }) : invoke(name, args));
    await emit('selection_ready', null);
    expect(source().value).toBe('');
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('');
    expect(calls('translation_run')).toHaveLength(1);
  });

  it('does not submit after a clipboard capture or restoration failure', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    await mount();
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'selection_take' ? Promise.resolve({ text: '', error: 'clipboard_restore_failed' }) : invoke(name, args));
    await emit('selection_ready', null);
    expect(source().value).toBe('');
    expect(screen.getAllByRole('alert')[0].textContent).toContain('restore the clipboard');
    expect(calls('translation_run')).toHaveLength(0);
  });

  it('shows an actionable permission error and sends no text when AX access is denied', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    await mount();
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'selection_take' ? Promise.resolve({ text: '', error: 'permission_required' }) : invoke(name, args));
    await emit('selection_ready', null);
    expect(screen.getAllByRole('alert')[0].textContent).toContain('Accessibility');
    expect(calls('translation_run')).toHaveLength(0);
  });

  it('restores runtime records supplied by native history instead of requiring a fixture', async () => {
    await mount(); const record = result('Runtime history');
    await emit('history_sample_selected', { sampleId: record.id, retranslate: false, record });
    expect(source().value).toBe('Runtime history');
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Translated Runtime history');
  });

  it('cancels pending output when the engine changes', async () => {
    const pending = deferred(); const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? pending.promise : invoke(name, args));
    await mount(); submit('Old engine'); await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await emit('translation_engine_changed', 'google');
    await act(async () => pending.resolve(result('Old engine')));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('');
  });

  it('shows service errors and removes the loading animation', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_run' ? Promise.reject('The translation request timed out. Try again.') : invoke(name, args));
    await mount(); submit('Synthetic failure');
    await waitFor(() => expect(screen.getAllByRole('alert')[0].textContent).toContain('timed out'));
    expect(screen.queryByRole('progressbar')).toBeNull();
  });
});


describe('translation engine settings', () => {
  it('offers only Edge and Google and sends the chosen engine to the backend', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'Translate' }));
    await user.click(screen.getByRole('button', { name: 'Translation service: Edge' }));
    expect(screen.getAllByRole('option').map((item) => item.textContent)).toEqual(['Edge', 'Google']);
    await user.click(screen.getByRole('option', { name: 'Google' }));
    expect(calls('translation_engine_set').at(-1)?.[1]).toEqual({ engine: 'google' });
  });
});
