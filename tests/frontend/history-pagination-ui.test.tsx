// @vitest-environment jsdom
import { act, fireEvent, renderHook, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';
import { historySamples, type HistorySnapshot } from '../../src/translation/history';
import { useHistory } from '../../src/translation/useHistory';

const rows = () => screen.queryAllByRole('button', { name: /^Translation:/ });
function populate(count = 240) {
  bridge.history.records = Array.from({ length: count }, (_, index) => ({
    ...historySamples[0], id: `page-${index}`, text: `Synthetic record ${index}`,
    translations: [`Translated record ${index}`], targetLanguages: ['en'],
    date: new Date(Date.UTC(2026, 0, 1, 0, 0, index)).toISOString(),
    isFavorite: index === 0, isPinned: false,
  }));
}
function bottom(element: HTMLElement) {
  Object.defineProperties(element, { clientHeight: { configurable: true, value: 300 }, scrollHeight: { configurable: true, value: 1_000 } });
  element.scrollTop = 750;
  fireEvent.scroll(element);
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

describe('paged database history', () => {
  it.each(['select', 'copy', 'favorite', 'delete', 'retranslate'] as const)('waits for the cached page query before executing %s on a record outside the backend projection', async (action) => {
    populate(3); bridge.snapshot.window.recordView = 'translation'; await mount('clipboard');
    const favorite = screen.getByRole('button', { name: 'Favorite history' });
    fireEvent.click(favorite);
    await waitFor(() => expect(rows()).toHaveLength(1));
    const pending = deferred<HistorySnapshot>(); const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: { favoritesOnly: boolean }) => name === 'history_query' && !args?.favoritesOnly ? pending.promise : original(name, args));
    fireEvent.click(favorite);
    const row = document.querySelector<HTMLElement>('#translation-history [data-record-id="page-1"]')!;
    expect(row).toBeTruthy();
    const command = action === 'copy' ? 'translation_sample_copy' : action === 'favorite' || action === 'delete' ? 'history_record_update' : 'history_sample_select';
    if (action === 'retranslate') { fireEvent.contextMenu(row); fireEvent.click(screen.getByRole('menuitem', { name: 'Retranslate' })); }
    else fireEvent.click(within(row).getByRole('button', { name: action === 'select' ? 'Translation: Synthetic record 1' : action === 'copy' ? 'Copy source text' : action === 'favorite' ? 'Favorite' : 'Delete history record' }));
    expect(calls(command)).toHaveLength(0);
    await act(async () => pending.resolve({ ...bridge.history, revision: bridge.history.revision + 1, generation: bridge.history.generation + 1, totalRecords: 3, hasMore: false }));
    await waitFor(() => expect(calls(command)).toHaveLength(1));
    expect(calls(command)[0][1]).toMatchObject(action === 'select' || action === 'retranslate' || action === 'copy' ? { sampleId: 'page-1' } : { recordId: 'page-1' });
  });

  it.each(['query', 'view', 'failure'] as const)('cancels a cached record action after %s changes while its query is pending', async (change) => {
    populate(3); bridge.snapshot.window.recordView = 'translation'; await mount('clipboard');
    const favorite = screen.getByRole('button', { name: 'Favorite history' });
    fireEvent.click(favorite); await waitFor(() => expect(rows()).toHaveLength(1));
    const pending = deferred<HistorySnapshot>(); const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: { query: string; favoritesOnly: boolean }) => name === 'history_query' && !args?.favoritesOnly && !args?.query ? pending.promise : original(name, args));
    fireEvent.click(favorite);
    fireEvent.click(screen.getByRole('button', { name: 'Translation: Synthetic record 1' }));
    if (change === 'query') fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'different' } });
    else if (change === 'view') await windowState({ recordView: 'clipboard', translationVisible: false });
    if (change === 'failure') await act(async () => pending.reject(new Error('storage_unavailable')));
    else await act(async () => pending.resolve({ ...bridge.history, revision: bridge.history.revision + 1, generation: bridge.history.generation + 1, totalRecords: 3, hasMore: false }));
    expect(calls('history_sample_select')).toHaveLength(0);
  });

  it('reconnects a failed history subscription before retrying and continues receiving events', async () => {
    populate();
    const original = bridge.listen.getMockImplementation()!;
    let fail = true;
    bridge.listen.mockImplementation((name: string, callback: (event: { payload: unknown }) => void) => {
      if (name === 'history_changed' && fail) { fail = false; return Promise.reject(new Error('listen_failed')); }
      return original(name, callback);
    });
    const view = await mount('history');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Retry' })).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(rows()).toHaveLength(100));
    await emit('history_changed', { ...bridge.history, revision: bridge.history.revision + 1, records: bridge.history.records.slice(0, 3), totalRecords: 3, hasMore: false });
    expect(rows()).toHaveLength(3);
    expect(bridge.listeners.get('history_changed')?.size).toBe(1);
    view.unmount();
    expect(bridge.listeners.get('history_changed')?.size).toBe(0);
  });

  it('restores an already browsed all page immediately while its authoritative query is still pending', async () => {
    populate(); await mount('history');
    const list = document.querySelector<HTMLElement>('.history-list')!;
    bottom(list);
    await waitFor(() => expect(rows()).toHaveLength(200));
    const allRecords = rows().map((row) => bridge.history.records.find((record) => record.id === row.closest('article')!.dataset.recordId)!);
    const favorite = screen.getByRole('button', { name: 'Favorite history' });
    fireEvent.click(favorite);
    await waitFor(() => expect(rows()).toHaveLength(1));
    const pending = deferred<HistorySnapshot>();
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: { query: string; favoritesOnly: boolean }) => name === 'history_query' && !args?.favoritesOnly ? pending.promise : original(name, args));
    fireEvent.click(favorite);
    expect(rows()).toHaveLength(200);
    expect(screen.getByText('240 records')).toBeTruthy();
    expect(screen.queryByRole('progressbar')).toBeNull();
    const loaded = calls('history_load_more').length;
    bottom(list);
    expect(calls('history_load_more')).toHaveLength(loaded);
    await act(async () => pending.resolve({ ...bridge.history, records: allRecords, revision: bridge.history.revision + 1, generation: bridge.history.generation + 1, totalRecords: 240, hasMore: true }));
    expect(rows()).toHaveLength(200);
  });

  it('invalidates alternate pages when a new record is excluded from an unchanged favorite snapshot', async () => {
    populate(3);
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    await act(async () => { await view.result.current.setQuery('', true, true); });
    expect(view.result.current.displaySnapshot('', false).records).toHaveLength(3);
    const favorite = bridge.history.records[0];
    const added = { ...bridge.history.records[1], id: 'excluded-new-record' };
    bridge.history.records.push(added); ++bridge.history.revision;
    await emit('history_changed', { ...bridge.history, records: [favorite], totalRecords: 1, hasMore: false });
    expect(view.result.current.displayReady('', false)).toBe(false);
    expect(view.result.current.displaySnapshot('', false).records).toEqual([favorite]);
    await act(async () => { await view.result.current.setQuery('', false, true); });
    expect(view.result.current.displaySnapshot('', false).records).toHaveLength(4);
    expect(view.result.current.displaySnapshot('', false).totalRecords).toBe(4);
  });

  it('does not revive alternate cached flags after a mutation or pages from a previous search', async () => {
    populate(3);
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    await act(async () => { await view.result.current.setQuery('', true, true); });
    expect(view.result.current.displayReady('', false)).toBe(true);
    await act(async () => { await view.result.current.update('page-0', 'favorite'); });
    expect(view.result.current.displayReady('', false)).toBe(false);
    await act(async () => { await view.result.current.setQuery('', false, true); });
    expect(view.result.current.displaySnapshot('', false).records.find((record) => record.id === 'page-0')?.isFavorite).toBe(false);
    await act(async () => { await view.result.current.setQuery('record 2', false); });
    expect(view.result.current.displayReady('', false)).toBe(false);
    expect(view.result.current.displayReady('record 2', false)).toBe(true);
    expect(view.result.current.displaySnapshot('record 2', false).records.map((record) => record.id)).toEqual(['page-2']);
  });

  it('keeps an alternate page invalid when an excluded record arrives between a query event and its response', async () => {
    populate(3);
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    const pending = deferred<HistorySnapshot>();
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_query' ? pending.promise : original(name, args));
    act(() => { void view.result.current.setQuery('', true, true); });
    const favorite = bridge.history.records[0];
    const first = { ...bridge.history, records: [favorite], totalRecords: 1, hasMore: false, generation: 1, revision: 1 };
    await emit('history_changed', first);
    expect(view.result.current.displayReady('', false)).toBe(true);
    await emit('history_changed', { ...first, revision: 2 });
    expect(view.result.current.displayReady('', false)).toBe(false);
    await act(async () => pending.resolve(first));
    expect(view.result.current.displayReady('', false)).toBe(false);
    expect(view.result.current.snapshot.revision).toBe(2);
    expect(view.result.current.displaySnapshot('', true).records).toEqual([favorite]);
  });

  it('renders only the first page and requests older records near the scroll boundary', async () => {
    populate(); await mount('history');
    expect(rows()).toHaveLength(100);
    expect(screen.queryByRole('button', { name: 'Translation: Synthetic record 0' })).toBeNull();
    bottom(document.querySelector('.history-list')!);
    await waitFor(() => expect(rows()).toHaveLength(200));
    expect(calls('history_load_more')).toHaveLength(1);
    bottom(document.querySelector('.history-list')!);
    await waitFor(() => expect(rows()).toHaveLength(240));
    bottom(document.querySelector('.history-list')!);
    expect(calls('history_load_more')).toHaveLength(2);
  });

  it('preserves backend order for records with identical timestamps', async () => {
    populate(3);for (const record of bridge.history.records) record.date='2026-01-01T00:00:00.000Z';
    const original=bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async(name:string,args?:unknown)=>name==='history_snapshot'?{...bridge.history,records:[bridge.history.records[1],bridge.history.records[0],bridge.history.records[2]]}:original(name,args));
    await mount('history');
    expect(rows().map((row)=>row.closest('article')?.dataset.recordId)).toEqual(['page-1','page-0','page-2']);
  });

  it('searches the database beyond loaded records and restores the initial page when cleared', async () => {
    const user = userEvent.setup(); populate(); await mount('history');
    await user.type(screen.getByRole('searchbox'), 'Translated record 0');
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: 'Translated record 0', favoritesOnly: false }));
    await waitFor(() => expect(rows()).toHaveLength(1));
    expect(rows()[0].getAttribute('aria-label')).toBe('Translation: Synthetic record 0');
    await user.click(screen.getByRole('button', { name: 'Clear history search' }));
    await waitFor(() => expect(rows()).toHaveLength(100));
    expect(calls('history_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: false });
  });

  it('keeps a full-text database match outside its summary and restores or copies full content by ID', async () => {
    const user = userEvent.setup(); populate();
    const full = bridge.history.records[0];
    full.text = `${'Source prefix '.repeat(30)}unabridged source tail`;
    full.translations = [`${'Translated prefix '.repeat(30)}hidden-translation-token`];
    const summarize = (snapshot: HistorySnapshot): HistorySnapshot => ({
      ...snapshot,
      records: snapshot.records.map((record) => ({ ...record, text: [...record.text].slice(0, 256).join(''), translations: record.translations.map((text) => [...text].slice(0, 256).join('')) })),
    });
    expect(full.translations[0].indexOf('hidden-translation-token')).toBeGreaterThan(256);
    expect(summarize({ ...bridge.history, records: [full] }).records[0].translations[0]).not.toContain('hidden-translation-token');
    const originalListen = bridge.listen.getMockImplementation()!;
    bridge.listen.mockImplementation((name: string, callback: (event: { payload: unknown }) => void) => originalListen(name, name === 'history_changed'
      ? ({ payload }: { payload: HistorySnapshot }) => callback({ payload: summarize(payload) }) : callback));
    const originalInvoke = bridge.invoke.getMockImplementation()!;
    const copied: string[] = [];
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'translation_sample_copy') {
        const { sampleId, part } = args as { sampleId: string; part: string };
        const record = bridge.history.records.find(({ id }) => id === sampleId)!;
        copied.push(part === 'source' ? record.text : record.translations[0]);
        return;
      }
      const result = await originalInvoke(name, args);
      return ['history_snapshot', 'history_query', 'history_load_more', 'history_record_update'].includes(name) ? summarize(result as HistorySnapshot) : result;
    });
    bridge.snapshot.capabilities.nativeHistory = false;
    bridge.snapshot.capabilities.nativeChrome = false;
    bridge.snapshot.window.recordView = 'translation';
    await mount();
    expect(rows()).toHaveLength(100);
    expect(rows().some((button) => button.closest('article')?.dataset.recordId === full.id)).toBe(false);
    await user.type(screen.getByRole('searchbox'), 'hidden-translation-token');
    await waitFor(() => expect(calls('history_query').at(-1)?.[1]).toEqual({ query: 'hidden-translation-token', favoritesOnly: false }));
    await waitFor(() => expect(rows()).toHaveLength(1));
    const match = rows()[0];
    expect(match.closest('article')?.dataset.recordId).toBe(full.id);
    expect(match.getAttribute('aria-label')).not.toContain('unabridged source tail');
    expect(match.closest('article')?.textContent).not.toContain('hidden-translation-token');
    await user.click(match);
    expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: full.id, retranslate: false });
    expect((screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement).value).toBe(full.text);
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe(full.translations[0]);
    await user.click(within(match.closest('article')!).getByRole('button', { name: 'Copy source text' }));
    expect(calls('translation_sample_copy').at(-1)?.[1]).toEqual({ sampleId: full.id, part: 'source' });
    expect(copied).toEqual([full.text]);
  });

  it('queries favorites from the full database immediately', async () => {
    const user = userEvent.setup(); populate(); await mount('history');
    await user.click(screen.getByRole('button', { name: 'Favorite history' }));
    expect(calls('history_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: true });
    expect(rows()).toHaveLength(1);
    expect(rows()[0].getAttribute('aria-label')).toBe('Translation: Synthetic record 0');
  });

  it('keeps filter queries quiet while waiting for the database without disabling pagination guards', async () => {
    populate(); const pending = deferred<HistorySnapshot>(); const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_query' ? pending.promise : original(name, args));
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    act(() => { void view.result.current.setQuery('', true, true); void view.result.current.loadMore(); });
    expect(view.result.current.loading).toBe(true);
    expect(view.result.current.showLoading).toBe(false);
    expect(calls('history_load_more')).toHaveLength(0);
    await act(async () => pending.resolve({ ...bridge.history, generation: 1, revision: 1, records: [bridge.history.records[0]] }));
    expect(view.result.current.loading).toBe(false);
    expect(view.result.current.snapshot.records).toHaveLength(1);
  });

  it('loads more from the pinned scroll region and coalesces repeated scroll events', async () => {
    populate(140); bridge.history.records.forEach((record) => { record.isPinned = true; });
    const pending = deferred<HistorySnapshot>(); const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_load_more' ? pending.promise : original(name, args));
    await mount('history');
    const scroller = document.querySelector<HTMLElement>('.history-pinned')!;
    bottom(scroller); bottom(scroller); bottom(scroller);
    expect(calls('history_load_more')).toHaveLength(1);
    expect(screen.getByRole('progressbar')).toBeTruthy();
    expect(screen.queryByText(/loading/i)).toBeNull();
    await act(async () => pending.resolve({ ...bridge.history, revision: 1, totalRecords: 140, hasMore: false }));
    expect(rows()).toHaveLength(140);
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('ignores a query response delivered after a later query even when its revision is greater', async () => {
    const first = deferred<HistorySnapshot>(); const second = deferred<HistorySnapshot>();
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: { query: string }) => name === 'history_query'
      ? (args?.query === 'old' ? first.promise : second.promise) : original(name, args));
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    act(() => { void view.result.current.setQuery('old', false); void view.result.current.setQuery('new', false); });
    await act(async () => second.resolve({ ...bridge.history, generation: 2, revision: 4, records: [bridge.history.records[1]] }));
    await act(async () => first.resolve({ ...bridge.history, generation: 1, revision: 5, records: [bridge.history.records[0]] }));
    expect(view.result.current.snapshot.records[0].id).toBe(bridge.history.records[1].id);
    expect(view.result.current.loading).toBe(false);
  });

  it('does not apply a late page from an earlier query and removes its listener on unmount', async () => {
    populate(); const page = deferred<HistorySnapshot>(); const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_load_more' ? page.promise : original(name, args));
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    act(() => { void view.result.current.loadMore(); });
    await act(async () => { await view.result.current.setQuery('record 0', false); });
    expect(view.result.current.snapshot.records).toHaveLength(1);
    await act(async () => page.resolve({ ...bridge.history, generation: 0, revision: 99 }));
    expect(view.result.current.snapshot.records).toHaveLength(1);
    const before = view.result.current.snapshot;
    view.unmount();
    await emit('history_changed', { ...bridge.history, revision: 100 });
    expect(view.result.current.snapshot).toBe(before);
    expect(bridge.listeners.get('history_changed')?.size).toBe(0);
  });

  it('preserves loaded records on page failure and permits retry', async () => {
    populate(); const original = bridge.invoke.getMockImplementation()!; let fail = true;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_load_more' && fail
      ? Promise.reject(new Error('storage_unavailable')) : original(name, args));
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    await act(async () => { await view.result.current.loadMore(); });
    expect(view.result.current.error).toBe(true);
    expect(view.result.current.snapshot.records).toHaveLength(100);
    expect(view.result.current.loading).toBe(false);
    fail = false;
    await act(async () => { await view.result.current.loadMore(); });
    expect(view.result.current.error).toBe(false);
    expect(view.result.current.snapshot.records).toHaveLength(200);
  });

  it('does not request an older page while a replacement query is pending', async () => {
    populate(); const pending = deferred<HistorySnapshot>(); const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'history_query' ? pending.promise : original(name, args));
    const view = renderHook(() => useHistory(true));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    act(() => { void view.result.current.setQuery('different', false); void view.result.current.loadMore(); });
    expect(calls('history_load_more')).toHaveLength(0);
    view.unmount();
    await act(async () => pending.resolve({ ...bridge.history, generation: 1, revision: 1, records: [] }));
    expect(bridge.listeners.get('history_changed')?.size).toBe(0);
  });

  it('cancels an unsubmitted search debounce when the panel unmounts', async () => {
    const user = userEvent.setup(); const view = await mount('history');
    await user.type(screen.getByRole('searchbox'), 'pending-query');
    view.unmount();
    await new Promise((resolve) => setTimeout(resolve, 180));
    expect(calls('history_query')).toHaveLength(0);
  });
});
