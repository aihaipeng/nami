// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, flushFrames, mount, preferReducedMotion, releasePointerCapture, scrollTo, setPointerCapture, windowState } from './harness';
import { historySamples } from '../../src/translation/history';
import languagesSvg from '../../src/assets/history/reference/languages.svg?raw';
import wordmarkSvg from '../../src/assets/history/reference/nami-wordmark.svg?raw';

const records = () => screen.queryAllByRole('button', { name: /^Translation:/ });
const row = (id: string) => {
  const record = historySamples.find((record) => record.id === id)!;
  return screen.getByRole('button', { name: `Translation: ${record.text.replace(/\s+/g, ' ').trim()}` }).closest('article')!;
};

describe('history list and session actions', () => {
  it('confirms inline copy success and blocks repeats for the full pending operation', async () => {
    await mount('history');
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_sample_copy' ? new Promise<void>((resolve) => { finish = resolve; }) : original(name, args));
    const button = within(row('sample-03')).getByRole('button', { name: 'Copy source text' });
    fireEvent.click(button); fireEvent.click(button);
    expect(calls('translation_sample_copy')).toHaveLength(1);
    expect(button.querySelector('.copy-success')).toBeNull();
    vi.useFakeTimers();
    try {
      await act(async () => finish());
      expect(button.querySelector('.copy-success')).toBeTruthy();
      fireEvent.click(button);
      expect(calls('translation_sample_copy')).toHaveLength(1);
      act(() => vi.advanceTimersByTime(999));
      expect(button.querySelector('.copy-success')).toBeTruthy();
      act(() => vi.advanceTimersByTime(1));
      expect(button.querySelector('.copy-success')).toBeNull();
    } finally { vi.useRealTimers(); }
  });

  it('does not show a late copy success after leaving the record session', async () => {
    await mount('clipboard');
    await windowState({ recordView: 'translation' });
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_sample_copy' ? new Promise<void>((resolve) => { finish = resolve; }) : original(name, args));
    const target = within(row('sample-03')).getByRole('button', { name: 'Copy source text' });
    fireEvent.click(target);
    await windowState({ recordView: 'clipboard' });
    await act(async () => finish());
    expect(target.querySelector('.copy-success')).toBeNull();
    expect(document.querySelector('.global-error')).toBeNull();
  });

  it('highlights only the context target without selecting it and clears the target when dismissed', async () => {
    await mount('history');
    await windowState({ translationVisible: false, translationExtent: 0 });
    const target = row('sample-03');
    const before = calls('history_sample_select').length;
    fireEvent.contextMenu(target);
    expect(target.classList.contains('context-target')).toBe(true);
    expect(target.classList.contains('selected')).toBe(false);
    expect(calls('history_sample_select')).toHaveLength(before);
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(target.classList.contains('context-target')).toBe(false);
    expect(calls('history_sample_select')).toHaveLength(before);
  });

  it('clears the search immediately without waiting for the typing debounce', async () => {
    await mount('history');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'train' } });
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: 'train', favoritesOnly: false }));
    vi.useFakeTimers();
    try {
      await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Clear history search' })));
      expect(calls('history_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: false });
      expect(screen.queryByRole('progressbar')).toBeNull();
    } finally { vi.useRealTimers(); }
  });

  it.each(['search', 'favorites'] as const)('follows the first unpinned result when %s excludes the open detail', async (filter) => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    if (filter === 'search') fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'updated design' } });
    else fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
    await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-04', retranslate: false }));
    expect(within(row('sample-04')).getByRole('button', { name: /^Translation:/ }).getAttribute('aria-pressed')).toBe('true');
  });

  it('clears an open detail for empty search results and retains a matching selection', async () => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    const before = calls('history_sample_select').length;
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'train' } });
    await waitFor(() => expect(records()).toHaveLength(1));
    expect(calls('history_sample_select')).toHaveLength(before);
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'no synthetic record matches this' } });
    await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: null, retranslate: false, clearDetail: true }));
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    expect(records()).toHaveLength(0);
  });

  it.each(['empty-search', 'favorite-roundtrip', 'clear-selection'] as const)('opens the same record after %s instead of treating it as a second click', async (reset) => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    if (reset === 'empty-search') {
      fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'no synthetic record matches this' } });
      await waitFor(() => expect(records()).toHaveLength(0));
      await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toMatchObject({ sampleId: null, clearDetail: true }));
      fireEvent.click(screen.getByRole('button', { name: 'Clear history search' }));
      await waitFor(() => expect(records()).toHaveLength(20));
    } else if (reset === 'favorite-roundtrip') {
      fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
      await waitFor(() => expect(records()).toHaveLength(5));
      fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
      await waitFor(() => expect(records()).toHaveLength(20));
    } else await emit('history_sample_selected', { sampleId: null, retranslate: false });
    const before = calls('history_sample_select').length;
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    await waitFor(() => expect(calls('history_sample_select')).toHaveLength(before + 1));
    expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-03', retranslate: false });
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(false));
  });

  it('falls back to a pinned result when a search has no ordinary records', async () => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'simple, calm' } });
    await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-02', retranslate: false }));
  });

  it('retains detail after a failed filter query and follows the results after retry', async () => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    const original = bridge.invoke.getMockImplementation()!;
    let fail = true;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'history_query' && fail) throw new Error('database_unavailable');
      return original(name, args);
    });
    const before = calls('history_sample_select').length;
    fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
    await screen.findByRole('alert');
    expect(calls('history_sample_select')).toHaveLength(before);
    fail = false;
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-04', retranslate: false }));
  });

  it.each(['close', 'new-selection'] as const)('does not reopen or replace details after %s while a filter request is pending', async (change) => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: (value: typeof bridge.history) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_query'
      ? new Promise<typeof bridge.history>((resolve) => { finish = resolve; }) : original(name, args));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
    await waitFor(() => expect(finish).toBeTypeOf('function'));
    if (change === 'close') await windowState({ translationVisible: false, translationExtent: 0 });
    else await emit('history_sample_selected', { sampleId: 'new-result-outside-filter', retranslate: false });
    const before = calls('history_sample_select').length;
    const favorites = bridge.history.records.filter(({ isFavorite }) => isFavorite);
    await act(async () => finish({ ...bridge.history, records: favorites, revision: bridge.history.revision + 1, generation: bridge.history.generation + 1, totalRecords: favorites.length }));
    expect(calls('history_sample_select')).toHaveLength(before);
  });

  it.each([['before-query', false], ['before-query', true], ['during-query', false], ['during-query', true]] as const)('preserves an edited draft cleared %s, with empty results=%s', async (timing, empty) => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    if (timing === 'before-query') await emit('history_sample_selected', { sampleId: null, retranslate: false });
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: (value: typeof bridge.history) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_query'
      ? new Promise<typeof bridge.history>((resolve) => { finish = resolve; }) : original(name, args));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
    await waitFor(() => expect(finish).toBeTypeOf('function'));
    if (timing === 'during-query') await emit('history_sample_selected', { sampleId: null, retranslate: false });
    const before = calls('history_sample_select').length;
    const favorites = empty ? [] : bridge.history.records.filter(({ isFavorite }) => isFavorite);
    await act(async () => finish({ ...bridge.history, records: favorites, revision: bridge.history.revision + 1, generation: bridge.history.generation + 1, totalRecords: favorites.length }));
    expect(calls('history_sample_select')).toHaveLength(before);
  });

  it('still follows results when a query snapshot only clears the visual selection', async () => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      const result = await original(name, args);
      return name === 'history_query' ? { ...result, selectedId: null } : result;
    });
    fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
    await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-04', retranslate: false }));
  });

  it.each([['favorite', 'inline'], ['favorite', 'menu'], ['delete', 'inline'], ['delete', 'menu']] as const)('follows remaining results after %s through %s removes the current detail', async (action, entry) => {
    await mount('history');
    if (action === 'favorite') {
      fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
      await waitFor(() => expect(records()).toHaveLength(5));
    }
    const targetId = action === 'favorite' ? 'sample-04' : 'sample-03';
    fireEvent.click(within(row(targetId)).getByRole('button', { name: /^Translation:/ }));
    if (entry === 'menu') {
      fireEvent.contextMenu(row(targetId));
      fireEvent.click(screen.getByRole('menuitem', { name: action === 'favorite' ? 'Remove favorite' : 'Delete' }));
      if (action === 'delete') fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    } else {
      const button = within(row(targetId)).getByRole('button', { name: action === 'favorite' ? 'Remove favorite' : 'Delete history record' });
      fireEvent.click(button);
      if (action === 'delete') fireEvent.click(button);
    }
    await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: action === 'favorite' ? 'sample-08' : 'sample-04', retranslate: false }));
  });

  it('clears detail when deleting the last search result', async () => {
    await mount('history');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'train' } });
    await waitFor(() => expect(records()).toHaveLength(1));
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: 'Delete history record' }));
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: 'Delete history record' }));
    await waitFor(() => expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: null, retranslate: false, clearDetail: true }));
  });

  it.each([null, 'new-retranslation-result'])('does not replace a new selection event (%s) while a record update is pending', async (sampleId) => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => Promise<void>;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_record_update'
      ? new Promise((resolve) => { finish = async () => { resolve(await original(name, args)); }; }) : original(name, args));
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: 'Delete history record' }));
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: 'Delete history record' }));
    await emit('history_sample_selected', { sampleId, retranslate: false });
    const before = calls('history_sample_select').length;
    await act(async () => finish());
    expect(calls('history_sample_select')).toHaveLength(before);
  });

  it('does not replace an out-of-filter retranslation selection when the filters did not change', async () => {
    await mount('history');
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'train' } });
    await waitFor(() => expect(records()).toHaveLength(1));
    const before = calls('history_sample_select').length;
    await emit('history_sample_selected', { sampleId: 'outside-filter-result', retranslate: false });
    await emit('history_changed', { ...bridge.history, records: [bridge.history.records.find(({ id }) => id === 'sample-03')!], revision: bridge.history.revision + 1 });
    expect(calls('history_sample_select')).toHaveLength(before);
  });

  it('preserves the backend pin order when a newer pin has an older translation date', async () => {
    const older = { ...bridge.history.records[3], id: 'recently-pinned', isPinned: true, date: '2020-01-01T00:00:00Z' };
    const newer = { ...bridge.history.records[2], id: 'previously-pinned', isPinned: true, date: '2020-01-02T00:00:00Z' };
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'history_snapshot'
      ? { ...bridge.history, records: [older, newer], totalRecords: 2 } : original(name, args));
    await mount('history');
    await waitFor(() => expect(document.querySelectorAll('.history-pinned article')).toHaveLength(2));
    expect([...document.querySelectorAll<HTMLElement>('.history-pinned article')].map((row) => row.dataset.recordId)).toEqual(['recently-pinned', 'previously-pinned']);
  });

  it('keeps a pending inline copy silent without a global loading beam', async () => {
    await mount('history');
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'translation_sample_copy' ? new Promise<void>((resolve) => { finish = resolve; }) : original(name, args));
    fireEvent.click(within(row('sample-03')).getByRole('button', { name: 'Copy source text' }));
    expect(screen.queryByRole('progressbar')).toBeNull();
    await act(async () => finish());
  });

  it('uses shared nonblocking copy failure feedback for inline and menu actions and clears it on hide', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'translation_sample_copy') throw new Error('clipboard_write');
      return original(name, args);
    });
    const user = userEvent.setup();
    await mount('history');
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Copy source text' }));
    const feedback = await screen.findByText('Content was not copied. Try again.');
    expect(feedback.className).toBe('record-feedback');
    expect(feedback.closest('#translation-history')).toBeTruthy();
    expect(document.querySelector('.global-error')).toBeNull();
    await windowState({ historyVisible: false, recordsSession: 1 });
    expect(screen.queryByText('Content was not copied. Try again.')).toBeNull();
    await windowState({ historyVisible: true });
    fireEvent.contextMenu(row('sample-03'));
    await user.click(screen.getByRole('menuitem', { name: 'Copy source text' }));
    expect(await screen.findByText('Content was not copied. Try again.')).toBeTruthy();
    expect(screen.queryByRole('menu')).toBeNull();
    expect(document.querySelector('.global-error')).toBeNull();
  });

  it('opens the shared five-action context menu without selecting or translating the row', async () => {
    await mount('history');
    const before = calls('history_sample_select').length;
    fireEvent.contextMenu(row('sample-03'), { clientX: 100, clientY: 180 });
    const menu = screen.getByRole('menu');
    expect(within(menu).getAllByRole('menuitem').map((item) => item.textContent)).toEqual(['Retranslate', 'Copy source text', 'Favorite', 'Pin record', 'Delete']);
    expect(within(menu).getByRole('menuitem', { name: 'Delete' }).classList.contains('clipboard-menu-delete')).toBe(true);
    expect(within(menu).getAllByRole('menuitem').every((item) => item.children.length === 2)).toBe(true);
    expect(calls('history_sample_select')).toHaveLength(before);
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    expect(document.activeElement).toBe(within(menu).getByRole('menuitem', { name: 'Delete' }));
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('dispatches context actions by record ID and preserves protected delete', async () => {
    await mount('history');
    fireEvent.contextMenu(row('sample-03'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy source text' }));
    expect(calls('translation_sample_copy').at(-1)?.[1]).toEqual({ sampleId: 'sample-03', part: 'source' });
    fireEvent.contextMenu(row('sample-03'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Favorite' }));
    await waitFor(() => expect(within(row('sample-03')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    fireEvent.contextMenu(row('sample-03'));
    expect((screen.getByRole('menuitem', { name: 'Delete' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.scroll(document.querySelector('.history-list')!);
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.contextMenu(row('sample-03'));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite history' }));
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('keeps context copy disabled when the platform cannot write clipboard', async () => {
    bridge.snapshot.capabilities.clipboardWrite = false;
    await mount('history');
    fireEvent.contextMenu(row('sample-03'));
    expect((screen.getByRole('menuitem', { name: 'Copy source text' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('dismisses the record menu and shortcuts when history hides without changing records', async () => {
    await mount('history');
    fireEvent.click(screen.getByRole('button', { name: 'Keyboard shortcuts' }));
    fireEvent.contextMenu(row('sample-03'));
    await windowState({ historyVisible: false, historyExtent: 0 });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(screen.queryByRole('dialog', { name: 'Keyboard shortcuts' })).toBeNull();
    await windowState({ historyVisible: true, historyExtent: 360 });
    expect(records()).toHaveLength(20);
  });

  it('loads all 20 synthetic records with the pinned section separate', async () => {
    await mount('history');
    expect(records()).toHaveLength(20);
    expect(document.querySelectorAll('.history-pinned article')).toHaveLength(2);
    expect(document.querySelectorAll('.history-items article')).toHaveLength(18);
    expect(screen.queryByRole('textbox', { name: 'Source text' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Translations' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('searches complete translations through the backend and clears the query back to all records', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.type(screen.getByRole('searchbox'), '安静');
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: '安静', favoritesOnly: false }));
    await waitFor(() => expect(records()).toHaveLength(1));
    expect(records()[0].getAttribute('aria-label')).toContain(historySamples[6].text);
    await user.click(screen.getByRole('button', { name: 'Clear history search' }));
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: false }));
    await waitFor(() => expect(records()).toHaveLength(20));
    expect(document.activeElement).toBe(screen.getByRole('searchbox'));
  });

  it('treats search symbols literally and shows a useful empty state', async () => {
    await mount('history');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: '100%_literal' } });
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: '100%_literal', favoritesOnly: false }));
    await waitFor(() => expect(records()).toHaveLength(1));
    expect(records()[0].getAttribute('aria-label')).toContain('Numbers and symbols');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'never present in these samples' } });
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: 'never present in these samples', favoritesOnly: false }));
    await waitFor(() => expect(records()).toHaveLength(0));
    expect(screen.getByRole('status').textContent).toBe('No matching translations');
  });

  it('filters the five favorites, permits a session favorite change and restores All', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(screen.getByRole('button', { name: 'Favorite history' }));
    expect(records()).toHaveLength(5);
    const target = row('sample-04');
    await user.click(within(target).getByRole('button', { name: 'Remove favorite' }));
    expect(records()).toHaveLength(4);
    await user.click(screen.getByRole('button', { name: 'Favorite history' }));
    expect(records()).toHaveLength(20);
    expect(within(row('sample-04')).getByRole('button', { name: 'Favorite' }).getAttribute('aria-pressed')).toBe('false');
    expect(historySamples.find(({ id }) => id === 'sample-04')!.isFavorite).toBe(true);
  });

  it('moves pinned records between independent lists without losing their source', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Pin record' }));
    expect(document.querySelectorAll('.history-pinned article')).toHaveLength(3);
    expect(row('sample-03').closest('.history-pinned')).toBeTruthy();
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Unpin record' }));
    expect(document.querySelectorAll('.history-pinned article')).toHaveLength(2);
    expect(row('sample-03').closest('.history-list')).toBeTruthy();
    expect(records()).toHaveLength(20);
  });

  it('uses shared icons and EcoPaste favorite color without rotating record pins', async () => {
    const user = userEvent.setup(); await mount('history');
    expect(screen.getByRole('button', { name: 'Clipboard' }).querySelector('[data-symbol="list.bullet"]')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Pin records' }).classList.contains('history-pin')).toBe(true);
    const target = row('sample-03');
    expect(target.querySelector('[data-symbol="languages"]')).toBeNull();
    expect(target.querySelector('[data-symbol="doc.on.doc"]')).toBeTruthy();
    expect(target.querySelector('[data-symbol="trash"]')).toBeTruthy();
    await user.click(within(target).getByRole('button', { name: 'Favorite' }));
    expect(target.querySelector('[data-action="favorite"] .record-favorite [data-symbol="star"]')).toBeTruthy();
    await user.click(within(target).getByRole('button', { name: 'Pin record' }));
    const pinned = row('sample-03');
    expect(pinned.classList.contains('pinned')).toBe(true);
    const pin = within(pinned).getByRole('button', { name: 'Unpin record' });
    expect(pin.querySelector('[data-symbol="pin.fill"]')).toBeTruthy();
    expect(pin.classList.contains('history-pin')).toBe(false);
    await user.click(pin);
    expect(row('sample-03').classList.contains('pinned')).toBe(false);
  });

  it('keeps retranslation in the context menu and aligns row actions with clipboard', async () => {
    await mount('history');
    const target = row('sample-03');
    expect(within(target).queryByRole('button', { name: 'Retranslate' })).toBeNull();
    expect([...target.querySelectorAll('.history-row-actions button')].map((button) => button.getAttribute('aria-label'))).toEqual(['Copy source text', 'Favorite', 'Pin record', 'Delete history record']);
    expect(target.style.getPropertyValue('--history-actions-width')).toBe('99px');
    fireEvent.contextMenu(target);
    const button = screen.getByRole('menuitem', { name: 'Retranslate' });
    const icon = button.querySelector<HTMLElement>('.history-symbol')!;
    expect(icon.dataset.symbol).toBe('languages');
    expect(icon.style.getPropertyValue('--symbol-url')).toContain('reference-languages.png');
    const svg = new DOMParser().parseFromString(languagesSvg, 'image/svg+xml').documentElement;
    expect([svg.getAttribute('width'), svg.getAttribute('height'), svg.getAttribute('stroke-width')]).toEqual(['14', '14', '1.5']);
    expect(svg.querySelectorAll('path')).toHaveLength(6);
    expect(svg.querySelector('path')?.getAttribute('d')).toBe('m5 8 6 6');
  });

  it('renders the toolbar wordmark as fixed outlines with the native dimensions and drag region', async () => {
    await mount('history');
    const wordmark = within(document.querySelector<HTMLElement>('.history-toolbar')!).getByRole('img', { name: 'Nami' });
    expect(wordmark.classList.contains('history-wordmark')).toBe(true);
    expect(wordmark.textContent).toBe('');
    expect(wordmark.hasAttribute('data-tauri-drag-region')).toBe(true);
    expect(wordmark.style.getPropertyValue('--wordmark-url')).toContain('nami-wordmark.svg');
    expect(wordmark.style.width).toBe('44px');
    expect(wordmark.style.height).toBe('20px');
    expect(wordmark.style.background).toBe('rgb(255, 255, 255)');
    expect(wordmark.style.fontFamily).toBe('');
    const svg = new DOMParser().parseFromString(wordmarkSvg, 'image/svg+xml').documentElement;
    expect(svg.getAttribute('viewBox')).toBe('155 340 680 280');
    expect(svg.querySelector('g')?.getAttribute('fill')).toBe('#FFFFFF');
    expect(wordmarkSvg).not.toContain('#202020');
    expect(svg.querySelectorAll('path')).toHaveLength(5);
    expect(svg.querySelector('text, tspan, foreignObject')).toBeNull();
    expect(wordmarkSvg).not.toMatch(/font-family|font-size|Didot/);
  });

  it('uses the flowing Nami mark by default and preserves a real source app icon', async () => {
    bridge.history.records = bridge.history.records.map((record) => ({ ...record, sourceApp: record.id === 'sample-03' ? { name: 'Synthetic Source', bundleId: 'test.source', iconDataUrl: 'data:image/png;base64,c3ludGhldGlj' } : undefined }));
    await mount('history');
    const fallback = within(row('sample-01')).getByRole('img', { name: 'Nami' });
    expect(fallback.classList.contains('history-symbol')).toBe(true);
    expect(fallback.style.getPropertyValue('--symbol-url')).toContain('reference-nami.png');
    expect(fallback.style.background).toBe('rgb(255, 255, 255)');
    const source = within(row('sample-03')).getByRole('img', { name: 'Synthetic Source' });
    expect(source.getAttribute('src')).toBe('data:image/png;base64,c3ludGhldGlj');
    expect(source.classList.contains('history-symbol')).toBe(false);
  });

  it('renders the history header wordmark from outlined artwork instead of font text', async () => {
    await mount('history');
    const mark = document.querySelector<HTMLElement>('.history-toolbar .history-wordmark')!;
    expect(mark.getAttribute('role')).toBe('img');
    expect(mark.getAttribute('aria-label')).toBe('Nami');
    expect(mark.textContent).toBe('');
    expect(mark.style.getPropertyValue('--wordmark-url')).toContain('nami-wordmark.svg');
    expect(mark.hasAttribute('data-tauri-drag-region')).toBe(true);
  });

  it('allows deleting only unprotected records and retains backend state on remount', async () => {
    const user = userEvent.setup(); const view = await mount('history');
    expect(within(row('sample-01')).queryByRole('button', { name: 'Delete history record' })).toBeNull();
    expect(within(row('sample-04')).queryByRole('button', { name: 'Delete history record' })).toBeNull();
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Delete history record' }));
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Delete history record' }));
    expect(records()).toHaveLength(19);
    view.unmount(); await mount('history');
    expect(records()).toHaveLength(19);
  });

  it('pins the history region through the native contract and reflects returned state', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(screen.getByRole('button', { name: 'Pin records' }));
    expect(calls('window_pin').at(-1)).toEqual(['window_pin', { region: 'history', pinned: true }]);
    await windowState({ historyPinned: true });
    await user.click(screen.getByRole('button', { name: 'Unpin records' }));
    expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'history', pinned: false });
  });

  it('sends only a fixture ID for normal selection, retranslation and source copying', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.click(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }));
    expect(calls('history_sample_select').at(-1)).toEqual(['history_sample_select', { sampleId: 'sample-03', retranslate: false }]);
    expect(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.contextMenu(row('sample-03'));
    await user.click(screen.getByRole('menuitem', { name: 'Retranslate' }));
    expect(calls('history_sample_select').at(-1)).toEqual(['history_sample_select', { sampleId: 'sample-03', retranslate: true }]);
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Copy source text' }));
    expect(calls('translation_sample_copy').at(-1)).toEqual(['translation_sample_copy', { sampleId: 'sample-03', part: 'source' }]);
  });

  it('retains query and session favorites while hidden and shown again', async () => {
    const user = userEvent.setup(); await mount('history');
    await user.type(screen.getByRole('searchbox'), 'train');
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: 'train', favoritesOnly: false }));
    await waitFor(() => expect(records()).toHaveLength(1));
    await user.click(within(row('sample-03')).getByRole('button', { name: 'Favorite' }));
    await windowState({ historyVisible: false, historyExtent: 0 });
    expect(records()).toHaveLength(0);
    expect(document.querySelector('.history-view')?.hasAttribute('inert')).toBe(true);
    await windowState({ historyVisible: true, historyExtent: 360 });
    expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('train');
    expect(records()).toHaveLength(1);
    expect(within(row('sample-03')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy();
  });

  it('accepts cross-view selection events and clears highlight without erasing records', async () => {
    await mount('history');
    await emit('history_sample_selected', { sampleId: 'sample-03', retranslate: false });
    expect(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }).getAttribute('aria-pressed')).toBe('true');
    await emit('history_sample_selected', { sampleId: null, retranslate: false });
    expect(within(row('sample-03')).getByRole('button', { name: /^Translation:/ }).getAttribute('aria-pressed')).toBe('false');
    expect(records()).toHaveLength(20);
  });

  it('dismisses with Escape or Command-W and suppresses dismissal during IME input', async () => {
    await mount('history'); const search = screen.getByRole('searchbox');
    fireEvent.keyDown(search, { key: 'Escape', isComposing: true });
    expect(calls('window_hide')).toHaveLength(0);
    fireEvent.keyDown(search, { key: 'Escape' });
    fireEvent.keyDown(search, { key: 'w', metaKey: true });
    expect(calls('window_hide')).toEqual([
      ['window_hide', { region: 'translation' }], ['window_hide', { region: 'translation' }],
    ]);
  });

  it('reveals a top-clipped record using only its own scroll region', async () => {
    const user = userEvent.setup(); await mount('history');
    const article = row('sample-03'); const list = article.closest<HTMLElement>('.history-list')!;
    list.scrollTop = 100;
    vi.spyOn(list, 'getBoundingClientRect').mockReturnValue({ top: 100, bottom: 500 } as DOMRect);
    vi.spyOn(article, 'getBoundingClientRect').mockReturnValue({ top: 80, bottom: 160 } as DOMRect);
    scrollTo.mockClear();
    await user.click(within(article).getByRole('button', { name: /^Translation:/ }));
    expect(scrollTo).toHaveBeenCalledWith({ top: 76, behavior: 'smooth' });
    expect(scrollTo.mock.contexts.at(-1)).toBe(list);
    expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-03', retranslate: false });
  });

  it('selects a pinned row without moving the ordinary records scroll region', async () => {
    const user = userEvent.setup(); await mount('history');
    const list = document.querySelector<HTMLElement>('.history-list')!;
    list.scrollTop = 100; scrollTo.mockClear();
    await user.click(within(row('sample-01')).getByRole('button', { name: /^Translation:/ }));
    expect(list.scrollTop).toBe(100);
    expect(scrollTo).not.toHaveBeenCalled();
    expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: 'sample-01', retranslate: false });
  });

  it('does not scroll a fully visible row and respects reduced motion for clipped rows', async () => {
    const user = userEvent.setup(); await mount('history');
    const article = row('sample-03'); const list = article.closest<HTMLElement>('.history-list')!;
    vi.spyOn(list, 'getBoundingClientRect').mockReturnValue({ top: 100, bottom: 300 } as DOMRect);
    const measure = vi.spyOn(article, 'getBoundingClientRect').mockReturnValue({ top: 110, bottom: 200 } as DOMRect);
    scrollTo.mockClear();
    await user.click(within(article).getByRole('button', { name: /^Translation:/ }));
    expect(scrollTo).not.toHaveBeenCalled();
    preferReducedMotion(); measure.mockReturnValue({ top: 250, bottom: 320 } as DOMRect);
    await user.click(within(article).getByRole('button', { name: /^Translation:/ }));
    expect(scrollTo).toHaveBeenCalledWith({ top: 24, behavior: 'instant' });
    expect(scrollTo.mock.contexts.at(-1)).toBe(list);
  });
});

