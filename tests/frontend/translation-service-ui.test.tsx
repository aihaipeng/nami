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

  it('immediately retranslates the changed target and creates a new complete record', async () => {
    const user = userEvent.setup(); await mount(); submit('Switch language');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Primary language: English' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]).toEqual({ text: 'Switch language', source: 'auto', targets: ['de', 'zh-CN'], fromSelection: false, reuse: { recordId: 'translation-test', targetIndex: 0 } });
    expect(bridge.history.records.at(-1)?.targetLanguages).toEqual(['de', 'zh-CN']);
    expect(bridge.history.records.at(-1)?.id).toBe('translation-test-2');
    expect(bridge.history.records.find(({ id }) => id === 'translation-test')?.targetLanguages).toEqual(['en', 'zh-CN']);
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
    expect(bridge.history.records.at(-1)?.sourceLanguage).toBe('de');
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

  it('records disabling and enabling the secondary column without losing the primary result', async () => {
    const user = userEvent.setup(); await mount(); submit('Column changes');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Secondary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'None' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(bridge.history.records.at(-1)?.targetLanguages).toEqual(['en']);
    await user.click(screen.getByRole('button', { name: 'Secondary language: None' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(3));
    expect(bridge.history.records.at(-1)?.targetLanguages).toEqual(['en', 'fr']);
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

  it('restores runtime records supplied by native history instead of requiring a fixture', async () => {
    await mount(); const record = result('Runtime history');
    await emit('history_sample_selected', { sampleId: record.id, retranslate: false, record });
    expect(source().value).toBe('Runtime history');
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Translated Runtime history');
  });

  it('references the original record when retranslating and drops its source after editing', async () => {
    await mount();
    const record = { ...result('Retranslate source', 'original-source'), sourceApp: { bundleId: 'test.source', name: 'Synthetic App', iconDataUrl: 'data:image/png;base64,c3ludGhldGlj' } };
    bridge.history.records.push(record);
    await emit('history_sample_selected', { sampleId: record.id, retranslate: true, record });
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(calls('translation_run')[0][1]).toMatchObject({ reuse: { recordId: record.id } });
    expect(calls('translation_run')[0][1]?.reuse).not.toHaveProperty('targetIndex');
    await waitFor(() => expect(bridge.history.records.find(({ id }) => id === 'translation-test')?.sourceApp).toEqual(record.sourceApp));
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('Synthetic translation 1');
    submit('Edited source');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]).not.toHaveProperty('reuse');
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