describe('history divider interaction contract', () => {
  it.each([
    ['ArrowLeft', 336], ['ArrowRight', 384], ['Home', 280], ['End', 720],
  ])('resizes using %s to request width %i', async (key, expected) => {
    await mount('history');
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize translation history' }), { key });
    expect(calls('history_width_set').at(-1)).toEqual(['history_width_set', { width: expected, initialize: false }]);
  });

  it('uses screen coordinates despite moving local coordinates, and commits the final release width', async () => {
    await mount('history'); const divider = screen.getByRole('separator');
    fireEvent.pointerDown(divider, { button: 0, pointerId: 3, screenX: 500, clientX: 350 });
    expect(setPointerCapture).toHaveBeenCalledWith(3);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 560, clientX: 340 });
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 580, clientX: 325 });
    await flushFrames();
    expect(calls('history_width_set').at(-1)).toEqual(['history_width_set', { width: 440, initialize: false }]);
    fireEvent.pointerUp(divider, { pointerId: 3, screenX: 590, clientX: 320 });
    expect(calls('history_width_set').at(-1)).toEqual(['history_width_set', { width: 450, initialize: false }]);
    expect(releasePointerCapture).toHaveBeenCalledWith(3);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 650 }); await flushFrames();
    expect(calls('history_width_set')).toHaveLength(2);
  });

  it('cancels queued resizing when pointer capture is lost and ignores non-primary buttons', async () => {
    await mount('history'); const divider = screen.getByRole('separator');
    fireEvent.pointerDown(divider, { button: 2, pointerId: 3, screenX: 500 });
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 580 }); await flushFrames();
    expect(calls('history_width_set')).toHaveLength(0);
    fireEvent.pointerDown(divider, { button: 0, pointerId: 4, screenX: 500 });
    fireEvent.pointerMove(divider, { pointerId: 4, screenX: 580 });
    fireEvent.lostPointerCapture(divider, { pointerId: 4 }); await flushFrames();
    expect(calls('history_width_set')).toHaveLength(0);
  });

  it('responds immediately when reversing after reaching either width limit, as FinderX does', async () => {
    await mount('history'); const divider = screen.getByRole('separator');
    fireEvent.pointerDown(divider, { button: 0, pointerId: 3, screenX: 500 });
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 1000 }); await flushFrames();
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(720);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 990 }); await flushFrames();
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(710);
    fireEvent.pointerMove(divider, { pointerId: 3, screenX: 0 }); await flushFrames();
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(280);
    fireEvent.pointerUp(divider, { pointerId: 3, screenX: 10 });
    expect(calls('history_width_set').at(-1)?.[1]?.width).toBe(290);
  });

  it('hides the internal divider when only history is shown and retains records', async () => {
    await mount('history'); await windowState({ translationVisible: false, translationExtent: 0 });
    expect(screen.queryByRole('separator')).toBeNull(); expect(records()).toHaveLength(20);
  });
});
