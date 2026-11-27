// @vitest-environment jsdom
// @ts-expect-error Vitest runs in Node; the app's TypeScript configuration includes browser types only.
import { readFileSync } from 'node:fs';
import { act, fireEvent, render, renderHook, screen, waitFor, within } from '@testing-library/react';
import { StrictMode } from 'react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';
import { ClipboardNotePreview } from '../../src/clipboard/ClipboardNotePreview';
import { useClipboard } from '../../src/clipboard/useClipboard';
import { translator } from '../../src/shared/i18n';
import { HistoryFooter } from '../../src/shared/HistoryFooter';
import { useRecordBrowser } from '../../src/shared/useRecordBrowser';
import type { ClipboardKind, ClipboardPage, ClipboardRecord } from '../../src/clipboard/useClipboard';

let records: ClipboardRecord[];
let queryHandler: ((args: { query: string; favoritesOnly: boolean; kind?: ClipboardKind; cursor: unknown }) => Promise<ClipboardPage>) | undefined;
const cursor = { pinned: false, createdAt: 1794650400000, sequence: 100 };
function page(query = '', favoritesOnly = false, kind?: ClipboardKind): ClipboardPage {
  const matches = records.filter((record) => (!favoritesOnly || record.isFavorite) && (!kind || record.kind === kind) && record.text.toLowerCase().includes(query.toLowerCase()));
  const pinPasteKeys = records.flatMap((record) => record.isPinned && record.pinPasteKey ? [{ id: record.id, key: record.pinPasteKey }] : []);
  return { records: structuredClone(matches), cursor: null, hasMore: false, totalRecords: matches.length, ...(pinPasteKeys.length ? { pinPasteKeys } : {}) };
}
const row = (id: string) => document.querySelector<HTMLElement>(`[data-record-id="${id}"]`)!;
function scrollNearEnd() {
  const element = document.querySelector<HTMLElement>('.history-list')!;
  Object.defineProperties(element, { clientHeight: { value: 100, configurable: true }, scrollHeight: { value: 500, configurable: true } });
  element.scrollTop = 400;
  fireEvent.wheel(element);
  fireEvent.scroll(element);
}
const listRows = () => document.querySelectorAll('.clipboard-panel .history-row:not([hidden])');

beforeEach(() => {
  records = [
    { id: 'first', kind: 'text', text: 'Saved text', date: '2026-11-14T12:00:00Z', isFavorite: false, isPinned: false },
    { id: 'pinned', kind: 'files', text: '/tmp/synthetic.txt', date: '2026-11-14T11:00:00Z', isFavorite: true, isPinned: true },
  ];
  queryHandler = undefined;
  const original = bridge.invoke.getMockImplementation()!;
  let revision = 0;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_query') {
      const request = args as { query: string; favoritesOnly: boolean; kind?: ClipboardKind; cursor: unknown };
      const encode = (page: ClipboardPage) => {
        const sources: Record<string, NonNullable<ClipboardRecord['sourceApp']>> = {};
        const records = page.records.map(({ sourceApp, ...record }) => {
          const id = sourceApp ? Object.keys(sources).find((key) => JSON.stringify(sources[key]) === JSON.stringify(sourceApp)) ?? String(Object.keys(sources).length + 1) : null;
          if (id) sources[id] = sourceApp!;
          return { ...record, sourceAppId: id ? Number(id) : null };
        });
        return { ...page, records, sources };
      };
      const result = queryHandler ? await queryHandler(request) : page(request.query, request.favoritesOnly, request.kind);
      return { page: encode(result), alternate: request.cursor ? null : encode(page(request.query, !request.favoritesOnly, request.kind)) };
    }
    if (name === 'clipboard_files') return [{ path: '/tmp/synthetic.txt', name: 'synthetic.txt', exists: true, isDir: false, isImage: false, iconDataUrl: null }];
    if (name === 'clipboard_record_detail') {
      const record = records.find(({ id }) => id === (args as { recordId: string }).recordId)!;
      return { title: record.title ?? '', text: record.text, kind: record.kind };
    }
    if (name === 'clipboard_record_update') {
      const { recordId, action, title, text } = args as { recordId: string; action: string; title?: string; text?: string };
      const record = records.find(({ id }) => id === recordId)!;
      if (action === 'edit') { record.title = title; if (text !== undefined) record.text = text; }
      if (action === 'delete') records = records.filter(({ id }) => id !== recordId);
      if (action === 'favorite') record.isFavorite = !record.isFavorite;
      if (action === 'pin') record.isPinned = !record.isPinned;
      bridge.listeners.get('clipboard_changed')?.forEach((callback) => callback({ payload: ++revision }));
      return;
    }
    if (name === 'clipboard_record_use') return { status: 'dispatched', copyStatus: 'current', code: null };
    return original(name, args);
  });
});

describe('clipboard type filters', () => {
  it('pastes the record on activation without showing a loading beam', async () => {
    await mount('clipboard'); await waitFor(() => expect(row('first')).toBeTruthy());
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: (value: unknown) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'clipboard_record_use' ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
    fireEvent.click(row('first').querySelector('.history-row-select')!);
    expect(screen.queryByRole('progressbar')).toBeNull();
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'activate' });
    await act(async () => finish({ status: 'dispatched', copyStatus: 'current', code: null }));
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('keeps the original rows and current category when an external capture creates a record', async () => {
    const original = { ...records[0], isFavorite: true };
    const newer = { ...original, id: 'newer', date: '2026-11-15T12:00:00Z' };
    records = [records[1], newer, original];
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    const favorites = screen.getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorites);
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await waitFor(() => expect(ids()).toEqual(['pinned', 'newer', 'first']));
    const originalRow = row('first');
    const list = originalRow.closest('.history-list')!;
    list.scrollTop = 20;
    expect(ids()).toEqual(['pinned', 'newer', 'first']);
    records = [records[0], { ...original, id: 'copy', isFavorite: false, date: '2099-01-01T00:00:00Z' }, newer, original];
    await emit('clipboard_changed', 1);
    expect(favorites.getAttribute('aria-pressed')).toBe('true');
    expect(row('first')).toBe(originalRow);
    expect(list.scrollTop).toBe(20);
    fireEvent.click(favorites);
    await waitFor(() => expect(ids()).toEqual(['pinned', 'copy', 'newer', 'first']));
  });

  it('explains a failed paste write without a loading beam', async () => {
    await mount('clipboard'); await waitFor(() => expect(row('first')).toBeTruthy());
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_use' ? { status: 'failed', copyStatus: 'not_written', code: 'clipboard_write' } : original(name, args));
    fireEvent.click(row('first').querySelector('.history-row-select')!);
    await screen.findByText('Could not paste the content. Try again.');
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('places a newly pinned old record ahead of existing pins while a scrolled page defers refresh', async () => {
    records[0].date = '2020-01-01T00:00:00Z';
    const originalDate = records[0].date;
    const view = renderHook(() => useClipboard(true, '', false, () => true));
    await waitFor(() => expect(view.result.current.page.records).toHaveLength(2));
    const queries = calls('clipboard_query').length;
    await act(async () => { await view.result.current.update('first', 'pin'); });
    expect(view.result.current.page.records.map(({ id }) => id)).toEqual(['first', 'pinned']);
    expect(view.result.current.page.records[0].date).toBe(originalDate);
    expect(calls('clipboard_query')).toHaveLength(queries);
    await act(async () => { await view.result.current.update('first', 'pin'); });
    expect(view.result.current.page.records.map(({ id }) => id)).toEqual(['pinned', 'first']);
  });

  it('switches favorite and type filters without combining them and resets on a new records session', async () => {
    records.push({ ...records[0], id: 'picture', kind: 'image', isFavorite: true });
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    const pressed = (name: string) => screen.getByRole('button', { name }).getAttribute('aria-pressed');
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(3));
    fireEvent.click(screen.getByRole('button', { name: 'File clipboard records' }));
    await waitFor(() => expect(ids()).toEqual(['pinned']));
    expect(pressed('Favorite clipboard records')).toBe('false');
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    await waitFor(() => expect(ids()).toEqual(['pinned', 'picture']));
    expect(pressed('File clipboard records')).toBe('false');
    expect(pressed('Favorite clipboard records')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Text clipboard records' }));
    await waitFor(() => expect(ids()).toEqual(['first']));
    expect(pressed('Favorite clipboard records')).toBe('false');
    expect(document.querySelector('.history-record-count')?.textContent).toBe('1 record');
    fireEvent.click(screen.getByRole('button', { name: 'Text clipboard records' }));
    await waitFor(() => expect(ids()).toEqual(['pinned', 'first', 'picture']));
    expect(pressed('All clipboard records')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    await waitFor(() => expect(ids()).toEqual(['picture']));
    fireEvent.click(screen.getByRole('button', { name: 'All clipboard records' }));
    await waitFor(() => expect(ids()).toEqual(['pinned', 'first', 'picture']));
    expect(pressed('All clipboard records')).toBe('true');
    expect(pressed('Image clipboard records')).toBe('false');
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    await waitFor(() => expect(listRows()).toHaveLength(3));
    expect(pressed('All clipboard records')).toBe('true');
    for (const name of ['Text clipboard records', 'Image clipboard records', 'File clipboard records', 'Favorite clipboard records']) expect(pressed(name)).toBe('false');
    expect(document.querySelector('.history-record-count')?.textContent).toBe('3 records');
  });

  it('shows shared and relevant shortcuts for each record view', () => {
    const props = { t: translator('en'), snapshot: bridge.snapshot, command: vi.fn(), native: true, active: true, search: { current: null }, total: 1 };
    const view = render(<HistoryFooter {...props} clipboard />);
    const helpButton = screen.getByRole('button', { name: 'HotKey' });
    expect(helpButton.title).toBe('HotKey');
    fireEvent.click(helpButton);
    expect(within(screen.getByRole('dialog', { name: 'HotKey' })).getByText('HotKey')).toBeTruthy();
    const entries = () => Object.fromEntries(Array.from(screen.getByRole('dialog', { name: 'HotKey' }).querySelectorAll('div'),
      (row) => [row.firstElementChild?.textContent, row.lastElementChild?.textContent]));
    expect(entries()).toMatchObject({ 'Paste selection': '↵', 'Navigate Records': '↑ / ↓', 'Switch Group': '← / →', 'Paste one of first 5 items': '⌘ 1～5', 'Open Clipboard History': '⌃ `', 'Paste as Plain Text': '⌘ ↵', 'Extract Text': '⇧ ⌘ O', Translate: '⌘ T', 'Hide window': 'Esc' });
    view.rerender(<HistoryFooter {...props} t={translator('zh-CN')} clipboard />);
    expect(entries()).toMatchObject({ '切换分组': '← / →', '粘贴前5项': '⌘ 1～5' });
    view.rerender(<HistoryFooter {...props} clipboard />);
    expect(entries()).not.toHaveProperty('Save Image');
    expect(entries()).not.toHaveProperty('Open Translation');
    expect(entries()).not.toHaveProperty('Copy Primary Translation');

    view.rerender(<HistoryFooter {...props} />);
    expect(entries()).toMatchObject({ 'Open selection': '↵', 'Navigate Records': '↑ / ↓', 'Open Translation': '⌘ T', 'Autofill Input': '⌘ T', Translate: '⌘ ↵', 'Copy Primary Translation': '⌘ 1' });
    expect(entries()).not.toHaveProperty('Switch Group');
    expect(entries()).not.toHaveProperty('Open Clipboard History');
    expect(entries()).not.toHaveProperty('Extract Text');

    const preferences = { ...bridge.snapshot.translationPreferences, secondaryLanguage: 'none', disabledShortcuts: ['translate'] as const };
    view.rerender(<HistoryFooter {...props} snapshot={{ ...props.snapshot, translationPreferences: { ...preferences, disabledShortcuts: [...preferences.disabledShortcuts] } }} />);
    expect(entries()).not.toHaveProperty('Translate');
    expect(entries()).toHaveProperty('Copy Secondary Translation', '⌘ 2');
  });

  it('uses the shared auto-hiding overlay scrollbar for overflowing HotKey popovers in both clipboard and translation views', () => {
    vi.useFakeTimers();
    try {
      const props = { t: translator('en'), snapshot: bridge.snapshot, command: vi.fn(), native: true, active: true, search: { current: null }, total: 1 };
      for (const clipboard of [true, false]) {
        const view = render(<HistoryFooter {...props} clipboard={clipboard} />);
        fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
        const dialog = screen.getByRole('dialog', { name: 'HotKey' });
        const scroller = dialog.querySelector<HTMLElement>('.history-shortcuts-scroll')!;
        expect(scroller.dataset.scrollable).toBe('false');
        expect(within(dialog).queryByRole('scrollbar')).toBeNull();

        Object.defineProperties(scroller, {
          clientHeight: { configurable: true, value: 216 },
          scrollHeight: { configurable: true, value: 516 },
        });
        scroller.scrollTop = 0;
        fireEvent.scroll(scroller);

        const scrollbar = within(dialog).getByRole('scrollbar', { name: 'Scroll HotKey' });
        expect(scroller.dataset.scrollable).toBe('true');
        expect(scrollbar.getAttribute('aria-valuemax')).toBe('300');
        expect(scrollbar.getAttribute('aria-valuenow')).toBe('0');
        expect(scrollbar.classList.contains('visible')).toBe(true);

        act(() => vi.advanceTimersByTime(1000));
        expect(scrollbar.classList.contains('visible')).toBe(false);

        fireEvent.pointerEnter(scrollbar);
        act(() => vi.advanceTimersByTime(1500));
        expect(scrollbar.classList.contains('visible')).toBe(true);
        fireEvent.pointerLeave(scrollbar);
        act(() => vi.advanceTimersByTime(1000));
        expect(scrollbar.classList.contains('visible')).toBe(false);

        fireEvent.keyDown(scrollbar, { key: 'End' });
        expect(scroller.scrollTop).toBe(300);
        expect(scrollbar.getAttribute('aria-valuenow')).toBe('300');
        expect(scrollbar.classList.contains('visible')).toBe(true);

        const thumb = scrollbar.firstElementChild as HTMLElement;
        fireEvent.pointerDown(thumb, { pointerId: 5, button: 0, clientY: 160 });
        fireEvent.pointerMove(scrollbar, { pointerId: 5, clientY: 110 });
        expect(scroller.scrollTop).toBeLessThan(300);
        act(() => vi.advanceTimersByTime(1500));
        expect(scrollbar.classList.contains('visible')).toBe(true);
        fireEvent.lostPointerCapture(scrollbar, { pointerId: 5 });

        fireEvent.keyDown(scrollbar, { key: 'Home' });
        expect(scroller.scrollTop).toBe(0);
        expect(scrollbar.getAttribute('aria-valuenow')).toBe('0');

        view.unmount();
        expect(vi.getTimerCount()).toBe(0);
      }
    } finally { vi.useRealTimers(); }
  });

  it('places mutually exclusive, cancellable type buttons after Favorites without changing translation controls', () => {
    const onKindChange = vi.fn();
    const props = { t: translator('en'), snapshot: bridge.snapshot, command: vi.fn(), native: true, active: true, search: { current: null } };
    const view = render(<HistoryFooter {...props} total={42} clipboard onFavorites={vi.fn()} kind="text" onKindChange={onKindChange} />);
    const filters = view.container.querySelector('.history-filters')!;
    expect(within(filters as HTMLElement).getAllByRole('button').map((button) => button.getAttribute('aria-label'))).toEqual(['All clipboard records', 'Favorite clipboard records', 'Text clipboard records', 'Image clipboard records', 'File clipboard records']);
    expect(screen.getByRole('button', { name: 'All clipboard records' }).getAttribute('aria-pressed')).toBe('false');
    expect(screen.getByRole('button', { name: 'Text clipboard records' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'All clipboard records' }));
    expect(onKindChange).toHaveBeenLastCalledWith(null);
    fireEvent.click(screen.getByRole('button', { name: 'Text clipboard records' }));
    expect(onKindChange).toHaveBeenLastCalledWith(null);
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    expect(onKindChange).toHaveBeenLastCalledWith('image');
    fireEvent.click(screen.getByRole('button', { name: 'File clipboard records' }));
    expect(onKindChange).toHaveBeenLastCalledWith('files');
    const calls = onKindChange.mock.calls.length;
    view.rerender(<HistoryFooter {...props} total={42} clipboard onFavorites={vi.fn()} kind={null} onKindChange={onKindChange} />);
    expect(screen.getByRole('button', { name: 'All clipboard records' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'All clipboard records' }));
    expect(onKindChange).toHaveBeenCalledTimes(calls);
    view.rerender(<HistoryFooter {...props} total={5} onFavorites={vi.fn()} />);
    expect(view.container.querySelectorAll('.history-kind')).toHaveLength(0);
    expect(screen.getByRole('button', { name: 'Favorite history' })).toBeTruthy();
  });

  it('queries authoritative type counts and isolates cached favorite pages by type without switch activity', async () => {
    records.push({ ...records[0], id: 'picture', kind: 'image', isFavorite: true, text: 'Saved image' });
    const hook = renderHook(({ kind, favorites }) => useClipboard(true, '', favorites, undefined, kind), { initialProps: { kind: null as ClipboardKind | null, favorites: false } });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(3));
    hook.rerender({ kind: 'image', favorites: false });
    expect(hook.result.current.showActivity).toBe(false);
    await waitFor(() => expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['picture']));
    await waitFor(() => {
      expect(hook.result.current.loading).toBe(false);
      expect(hook.result.current.page.totalRecords).toBe(1);
    });
    expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: false, kind: 'image', cursor: null });
    const queries = calls('clipboard_query').length;
    hook.rerender({ kind: 'image', favorites: true });
    expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['picture']);
    expect(calls('clipboard_query')).toHaveLength(queries);
    hook.rerender({ kind: 'files', favorites: true });
    expect(hook.result.current.showActivity).toBe(false);
    await waitFor(() => expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['pinned']));
    expect(hook.result.current.page.totalRecords).toBe(1);
    hook.rerender({ kind: 'image', favorites: false });
    expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['picture']);
    expect(calls('clipboard_query')).toHaveLength(queries + 1);
    hook.rerender({ kind: null, favorites: false });
    expect(hook.result.current.page.records).toHaveLength(3);
    expect(calls('clipboard_query')).toHaveLength(queries + 1);
  });

  it('patches cached type totals independently when a favorite changes during deferred scrolling', async () => {
    records.push({ ...records[0], id: 'picture', kind: 'image', isFavorite: true, text: 'Saved image' });
    const hook = renderHook(({ kind }) => useClipboard(true, '', false, () => true, kind), { initialProps: { kind: null as ClipboardKind | null } });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(3));
    hook.rerender({ kind: 'files' });
    await waitFor(() => expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['pinned']));
    hook.rerender({ kind: 'image' });
    await waitFor(() => expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['picture']));
    await act(async () => hook.result.current.update('picture', 'favorite'));
    expect(hook.result.current.page.totalRecords).toBe(1);
    hook.rerender({ kind: 'files' });
    expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['pinned']);
    expect(hook.result.current.page.totalRecords).toBe(1);
    hook.rerender({ kind: null });
    expect(hook.result.current.page.totalRecords).toBe(3);
    expect(hook.result.current.page.records.find(({ id }) => id === 'picture')?.isFavorite).toBe(false);
  });

  it('rejects late type responses and uses the current type in search and cursor requests', async () => {
    records.push({ ...records[0], id: 'picture', kind: 'image', text: 'Saved image' });
    let finish: (value: ClipboardPage) => void = () => {};
    queryHandler = async (request) => request.kind === 'image' ? new Promise((resolve) => { finish = resolve; }) : { ...page(request.query, request.favoritesOnly, request.kind), cursor, hasMore: !request.cursor };
    const hook = renderHook(({ kind, query }) => useClipboard(true, query, false, undefined, kind), { initialProps: { kind: null as ClipboardKind | null, query: '' } });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(3));
    hook.rerender({ kind: 'image', query: '' });
    await waitFor(() => expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ kind: 'image' }));
    hook.rerender({ kind: 'text', query: 'Saved' });
    await waitFor(() => expect(hook.result.current.displayReady).toBe(true));
    expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['first']);
    await act(async () => finish(page('', false, 'image')));
    expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['first']);
    await act(async () => hook.result.current.loadMore());
    expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: 'Saved', favoritesOnly: false, kind: 'text', cursor });
  });

  it('preserves independent scroll positions for type and favorite combinations and resets them per session', () => {
    const element = document.createElement('div');
    Object.defineProperties(element, { clientHeight: { value: 100 }, scrollHeight: { value: 2000 } });
    element.scrollTo = vi.fn((options?: ScrollToOptions | number, y?: number) => { element.scrollTop = typeof options === 'number' ? y ?? 0 : options?.top ?? 0; });
    const hook = renderHook(({ kind, session }) => useRecordBrowser(session, true, { current: element }, () => true, () => undefined, kind), { initialProps: { kind: null as ClipboardKind | null, session: 1 } });
    element.scrollTop = 320;
    act(() => hook.result.current.remember());
    hook.rerender({ kind: 'image', session: 1 });
    expect(element.scrollTop).toBe(0);
    element.scrollTop = 120;
    act(() => hook.result.current.toggleFavorites());
    expect(element.scrollTop).toBe(0);
    element.scrollTop = 45;
    act(() => hook.result.current.remember());
    hook.rerender({ kind: null, session: 1 });
    expect(element.scrollTop).toBe(0);
    act(() => hook.result.current.toggleFavorites());
    expect(element.scrollTop).toBe(320);
    hook.rerender({ kind: 'image', session: 1 });
    expect(element.scrollTop).toBe(120);
    hook.rerender({ kind: 'image', session: 2 });
    expect(element.scrollTop).toBe(0);
    hook.rerender({ kind: null, session: 2 });
    expect(element.scrollTop).toBe(0);
  });
});

describe('clipboard text processing', () => {
  it('shows translation and OCR for eligible records regardless of favorite status in translate / OCR order', async () => {
    records = [
      { ...records[0], isFavorite: true },
      { ...records[0], id: 'ordinary' },
      { ...records[0], id: 'blank', text: '   ', isFavorite: true },
      { ...records[0], id: 'text-image', kind: 'image', isFavorite: true },
      { ...records[0], id: 'ordinary-image', kind: 'image' },
      { ...records[1] },
    ];
    await mount('clipboard');
    await waitFor(() => expect(within(row('text-image')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    const local = 'Recognizes text on this Mac without sending the image or text.';
    const online = 'Sends text to the selected translation service.';
    expect(within(row('text-image')).getByRole('button', { name: 'Extract text' }).getAttribute('aria-description')).toBe(local);
    expect(within(row('text-image')).getByRole('button', { name: 'Translate' }).getAttribute('aria-description')).toBe(online);
    fireEvent.contextMenu(row('text-image'));
    expect(screen.getByRole('menuitem', { name: 'Extract Text' }).getAttribute('aria-description')).toBe(local);
    expect(screen.getByRole('menuitem', { name: 'Translate' }).getAttribute('aria-description')).toBe(online);
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    const actions = (id: string) => within(row(id)).getAllByRole('button').filter((button) => button.closest('.history-row-actions')).map((button) => button.getAttribute('aria-label'));
    expect(actions('first')).toEqual(['Translate', 'Note', 'Remove favorite', 'Pin record', 'Delete clipboard record']);
    expect(actions('text-image')).toEqual(['Translate', 'Extract text', 'Note', 'Remove favorite', 'Pin record', 'Delete clipboard record']);
    expect(actions('ordinary')).toEqual(['Translate', 'Note', 'Favorite', 'Pin record', 'Delete clipboard record']);
    expect(actions('ordinary-image')).toEqual(['Translate', 'Extract text', 'Note', 'Favorite', 'Pin record', 'Delete clipboard record']);
    for (const id of ['blank', 'pinned']) {
      expect(within(row(id)).queryByRole('button', { name: 'Translate' })).toBeNull();
      expect(within(row(id)).queryByRole('button', { name: 'Extract text' })).toBeNull();
    }
    expect(calls('clipboard_record_has_text')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(within(row('first')).getByRole('button', { name: 'Translate' })).toBeTruthy();
  });

  it('omits OCR and Translate actions for all file records including single image files while keeping thumbnail preview', async () => {
    records = ['image-file', 'blank-file', 'document-file', 'multiple-files'].map((id) => ({ ...records[0], id, kind: 'files' }));
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name !== 'clipboard_files') return original(name, args);
      const { recordId } = args as { recordId: string };
      return Array.from({ length: recordId === 'multiple-files' ? 2 : 1 }, (_, i) => ({ path: `/synthetic/${i}`, name: String(i), exists: true, isDir: false, isImage: recordId !== 'document-file', iconDataUrl: null }));
    });
    await mount('clipboard');
    await waitFor(() => expect(row('image-file').querySelector('.clipboard-image-preview img')).toBeTruthy());
    for (const id of ['image-file', 'blank-file', 'document-file', 'multiple-files']) {
      expect(within(row(id)).queryByRole('button', { name: 'Translate' })).toBeNull();
      expect(within(row(id)).queryByRole('button', { name: 'Extract text' })).toBeNull();
    }
    expect(calls('clipboard_record_has_text')).toHaveLength(0);
    // Context menu also omits Translate and Extract Text for file records.
    for (const id of ['image-file', 'document-file']) {
      fireEvent.contextMenu(row(id));
      const menu = screen.getByRole('menu');
      expect(within(menu).queryByRole('menuitem', { name: /Translate/i })).toBeNull();
      expect(within(menu).queryByRole('menuitem', { name: /Extract text/i })).toBeNull();
      fireEvent.pointerDown(document.body);
    }
  });

  it('rechecks image files after reopening and switches preview back to file list when the file changes or disappears', async () => {
    records = [{ ...records[0], id: 'image-file', kind: 'files' }];
    let isImage = true, missing = false;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_files') return [{ path: '/synthetic/file', name: 'file', exists: !missing, isDir: false, isImage, iconDataUrl: null }];
      return original(name, args);
    });
    await mount('clipboard');
    await waitFor(() => expect(row('image-file').querySelector('.clipboard-image-preview img')).toBeTruthy());
    expect(within(row('image-file')).queryByRole('button', { name: 'Extract text' })).toBeNull();
    expect(within(row('image-file')).queryByRole('button', { name: 'Translate' })).toBeNull();
    const reopen = async (update: () => void) => {
      await windowState({ clipboardVisible: false, clipboardExtent: 0 });
      update();
      await windowState({ clipboardVisible: true, clipboardExtent: bridge.snapshot.window.clipboardWidth });
    };
    await reopen(() => { isImage = false; });
    await waitFor(() => expect(row('image-file').querySelector('.clipboard-files')).toBeTruthy());
    await reopen(() => { isImage = true; });
    await waitFor(() => expect(row('image-file').querySelector('.clipboard-image-preview img')).toBeTruthy());
    await reopen(() => { missing = true; });
    await waitFor(() => expect(row('image-file').querySelector('.clipboard-file[data-missing="true"]')).toBeTruthy());
    expect(calls('clipboard_record_has_text')).toHaveLength(0);
    expect(calls('clipboard_files')).toHaveLength(4);
  });

  it('keeps text translation available without image OCR capability and does not probe images', async () => {
    bridge.snapshot.capabilities.imageOcr = false;
    records = [{ ...records[0], isFavorite: true }, { ...records[0], id: 'picture', kind: 'image', isFavorite: true }];
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(within(row('first')).getByRole('button', { name: 'Translate' })).toBeTruthy();
    expect(within(row('picture')).queryByRole('button', { name: 'Translate' })).toBeNull();
    expect(within(row('picture')).queryByRole('button', { name: 'Extract text' })).toBeNull();
    expect(calls('clipboard_record_has_text')).toHaveLength(0);
  });

  it.each([['Extract Text', 'ocr'], ['Translate', 'translate']])('offers explicit %s on unknown images without probing and hides both when nothing is recognized', async (label, action) => {
    records = [{ ...records[0], id: 'picture', kind: 'image' }];
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => {
      if (name === 'clipboard_record_process') throw 'ocr_no_text';
      return invoke(name, args);
    });
    await mount('clipboard');
    await waitFor(() => expect(row('picture')).toBeTruthy());
    await act(async () => {});
    expect(calls('clipboard_record_has_text')).toHaveLength(0);
    expect(calls('clipboard_record_process')).toHaveLength(0);
    fireEvent.contextMenu(row('picture'));
    fireEvent.click(screen.getByRole('menuitem', { name: label }));
    await waitFor(() => expect(within(row('picture')).queryByRole('button', { name: 'Extract text' })).toBeNull());
    expect(within(row('picture')).queryByRole('button', { name: 'Translate' })).toBeNull();
    expect(screen.queryByText('No text recognized')).toBeNull();
    expect(screen.queryByText('未识别到文字')).toBeNull();
    expect(calls('clipboard_record_process').map(([, args]) => args)).toEqual([{ recordId: 'picture', action }]);
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('hides extract and translate on an image whose stored OCR text is empty', async () => {
    records = [{ ...records[0], id: 'blank', kind: 'image', text: 'Image', imageTextEmpty: true }];
    await mount('clipboard');
    await waitFor(() => expect(row('blank')).toBeTruthy());
    expect(within(row('blank')).queryByRole('button', { name: 'Extract text' })).toBeNull();
    expect(within(row('blank')).queryByRole('button', { name: 'Translate' })).toBeNull();
    expect(screen.queryByText('No text recognized')).toBeNull();
    fireEvent.contextMenu(row('blank'));
    expect(screen.queryByRole('menuitem', { name: 'Extract Text' })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: 'Translate' })).toBeNull();
  });

  it('opens the highlighted record note from search and leaves typing in the editor alone', async () => {
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    const search = screen.getByRole('searchbox');
    fireEvent.mouseMove(row('pinned'));
    fireEvent.keyDown(search, { key: 'n', code: 'KeyN', metaKey: true });
    const editor = await screen.findByRole('dialog', { name: 'Note' });
    expect(calls('clipboard_record_detail').at(-1)?.[1]).toEqual({ recordId: 'pinned' });
    fireEvent.keyDown(within(editor).getByRole('textbox', { name: 'Title' }), { key: 't', code: 'KeyT', metaKey: true });
    expect(calls('clipboard_record_process')).toHaveLength(0);
  });

  it('uses the eligible highlighted record for Translate and OCR and ignores IME, disabled and busy shortcuts', async () => {
    records = [{ ...records[0] }, { ...records[0], id: 'picture', kind: 'image', text: 'Example image' }];
    let finish: () => void = () => {};
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_process') return new Promise<void>((resolve) => { finish = resolve; });
      return original(name, args);
    });
    await mount('clipboard');
    const search = screen.getByRole('searchbox');
    await waitFor(() => expect(within(row('picture')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    const press = (key: string, options = {}) => fireEvent.keyDown(search, { key, code: `Key${key}`, metaKey: true, shiftKey: key === 'O', ...options });
    press('O');
    press('T', { isComposing: true });
    expect(calls('clipboard_record_process')).toHaveLength(0);
    press('T');
    expect(calls('clipboard_record_process').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'translate' });
    press('T');
    expect(calls('clipboard_record_process')).toHaveLength(1);
    await act(async () => finish());
    fireEvent.change(search, { target: { value: 'Example image' } });
    await waitFor(() => expect(row('picture').hasAttribute('data-keyboard-current')).toBe(true));
    const { shortcuts } = bridge.snapshot.clipboardControls;
    await emit('clipboard_controls_changed', { shortcuts: { ...shortcuts, ocr: { ...shortcuts.ocr, enabled: false } } });
    press('O');
    expect(calls('clipboard_record_process')).toHaveLength(1);
    await emit('clipboard_controls_changed', { shortcuts });
    press('O');
    expect(calls('clipboard_record_process').at(-1)?.[1]).toEqual({ recordId: 'picture', action: 'ocr' });
    await act(async () => finish());
    press('T');
    expect(calls('clipboard_record_process').at(-1)?.[1]).toEqual({ recordId: 'picture', action: 'translate' });
    await act(async () => finish());
  });

  it('routes the global Translate key to the current menu record, but never while composing or out of focus', async () => {
    records = [{ ...records[0] }, { ...records[0], id: 'picture', kind: 'image' }];
    await mount('clipboard');
    const search = screen.getByRole('searchbox');
    await waitFor(() => expect(row('picture')).toBeTruthy());
    search.focus();
    fireEvent.compositionStart(search);
    await emit('clipboard_translate_requested', null);
    fireEvent.compositionEnd(search);
    const focused = vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    try { await emit('clipboard_translate_requested', null); }
    finally { focused.mockRestore(); }
    expect(calls('clipboard_record_process')).toHaveLength(0);
    fireEvent.contextMenu(row('picture'));
    expect(screen.getByRole('menuitem', { name: 'Translate' }).querySelector('kbd')?.textContent).toBe('⌘T');
    await emit('clipboard_translate_requested', null);
    expect(calls('clipboard_record_process')[0]?.[1]).toEqual({ recordId: 'picture', action: 'translate' });
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('dispatches text/image translation and OCR by record ID without invoking copy or paste', async () => {
    records = [{ ...records[0] }, { ...records[0], id: 'picture', kind: 'image' }];
    await mount('clipboard'); await waitFor(() => expect(within(row('picture')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    for (const [id, label] of [['first', 'Translate'], ['picture', 'Translate'], ['picture', 'Extract text']]) {
      await act(async () => fireEvent.click(within(row(id)).getByRole('button', { name: label })));
    }
    expect(calls('clipboard_record_process').map(([, args]) => args)).toEqual([{ recordId: 'first', action: 'translate' }, { recordId: 'picture', action: 'translate' }, { recordId: 'picture', action: 'ocr' }]);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(document.querySelector('.record-feedback')).toBeNull();
  });

  it.each([['text', 'Translate', 'Unable to open this text for translation. Try again.'], ['image', 'Extract text', 'Unable to extract text. Try again.']] as const)('reports %s processing failures in English and allows another attempt', async (kind, label, message) => {
    records = [{ ...records[0], kind, isFavorite: true }];
    let fail = true;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_process') { if (fail) throw new Error('synthetic_failure'); return; }
      return original(name, args);
    });
    await mount('clipboard');
    const button = await within(row('first') ?? document.body).findByRole('button', { name: label });
    await act(async () => fireEvent.click(button));
    expect(screen.getByRole('status').textContent).toBe(message);
    expect((button as HTMLButtonElement).disabled).toBe(false);
    fail = false;
    await act(async () => fireEvent.click(button));
    expect(document.querySelector('.record-feedback')).toBeNull();
    expect(calls('clipboard_record_process')).toHaveLength(2);
  });

  it('releases processing controls after another allowed clipboard operation while OCR is pending', async () => {
    records = [{ ...records[0], kind: 'image', isFavorite: true }];
    let finish: () => void = () => {};
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_process') return new Promise<void>((resolve) => { finish = resolve; });
      return original(name, args);
    });
    await mount('clipboard');
    const process = await within(row('first') ?? document.body).findByRole('button', { name: 'Extract text' }) as HTMLButtonElement;
    fireEvent.click(process);
    await act(async () => fireEvent.click(row('first').querySelector('.history-row-select')!));
    await act(async () => finish());
    expect(process.disabled).toBe(false);
    expect(document.querySelector('.record-feedback')).toBeNull();
  });

  it('blocks repeat processing while pending and clears stale feedback when leaving the view', async () => {
    records = [{ ...records[0], isFavorite: true }];
    let reject: (reason: Error) => void = () => {};
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_process' ? new Promise((_, rejectRequest) => { reject = rejectRequest; }) : original(name, args));
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(1));
    const button = within(row('first')).getByRole('button', { name: 'Translate' }) as HTMLButtonElement;
    fireEvent.click(button);
    expect(button.disabled).toBe(true);
    fireEvent.click(button);
    expect(calls('clipboard_record_process')).toHaveLength(1);
    await windowState({ clipboardVisible: false, clipboardExtent: 0 });
    await act(async () => reject(new Error('late_failure')));
    await windowState({ clipboardVisible: true, clipboardExtent: bridge.snapshot.window.clipboardWidth });
    expect(document.querySelector('.record-feedback')).toBeNull();
    expect((within(row('first')).getByRole('button', { name: 'Translate' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('selects the text record created by OCR, clearing filters that would hide it', async () => {
    records = [
      { id: 'picture', kind: 'image', text: 'Image', date: '2026-11-14T12:00:00Z', isFavorite: false, isPinned: false },
      { id: 'pinned', kind: 'text', text: 'Pinned text', date: '2026-11-14T11:00:00Z', isFavorite: true, isPinned: true },
    ];
    await mount('clipboard');
    await waitFor(() => expect(row('picture')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'image' } });
    await waitFor(() => expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ query: 'image', kind: 'image' }));
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('picture'));
    act(() => row('picture').querySelector<HTMLButtonElement>('.history-row-select')!.focus());
    records.unshift({ id: 'ocr-text', kind: 'text', text: 'Recognized sentence', date: '2026-11-14T13:00:00Z', isFavorite: false, isPinned: false });
    await windowState({ recordsBrowsing: false, clipboardDetailId: 'ocr-text' });
    await emit('clipboard_text_created', 'ocr-text');
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('ocr-text'));
    expect(document.activeElement).toBe(row('ocr-text').querySelector('.history-row-select'));
    fireEvent.keyDown(document.activeElement!, { key: 'Enter', code: 'Enter' });
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'ocr-text', action: 'activate' });
    expect(row('ocr-text').classList.contains('selected')).toBe(true);
    expect(row('picture').classList.contains('selected')).toBe(false);
    expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('');
    expect(screen.getByRole('button', { name: 'Image clipboard records' }).getAttribute('aria-pressed')).toBe('false');
    const selects = calls('clipboard_details_select').length;
    records.unshift({ id: 'ocr-later', kind: 'text', text: 'Second sentence', date: '2026-11-14T14:00:00Z', isFavorite: false, isPinned: false });
    fireEvent.click(screen.getByRole('button', { name: 'Collapse details' }));
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailVisible).toBe(false));
    await windowState({ recordsBrowsing: false, clipboardDetailId: 'ocr-later' });
    await emit('clipboard_text_created', 'ocr-later');
    await waitFor(() => expect(row('ocr-later').classList.contains('selected')).toBe(true));
    expect(calls('clipboard_details_select')).toHaveLength(selects);
    expect(bridge.snapshot.window.clipboardDetailVisible).toBe(false);
  });

  it('does not replace an explicitly browsed record when an OCR completion arrives later', async () => {
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('first'));
    const selections = calls('clipboard_details_select').length;
    records.unshift({ ...records[0], id: 'late-ocr', text: 'Late recognized text' });
    await emit('clipboard_text_created', 'late-ocr');
    expect(bridge.snapshot.window.clipboardDetailId).toBe('first');
    expect(calls('clipboard_details_select')).toHaveLength(selections);
  });
});

describe('database clipboard history', () => {
  it('disables unavailable menu paste while keeping focus navigation available', async () => {
    bridge.snapshot.capabilities.autoPaste = false;
    bridge.snapshot.capabilities.clipboardWrite = false;
    await mount('clipboard'); await waitFor(()=>expect(row('first')).toBeTruthy());
    fireEvent.contextMenu(row('first'));
    const paste = screen.getByRole('menuitem', { name: 'Paste' });
    expect(paste).toHaveProperty('disabled', true);
    expect(paste.querySelector('kbd')?.textContent).toBe('—');
    expect(screen.getByRole('menuitem', { name: 'Paste as Plain Text' })).toHaveProperty('disabled', true);
    expect(screen.queryByRole('menuitem',{name:'Extract text'})).toBeNull();
    fireEvent.click(paste);
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('marks a right-click target without changing the selected record and clears it on dismiss', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.focus(row('first').querySelector('button')!);
    fireEvent.contextMenu(row('pinned'));
    expect(row('pinned').classList.contains('context-target')).toBe(true);
    expect(row('first').querySelector('button')?.getAttribute('aria-pressed')).toBe('true');
    expect(row('pinned').querySelector('button')?.getAttribute('aria-pressed')).toBe('false');
    fireEvent.pointerDown(document.body);
    expect(row('pinned').classList.contains('context-target')).toBe(false);
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('dispatches available image processing from the menu through the same record actions', async () => {
    records = [{ ...records[0], kind: 'image' }];
    await mount('clipboard');
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    for (const [label, action] of [['Translate', 'translate'], ['Extract Text', 'ocr']]) {
      fireEvent.contextMenu(row('first'));
      fireEvent.click(screen.getByRole('menuitem', { name: label }));
      await waitFor(() => expect(calls('clipboard_record_process').at(-1)?.[1]).toEqual({ recordId: 'first', action }));
    }
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('opens capability-matched record actions without pasting, with icons and confirmed delete for marked records', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.contextMenu(row('first'), { clientX: 190, clientY: 200 });
    const menu = screen.getByRole('menu');
    const items = within(menu).getAllByRole('menuitem');
    const layout = [...menu.querySelectorAll('[role=menuitem], [role=separator]')].map((item) =>
      item.getAttribute('role') === 'separator' ? '/' : [item.querySelector('.record-menu-label')?.textContent, item.querySelector('kbd')?.textContent]);
    expect(layout).toEqual([
      ['Paste', '↵'], ['Paste as Plain Text', '⌘↵'], '/',
      ['Favorite', '⌘D'], ['Pin', '⌥⌘P'], ['Note', '⌘N'], '/', ['Delete', '⌘⌫'],
    ]);
    expect(items.every((item) => item.children.length === 3)).toBe(true);
    expect(items.at(-1)?.classList.contains('clipboard-menu-delete')).toBe(true);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(document.activeElement).toBe(items[0]);
    fireEvent.keyDown(items[0], { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Favorite' }));
    await waitFor(() => expect(records[0].isFavorite).toBe(true));
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.contextMenu(row('pinned'));
    expect(screen.queryByRole('menuitem', { name: 'Paste as Plain Text' })).toBeNull();
    const remove = within(screen.getByRole('menu')).getByRole('menuitem', { name: 'Delete' });
    fireEvent.click(remove);
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete this record?' });
    expect(row('pinned')).toBeTruthy();
    fireEvent.keyDown(dialog, { key: 'Enter', code: 'Enter', metaKey: true });
    expect(screen.getByRole('alertdialog', { name: 'Delete this record?' })).toBe(dialog);
    expect(calls('clipboard_record_update').filter(([, args]) => (args as { action?: string }).action === 'delete')).toHaveLength(0);
    const user = userEvent.setup();
    within(dialog).getByRole('button', { name: 'Cancel' }).focus();
    await user.keyboard('{Enter}');
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(row('pinned')).toBeTruthy();
    fireEvent.contextMenu(row('pinned'));
    fireEvent.click(within(screen.getByRole('menu')).getByRole('menuitem', { name: 'Delete' }));
    fireEvent.keyDown(screen.getByRole('alertdialog'), { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(row('pinned')).toBeNull());
    expect(screen.queryByRole('menu')).toBeNull();
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('pastes by menu record ID and dismisses on outside pointer or scroll', async () => {
    const user = userEvent.setup();
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.contextMenu(row('first'));
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Paste' }));
    await user.keyboard('{Enter}');
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'paste' }));
    fireEvent.contextMenu(row('first'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Paste as Plain Text' }));
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'pastePlain' }));
    fireEvent.contextMenu(row('first'));
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.contextMenu(row('first'));
    fireEvent.scroll(document.querySelector('.history-list')!);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('uses Command-Return for plain text directly on text rows and in the text menu, ignoring non-text rows', async () => {
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    const key = { key: 'Enter', code: 'Enter', metaKey: true };
    fireEvent.contextMenu(row('pinned'));
    expect(fireEvent.keyDown(document.activeElement!, key)).toBe(false);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    fireEvent.keyDown(row('pinned').querySelector('button')!, key);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.contextMenu(row('first'));
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Delete' }));
    fireEvent.keyDown(document.activeElement!, { ...key, repeat: true });
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.keyDown(document.activeElement!, key);
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'pastePlain' }));
    expect(calls('clipboard_record_update')).toHaveLength(0);
  });

  it('ignores the retired image-save shortcut on the row and in its context menu', async () => {
    records[0] = { ...records[0], id: 'picture', kind: 'image' };
    await mount('clipboard');
    await waitFor(() => expect(row('picture')).toBeTruthy());
    const select = row('picture').querySelector<HTMLButtonElement>('.history-row-select')!;
    const shortcut = { key: 's', code: 'KeyS', metaKey: true };
    select.focus();
    fireEvent.keyDown(select, shortcut);
    expect(calls('clipboard_image_save')).toHaveLength(0);
    expect(document.activeElement).toBe(select);

    fireEvent.contextMenu(row('picture'));
    const menu = screen.getByRole('menu');
    expect(within(menu).queryByRole('menuitem', { name: 'Save' })).toBeNull();
    const focused = document.activeElement!;
    fireEvent.keyDown(focused, shortcut);
    expect(screen.getByRole('menu')).toBe(menu);
    expect(document.activeElement).toBe(focused);
    expect(calls('clipboard_image_save')).toHaveLength(0);
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('groups and labels image menu actions and runs the focused item on Return', async () => {
    records[0] = { ...records[0], id: 'picture', kind: 'image' };
    await mount('clipboard');
    await waitFor(() => expect(row('picture')).toBeTruthy());
    const open = () => { fireEvent.contextMenu(row('picture')); return screen.getByRole('menu'); };
    const menu = open();
    expect(menu.style.width).toBe('220px');
    const layout = [...menu.querySelectorAll('[role=menuitem], [role=separator]')].map((item) =>
      item.getAttribute('role') === 'separator' ? '/' : [item.querySelector('.record-menu-label')?.textContent, item.querySelector('kbd')?.textContent]);
    expect(layout).toEqual([
      ['Paste', '↵'], '/',
      ['Favorite', '⌘D'], ['Pin', '⌥⌘P'], ['Note', '⌘N'], '/',
      ['Translate', '⌘T'], ['Extract Text', '⇧⌘O'], '/', ['Delete', '⌘⌫'],
    ]);
    fireEvent.scroll(menu);
    expect(screen.getByRole('menu')).toBe(menu);
    fireEvent.keyDown(document.activeElement!, { key: 'End', code: 'End' });
    expect(document.activeElement).toBe(within(menu).getByRole('menuitem', { name: 'Delete' }));
    const user = userEvent.setup();
    await user.keyboard('{Home}{ArrowDown}');
    const favorite = within(menu).getByRole('menuitem', { name: 'Favorite' });
    expect(document.activeElement).toBe(favorite);
    expect(fireEvent.keyDown(favorite, { key: 'ArrowDown', altKey: true })).toBe(true);
    expect(document.activeElement).toBe(favorite);
    await user.keyboard('{Enter}');
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'picture', action: 'favorite' }));
    expect(calls('clipboard_record_use')).toHaveLength(0);
    open();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'picture', action: 'paste' }));
  });

  it('reflects customized and disabled shortcuts without stealing a conflicting menu chord', async () => {
    records[0] = { ...records[0], id: 'picture', kind: 'image', isPinned: true, isFavorite: true };
    const shortcuts = bridge.snapshot.clipboardControls.shortcuts;
    shortcuts.pin = { binding: { code: 'KeyU', modifiers: ['meta'] }, enabled: true };
    bridge.snapshot.translationPreferences.shortcuts.openSettings = { code: 'KeyN', modifiers: ['meta'] };
    shortcuts.favorite = { ...shortcuts.favorite, enabled: false };
    bridge.snapshot.capabilities.imageOcr = false;
    await mount('clipboard');
    await waitFor(() => expect(row('picture')).toBeTruthy());
    fireEvent.contextMenu(row('picture'));
    expect(screen.getAllByRole('separator')).toHaveLength(2); // The empty translation/OCR group is omitted.
    const item = (name: string) => screen.getByRole('menuitem', { name });
    expect(item('Unfavorite').querySelector('kbd')?.textContent).toBe('—');
    expect(item('Unpin').querySelector('kbd')?.textContent).toBe('⌘U');
    expect(item('Note').querySelector('kbd')?.textContent).toBe('—');
    fireEvent.keyDown(document.activeElement!, { key: 'd', code: 'KeyD', metaKey: true });
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.keyDown(document.activeElement!, { key: 'u', code: 'KeyU', metaKey: true });
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'picture', action: 'pin' }));
  });

  it('updates Paste as Plain Text and its actual key handler while the right-click menu is open', async () => {
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    fireEvent.contextMenu(row('first'));
    const menu = screen.getByRole('menu');
    const item = within(menu).getByRole('menuitem', { name: 'Paste as Plain Text' });
    const save = async (change: object) => {
      await act(async () => { await bridge.invoke('clipboard_record_shortcut_set', { action: 'pastePlain', ...change }); });
    };
    const key = { key: 'j', code: 'KeyJ', altKey: true };
    await save({ binding: { code: 'KeyJ', modifiers: ['alt'] } });
    expect(item.querySelector('kbd')?.textContent).toBe('⌥J');
    fireEvent.keyDown(document.activeElement!, { key: 'Enter', code: 'Enter', metaKey: true });
    expect(calls('clipboard_record_use')).toHaveLength(0);
    await save({ enabled: false });
    expect(item.querySelector('kbd')?.textContent).toBe('—');
    fireEvent.keyDown(document.activeElement!, key);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    await save({ enabled: true });
    expect(item.querySelector('kbd')?.textContent).toBe('⌥J');
    await save({ binding: null });
    expect(item.querySelector('kbd')?.textContent).toBe('—');
    await save({ binding: { code: 'KeyJ', modifiers: ['alt'] } });
    expect(screen.getByRole('menu')).toBe(menu);
    fireEvent.keyDown(document.activeElement!, key);
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'pastePlain' }));
  });

  it('pastes plain text directly from a text row but ignores the shortcut on an image row', async () => {
    records = [
      { id: 'text-row', kind: 'text', text: 'Formatted body', date: '2026-11-24T00:00:00Z', isFavorite: false, isPinned: false },
      { id: 'img-row', kind: 'image', text: 'Screenshot', date: '2026-11-24T00:00:00Z', isFavorite: false, isPinned: false },
    ];
    await mount('clipboard');
    await waitFor(() => expect(row('text-row')).toBeTruthy());
    const textSelect = row('text-row').querySelector<HTMLButtonElement>('.history-row-select')!;
    textSelect.focus();
    fireEvent.keyDown(textSelect, { key: 'Enter', code: 'Enter', metaKey: true });
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'text-row', action: 'pastePlain' }));

    const imgSelect = row('img-row').querySelector<HTMLButtonElement>('.history-row-select')!;
    imgSelect.focus();
    fireEvent.keyDown(imgSelect, { key: 'Enter', code: 'Enter', metaKey: true });
    expect(calls('clipboard_record_use')).toHaveLength(1);
  });

  it('allows resuming paused clipboard recording directly from the empty state', async () => {
    records = [];
    bridge.snapshot.clipboardCaptureEnabled = false;
    await mount('clipboard');
    const resume = await screen.findByRole('button', { name: 'Resume Recording' });
    fireEvent.click(resume);
    await waitFor(() => expect(calls('clipboard_capture_set').at(-1)?.[1]).toEqual({ enabled: true }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Resume Recording' })).toBeNull());
  });

  it('opens a note card with empty title and full content, saves original text and renders the title', async () => {
    records[0].text = 'Full original content '.repeat(40);
    const user = userEvent.setup();
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.contextMenu(row('first'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    const dialog = await screen.findByRole('dialog', { name: 'Note' });
    const title = await within(dialog).findByRole('textbox', { name: 'Title' }) as HTMLInputElement;
    const content = within(dialog).getByRole('textbox', { name: 'Content' }) as HTMLTextAreaElement;
    expect(title.value).toBe(''); expect(content.value).toBe(records[0].text);
    expect(content.readOnly).toBe(false);
    expect(within(dialog).queryByRole('heading')).toBeNull();
    expect(within(dialog).getByText('Content').classList.contains('clipboard-editor-label')).toBe(true);
    expect(document.activeElement).toBe(title);
    await user.type(title, 'A title');
    expect(within(dialog).getByRole('button', { name: 'Cancel' }).textContent).toBe('CancelEsc');
    expect(within(dialog).getByRole('button', { name: 'Save' }).textContent).toBe('Save↵');
    fireEvent.change(content, { target: { value: 'Edited original text' } });
    expect(fireEvent.keyDown(content, { key: 'Enter', shiftKey: true })).toBe(true);
    fireEvent.keyDown(content, { key: 'Enter', isComposing: true });
    fireEvent.keyDown(content, { key: 'Escape', isComposing: true });
    expect(calls('clipboard_record_update')).toHaveLength(0);
    expect(dialog.parentElement?.classList.contains('closing')).toBe(false);
    fireEvent.keyDown(content, { key: 'Enter' });
    await waitFor(() => expect(dialog.parentElement?.classList.contains('closing')).toBe(true));
    expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'edit', title: 'A title', text: 'Edited original text' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(within(row('first')).getByText('A title')).toBeTruthy();
    fireEvent.mouseEnter(row('first'));
    await waitFor(() => expect(within(row('first')).getByText('Edited original text')).toBeTruthy());
    fireEvent.mouseLeave(row('first'));
    fireEvent.contextMenu(row('first'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    await waitFor(() => expect((screen.getByRole('textbox', { name: 'Title' }) as HTMLInputElement).value).toBe('A title'));
    const reopenedTitle = screen.getByRole('textbox', { name: 'Title' });
    fireEvent.change(reopenedTitle, { target: { value: 'Discard this draft' } });
    fireEvent.keyDown(reopenedTitle, { key: 'Escape' });
    expect(screen.getByRole('dialog')).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(calls('window_hide')).toHaveLength(0);
    expect(calls('clipboard_record_update')).toHaveLength(1);
  });

  it.each(['image', 'files'] as const)('keeps the Content label and shows disabled non-editable text for %s, saving only title', async (kind) => {
    records = [{ ...records[0], kind, text: kind === 'files' ? '["/synthetic/a.txt"]' : '' }];
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(1));
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    const content = await screen.findByRole('textbox', { name: 'Content' }) as HTMLTextAreaElement;
    expect(content.readOnly).toBe(true);
    expect(screen.queryByText('This content cannot be edited.')).toBeNull();
    expect(screen.queryByRole('heading', { name: 'Note' })).toBeNull();
    expect(content.classList.contains('non-editable')).toBe(true);
    expect(content.disabled).toBe(true);
    expect(screen.getByText('Content').classList.contains('clipboard-editor-label')).toBe(true);
    expect(screen.getByText('Title').classList.contains('clipboard-editor-label')).toBe(true);
    expect(content.value).toBe('Non-editable content');
    const user = userEvent.setup();
    await user.click(content);
    await user.type(content, 'Cannot edit');
    expect(content.value).toBe('Non-editable content');
    fireEvent.change(screen.getByRole('textbox', { name: 'Title' }), { target: { value: 'Media title' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'edit', title: 'Media title' });
    expect(records[0].text).toBe(kind === 'files' ? '["/synthetic/a.txt"]' : '');
  });

  it('retains unsaved edits on failure, traps focus, and cancels without writing', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_update') throw new Error('synthetic failure');
      return original(name, args);
    });
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    const title = await screen.findByRole('textbox', { name: 'Title' });
    fireEvent.change(title, { target: { value: 'Unsaved title' } });
    fireEvent.keyDown(title, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Save' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await screen.findByRole('alert');
    expect((title as HTMLInputElement).value).toBe('Unsaved title');
    expect(records[0].title).toBeUndefined();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(calls('clipboard_record_update')).toHaveLength(1);
  });

  it('disables menu paste without platform support and retries a failed detail read', async () => {
    bridge.snapshot.capabilities.autoPaste = false;
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const original = bridge.invoke.getMockImplementation()!;
    let fail = true;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_detail' && fail) throw new Error('synthetic failure');
      return original(name, args);
    });
    fireEvent.contextMenu(row('first'));
    expect((screen.getByRole('menuitem', { name: 'Paste' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    await screen.findByRole('alert');
    expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true);
    fail = false; fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    const content = await screen.findByRole('textbox', { name: 'Content' }) as HTMLTextAreaElement;
    expect(content.value).toBe('Saved text');
  });

  it('shows a note title, lazily expands complete content on hover, and reuses nodes and the loaded content', async () => {
    records[0].title = 'A compact note';
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const record = row('first');
    const title = record.querySelector('.clipboard-record-title')!;
    const content = record.querySelector('.clipboard-record-content')!;
    const preview = content.querySelector('.history-preview');
    expect(title.textContent).toBe('A compact note');
    expect(content.textContent).toBe('');
    expect(calls('clipboard_record_detail')).toHaveLength(0);
    expect(title.getAttribute('aria-hidden')).toBe('false');
    expect(content.getAttribute('aria-hidden')).toBe('true');
    for (let index = 0; index < 5; index++) {
      fireEvent.mouseEnter(record);
      expect(record.dataset.noteRevealed).toBe('true');
      await waitFor(() => expect(title.getAttribute('aria-hidden')).toBe('true'));
      expect(content.getAttribute('aria-hidden')).toBe('false');
      expect(content.textContent).toBe('Saved text');
      fireEvent.mouseLeave(record);
      expect(record.dataset.noteRevealed).toBe('false');
      await waitFor(() => expect(title.getAttribute('aria-hidden')).toBe('false'));
      expect(content.getAttribute('aria-hidden')).toBe('true');
      expect(content.querySelector('.history-preview')).toBe(preview);
    }
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(calls('clipboard_record_detail')).toHaveLength(1);
    expect(row('pinned').querySelector('.clipboard-record-body')).toBeNull();
  });

  it('loads beyond the list summary and never reveals a late response after the pointer leaves', async () => {
    records[0].title = 'Long note';
    records[0].text = 'Line of full clipboard text\n'.repeat(30) + 'full-content-tail';
    queryHandler = async (request) => ({ ...page(request.query, request.favoritesOnly), records: page(request.query, request.favoritesOnly).records.map((record) => ({ ...record, text: record.text.slice(0, 256) })) });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const original = bridge.invoke.getMockImplementation()!;
    let finish: (value: unknown) => void = () => {};
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_detail' ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
    fireEvent.mouseEnter(row('first'));
    expect(within(row('first')).getByRole('progressbar')).toBeTruthy();
    fireEvent.mouseLeave(row('first'));
    await act(async () => finish({ text: records[0].text }));
    expect(row('first').querySelector('.clipboard-record-body')?.getAttribute('data-expanded')).toBe('false');
    bridge.invoke.mockImplementation(original);
    fireEvent.mouseEnter(row('first'));
    await waitFor(() => expect(row('first').querySelector('.clipboard-record-content')?.textContent).toBe(records[0].text));
    expect(row('first').querySelector('.clipboard-record-content')?.textContent).toContain('full-content-tail');
    await waitFor(() => expect(row('first').querySelector('.clipboard-record-body')?.getAttribute('data-expanded')).toBe('true'));
  });

  it('keeps the note available after content loading fails and retries on the next hover', async () => {
    records[0].title = 'Retryable note';
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_detail') throw new Error('synthetic failure');
      return original(name, args);
    });
    fireEvent.mouseEnter(row('first'));
    await within(row('first')).findByRole('alert');
    fireEvent.mouseLeave(row('first'));
    await waitFor(() => expect(row('first').querySelector('.clipboard-record-title')?.getAttribute('aria-hidden')).toBe('false'));
    bridge.invoke.mockImplementation(original);
    fireEvent.mouseEnter(row('first'));
    await waitFor(() => expect(row('first').querySelector('.clipboard-record-content')?.textContent).toBe('Saved text'));
    expect(within(row('first')).queryByRole('alert')).toBeNull();
  });

  it('matches EcoPaste by switching hover state immediately without enter or exit timers', () => {
    vi.useFakeTimers();
    try {
      const record: ClipboardRecord = { ...records[0], title: 'Note', kind: 'image' };
      const view = render(<ClipboardNotePreview record={record} revealed={false} native />);
      const expanded = () => view.container.querySelector('.clipboard-record-body')?.getAttribute('data-expanded');
      for (let index = 0; index < 5; index++) {
        view.rerender(<ClipboardNotePreview record={record} revealed native />);
        expect(expanded()).toBe('true');
        view.rerender(<ClipboardNotePreview record={record} revealed={false} native />);
        expect(expanded()).toBe('false');
      }
      expect(vi.getTimerCount()).toBe(0);
      view.unmount();
    } finally { vi.useRealTimers(); }
  });

  it('switches adjacent notes without repeatedly restoring the scroll anchor or reloading their content', async () => {
    records = [0, 1].map((index) => ({ ...records[0], id: `adjacent-${index}`, title: `Note ${index}`, text: `Complete adjacent text ${index}` }));
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const scroller = document.querySelector<HTMLElement>('.history-list')!;
    Object.defineProperties(scroller, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 1000 } });
    const first = row('adjacent-0'), second = row('adjacent-1');
    Object.defineProperties(first, { offsetTop: { configurable: true, value: 100 }, offsetHeight: { configurable: true, value: 40 } });
    Object.defineProperties(second, { offsetTop: { configurable: true, value: 200 }, offsetHeight: { configurable: true, value: 40 } });
    scroller.scrollTop = 120;
    fireEvent.scroll(scroller);
    let top = 120;
    const writes = vi.fn((value: number) => { top = value; });
    Object.defineProperty(scroller, 'scrollTop', { configurable: true, get: () => top, set: writes });
    Object.defineProperty(first, 'offsetTop', { configurable: true, value: 80 });
    for (let index = 0; index < 4; index++) {
      fireEvent.mouseEnter(first);
      await waitFor(() => expect(first.querySelector('.clipboard-record-body')?.getAttribute('data-expanded')).toBe('true'));
      fireEvent.mouseLeave(first);
      fireEvent.mouseEnter(second);
      await waitFor(() => expect(second.querySelector('.clipboard-record-body')?.getAttribute('data-expanded')).toBe('true'));
      expect(first.querySelector('.clipboard-record-body')?.getAttribute('data-expanded')).toBe('false');
      fireEvent.mouseLeave(second);
    }
    expect(writes).not.toHaveBeenCalled();
    expect(calls('clipboard_record_detail')).toHaveLength(2);
  });

  it.each([false, true])('uses full workspace sessions to dismiss drafts and restore defaults when pinned=%s', async (pinned) => {
    bridge.snapshot.window.clipboardPinned = pinned;
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Saved' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    const title = await screen.findByRole('textbox', { name: 'Title' });
    fireEvent.change(title, { target: { value: 'Unsaved draft' } });
    await emit('window://visibility', { label: 'translation', visible: false });
    expect(screen.getByRole('dialog')).toBeTruthy();
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('');
    await windowState({ recordsFocus: (bridge.snapshot.window.recordsFocus ?? 0) + 1 });
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(screen.getByRole('button', { name: 'Favorite clipboard records' }).getAttribute('aria-pressed')).toBe('false');
    // A fresh session enters on the first unpinned record instead of restoring an earlier selection.
    expect([...document.querySelectorAll('.history-row.selected')]).toEqual([row('first')]);
    expect(document.activeElement).toBe(row('first').querySelector('.history-row-select'));
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    expect((await screen.findByRole('textbox', { name: 'Title' }) as HTMLInputElement).value).toBe('');
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    await windowState({ recordsFocus: (bridge.snapshot.window.recordsFocus ?? 0) + 1 });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button', { name: 'Favorite clipboard records' }).getAttribute('aria-pressed')).toBe('false');
    fireEvent.contextMenu(row('first'));
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('does not restore the editor after hiding during an in-flight save and removes the visibility listener', async () => {
    const view = await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const original = bridge.invoke.getMockImplementation()!;
    let finish: () => void = () => {};
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_update') return new Promise<void>((resolve) => { finish = resolve; });
      return original(name, args);
    });
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    await screen.findByRole('textbox', { name: 'Title' });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    await act(async () => finish());
    await windowState({ recordsFocus: (bridge.snapshot.window.recordsFocus ?? 0) + 1 });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect([...document.querySelectorAll('.history-row.selected')]).toEqual([row('first')]);
    expect(document.activeElement).toBe(row('first').querySelector('.history-row-select'));
    view.unmount();
    expect(bridge.listeners.get('window://visibility')?.size).toBe(0);
  });

  it('keeps a shared viewport, one full-height scrollbar, pinned rows and total count footer', async () => {
    queryHandler = async () => ({ ...page(), totalRecords: 100002 });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(screen.getByText('100002 records').closest('.history-footer')).toBeTruthy();
    const viewport = document.querySelector('.history-viewport')!;
    expect(viewport.contains(row('pinned'))).toBe(true);
    expect(viewport.contains(row('first'))).toBe(true);
    expect(screen.queryByRole('button', { name: 'Load more' })).toBeNull();
    scrollNearEnd();
    const scrollbar = screen.getByRole('scrollbar', { name: 'Scroll records' });
    expect(scrollbar.parentElement).toBe(viewport);
    expect(scrollbar.closest('.history-list, .history-pinned')).toBeNull();
    expect(scrollbar.getAttribute('aria-valuenow')).toBe('400');
    fireEvent.keyDown(scrollbar, { key: 'Home' });
    expect(document.querySelector('.history-list')!.scrollTop).toBe(0);
    expect(row('pinned').parentElement?.scrollTop).toBe(0);
  });

  it('drags the full viewport scrollbar without moving pinned records and releases its handler', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    scrollNearEnd();
    Object.defineProperty(document.querySelector('.history-list'), 'clientHeight', { configurable: true, value: 400 });
    fireEvent.scroll(document.querySelector('.history-list')!);
    const scrollbar = screen.getByRole('scrollbar');
    const thumb = scrollbar.firstElementChild!;
    fireEvent.pointerDown(thumb, { pointerId: 9, button: 0, clientY: 300 });
    fireEvent.pointerMove(scrollbar, { pointerId: 9, clientY: 270 });
    const element = document.querySelector('.history-list')!;
    expect(element.scrollTop).toBe(62.5);
    expect(document.querySelector('.history-pinned')!.scrollTop).toBe(0);
    fireEvent.lostPointerCapture(scrollbar, { pointerId: 9 });
    fireEvent.pointerMove(scrollbar, { pointerId: 9, clientY: 0 });
    expect(element.scrollTop).toBe(62.5);
  });

  it('hides the scrollbar after inactivity, reveals it on scroll and keeps it visible during drag', async () => {
    const mounted = await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    await act(async () => {});
    vi.useFakeTimers();
    try {
      scrollNearEnd();
      const scrollbar = screen.getByRole('scrollbar');
      expect(scrollbar.classList.contains('visible')).toBe(true);
      act(() => vi.advanceTimersByTime(1000));
      expect(scrollbar.classList.contains('visible')).toBe(false);
      fireEvent.wheel(document.querySelector('.history-list')!);
      fireEvent.scroll(document.querySelector('.history-list')!);
      expect(scrollbar.classList.contains('visible')).toBe(true);
      fireEvent.pointerDown(scrollbar.firstElementChild!, { pointerId: 7, button: 0, clientY: 20 });
      act(() => vi.advanceTimersByTime(1500));
      expect(scrollbar.classList.contains('visible')).toBe(true);
      fireEvent.lostPointerCapture(scrollbar, { pointerId: 7 });
      act(() => vi.advanceTimersByTime(1000));
      expect(scrollbar.classList.contains('visible')).toBe(false);
      fireEvent.focus(scrollbar);
      act(() => vi.advanceTimersByTime(1500));
      expect(scrollbar.classList.contains('visible')).toBe(true);
      fireEvent.blur(scrollbar);
      act(() => vi.advanceTimersByTime(1000));
      expect(scrollbar.classList.contains('visible')).toBe(false);
      fireEvent.wheel(document.querySelector('.history-list')!);
      fireEvent.scroll(document.querySelector('.history-list')!);
      // Removing a focused control queues jsdom's focus repair on a 1000/60 interval.
      // Vitest's next animation frame is at most 16ms and can stop short of that delay.
      fireEvent.focus(scrollbar);
      mounted.unmount();
      act(() => {
        for (let attempt = 0; attempt < 4 && vi.getTimerCount() > 0; attempt += 1) vi.advanceTimersByTime(Math.ceil(1000 / 60));
      });
      expect(vi.getTimerCount()).toBe(0);
    } finally { vi.useRealTimers(); }
  });

  it('shows a scoped paste failure after auto-hide reset without accepting an old invoke result', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: (value: { status: string; copyStatus: string; code: string }) => void;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_use') return new Promise((resolve) => { finish = resolve; });
      return original(name, args);
    });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.click(within(row('first')).getByRole('button', { name: 'Clipboard: Saved text' }));
    await windowState({ recordsSession: 1 });
    await act(async () => finish({ status: 'failed', copyStatus: 'not_written', code: 'clipboard_file_missing' }));
    expect(screen.queryByRole('status')).toBeNull();
    await emit('clipboard_use_failed', { status: 'failed', copyStatus: 'not_written', code: 'clipboard_image_missing' });
    expect(screen.getByRole('status').textContent).toContain('image');
  });

  it('retains appended pages and their scroll position across cached favorite switches', async () => {
    const sample = records[0];
    records = Array.from({ length: 102 }, (_, index) => ({ ...sample, id: `deep-${index}`, text: `Deep text ${index}`, isFavorite: index === 101 }));
    queryHandler = async ({ cursor: next }) => ({ records: next ? records.slice(100) : records.slice(0, 100), cursor: next ? null : cursor, hasMore: !next, totalRecords: records.length });
    await mount('clipboard');
    const list = document.querySelector<HTMLElement>('.history-list')!;
    const items = document.querySelector<HTMLElement>('.history-items')!;
    await waitFor(() => expect(items.dataset.loadedCount).toBe('100'));
    scrollNearEnd(); await waitFor(() => expect(items.dataset.loadedCount).toBe('102'));
    expect(items.dataset.virtualized).toBe('true');
    expect(listRows().length).toBeLessThan(60);
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 102 * 72 } });
    list.scrollTop = 101 * 72; fireEvent.scroll(list);
    expect(row('deep-101')).toBeTruthy();
    const favorite = screen.getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite); expect(listRows()).toHaveLength(1);
    fireEvent.click(favorite); expect(items.dataset.loadedCount).toBe('102');
    expect(list.scrollTop).toBe(101 * 72);
    expect(row('deep-101')).toBeTruthy();
  });

  it('updates a saved note immediately without discarding deep pages or the current scroll position', async () => {
    const sample = records[0];
    records = Array.from({ length: 102 }, (_, index) => ({ ...sample, id: `note-${index}`, text: `Deep note ${index}`, isFavorite: index === 101 }));
    queryHandler = async ({ cursor: next, favoritesOnly }) => ({ records: favoritesOnly ? records.filter((record) => record.isFavorite) : next ? records.slice(100) : records.slice(0, 100), cursor: favoritesOnly || next ? null : cursor, hasMore: !favoritesOnly && !next, totalRecords: favoritesOnly ? 1 : records.length });
    await mount('clipboard');
    const list = document.querySelector<HTMLElement>('.history-list')!;
    const items = document.querySelector<HTMLElement>('.history-items')!;
    await waitFor(() => expect(items.dataset.loadedCount).toBe('100'));
    scrollNearEnd(); await waitFor(() => expect(items.dataset.loadedCount).toBe('102'));
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 102 * 72 } });
    list.scrollTop = 101 * 72; fireEvent.scroll(list);
    fireEvent.mouseEnter(row('note-101'));
    fireEvent.contextMenu(row('note-101'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Content' })).toHaveProperty('value', 'Deep note 101'));
    fireEvent.change(screen.getByRole('textbox', { name: 'Title' }), { target: { value: 'Updated deep title' } });
    fireEvent.change(screen.getByRole('textbox', { name: 'Content' }), { target: { value: 'Updated full note content' } });
    const queried = calls('clipboard_query').length;
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Note' })).toBeNull());
    expect(items.dataset.loadedCount).toBe('102');
    expect(within(row('note-101')).getByText('Updated deep title')).toBeTruthy();
    fireEvent.mouseEnter(row('note-101'));
    await waitFor(() => expect(within(row('note-101')).getByText('Updated full note content')).toBeTruthy());
    expect(calls('clipboard_query')).toHaveLength(queried);
    expect(list.scrollTop).toBe(101 * 72);
    const favorite = screen.getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite);
    expect(within(row('note-101')).getByText('Updated deep title')).toBeTruthy();
    fireEvent.click(favorite);
    expect(items.dataset.loadedCount).toBe('102');
    expect(list.scrollTop).toBe(101 * 72);
  });

  it('removes an edited record from cached search results and updates the matching count without replacing browsed pages', async () => {
    queryHandler = async ({ query, favoritesOnly }) => {
      const matching = records.filter((record) => (!favoritesOnly || record.isFavorite) && record.text.toLowerCase().includes(query.toLowerCase()));
      return { records: matching, cursor: null, hasMore: false, totalRecords: matching.length };
    };
    const { result } = renderHook(() => useClipboard(true, 'Saved', false, () => true));
    await waitFor(() => expect(result.current.page.records).toHaveLength(1));
    const queried = calls('clipboard_query').length;
    act(() => result.current.applyEdit('first', { title: 'Different title', text: 'Different original content' }));
    expect(result.current.page.records).toHaveLength(0);
    expect(result.current.page.totalRecords).toBe(0);
    expect(calls('clipboard_query')).toHaveLength(queried);
  });

  it('keeps edited title matches in cached search results and removes them when the title is cleared', async () => {
    records[0].isFavorite = true;
    queryHandler = async ({ favoritesOnly }) => {
      const matching = records.filter((record) => (!favoritesOnly || record.isFavorite) && record.text.includes('Saved'));
      return { records: matching, cursor: null, hasMore: false, totalRecords: matching.length };
    };
    const { result, rerender } = renderHook(({ favorite }) => useClipboard(true, 'Saved', favorite, () => true), { initialProps: { favorite: false } });
    await waitFor(() => expect(result.current.page.records).toHaveLength(1));
    const queried = calls('clipboard_query').length;
    act(() => result.current.applyEdit('first', { title: 'SAVED 海浪', text: 'Different original content' }));
    expect(result.current.page.records).toHaveLength(1);
    expect(result.current.page.totalRecords).toBe(1);
    rerender({ favorite: true });
    await waitFor(() => expect(result.current.page.totalRecords).toBe(1));
    act(() => result.current.applyEdit('first', { title: '', text: 'Different original content' }));
    expect(result.current.page.records).toHaveLength(0);
    expect(result.current.page.totalRecords).toBe(0);
    rerender({ favorite: false });
    expect(result.current.page.totalRecords).toBe(0);
    expect(calls('clipboard_query')).toHaveLength(queried);
  });

  it('keeps a deep page after a local favorite followed by an uncached filter response with a short alternate page', async () => {
    const sample = records[0];
    records = Array.from({ length: 102 }, (_, index) => ({ ...sample, id: `deep-${index}`, text: `Deep text ${index}` }));
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name !== 'clipboard_query') return original(name, args);
      const request = args as { favoritesOnly: boolean; kind?: ClipboardKind; cursor: unknown };
      const wire = (rows: ClipboardRecord[], more: boolean) => ({ records: rows.map((record) => ({ ...record, sourceAppId: null })), sources: {}, totalRecords: request.favoritesOnly ? records.filter((record) => record.isFavorite).length : records.length, hasMore: more, cursor: more ? cursor : null });
      return request.favoritesOnly
        ? { page: wire(records.filter((record) => record.isFavorite), false), alternate: { ...wire(records.slice(0, 100), true), totalRecords: records.length } }
        : { page: wire(request.cursor ? records.slice(100) : records.slice(0, 100), !request.cursor), alternate: null };
    });
    await mount('clipboard');
    const list = document.querySelector<HTMLElement>('.history-list')!;
    const items = document.querySelector<HTMLElement>('.history-items')!;
    await waitFor(() => expect(items.dataset.loadedCount).toBe('100'));
    scrollNearEnd(); await waitFor(() => expect(items.dataset.loadedCount).toBe('102'));
    let physicalTop = 400;
    Object.defineProperties(list, {
      scrollHeight: { configurable: true, get: () => Number(items.dataset.loadedCount ?? 0) * 72 },
      scrollTop: { configurable: true, get: () => Math.max(0, Math.min(physicalTop, list.scrollHeight - list.clientHeight)), set: (value: number) => { physicalTop = Math.max(0, Math.min(value, list.scrollHeight - list.clientHeight)); } },
    });
    const deepTop = 101 * 72 - 100;
    list.scrollTop = deepTop; fireEvent.scroll(list);
    fireEvent.click(within(row('deep-101')).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(row('deep-101')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    const favorite = screen.getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite); await waitFor(() => expect(listRows()).toHaveLength(1));
    fireEvent.click(favorite);
    expect(items.dataset.loadedCount).toBe('102');
    expect(row('deep-101')).toBeTruthy();
    expect(list.scrollTop).toBe(deepTop);
    expect(records[101].isFavorite).toBe(true);
  });

  it('serializes rapid local flag toggles so both cached filters retain the final backend value', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const list = document.querySelector<HTMLElement>('.history-list')!;
    list.scrollTop = 60; fireEvent.scroll(list);
    const favorite = within(row('first')).getByRole('button', { name: 'Favorite' });
    fireEvent.click(favorite); fireEvent.click(favorite);
    await waitFor(() => expect(calls('clipboard_record_update')).toHaveLength(2));
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Favorite' })).toBeTruthy());
    expect(records[0].isFavorite).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(listRows()).toHaveLength(1);
    expect(row('first')?.hidden).toBe(true);
  });

  it('updates a deep local favorite without discarding browsed pages or fetching the first page', async () => {
    const sample = records[0];
    records = Array.from({ length: 102 }, (_, index) => ({ ...sample, id: `deep-${index}`, text: `Deep text ${index}` }));
    queryHandler = async ({ cursor: next }) => ({ records: next ? records.slice(100) : records.slice(0, 100), cursor: next ? null : cursor, hasMore: !next, totalRecords: records.length });
    await mount('clipboard');
    const list = document.querySelector<HTMLElement>('.history-list')!;
    const items = document.querySelector<HTMLElement>('.history-items')!;
    await waitFor(() => expect(items.dataset.loadedCount).toBe('100'));
    scrollNearEnd(); await waitFor(() => expect(items.dataset.loadedCount).toBe('102'));
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 102 * 72 } });
    list.scrollTop = 101 * 72; fireEvent.scroll(list);
    const before = calls('clipboard_query').length;
    fireEvent.click(within(row('deep-101')).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(row('deep-101')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    expect(items.dataset.loadedCount).toBe('102');
    expect(calls('clipboard_query')).toHaveLength(before);
    expect(list.scrollTop).toBe(101 * 72);
  });

  it('defers external clipboard refresh while browsing and refreshes on return to the top', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const list = document.querySelector<HTMLElement>('.history-list')!;
    list.scrollTop = 80; fireEvent.scroll(list);
    const before = calls('clipboard_query').length;
    records.unshift({ ...records[0], id: 'new', text: 'New external text' });
    await emit('clipboard_changed', 1);
    expect(calls('clipboard_query')).toHaveLength(before);
    expect(row('first')).toBeTruthy();
    expect(row('new')).toBeNull();
    list.scrollTop = 0; fireEvent.scroll(list);
    await waitFor(() => expect(calls('clipboard_query').length).toBe(before + 1));
    await waitFor(() => expect(row('new')).toBeTruthy());
    expect(list.scrollTop).toBe(0);
  });

  it('pastes the first five unpinned results using fixed Command digits in each group', async () => {
    records.splice(0, 0, records.pop()!);
    for (let index = 2; index <= 6; index++) records.push({ ...records[1], id: `item-${index}`, kind: index % 2 === 0 ? 'image' : 'text', isFavorite: index === 4 });
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(7));
    const search = screen.getByRole('searchbox');
    const digit = (number: number, extra = {}) => fireEvent.keyDown(search, { key: String(number), code: `Digit${number}`, metaKey: true, ...extra });
    await act(async () => { for (let index = 1; index <= 5; index++) digit(index); });
    for (let index = 1; index <= 5; index++) {
      expect(calls('clipboard_record_use')[index - 1]?.[1]).toEqual({ recordId: index === 1 ? 'first' : `item-${index}`, action: 'paste' });
    }
    digit(6); digit(1, { repeat: true }); digit(1, { isComposing: true }); digit(1, { shiftKey: true });
    expect(calls('clipboard_record_use')).toHaveLength(5);
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    await waitFor(() => expect(listRows()).toHaveLength(3));
    await act(async () => { digit(2); digit(5); });
    expect(calls('clipboard_record_use').slice(5).map(([, args]) => args)).toEqual([{ recordId: 'item-4', action: 'paste' }]);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    await waitFor(() => expect(listRows()).toHaveLength(2));
    await act(async () => { digit(1); digit(2); });
    expect(calls('clipboard_record_use').slice(6).map(([, args]) => args)).toEqual([{ recordId: 'item-4', action: 'paste' }]);
  });

  it('refreshes an active query after search preferences change and never pastes a stale numbered result', async () => {
    const initial = records[0];
    const ocr = { ...initial, id: 'ocr', kind: 'image' as const, text: '', imageTextMatch: true };
    let respond: (() => void) | undefined;
    queryHandler = async ({ query }) => {
      if (query === 'Saved' && bridge.snapshot.clipboardControls.search?.imageText && !bridge.snapshot.clipboardControls.search.body) {
        return new Promise((resolve) => { respond = () => { resolve({ ...page(), records: [ocr], totalRecords: 1 }); }; });
      }
      return page(query);
    };
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox');
    fireEvent.change(search, { target: { value: 'Saved' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    const controls = bridge.snapshot.clipboardControls;
    controls.search = { mode: 'exact', body: false, note: false, imageText: true };
    await emit('clipboard_controls_changed', controls);
    fireEvent.keyDown(search, { key: '1', code: 'Digit1', metaKey: true });
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(listRows()).toHaveLength(0);
    await waitFor(() => expect(respond).toBeDefined());
    await act(async () => respond!());
    expect(within(row('ocr')).getByText('Matched image text')).toBeTruthy();
    expect(within(row('ocr')).getByRole('button', { name: /^Clipboard:/ }).getAttribute('aria-description')).toBe('Matched image text');
    fireEvent.keyDown(search, { key: '1', code: 'Digit1', metaKey: true });
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'ocr', action: 'paste' }));
  });

  it('reports invalid regex without displaying stale rows or a generic retry', async () => {
    bridge.snapshot.clipboardControls.search = { mode: 'regex', body: true, note: true, imageText: true };
    queryHandler = async ({ query }) => { if (query === '[') throw 'clipboard_invalid_regex'; return page(query); };
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox');
    fireEvent.change(search, { target: { value: '[' } });
    expect((await screen.findByRole('alert')).textContent).toContain('regular expression is invalid');
    expect(listRows()).toHaveLength(0);
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
    fireEvent.change(search, { target: { value: 'Saved' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows the actual quick-paste slots while Command is held and clears them on release or blur', async () => {
    records.push({ ...records[0], id: 'image', kind: 'image' });
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(3));
    const search = screen.getByRole('searchbox');
    const hint = (id: string) => row(id).querySelector('.clipboard-quick-paste-hint')?.textContent;
    const hold = () => fireEvent.keyDown(search, { key: 'Meta', code: 'MetaLeft', metaKey: true });
    expect(hint('first')).toBeUndefined();
    hold();
    expect(hint('first')).toBe('1');
    expect(hint('image')).toBe('2');
    expect(hint('pinned')).toBeUndefined();
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    await waitFor(() => expect(hint('image')).toBe('1'));
    expect(hint('first')).toBeUndefined();
    fireEvent.keyUp(search, { key: '1', code: 'Digit1', metaKey: true });
    expect(hint('image')).toBe('1');
    fireEvent.keyUp(search, { key: 'Meta', code: 'MetaLeft' });
    expect(hint('image')).toBeUndefined();
    hold();
    expect(hint('image')).toBe('1');
    fireEvent.blur(window);
    expect(hint('image')).toBeUndefined();
  });

  it('replaces the pin, search, and shortcut icons with their Command letters while Command is held', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const pin = () => screen.getByRole('button', { name: 'Pin records' });
    const keys = () => screen.getByRole('button', { name: 'HotKey' });
    const searchIcon = () => document.querySelector('.history-search > .history-symbol, .history-search > .history-command-hint');
    const search = screen.getByRole('searchbox');
    expect(pin().querySelector('.history-symbol')?.getAttribute('data-symbol')).toBe('pin');
    expect(searchIcon()?.getAttribute('data-symbol')).toBe('magnifyingglass');
    expect(keys().querySelector('.history-symbol')?.getAttribute('data-symbol')).toBe('keyboard');
    expect(screen.getByRole('button', { name: 'Expand details' }).querySelector('.history-symbol')).toBeTruthy();
    const hold = (extra: Record<string, boolean> = {}) => fireEvent.keyDown(search, { key: 'Meta', code: 'MetaLeft', metaKey: true, ...extra });
    hold();
    expect(pin().querySelector('.history-command-hint')?.textContent).toBe('P');
    expect(searchIcon()?.textContent).toBe('F');
    expect(keys().querySelector('.history-command-hint')?.textContent).toBe('K');
    expect(pin().querySelector('.history-symbol')).toBeNull();
    expect(screen.getByRole('button', { name: 'Expand details' }).querySelector('.history-symbol')?.getAttribute('data-symbol')).toBe('chevron.right');
    fireEvent.keyDown(search, { key: 'Shift', code: 'ShiftLeft', metaKey: true, shiftKey: true });
    expect(pin().querySelector('.history-symbol')?.getAttribute('data-symbol')).toBe('pin');
    expect(document.querySelector('.history-command-hint')).toBeNull();
    hold();
    fireEvent.keyUp(search, { key: 'Meta', code: 'MetaLeft' });
    expect(pin().querySelector('.history-symbol')?.getAttribute('data-symbol')).toBe('pin');
    hold();
    fireEvent.blur(window);
    expect(document.querySelector('.history-command-hint')).toBeNull();
  });

  it('keeps a chrome icon when its shortcut is not an available plain Command letter', async () => {
    const preferences = bridge.snapshot.translationPreferences;
    preferences.shortcuts.focusSearch = { code: 'KeyJ', modifiers: ['meta'] };
    preferences.shortcuts.pin = { code: 'KeyP', modifiers: ['alt', 'meta'] };
    preferences.disabledShortcuts = ['showShortcuts'];
    bridge.snapshot.clipboardControls.shortcuts.favorite.binding = { code: 'KeyJ', modifiers: ['meta'] };
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Meta', code: 'MetaLeft', metaKey: true });
    expect(document.querySelector('.history-search > .history-symbol')?.getAttribute('data-symbol')).toBe('magnifyingglass');
    expect(screen.getByRole('button', { name: 'Pin records' }).querySelector('.history-symbol')?.getAttribute('data-symbol')).toBe('pin');
    expect(screen.getByRole('button', { name: 'HotKey' }).querySelector('.history-symbol')?.getAttribute('data-symbol')).toBe('keyboard');
    preferences.shortcuts.focusSearch = { code: 'KeyJ', modifiers: ['meta'] };
    bridge.snapshot.clipboardControls.shortcuts.favorite.binding = { code: 'KeyD', modifiers: ['meta'] };
    await emit('clipboard_controls_changed', bridge.snapshot.clipboardControls);
    await emit('translation_preferences_changed', preferences);
    expect(document.querySelector('.history-search > .history-command-hint')?.textContent).toBe('J');
  });

  it('hides the last clipboard action strip after the pointer leaves the records', async () => {
    const style = document.createElement('style');
    const lines = (readFileSync('src/styles.css', 'utf8') as string).split('\n').map((line) => line.trim());
    style.textContent = lines.filter((line) => line.includes('.history-row-actions {') || /\.history-row:(hover|focus-within) \.history-row-actions/.test(line) || line.includes('[data-pointer-outside]')).join('\n');
    document.head.append(style);
    try {
      await mount('clipboard');
      const panel = () => document.querySelector('.clipboard-panel')!;
      const button = (id: string) => row(id).querySelector('.history-row-select')!;
      const strip = (id: string) => getComputedStyle(row(id).querySelector('.history-row-actions')!);
      await waitFor(() => expect(document.activeElement).toBe(button('first')));
      expect(panel().hasAttribute('data-pointer-outside')).toBe(false);
      expect(strip('first').visibility).toBe('visible');
      fireEvent.pointerLeave(row('first'), { relatedTarget: row('first').querySelector('.history-row-actions button') });
      fireEvent.pointerLeave(row('first'), { relatedTarget: row('pinned') });
      expect(panel().hasAttribute('data-pointer-outside')).toBe(false);
      expect(strip('first').visibility).toBe('visible');
      fireEvent.pointerLeave(row('first'), { relatedTarget: screen.getByRole('searchbox') });
      expect(panel().getAttribute('data-pointer-outside')).toBe('true');
      expect(strip('first').visibility).toBe('hidden');
      fireEvent.pointerEnter(row('first'));
      expect(panel().hasAttribute('data-pointer-outside')).toBe(false);
      expect(strip('first').visibility).toBe('visible');
      fireEvent.pointerLeave(row('first'));
      fireEvent.pointerEnter(row('pinned'));
      expect(panel().getAttribute('data-pointer-outside')).toBe('true');
      expect(document.activeElement).toBe(button('first'));
      fireEvent.mouseMove(row('pinned'), { clientX: 12, clientY: 40 });
      expect(panel().hasAttribute('data-pointer-outside')).toBe(false);
      expect(document.activeElement).toBe(button('pinned'));
      expect(strip('pinned').visibility).toBe('visible');
      fireEvent.pointerLeave(row('pinned'), { relatedTarget: screen.getByRole('searchbox') });
      expect(strip('pinned').visibility).toBe('hidden');
      fireEvent.keyDown(button('pinned'), { key: 'ArrowDown', code: 'ArrowDown' });
      expect(panel().hasAttribute('data-pointer-outside')).toBe(false);
      expect(document.activeElement).toBe(button('first'));
      expect(strip('first').visibility).toBe('visible');
      fireEvent.blur(window);
      expect(panel().getAttribute('data-pointer-outside')).toBe('true');
      expect(strip('first').visibility).toBe('hidden');
      fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'ArrowDown', code: 'ArrowDown' });
      expect(panel().hasAttribute('data-pointer-outside')).toBe(false);
      expect(strip('first').visibility).toBe('visible');
    } finally { style.remove(); }
  });

  it('pastes a pinned record from its saved Command letter, including when that row is filtered out', async () => {
    records[1].pinPasteKey = 'B';
    records.push({ ...records[0], id: 'second', text: 'Second text' });
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(3));
    const search = screen.getByRole('searchbox');
    const hint = (id: string) => row(id).querySelector('.clipboard-quick-paste-hint')?.textContent;
    fireEvent.keyDown(search, { key: 'Meta', code: 'MetaLeft', metaKey: true });
    expect(hint('pinned')).toBe('B');
    expect(hint('first')).toBe('1');
    expect(hint('second')).toBe('2');
    expect(row('pinned').querySelector('.history-row-select')?.getAttribute('aria-keyshortcuts')).toBe('Meta+B');
    const press = (code: string, extra = {}) => fireEvent.keyDown(search, { key: code.slice(3).toLowerCase(), code, metaKey: true, ...extra });
    press('KeyB');
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'pinned', action: 'paste' });
    fireEvent.keyDown(search, { key: '1', code: 'Digit1', metaKey: true });
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'paste' });
    press('KeyB', { repeat: true });
    press('KeyB', { isComposing: true });
    press('KeyB', { shiftKey: true });
    press('KeyA');
    expect(calls('clipboard_record_use')).toHaveLength(2);
    fireEvent.click(within(row('pinned')).getByRole('button', { name: 'Delete clipboard record' }));
    await screen.findByRole('alertdialog', { name: 'Delete this record?' });
    press('KeyB');
    expect(calls('clipboard_record_use')).toHaveLength(2);
    fireEvent.keyDown(document.body, { key: 'Escape', code: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    fireEvent.change(search, { target: { value: 'Second' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    expect(row('pinned').hidden).toBe(true);
    press('KeyB');
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'pinned', action: 'paste' });
    const { shortcuts } = bridge.snapshot.clipboardControls;
    await emit('clipboard_controls_changed', { shortcuts: { ...shortcuts, favorite: { ...shortcuts.favorite, binding: { code: 'KeyB', modifiers: ['meta'] } } } });
    press('KeyB');
    expect(calls('clipboard_record_use')).toHaveLength(3);
  });

  it('shows source and copy time on records without exposing source or date filters', async () => {
    records[0].sourceApp = { name: 'Synthetic Browser' };
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(screen.queryByRole('button', { name: 'Clipboard Filters' })).toBeNull();
    expect(within(row('first')).getByRole('img', { name: 'Synthetic Browser' })).toBeTruthy();
    expect(row('first').querySelector('time')?.dateTime).toBe(records[0].date);
    expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: false, cursor: null });
  });

  it('starts with a keyboard target, keeps typing Space, and runs record actions from search without a click', async () => {
    bridge.snapshot.window.clipboardDetailVisible = false;
    bridge.snapshot.clipboardControls.recordsOpenFocus = 'search';
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox');
    expect(document.activeElement).toBe(search);
    expect(row('first').getAttribute('data-keyboard-current')).not.toBeNull();
    fireEvent.mouseMove(row('pinned'));
    fireEvent.keyDown(search, { key: 'Enter', code: 'Enter' });
    expect(fireEvent.keyDown(search, { key: ' ', code: 'Space' })).toBe(true);
    expect(calls('records_details_toggle')).toHaveLength(0);
    fireEvent.mouseMove(row('pinned'));
    fireEvent.keyDown(search, { key: 'c', code: 'KeyC', metaKey: true });
    fireEvent.keyDown(search, { key: 'Enter', code: 'Enter', shiftKey: true });
    fireEvent.keyDown(search, { key: 'd', code: 'KeyD', metaKey: true });
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'pinned', action: 'favorite' }));
    fireEvent.keyDown(search, { key: 'p', code: 'KeyP', altKey: true, metaKey: true });
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'pinned', action: 'pin' }));
    expect(calls('records_details_toggle')).toHaveLength(0);
    expect(calls('clipboard_record_use').map(([, args]) => args)).toEqual([{ recordId: 'pinned', action: 'activate' }]);
    expect(document.activeElement).toBe(search);
  });

  it('preserves text editing and blocks commands against a previous query while new results load', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole<HTMLInputElement>('searchbox');
    let complete: (value: ClipboardPage) => void = () => {};
    queryHandler = () => new Promise(resolve => { complete = resolve; });
    const enter = () => fireEvent.keyDown(search, { key: 'Enter', code: 'Enter' });
    expect(fireEvent.keyDown(search, { key: ' ', code: 'Space' })).toBe(true);
    fireEvent.change(search, { target: { value: 'synthetic' } });
    await waitFor(() => expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ query: 'synthetic' }));
    enter();
    expect(calls('clipboard_record_use')).toHaveLength(0);
    await act(async () => complete(page('synthetic')));
    await waitFor(() => expect(row('pinned').getAttribute('data-keyboard-current')).not.toBeNull());
    search.setSelectionRange(0, 4);
    expect(fireEvent.keyDown(search, { key: 'c', code: 'KeyC', metaKey: true })).toBe(true);
    expect(fireEvent.keyDown(search, { key: 'Backspace', code: 'Backspace', altKey: true })).toBe(true);
    fireEvent.keyDown(search, { key: 'Enter', code: 'Enter', isComposing: true });
    fireEvent.keyDown(search, { key: 'Enter', code: 'Enter', repeat: true });
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(calls('clipboard_record_update')).toHaveLength(0);
    enter();
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'pinned', action: 'activate' });
  });

  it('leaves native text copy alone and does not actively copy records', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox');
    expect(fireEvent.copy(search)).toBe(true);
    expect(fireEvent.keyDown(search, { key: 'c', code: 'KeyC', metaKey: true })).toBe(true);
    expect(fireEvent.keyDown(search, { key: 'Enter', code: 'Enter', shiftKey: true })).toBe(true);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(fireEvent.keyDown(row('first').querySelector('.history-row-select')!, { key: 'c', code: 'KeyC', metaKey: true })).toBe(true);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.contextMenu(row('first'));
    const menu = within(screen.getByRole('menu'));
    expect(menu.queryByRole('menuitem', { name: 'Copy' })).toBeNull();
    expect(menu.queryByRole('menuitem', { name: 'Copy as Plain Text' })).toBeNull();
  });

  it('deletes a searched record with Command-Delete without editing the query', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox');
    const remove = (node: Element) => fireEvent.keyDown(node, { key: 'Backspace', code: 'Backspace', metaKey: true });
    fireEvent.change(search, { target: { value: 'Saved' } });
    await waitFor(() => expect(row('first').hasAttribute('data-keyboard-current')).toBe(true));
    expect(fireEvent.keyDown(search, { key: 'Backspace', code: 'Backspace', altKey: true })).toBe(true);
    remove(search);
    await waitFor(() => expect(row('first')).toBeNull());
    expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'delete' });
    expect((search as HTMLInputElement).value).toBe('Saved');
  });

  it.each(['Backspace', 'Enter'])('deletes with a custom %s binding, ignores repeat and keeps search focus', async (code) => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox');
    {
      const { shortcuts } = bridge.snapshot.clipboardControls;
      await emit('clipboard_controls_changed', { shortcuts: { ...shortcuts, delete: { ...shortcuts.delete, binding: { code, modifiers: ['alt'] } } } });
    }
    const remove = (target: Element, repeat = false) => fireEvent.keyDown(target, { key: code, code, altKey: true, repeat });
    act(() => search.focus());
    fireEvent.mouseMove(row('pinned'));
    remove(search, true);
    expect(calls('clipboard_record_update')).toHaveLength(0);
    remove(search);
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete this record?' });
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.keyDown(dialog, { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(row('pinned')).toBeNull());
    expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'pinned', action: 'delete' });
    expect(document.activeElement).toBe(search);
  });

  it.each([
    ['pinned', { isPinned: true, isFavorite: false }, 'This record is pinned and cannot be recovered.'],
    ['favorite', { isPinned: false, isFavorite: true }, 'This record is favorited and cannot be recovered.'],
    ['both', { isPinned: true, isFavorite: true }, 'This record is pinned and favorited and cannot be recovered.'],
  ] as const)('asks before deleting a %s record and accepts Esc or Enter', async (id, marks, text) => {
    records = [{ ...records[0], id, ...marks }];
    await mount('clipboard');
    await waitFor(() => expect(row(id)).toBeTruthy());
    fireEvent.click(within(row(id)).getByRole('button', { name: 'Delete clipboard record' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete this record?' });
    expect(dialog.textContent).toContain(text);
    const cancel = within(dialog).getByRole('button', { name: 'Cancel' });
    const confirm = within(dialog).getByRole('button', { name: 'Delete' });
    expect(cancel.querySelector('kbd')?.textContent).toBe('Esc');
    expect(confirm.querySelector('kbd')?.textContent).toBe('↵');
    expect(cancel.getAttribute('aria-keyshortcuts')).toBe('Escape');
    expect(confirm.getAttribute('aria-keyshortcuts')).toBe('Enter');
    expect(document.activeElement).not.toBe(cancel);
    fireEvent.keyDown(document.body, { key: 'Escape', code: 'Escape' });
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.contextMenu(row(id));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    fireEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Cancel' }));
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.keyDown(row(id).querySelector('.history-row-select')!, { key: 'Backspace', code: 'Backspace', metaKey: true });
    fireEvent.keyDown(await screen.findByRole('alertdialog'), { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: id, action: 'delete' }));
  });

  it('previews the focused record without restoring the record', async () => {
    bridge.snapshot.window.clipboardDetailVisible = false;
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    const target = within(row('first')).getByRole('button', { name: /Clipboard: Saved text/ });
    act(() => target.focus());
    fireEvent.mouseMove(row('pinned'));
    fireEvent.keyDown(target, { key: ' ', code: 'Space' });
    expect(calls('records_details_toggle').at(-1)?.[1]).toEqual({ recordId: 'first' });
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.keyDown(target, { key: 'c', code: 'KeyC', metaKey: true, shiftKey: true });
    fireEvent.keyDown(target, { key: 'Enter', code: 'Enter', shiftKey: true });
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'p', code: 'KeyP', metaKey: true });
    fireEvent.keyDown(target, { key: 'p', code: 'KeyP', metaKey: true, isComposing: true });
    expect(calls('clipboard_record_update')).toHaveLength(0);
  });

  it('uses a remapped preview and guarded deletion while ignoring disabled shortcuts', async () => {
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    const target = within(row('first')).getByRole('button', { name: /Clipboard: Saved text/ });
    const binding = (code: string, enabled = true) => ({ enabled, binding: { code, modifiers: ['alt' as const] } });
    const shortcuts = { ...bridge.snapshot.clipboardControls.shortcuts, preview: binding('KeyJ'), favorite: binding('KeyL', false), delete: binding('KeyM') };
    await emit('clipboard_controls_changed', { shortcuts });
    act(() => target.focus());
    const press = (node: Element, code: string, repeat = false) => fireEvent.keyDown(node, { key: code.slice(-1).toLowerCase(), code, altKey: true, repeat });
    expect(fireEvent.keyDown(target, { key: ' ', code: 'Space' })).toBe(false);
    fireEvent.keyDown(target, { key: 'c', code: 'KeyC', metaKey: true, shiftKey: true });
    fireEvent.keyDown(target, { key: 'Enter', code: 'Enter', shiftKey: true });
    press(target, 'KeyL');
    fireEvent.keyDown(target, { key: 'Backspace', code: 'Backspace', altKey: true });
    expect(calls('records_details_toggle')).toHaveLength(0);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(calls('clipboard_record_update')).toHaveLength(0);
    press(target, 'KeyJ');
    expect(calls('records_details_toggle').at(-1)?.[1]).toEqual({ recordId: 'first' });
    expect(calls('clipboard_record_update')).toHaveLength(0);
    act(() => target.focus());
    press(target, 'KeyM', true);
    expect(calls('clipboard_record_update')).toHaveLength(0);
    press(target, 'KeyM');
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'delete' }));
  });

  it('applies remapped pin to the focused record and retains focus after it moves', async () => {
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    const target = within(row('first')).getByRole('button', { name: /Clipboard: Saved text/ });
    const shortcuts = bridge.snapshot.clipboardControls.shortcuts;
    await emit('clipboard_controls_changed', { shortcuts: { ...shortcuts, pin: { enabled: true, binding: { code: 'KeyJ', modifiers: ['alt'] } }, favorite: { ...shortcuts.favorite, enabled: false } } });
    act(() => target.focus());
    fireEvent.keyDown(target, { key: 'p', code: 'KeyP', metaKey: true });
    fireEvent.keyDown(target, { key: 'd', code: 'KeyD', metaKey: true });
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.keyDown(target, { key: 'j', code: 'KeyJ', altKey: true });
    await waitFor(() => expect(row('first').classList.contains('pinned')).toBe(true));
    expect(document.activeElement).toBe(row('first').querySelector('.history-row-select'));
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const help = screen.getByRole('dialog', { name: 'HotKey' });
    expect(help.textContent).toContain('Paste selection');
    expect(help.textContent).toContain('Pin/Unpin⌥ J');
    expect(help.textContent).not.toContain('Favorite');
  });

  it('deletes on the first deliberate keystroke and moves focus to the next record', async () => {
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    const remove = (target: Element, repeat = false) => fireEvent.keyDown(target, { key: 'Backspace', code: 'Backspace', metaKey: true, repeat });
    const protectedRow = row('pinned').querySelector<HTMLButtonElement>('.history-row-select')!;
    const target = row('first').querySelector<HTMLButtonElement>('.history-row-select')!;
    act(() => target.focus());
    remove(target, true);
    expect(calls('clipboard_record_update')).toHaveLength(0);
    remove(target);
    await waitFor(() => expect(row('first')).toBeNull());
    expect(calls('clipboard_record_update').map(([, args]) => args)).toEqual([{ recordId: 'first', action: 'delete' }]);
    expect(document.activeElement).toBe(protectedRow);
  });

  it('toggles shortcut help, closes it before hiding the card, and dismisses outside', async () => {
    const user = userEvent.setup(); await mount('clipboard');
    const keyboard = screen.getByRole('button', { name: 'HotKey' });
    await user.click(keyboard);
    expect(screen.getByRole('dialog', { name: 'HotKey' }).textContent).toContain('Paste selection');
    expect(keyboard.querySelector('[data-symbol="keyboard"]')).toBeTruthy();
    fireEvent.keyDown(keyboard, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(calls('window_hide')).toHaveLength(0);
    expect(document.activeElement).toBe(keyboard);
    await user.click(keyboard);
    fireEvent.pointerDown(screen.getByRole('searchbox'));
    expect(screen.queryByRole('dialog')).toBeNull();
    await user.click(keyboard);
    await user.click(keyboard);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('subscribes before loading real records and separates pinned rows', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(bridge.listen.mock.invocationCallOrder[bridge.listen.mock.calls.findIndex(([name]) => name === 'clipboard_changed')]).toBeLessThan(bridge.invoke.mock.invocationCallOrder[bridge.invoke.mock.calls.findIndex(([name]) => name === 'clipboard_query')]);
    expect(row('pinned').parentElement?.className).toContain('history-pinned');
    expect(within(row('pinned')).getByRole('button', { name: 'Delete clipboard record' })).toBeTruthy();
    expect(within(row('first')).getByRole('button', { name: 'Delete clipboard record' })).toBeTruthy();
    expect(screen.queryByText('Sample mode')).toBeNull();
  });

  it('uses the translation history toolbar and row styling without a separate type icon', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(document.querySelector('.clipboard-panel')?.classList.contains('history-panel')).toBe(true);
    const wordmark = within(document.querySelector<HTMLElement>('.history-toolbar')!).getByRole('img', { name: 'Nami' });
    expect(wordmark.classList.contains('history-wordmark')).toBe(true);
    expect(wordmark.textContent).toBe('');
    expect(wordmark.hasAttribute('data-tauri-drag-region')).toBe(true);
    expect(wordmark.style.getPropertyValue('--wordmark-url')).toContain('nami-wordmark.svg');
    expect(document.querySelector('.history-toolbar .history-search .history-symbol')).toBeTruthy();
    expect(row('first').querySelector('.history-row-heading')?.children.length).toBe(3);
    expect(row('first').querySelector('.history-preview')).toBeTruthy();
    expect(row('first').querySelector('.history-row-actions')).toBeTruthy();
    expect(document.querySelector('.clipboard-kind-icon')).toBeNull();
  });

  it('shares the history fallback mark and preserves real source application icons', async () => {
    records[1].sourceApp = { name: 'Synthetic Source', iconDataUrl: 'data:image/png;base64,c3ludGhldGlj' };
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const fallback = within(row('first')).getByRole('img', { name: 'Nami' });
    expect(fallback.classList.contains('history-symbol')).toBe(true);
    expect(fallback.style.getPropertyValue('--symbol-url')).toContain('reference-nami.png');
    expect(fallback.style.borderRadius).toBe('0px');
    expect(fallback.style.background).toBe(''); // The shared mask uses the theme foreground.
    expect(fallback.closest('.history-row-heading')).toBeTruthy();
    const source = within(row('pinned')).getByRole('img', { name: 'Synthetic Source' });
    expect(source.getAttribute('src')).toBe('data:image/png;base64,c3ludGhldGlj');
    expect(source.classList.contains('history-symbol')).toBe(false);
  });

  it('queries search and favorites in the database and clears search', async () => {
    const user = userEvent.setup(); await mount('clipboard');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'synthetic' } });
    await waitFor(() => expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: 'synthetic', favoritesOnly: false, cursor: null }));
    await waitFor(() => expect(listRows()).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Clear clipboard search' }));
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(document.activeElement).toBe(screen.getByRole('searchbox'));
    await user.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    await waitFor(() => expect(listRows()).toHaveLength(1));
    expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: true, cursor: null });
  });

  it('switches cached filters immediately without an activity beam or another database round trip', async () => {
    queryHandler = async (request) => ({ ...page(request.query, request.favoritesOnly), hasMore: !request.favoritesOnly, cursor: request.favoritesOnly ? null : cursor });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const firstNode = row('first');
    queryHandler = async () => { throw new Error('switch must not query'); };
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(row('first')).toBe(firstNode);
    expect(firstNode.hidden).toBe(true);
    expect(listRows()).toHaveLength(1);
    expect(screen.queryByRole('progressbar')).toBeNull();
    const queries = calls('clipboard_query').length;
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(listRows()).toHaveLength(2);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(listRows()).toHaveLength(1);
    expect(calls('clipboard_query')).toHaveLength(queries);
    records[0].isFavorite = true;
    queryHandler = undefined;
    await emit('clipboard_changed', 8);
    await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    await waitFor(() => expect(calls('clipboard_query')).toHaveLength(queries + 1));
  });

  it('rejects a late refresh after switching to a prefetched page and resets both views on search', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    let finish: (value: ClipboardPage) => void = () => {};
    queryHandler = async () => new Promise((resolve) => { finish = resolve; });
    await emit('clipboard_changed', 9);
    // A cache invalidation needs authoritative data; a newer search must win over that response.
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Saved' } });
    queryHandler = undefined;
    await waitFor(() => expect(listRows()).toHaveLength(1));
    await act(async () => finish(page('', true)));
    expect(listRows()).toHaveLength(1);
    const queries = calls('clipboard_query').length;
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(listRows()).toHaveLength(0);
    expect(calls('clipboard_query')).toHaveLength(queries);
    expect(row('pinned').hidden).toBe(true);
  });

  it('prefetches favorites outside the first all page and keeps native image nodes mounted across switches', async () => {
    const pageSize = 2;
    records = Array.from({ length: pageSize + 1 }, (_, index) => ({ ...records[0], id: `record-${index}`, isFavorite: index === pageSize, sourceApp: { name: 'Synthetic', iconDataUrl: 'data:image/png;base64,c3ludGhldGlj' } }));
    queryHandler = async (request) => ({ ...page(request.query, request.favoritesOnly), records: records.slice(0, pageSize), hasMore: true, cursor });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(pageSize));
    const original = row('record-0'); const source = original.querySelector('img');
    expect(row(`record-${pageSize}`).hidden).toBe(true);
    const queries = calls('clipboard_query').length;
    for (let index = 0; index < 8; index++) {
      fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
      expect(listRows()).toHaveLength(1); expect(row(`record-${pageSize}`).hidden).toBe(false);
      fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
      expect(listRows()).toHaveLength(pageSize); expect(row('record-0')).toBe(original);
      expect(original.querySelector('img')).toBe(source);
    }
    expect(calls('clipboard_query')).toHaveLength(queries);
    expect(document.querySelectorAll('.clipboard-panel .history-row')).toHaveLength(pageSize + 1);
  });

  it('moves a pasted record to the top of its pin group', async () => {
    records = [
      { id: 'plain-new', kind: 'text', text: 'Newer text', date: '2020-01-02T00:00:00Z', isFavorite: false, isPinned: false },
      { id: 'plain-old', kind: 'text', text: 'Older text', date: '2020-01-01T00:00:00Z', isFavorite: false, isPinned: false },
      { id: 'pin-new', kind: 'files', text: '/tmp/newer.txt', date: '2020-01-02T00:00:00Z', isFavorite: true, isPinned: true },
      { id: 'pin-old', kind: 'files', text: '/tmp/older.txt', date: '2020-01-01T00:00:00Z', isFavorite: true, isPinned: true },
    ];
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['pin-new', 'pin-old', 'plain-new', 'plain-old']));
    await act(async () => fireEvent.click(row('pin-old').querySelector('.history-row-select')!));
    expect(ids()).toEqual(['pin-old', 'pin-new', 'plain-new', 'plain-old']);
    await act(async () => fireEvent.click(within(row('pin-new')).getByRole('button', { name: 'Clipboard: /tmp/newer.txt' })));
    expect(ids()).toEqual(['pin-new', 'pin-old', 'plain-new', 'plain-old']);
    await act(async () => fireEvent.click(within(row('plain-old')).getByRole('button', { name: 'Clipboard: Older text' })));
    expect(ids()).toEqual(['pin-new', 'pin-old', 'plain-old', 'plain-new']);
  });

  it('keeps a newly captured image ahead of an image reused earlier', async () => {
    records = [
      { id: 'img-b', kind: 'image', text: '', date: '2020-01-02T00:00:00.000Z', isFavorite: true, isPinned: false },
      { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: true, isPinned: false },
    ];
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['img-b', 'img-a']));
    await act(async () => fireEvent.click(row('img-a').querySelector('.history-row-select')!));
    expect(ids()).toEqual(['img-a', 'img-b']);
    records = [
      { id: 'img-c', kind: 'image', text: '', date: '2099-01-01T00:00:00.000Z', isFavorite: false, isPinned: false },
      { id: 'img-b', kind: 'image', text: '', date: '2020-01-02T00:00:00.000Z', isFavorite: true, isPinned: false },
      { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: true, isPinned: false },
    ];
    await emit('clipboard_changed', 3);
    await waitFor(() => expect(ids()).toEqual(['img-c', 'img-b', 'img-a']));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(ids()).toEqual(['img-b', 'img-a']);
  });

  it('requeries unknown pin order across kind caches while retaining the browsed favorite depth', async () => {
    const sample:ClipboardRecord={id:'a',kind:'text',text:'A',date:'2020-01-01T00:00:00Z',isFavorite:false,isPinned:true};
    records=[sample,...Array.from({length:102},(_,index)=>({...sample,id:`b-${index}`,kind:'files' as const,text:`/tmp/b-${index}`,isFavorite:true}))];
    queryHandler=async(request)=>{
      const matching=records.filter(record=>(!request.favoritesOnly||record.isFavorite)&&(!request.kind||record.kind===request.kind));
      return {records:matching.slice(request.cursor?100:0,request.cursor?200:100),totalRecords:matching.length,hasMore:!request.cursor&&matching.length>100,cursor:!request.cursor&&matching.length>100?cursor:null};
    };
    const view=renderHook(({favorites,kind}:{favorites:boolean;kind:ClipboardKind|null})=>useClipboard(true,'',favorites,()=>true,kind),{initialProps:{favorites:true,kind:null as ClipboardKind|null}});
    await waitFor(()=>expect(view.result.current.page.records).toHaveLength(100));
    await act(async()=>{await view.result.current.loadMore();});
    expect(view.result.current.page.records).toHaveLength(102);
    view.rerender({favorites:false,kind:'text'});
    await waitFor(()=>expect(view.result.current.page.records.map(({id})=>id)).toEqual(['a']));
    const before=calls('clipboard_query').length;
    await act(async()=>{await view.result.current.update('a','favorite');});
    expect(calls('clipboard_query')).toHaveLength(before);
    view.rerender({favorites:true,kind:null});
    await waitFor(()=>expect(view.result.current.page.records).toHaveLength(103));
    expect(view.result.current.page.records[0].id).toBe('a');
    expect(view.result.current.page.records[102].id).toBe('b-101');
    expect(calls('clipboard_query')).toHaveLength(before+2);
  });

  it('inserts a newly favorited pin by the known All order without disturbing a scrolled page', async () => {
    records = [
      { id:'a',kind:'text',text:'A',date:'2020-01-01T00:00:00Z',isFavorite:false,isPinned:true },
      { id:'b',kind:'text',text:'B',date:'2020-01-02T00:00:00Z',isFavorite:true,isPinned:true },
    ];
    const view=renderHook(({favorites})=>useClipboard(true,'',favorites,()=>true),{initialProps:{favorites:false}});
    await waitFor(()=>expect(view.result.current.page.records).toHaveLength(2));
    const queries=calls('clipboard_query').length;
    await act(async()=>{await view.result.current.update('a','favorite');});
    expect(view.result.current.page.records.map(({id})=>id)).toEqual(['a','b']);
    view.rerender({favorites:true});
    await waitFor(()=>expect(view.result.current.page.records.map(({id})=>id)).toEqual(['a','b']));
    expect(calls('clipboard_query')).toHaveLength(queries);
  });

  it('accepts the acknowledged copy order when a later pin has an older display date', async () => {
    records = [
      { id: 'a', kind: 'text', text: 'A', date: '2020-01-02T00:00:00Z', isFavorite: false, isPinned: true },
      { id: 'b', kind: 'text', text: 'B', date: '2020-01-01T00:00:00Z', isFavorite: false, isPinned: true },
    ];
    const view = renderHook(() => useClipboard(true, '', false));
    await waitFor(() => expect(view.result.current.page.records).toHaveLength(2));
    act(() => view.result.current.noteCopied('b'));
    const copiedDate = new Date(Date.now() - 10).toISOString();
    records = [records[0], { ...records[1], date: copiedDate }];
    await emit('clipboard_changed', 100);
    await waitFor(() => expect(view.result.current.page.records.map(({ id }) => id)).toEqual(['a', 'b']));
    expect(view.result.current.page.records[1].date).toBe(copiedDate);
  });

  it('keeps the clicked record first while the returned page is still older', async () => {
    records = [
      { id: 'img-b', kind: 'image', text: '', date: '2020-01-02T00:00:00.000Z', isFavorite: false, isPinned: false },
      { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: false, isPinned: false },
    ];
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['img-b', 'img-a']));
    await act(async () => fireEvent.click(row('img-a').querySelector('.history-row-select')!));
    records = [
      { id: 'img-b', kind: 'image', text: 'refreshed', date: '2020-01-02T00:00:00.000Z', isFavorite: false, isPinned: false },
      { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: false, isPinned: false },
    ];
    await emit('clipboard_changed', 4);
    await waitFor(() => expect(within(row('img-b')).getByRole('button', { name: 'Clipboard: refreshed' })).toBeTruthy());
    expect(ids()).toEqual(['img-a', 'img-b']);
  });

  it('keeps a stale click on this page and a newer favorite on the other page', async () => {
    records = [
      { id: 'img-new', kind: 'image', text: '', date: '2099-01-01T00:00:00.000Z', isFavorite: true, isPinned: false },
      { id: 'img-b', kind: 'image', text: '', date: '2020-01-02T00:00:00.000Z', isFavorite: true, isPinned: false },
      { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: true, isPinned: false },
    ];
    let refreshed = false;
    queryHandler = async (request) => request.favoritesOnly
      ? page(request.query, true)
      : {
          records: [
            { id: 'img-b', kind: 'image', text: refreshed ? 'refreshed' : '', date: '2020-01-02T00:00:00.000Z', isFavorite: true, isPinned: false },
            { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: true, isPinned: false },
          ],
          cursor: null,
          hasMore: false,
          totalRecords: 2,
        };
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['img-b', 'img-a']));
    await act(async () => fireEvent.click(row('img-a').querySelector('.history-row-select')!));
    refreshed = true;
    await emit('clipboard_changed', 5);
    await waitFor(() => expect(within(row('img-b')).getByRole('button', { name: 'Clipboard: refreshed' })).toBeTruthy());
    expect(ids()).toEqual(['img-a', 'img-b']);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(ids()).toEqual(['img-new', 'img-b', 'img-a']);
  });

  it('still moves a clicked record to the top when closing the panel ends the session first', async () => {
    records = [
      { id: 'plain-new', kind: 'text', text: 'Newer text', date: '2020-01-02T00:00:00Z', isFavorite: false, isPinned: false },
      { id: 'plain-old', kind: 'text', text: 'Older text', date: '2020-01-01T00:00:00Z', isFavorite: false, isPinned: false },
    ];
    const original = bridge.invoke.getMockImplementation()!;
    let finish: (value: { status: string; copyStatus: string; code: null }) => void = () => {};
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_use' ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['plain-new', 'plain-old']));
    fireEvent.click(within(row('plain-old')).getByRole('button', { name: 'Clipboard: Older text' }));
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    await act(async () => finish({ status: 'dispatched', copyStatus: 'current', code: null }));
    expect(ids()).toEqual(['plain-old', 'plain-new']);
  });

  it('leaves the order unchanged when a click does not write the clipboard', async () => {
    records = [
      { id: 'plain-new', kind: 'text', text: 'Newer text', date: '2020-01-02T00:00:00Z', isFavorite: false, isPinned: false },
      { id: 'plain-old', kind: 'text', text: 'Older text', date: '2020-01-01T00:00:00Z', isFavorite: false, isPinned: false },
    ];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_use' ? { status: 'failed', copyStatus: 'not_written', code: 'clipboard_write' } : original(name, args));
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['plain-new', 'plain-old']));
    await act(async () => fireEvent.click(within(row('plain-old')).getByRole('button', { name: 'Clipboard: Older text' })));
    expect(ids()).toEqual(['plain-new', 'plain-old']);
  });

  it('keeps success quiet and explains manual paste when dispatch fails after copying', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    let status = 'dispatched';
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_use' ? { status, copyStatus: 'current', code: null } : original(name, args));
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    await act(async () => fireEvent.click(row('first').querySelector('.history-row-select')!));
    expect(document.querySelector('.record-feedback')).toBeNull();
    status = 'failed';
    await act(async () => fireEvent.click(row('first').querySelector('.history-row-select')!));
    expect(screen.getByText('Content copied. Press ⌘V to paste manually.')).toBeTruthy();
    await emit('clipboard_use_failed', { status: 'failed', copyStatus: 'superseded', code: 'clipboard_changed' });
    expect(screen.getByText('Clipboard content changed. Paste the record again.')).toBeTruthy();
  });

  it('refreshes both first pages only once per committed record update event', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const initial=calls('clipboard_query').length;
    await act(async () => fireEvent.click(within(row('first')).getByRole('button', { name: 'Favorite' })));
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    expect(calls('clipboard_query')).toHaveLength(initial+1);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(listRows()).toHaveLength(2);
    expect(calls('clipboard_query')).toHaveLength(initial+1);
  });

  it('uses record IDs for pasting and persists record actions', async () => {
    const user = userEvent.setup(); await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    await user.click(within(row('first')).getByRole('button', { name: 'Clipboard: Saved text' }));
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'activate' });
    await user.click(within(row('first')).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    await user.click(within(row('first')).getByRole('button', { name: 'Remove favorite' }));
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Delete clipboard record' })).toBeTruthy());
    await user.click(within(row('first')).getByRole('button', { name: 'Pin record' }));
    await waitFor(() => expect(row('first').parentElement?.className).toContain('history-pinned'));
    await user.click(within(row('first')).getByRole('button', { name: 'Unpin record' }));
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Delete clipboard record' })).toBeTruthy());
    await user.click(within(row('first')).getByRole('button', { name: 'Delete clipboard record' }));
    await waitFor(() => expect(row('first')).toBeNull());
  });

  it('retains small default-page row instances through search and restores them immediately without another query', async () => {
    const sample = records[0];
    records = Array.from({ length: 20 }, (_, index) => ({ ...sample, id: `home-${index}`, text: `Home text ${index}` }));
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(20));
    const first = row('home-0'); const last = row('home-19');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Home text 19' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    expect(row('home-0')).toBe(first);
    expect(first.hidden).toBe(true);
    expect(row('home-19')).toBe(last);
    const queries = calls('clipboard_query').length;
    fireEvent.click(screen.getByRole('button', { name: 'Clear clipboard search' }));
    expect(listRows()).toHaveLength(20);
    expect(row('home-0')).toBe(first);
    expect(row('home-19')).toBe(last);
    expect(calls('clipboard_query')).toHaveLength(queries);
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('restores only the authoritative default first page with its original pagination cursor', async () => {
    const sample = records[0];
    records = Array.from({ length: 202 }, (_, index) => ({ ...sample, id: `home-${index}`, text: `Home text ${index}` }));
    const secondCursor = { ...cursor, sequence: 200 };
    queryHandler = async (request) => request.query ? { ...page(request.query), records: [records[99]] } : request.cursor ? { records: records.slice(100, 200), totalRecords: records.length, hasMore: true, cursor: secondCursor } : { records: records.slice(0, 100), totalRecords: records.length, hasMore: true, cursor };
    const hook = renderHook(({ query }) => useClipboard(true, query, false), { initialProps: { query: '' } });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(100));
    await act(async () => hook.result.current.loadMore());
    expect(hook.result.current.page.records).toHaveLength(200);
    hook.rerender({ query: 'Home text 99' });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(1));
    const queried = calls('clipboard_query').length;
    hook.rerender({ query: '' });
    expect(hook.result.current.page.records).toHaveLength(100);
    expect(hook.result.current.page.cursor).toEqual(cursor);
    expect(hook.result.current.page.hasMore).toBe(true);
    expect(hook.result.current.retainedRecords).toHaveLength(100);
    expect(calls('clipboard_query')).toHaveLength(queried);
    await act(async () => hook.result.current.loadMore());
    expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ cursor });
  });

  it('invalidates the default page on data events and does not restore a stale default after search', async () => {
    const hook = renderHook(({ query }) => useClipboard(true, query, false), { initialProps: { query: '' } });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(2));
    hook.rerender({ query: 'Saved' });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(1));
    records = records.filter(({ id }) => id !== 'first');
    await emit('clipboard_changed', 1);
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(0));
    let finish: (value: ClipboardPage) => void = () => {};
    queryHandler = async () => new Promise((resolve) => { finish = resolve; });
    const queried = calls('clipboard_query').length;
    hook.rerender({ query: '' });
    expect(calls('clipboard_query')).toHaveLength(queried + 1);
    expect(hook.result.current.page.records).toHaveLength(0);
    expect(hook.result.current.showActivity).toBe(false);
    await act(async () => finish(page()));
    expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['pinned']);
  });

  it('keeps current record fields ahead of retained defaults and removes explicitly deleted cached rows', async () => {
    const hook = renderHook(({ query }) => useClipboard(true, query, false), { initialProps: { query: '' } });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(2));
    records[0] = { ...records[0], title: 'Current title', text: 'Saved current text' };
    hook.rerender({ query: 'Saved' });
    await waitFor(() => expect(hook.result.current.page.records[0]?.title).toBe('Current title'));
    expect(hook.result.current.retainedRecords.find(({ id }) => id === 'first')?.text).toBe('Saved current text');
    expect(hook.result.current.retainedRecords.find(({ id }) => id === 'first')?.title).toBe('Current title');
    await act(async () => hook.result.current.update('first', 'delete'));
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(0));
    expect(hook.result.current.retainedRecords.some(({ id }) => id === 'first')).toBe(false);
    let finish: (value: ClipboardPage) => void = () => {};
    queryHandler = async () => new Promise((resolve) => { finish = resolve; });
    const queried = calls('clipboard_query').length;
    hook.rerender({ query: '' });
    expect(calls('clipboard_query')).toHaveLength(queried + 1);
    expect(hook.result.current.page.records.some(({ id }) => id === 'first')).toBe(false);
    await act(async () => finish(page()));
    expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['pinned']);
  });

  it('invalidates the default page for local edits and keeps the edited row fields while hidden', async () => {
    const hook = renderHook(({ query }) => useClipboard(true, query, false), { initialProps: { query: '' } });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(2));
    hook.rerender({ query: 'Saved' });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(1));
    records[0] = { ...records[0], title: 'Edited title', text: 'Changed content' };
    act(() => hook.result.current.applyEdit('first', { title: 'Edited title', text: 'Changed content' }));
    expect(hook.result.current.page.records).toHaveLength(0);
    expect(hook.result.current.retainedRecords.find(({ id }) => id === 'first')).toMatchObject({ title: 'Edited title', text: 'Changed content' });
    let finish: (value: ClipboardPage) => void = () => {};
    queryHandler = async () => new Promise((resolve) => { finish = resolve; });
    const queried = calls('clipboard_query').length;
    hook.rerender({ query: '' });
    expect(calls('clipboard_query')).toHaveLength(queried + 1);
    expect(hook.result.current.page.records).toHaveLength(0);
    await act(async () => finish(page()));
    expect(hook.result.current.page.records[0]).toMatchObject({ title: 'Edited title', text: 'Changed content' });
  });

  it('does not revalidate a default response that raced a deferred data-change event', async () => {
    let finish: (value: ClipboardPage) => void = () => {};
    queryHandler = async () => new Promise((resolve) => { finish = resolve; });
    const hook = renderHook(({ query }) => useClipboard(true, query, false, () => true), { initialProps: { query: '' } });
    await waitFor(() => expect(calls('clipboard_query')).toHaveLength(1));
    await emit('clipboard_changed', 1);
    await act(async () => finish(page()));
    expect(hook.result.current.page.records).toHaveLength(2);
    queryHandler = undefined;
    hook.rerender({ query: 'Saved' });
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(1));
    const queried = calls('clipboard_query').length;
    hook.rerender({ query: '' });
    expect(calls('clipboard_query')).toHaveLength(queried + 1);
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(2));
  });

  it('restores the default query immediately without a clear-search debounce or activity and rejects late search results', async () => {
    const hook = renderHook(({ query }) => useClipboard(true, query, false), { initialProps: { query: 'Saved' } });
    await waitFor(() => expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['first']));
    let finishSearch: (value: ClipboardPage) => void = () => {};
    let finishDefault: (value: ClipboardPage) => void = () => {};
    queryHandler = (request) => new Promise((resolve) => { if (request.query) finishSearch = resolve; else finishDefault = resolve; });
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const initialQueries = calls('clipboard_query').length;
      hook.rerender({ query: 'slow' });
      expect(calls('clipboard_query')).toHaveLength(initialQueries);
      act(() => vi.advanceTimersByTime(149));
      expect(calls('clipboard_query')).toHaveLength(initialQueries);
      act(() => vi.advanceTimersByTime(1));
      expect(calls('clipboard_query')).toHaveLength(initialQueries + 1);
      expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: 'slow', favoritesOnly: false, cursor: null });
      hook.rerender({ query: '' });
      expect(calls('clipboard_query')).toHaveLength(initialQueries + 2);
      expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: false, cursor: null });
      expect(hook.result.current.showActivity).toBe(false);
      await act(async () => finishDefault(page()));
      expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['first', 'pinned']);
      expect(hook.result.current.loading).toBe(false);
      await act(async () => finishSearch({ records: [], totalRecords: 0, hasMore: false, cursor: null }));
      expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['first', 'pinned']);
      expect(hook.result.current.showActivity).toBe(false);
      act(() => vi.advanceTimersByTime(150));
      expect(calls('clipboard_query')).toHaveLength(initialQueries + 2);
    } finally { vi.useRealTimers(); }
  });

  it('keeps a newer search when an older result finishes late and refreshes current filters on capture', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    let stale: (value: ClipboardPage) => void = () => {};
    queryHandler = async (request) => request.query === 'slow' ? new Promise((resolve) => { stale = resolve; }) : page(request.query, request.favoritesOnly, request.kind);
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'slow' } });
    await waitFor(() => expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ query: 'slow' }));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Saved' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    await act(async () => stale({ records: [], cursor: null, hasMore: false, totalRecords: 0 }));
    expect(listRows()).toHaveLength(1);
    records.push({ ...records[0], id: 'new', text: 'Saved capture' });
    await emit('clipboard_changed', 1);
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: 'Saved', favoritesOnly: false, cursor: null });
  });

  it('appends a page once without duplicating records', async () => {
    queryHandler = async (request) => request.cursor ? { ...page(), records: [records[0], { ...records[0], id: 'second' }, { ...records[0], id: 'second' }], hasMore: false } : { ...page(), cursor, hasMore: true };
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    scrollNearEnd();
    await waitFor(() => expect(listRows()).toHaveLength(3));
    expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ cursor });
  });

  it('discards an old page after a new search and blocks pagination during its debounce', async () => {
    let finishPage: (value: ClipboardPage) => void = () => {};
    queryHandler = async (request) => request.cursor ? new Promise((resolve) => { finishPage = resolve; }) : { ...page(request.query), cursor, hasMore: !request.query };
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    scrollNearEnd();
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Saved' } });
    const scroller = document.querySelector('.history-list')!;
    Object.defineProperties(scroller, { clientHeight: { value: 100, configurable: true }, scrollHeight: { value: 100, configurable: true } });
    fireEvent.scroll(scroller);
    await waitFor(() => expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: 'Saved', favoritesOnly: false, cursor: null }));
    await waitFor(() => expect(listRows()).toHaveLength(1));
    await act(async () => finishPage({ ...page(), records: [{ ...records[0], id: 'stale-page' }] }));
    expect(listRows()).toHaveLength(1);
    expect(row('stale-page')).toBeNull();
  });

  it('renders native file icons, basenames, full-path tooltips and missing states instead of path summaries', async () => {
    records = [{ ...records[0], kind: 'files', text: 'truncated path summary' }];
    const entries = [
      { path: '/synthetic/report.pdf', name: 'report.pdf', exists: true, isDir: false, isImage: false, iconDataUrl: 'data:image/png;base64,aWNvbg==' },
      { path: '/synthetic/folder', name: 'folder', exists: true, isDir: true, isImage: false, iconDataUrl: null },
      { path: '/synthetic/deleted-folder', name: 'deleted-folder', exists: false, isDir: true, isImage: false, iconDataUrl: null },
      { path: '/synthetic/missing.png', name: 'missing.png', exists: false, isDir: false, isImage: true, iconDataUrl: null },
    ];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_files' ? entries : original(name, args));
    await mount('clipboard');
    await screen.findByText('report.pdf');
    expect(row('first').querySelectorAll('.clipboard-file')).toHaveLength(4);
    expect(row('first').querySelector('[title="/synthetic/report.pdf"] img')?.getAttribute('src')).toBe(entries[0].iconDataUrl);
    expect(screen.getByText('folder').parentElement?.title).toBe('/synthetic/folder');
    const deletedFolder = screen.getByText('deleted-folder').parentElement!;
    expect(deletedFolder.dataset.missing).toBe('true');
    expect(deletedFolder.querySelector('svg.lucide-folder')).not.toBeNull();
    expect(deletedFolder.querySelector('svg.lucide-file')).toBeNull();
    expect(screen.getByText('missing.png').parentElement?.dataset.missing).toBe('true');
    expect(screen.getByText('missing.png').parentElement?.querySelector('svg.lucide-file')).not.toBeNull();
    expect(screen.queryByText('truncated path summary')).toBeNull();
    expect(calls('clipboard_image')).toHaveLength(0);
  });

  it('previews only a single existing image file and falls back to its filename when decoding fails', async () => {
    records = [{ ...records[0], kind: 'files' }];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_files') return [{ path: '/synthetic/image.png', name: 'image.png', exists: true, isDir: false, isImage: true, iconDataUrl: null }];
      return original(name, args);
    });
    await mount('clipboard');
    const image = await screen.findByRole('img', { name: 'Image' });
    expect(image.getAttribute('src')).toBe('nami-media://localhost/image/first?w=1024&h=64');
    expect(calls('clipboard_image')).toHaveLength(0);
    fireEvent.error(image);
    await screen.findByText('image.png');
    expect(row('first').querySelector('.clipboard-file')?.getAttribute('title')).toBe('/synthetic/image.png');
  });

  it('keeps multiple images as a file list and shares the same media renderer with titled records', async () => {
    records = [{ ...records[0], kind: 'files', title: 'Media note' }];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_files' ? ['a.png', 'b.png'].map((name) => ({ path: `/synthetic/${name}`, name, exists: true, isDir: false, isImage: true, iconDataUrl: null })) : original(name, args));
    await mount('clipboard');
    await screen.findByText('a.png');
    fireEvent.mouseEnter(row('first'));
    expect(row('first').querySelector('.clipboard-record-body')?.getAttribute('data-expanded')).toBe('true');
    expect(row('first').querySelectorAll('.clipboard-file')).toHaveLength(2);
    expect(calls('clipboard_image')).toHaveLength(0);
    expect(calls('clipboard_record_detail')).toHaveLength(0);
  });

  it('does not request offscreen media, ignores late responses after unmount and disconnects observers', async () => {
    records = [{ ...records[0], kind: 'files' }];
    let intersect: IntersectionObserverCallback | undefined;
    const disconnect = vi.fn();
    const previousObserver = globalThis.IntersectionObserver;
    vi.stubGlobal('IntersectionObserver', class {
      constructor(callback: IntersectionObserverCallback) { intersect = callback; }
      observe() {} disconnect = disconnect;
    });
    try {
      const view = await mount('clipboard');
      await waitFor(() => expect(listRows()).toHaveLength(1));
      expect(calls('clipboard_files')).toHaveLength(0);
      let finish!: (entries: unknown[]) => void;
      const original = bridge.invoke.getMockImplementation()!;
      bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_files' ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
      act(() => intersect?.([{ isIntersecting: true }] as IntersectionObserverEntry[], {} as IntersectionObserver));
      await waitFor(() => expect(calls('clipboard_files')).toHaveLength(1));
      view.unmount();
      await act(async () => finish([]));
      expect(disconnect).toHaveBeenCalled();
    } finally { vi.stubGlobal('IntersectionObserver', previousObserver); }
  });

  it('streams Retina-sized previews over nami-media without JSON IPC and caps pixel work', async () => {
    const previousWidth = window.innerWidth, previousScale = window.devicePixelRatio;
    records = [{ ...records[0], id: 'retina-image', kind: 'image', text: '' }];
    Object.defineProperties(window, { innerWidth: { configurable: true, value: 420 }, devicePixelRatio: { configurable: true, value: 2 } });
    try {
      const view = await mount('clipboard');
      const image = await screen.findByRole('img', { name: 'Image' });
      expect(image.getAttribute('src')).toBe('nami-media://localhost/image/retina-image?w=1024&h=128');
      expect(image.getAttribute('decoding')).toBe('async');
      expect(calls('clipboard_image')).toHaveLength(0);

      Object.defineProperty(window, 'innerWidth', { value: 600 });
      fireEvent(window, new Event('resize'));
      await waitFor(() => expect(image.getAttribute('src')).toBe('nami-media://localhost/image/retina-image?w=1280&h=128'));

      Object.defineProperty(window, 'innerWidth', { value: 10000 });
      fireEvent(window, new Event('resize'));
      await waitFor(() => expect(image.getAttribute('src')).toBe('nami-media://localhost/image/retina-image?w=2048&h=128'));

      view.unmount();
      fireEvent(window, new Event('resize'));
      expect(calls('clipboard_image')).toHaveLength(0);
    } finally {
      Object.defineProperties(window, { innerWidth: { configurable: true, value: previousWidth }, devicePixelRatio: { configurable: true, value: previousScale } });
    }
  });

  it('upgrades only intersecting images and keeps the existing image URL when reentering at the same size', async () => {
    records = [{ ...records[0], id: 'visible-image', kind: 'image', text: '' }];
    const previousObserver = globalThis.IntersectionObserver, previousWidth = window.innerWidth;
    let intersect!: IntersectionObserverCallback;
    const disconnect = vi.fn();
    vi.stubGlobal('IntersectionObserver', class {
      constructor(callback: IntersectionObserverCallback) { intersect = callback; }
      observe() {} disconnect = disconnect;
    });
    const enter = (value: boolean) => act(() => intersect([{ isIntersecting: value }] as IntersectionObserverEntry[], {} as IntersectionObserver));
    try {
      const view = await mount('clipboard');
      await waitFor(() => expect(listRows()).toHaveLength(1));
      expect(screen.queryByRole('img', { name: 'Image' })).toBeNull();
      enter(true);
      const img = await screen.findByRole('img', { name: 'Image' });
      expect(img.getAttribute('src')).toBe('nami-media://localhost/image/visible-image?w=1024&h=64');
      enter(false);
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1500 });
      fireEvent(window, new Event('resize'));
      expect(img.getAttribute('src')).toBe('nami-media://localhost/image/visible-image?w=1024&h=64');
      enter(true);
      await waitFor(() => expect(img.getAttribute('src')).toBe('nami-media://localhost/image/visible-image?w=1536&h=64'));
      enter(false); enter(true);
      await act(async () => {});
      expect(img.getAttribute('src')).toBe('nami-media://localhost/image/visible-image?w=1536&h=64');
      view.unmount();
      expect(disconnect).toHaveBeenCalled();
    } finally {
      vi.stubGlobal('IntersectionObserver', previousObserver);
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: previousWidth });
    }
  });

  it('does not load image previews while hidden and hibernates image DOM after prolonged hiding', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      records = [{ ...records[0], id: 'hidden-image', kind: 'image', text: '' }];
      const original = bridge.invoke.getMockImplementation()!;
      bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
        if (name === 'app_snapshot') {
          const snap = await original(name, args) as typeof bridge.snapshot;
          return { ...snap, window: { ...snap.window, clipboardVisible: false, clipboardExtent: 0 } };
        }
        return original(name, args);
      });
      await mount('clipboard');
      await waitFor(() => expect(listRows()).toHaveLength(1));
      expect(screen.queryByRole('img', { name: 'Image' })).toBeNull();
      expect(document.querySelector('.clipboard-image-preview .activity-beam')).toBeNull();

      await windowState({ clipboardVisible: true });
      const image = await screen.findByRole('img', { name: 'Image' });
      expect(image.getAttribute('src')).toBe('nami-media://localhost/image/hidden-image?w=1024&h=64');

      // Hiding briefly keeps the decoded image mounted for instant reopen.
      await windowState({ clipboardVisible: false });
      act(() => { vi.advanceTimersByTime(10_000); });
      expect(screen.queryByRole('img', { name: 'Image' })).not.toBeNull();

      // Prolonged idle hiding releases the image node without mounting an animated placeholder.
      act(() => { vi.advanceTimersByTime(15_000); });
      await waitFor(() => expect(screen.queryByRole('img', { name: 'Image' })).toBeNull());
      expect(document.querySelector('.clipboard-image-preview .activity-beam')).toBeNull();

      // Reopening restores the preview immediately.
      await windowState({ clipboardVisible: true });
      await screen.findByRole('img', { name: 'Image' });
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not fall back to mounting all hidden cached rows when filtered results shrink below the virtualize threshold', async () => {
    const clientHeight = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(520);
    const scrollHeight = vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(2880);
    try {
      records = Array.from({ length: 40 }, (_, index) => ({
        ...records[0],
        id: `bulk-${index}`,
        text: `Bulk record ${index}`,
        date: new Date(Date.UTC(2026, 3, 10, 12, 0, 40 - index)).toISOString(),
        isFavorite: index < 2,
        isPinned: false,
      }));
      await mount('clipboard');
      await waitFor(() => {
        const items = document.querySelector('.clipboard-panel .history-items');
        expect(items?.getAttribute('data-virtualized')).toBe('true');
      });

      // Switch to Favorites (2 matching records <= 24) while 40 records remain cached in retainedRecords.
      fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
      await waitFor(() => {
        const mountedRows = document.querySelectorAll('.clipboard-panel .history-row');
        expect(mountedRows).toHaveLength(2);
      });
    } finally {
      clientHeight.mockRestore();
      scrollHeight.mockRestore();
    }
  });

  describe('hidden page reclamation', () => {
    async function open() {
      const hook = renderHook(({ active, kind }) => useClipboard(true, '', false, undefined, kind, '', active),
        { initialProps: { active: true, kind: null as ClipboardKind | null } });
      await act(async () => {});
      return hook;
    }
    const advance = (ms: number) => act(() => { vi.advanceTimersByTime(ms); });
    const batch = (offset: number): ClipboardPage => ({
      records: records.slice(offset, offset + 100), cursor: offset ? null : cursor,
      hasMore: !offset, totalRecords: records.length,
    });
    beforeEach(() => {
      vi.useFakeTimers();
      records = Array.from({ length: 200 }, (_, i) => ({ ...records[0], id: `synthetic-${i}` }));
      queryHandler = async (request) => batch(request.cursor ? 100 : 0);
    });
    afterEach(() => { vi.useRealTimers(); });

    it('ignores a late append after 20 seconds hidden and releases the cursor for reuse', async () => {
      const hook = await open();
      expect(hook.result.current.page.records).toHaveLength(100);
      let finish!: (page: ClipboardPage) => void;
      queryHandler = () => new Promise((resolve) => { finish = resolve; });
      act(() => { void hook.result.current.loadMore(); });
      hook.rerender({ active: false, kind: null });
      advance(20_000);
      await act(async () => finish(batch(100)));
      expect(hook.result.current.page.records).toHaveLength(100);
      expect(hook.result.current.retainedRecords).toHaveLength(100);
      queryHandler = async (request) => batch(request.cursor ? 100 : 0);
      await act(async () => hook.result.current.loadMore());
      expect(calls('clipboard_query')).toHaveLength(2);
      hook.rerender({ active: true, kind: null });
      await act(async () => hook.result.current.loadMore());
      expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ cursor });
      expect(hook.result.current.page.records).toHaveLength(200);
    });

    it('publishes cache-only eviction to React while still hidden', async () => {
      records[0].kind = 'image';
      queryHandler = async ({ kind }) => kind ? page('', false, kind) : batch(0);
      const hook = await open();
      hook.rerender({ active: true, kind: 'image' });
      await act(async () => {});
      expect(hook.result.current.page.records).toHaveLength(1);
      hook.rerender({ active: false, kind: 'image' });
      advance(19_999);
      expect(hook.result.current.retainedRecords).toHaveLength(100);
      advance(1);
      expect(hook.result.current.retainedRecords.map(({ id }) => id)).toEqual(['synthetic-0']);
      expect(calls('clipboard_query')).toHaveLength(2);
    });

    it('keeps a deep page on quick reopen but requires a valid first-page cursor after trimming', async () => {
      const hook = await open();
      await act(async () => hook.result.current.loadMore());
      const deep = hook.result.current.page;
      hook.rerender({ active: false, kind: null });
      advance(19_999);
      hook.rerender({ active: true, kind: null });
      advance(1);
      expect(hook.result.current.page).toBe(deep);
      expect(deep.records).toHaveLength(200);
      expect(calls('clipboard_query')).toHaveLength(2);
      hook.rerender({ active: false, kind: null });
      advance(20_000);
      expect(hook.result.current.page.records).toHaveLength(100);
      queryHandler = async () => { throw new Error('storage_unavailable'); };
      hook.rerender({ active: true, kind: null });
      await act(async () => {});
      expect(hook.result.current.error).toBe(true);
      await act(async () => hook.result.current.loadMore());
      expect(calls('clipboard_query')).toHaveLength(3);
      const freshCursor = { ...cursor, sequence: 300 };
      queryHandler = async (request) => request.cursor ? batch(100) : { ...batch(0), cursor: freshCursor };
      await act(async () => hook.result.current.refresh());
      await act(async () => hook.result.current.loadMore());
      expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ cursor: freshCursor });
      expect(hook.result.current.page.records).toHaveLength(200);
      expect(hook.result.current.error).toBe(false);
    });

    it('consumes hidden changes when the initial subscription becomes ready after reopen', async () => {
      const listen = bridge.listen.getMockImplementation()!;
      let subscribed!: () => void;
      bridge.listen.mockImplementation(async (name, callback) => {
        if (name === 'clipboard_capture_error') await new Promise<void>((resolve) => { subscribed = resolve; });
        return listen(name, callback);
      });
      const hook = await open();
      hook.rerender({ active: false, kind: null });
      advance(20_000);
      await emit('clipboard_changed', 1);
      hook.rerender({ active: true, kind: null });
      await act(async () => subscribed());
      expect(calls('clipboard_query')).toHaveLength(1);
      expect(hook.result.current.page.records).toHaveLength(100);
      expect(hook.result.current.loading).toBe(false);
      expect(hook.result.current.displayReady).toBe(true);
    });

    it.each([false, true])('refreshes once after a pending local write settles on reopen (failed=%s)', async (failed) => {
      const invoke = bridge.invoke.getMockImplementation()!;
      let finishWrite!: () => void;
      bridge.invoke.mockImplementation(async (name, args) => {
        if (name === 'clipboard_record_update') {
          await new Promise<void>((resolve) => { finishWrite = resolve; });
          if (failed) throw new Error('storage_unavailable');
        }
        return invoke(name, args);
      });
      const hook = await open();
      act(() => { void hook.result.current.update('synthetic-0', 'favorite'); });
      hook.rerender({ active: false, kind: null });
      advance(20_000);
      await emit('clipboard_changed', 1);
      hook.rerender({ active: true, kind: null });
      expect(hook.result.current.displayReady).toBe(false);
      expect(calls('clipboard_query')).toHaveLength(1);
      await act(async () => finishWrite());
      expect(calls('clipboard_query')).toHaveLength(2);
      expect(hook.result.current.page.records[0].isFavorite).toBe(!failed);
      expect(hook.result.current.error).toBe(failed);
      expect(hook.result.current.displayReady).toBe(true);
    });

    it('keeps a write rejected while hidden through the reopen query until the next explicit write', async () => {
      const hook = await open();
      let rejectWrite!: (error: Error) => void;
      bridge.invoke.mockImplementationOnce(() => new Promise((_, reject) => { rejectWrite = reject; }));
      await act(async () => { void hook.result.current.update('synthetic-0', 'favorite'); });
      hook.rerender({ active: false, kind: null });
      advance(20_000);
      await emit('clipboard_changed', 1);
      await act(async () => rejectWrite(new Error('storage_unavailable')));
      expect(hook.result.current.error).toBe(true);
      expect(calls('clipboard_query')).toHaveLength(1);

      hook.rerender({ active: true, kind: null });
      await act(async () => {});
      expect(calls('clipboard_query')).toHaveLength(2);
      expect(hook.result.current.displayReady).toBe(true);
      expect(hook.result.current.error).toBe(true);
      await act(async () => hook.result.current.update('synthetic-1', 'favorite'));
      expect(hook.result.current.error).toBe(false);
      expect(hook.result.current.page.records.slice(0, 2).map((record) => record.isFavorite)).toEqual([false, true]);
      expect(calls('clipboard_record_update').map(([, args]) => args)).toEqual([
        { recordId: 'synthetic-0', action: 'favorite' }, { recordId: 'synthetic-1', action: 'favorite' },
      ]);
    });

    it.each(['initial', 'refresh'])('restarts an unfinished %s query once on reopen', async (phase) => {
      let finish!: (page: ClipboardPage) => void;
      let fail!: (error: Error) => void;
      const pending = () => new Promise<ClipboardPage>((resolve, reject) => { finish = resolve; fail = reject; });
      if (phase === 'initial') queryHandler = pending;
      const hook = await open();
      if (phase === 'refresh') {
        queryHandler = pending;
        act(() => hook.result.current.refresh());
      }
      const queried = calls('clipboard_query').length;
      hook.rerender({ active: false, kind: null });
      if (phase === 'refresh') await emit('clipboard_changed', 1);
      advance(20_000);
      expect(hook.result.current.loading).toBe(false);
      if (phase === 'refresh') {
        await emit('clipboard_changed', 2);
        await act(async () => hook.result.current.refreshDeferred());
      }
      expect(calls('clipboard_query')).toHaveLength(queried);
      const late = phase === 'initial' ? () => finish(batch(0)) : () => fail(new Error('stale'));
      let finishNew!: (page: ClipboardPage) => void;
      queryHandler = () => new Promise((resolve) => { finishNew = resolve; });
      hook.rerender({ active: true, kind: null });
      expect(calls('clipboard_query')).toHaveLength(queried + 1);
      await act(async () => late());
      expect(hook.result.current.loading).toBe(true);
      expect(hook.result.current.error).toBe(false);
      if (phase === 'initial') expect(hook.result.current.page.records).toHaveLength(0);
      await act(async () => finishNew({ ...batch(0), records: [records[199]] }));
      expect(hook.result.current.page.records.map(({ id }) => id)).toEqual(['synthetic-199']);
      expect(hook.result.current.loading).toBe(false);
    });
  });

  it('defers clipboard_changed requeries while hidden and refreshes immediately on reopen', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'app_snapshot') {
        const snap = await original(name, args) as typeof bridge.snapshot;
        return { ...snap, window: { ...snap.window, clipboardVisible: false, clipboardExtent: 0 } };
      }
      return original(name, args);
    });
    await mount('clipboard');
    await waitFor(() => expect(calls('clipboard_query')).toHaveLength(1));

    records = [{ ...records[0], id: 'bg-new', text: 'Copied in background' }, ...records];
    await emit('clipboard_changed', 2);
    await emit('clipboard_changed', 3);
    await act(async () => {});
    expect(calls('clipboard_query')).toHaveLength(1);

    await windowState({ clipboardVisible: true });
    await waitFor(() => expect(calls('clipboard_query')).toHaveLength(2));
    await screen.findByText('Copied in background');
  });

  it('uses shared nonblocking feedback for a rejected activation without adding a global error', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_use') throw new Error('clipboard_write');
      return original(name, args);
    });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.click(row('first').querySelector('.history-row-select')!);
    const feedback = await screen.findByText('The native operation did not complete. Retry or check the P0 validation notes.');
    expect(feedback.className).toBe('record-feedback');
    expect(feedback.closest('.clipboard-panel')).toBeTruthy();
    expect(document.querySelector('.global-error')).toBeNull();
    await windowState({ recordsSession: 1 });
    expect(screen.queryByText('The native operation did not complete. Retry or check the P0 validation notes.')).toBeNull();
  });

  it('declines clipboard activation on unsupported platforms', async () => {
    bridge.snapshot.capabilities.clipboardWrite = false;
    bridge.snapshot.capabilities.autoPaste = false;
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(within(row('first')).queryByRole('button', { name: 'Copy' })).toBeNull();
    const firstButton = within(row('first')).getByRole('button', { name: 'Clipboard: Saved text' }) as HTMLButtonElement;
    expect(firstButton.disabled).toBe(false);
    firstButton.focus();
    fireEvent.keyDown(firstButton, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(row('pinned').querySelector('button'));
    fireEvent.click(firstButton);
    fireEvent.keyDown(firstButton, { key: 'Enter' });
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('shows capture and storage failures, recovers on a new event, and cleans listeners', async () => {
    const result = await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    await emit('clipboard_capture_error', 'storage_unavailable');
    expect(screen.getByRole('alert')).toBeTruthy();
    await emit('clipboard_changed', 2);
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
    queryHandler = async () => { throw new Error('storage_unavailable'); };
    await emit('clipboard_changed', 3);
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    result.unmount();
    expect(bridge.listeners.get('clipboard_changed')?.size).toBe(0);
    expect(bridge.listeners.get('clipboard_capture_error')?.size).toBe(0);
  });

  it.each([
    ['Text', 'text', 'No text clipboard records yet', false],
    ['Image', 'image', 'No image clipboard records yet', false],
    ['File', 'files', 'No file clipboard records yet', false],
    ['Image', 'image', 'No image clipboard records yet', true],
  ] as const)('explains an empty %s filter (%s, %s, capture=%s) and clears it', async (label, kind, message, capture) => {
    records = [{ ...records[0], kind: kind === 'text' ? 'files' : 'text' }];
    bridge.snapshot.clipboardCaptureEnabled = capture;
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(1));
    fireEvent.click(screen.getByRole('button', { name: `${label} clipboard records` }));
    await screen.findByText(message);
    expect(screen.queryByRole('button', { name: 'Open settings' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Clear filter' }));
    await waitFor(() => expect(listRows()).toHaveLength(1));
    expect(screen.getByRole('button', { name: `${label} clipboard records` }).getAttribute('aria-pressed')).toBe('false');
    expect(screen.queryByText(message)).toBeNull();
  });

  it('does not show a retired automatic-indexing hint when image text is missing', async () => {
    await mount('clipboard');
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search clipboard' }), { target: { value: 'Missing OCR phrase' } });
    expect(await screen.findByText('No matching clipboard records')).toBeTruthy();
    expect(screen.queryByText(/Automatic image indexing is off/)).toBeNull();
  });

  it('preserves a search when clearing an empty kind filter', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox', { name: 'Search clipboard' });
    fireEvent.change(search, { target: { value: 'Saved' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    await screen.findByText('No matching clipboard records');
    fireEvent.click(screen.getByRole('button', { name: 'Clear filter' }));
    await waitFor(() => expect(row('first')).toBeTruthy());
    expect(search).toHaveProperty('value', 'Saved');
    expect(screen.getByRole('button', { name: 'Image clipboard records' }).getAttribute('aria-pressed')).toBe('false');
  });

  it('directs an empty paused history to the menu bar instead of settings', async () => {
    records = []; await mount('clipboard');
    await screen.findByText('Recording paused. Resume from the menu bar.');
    expect(screen.queryByRole('button', { name: 'Open settings' })).toBeNull();
  });

  it('reports missing saved resources when the restore command rejects before writing', async () => {
    let failure = 'clipboard_image_missing';
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_use') throw failure;
      return original(name, args);
    });
    const user = userEvent.setup(); await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    await user.click(row('first').querySelector('.history-row-select')!);
    expect(screen.getByRole('status').textContent).toBe('The saved image is unavailable and cannot be restored');
    failure = 'clipboard_file_missing';
    await user.click(row('first').querySelector('.history-row-select')!);
    expect(screen.getByRole('status').textContent).toBe('The original file was moved or deleted and cannot be restored');
  });

  it('releases a failed refresh so the same query can still load its next page', async () => {
    queryHandler = async (request) => request.cursor ? { ...page(), records: [{ ...records[0], id: 'second' }] } : { ...page(), cursor, hasMore: true };
    const user = userEvent.setup(); await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const successful = queryHandler;
    queryHandler = async () => { throw new Error('storage_unavailable'); };
    await emit('clipboard_changed', 1);
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    queryHandler = successful;
    scrollNearEnd();
    await waitFor(() => expect(listRows()).toHaveLength(3));
    expect(calls('clipboard_query').at(-1)?.[1]).toMatchObject({ cursor });
  });

  it('keeps a failed favorite visible on reopen and makes Retry query without repeating the write', async () => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = 'list';
    const user = userEvent.setup();
    await mount('clipboard');
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => {
      if (name === 'clipboard_record_update') throw new Error('storage_unavailable');
      return invoke(name, args);
    });
    await user.click(within(row('first')).getByRole('button', { name: 'Favorite' }));
    expect(await screen.findByRole('alert')).toBeTruthy();
    await windowState({ clipboardVisible: false, clipboardExtent: 0, recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    await emit('clipboard_changed', 1);
    await windowState({ clipboardVisible: true, clipboardExtent: bridge.snapshot.window.clipboardWidth, recordsFocus: (bridge.snapshot.window.recordsFocus ?? 0) + 1 });

    const first = row('first').querySelector<HTMLButtonElement>('.history-row-select')!;
    expect(document.activeElement).toBe(first);
    expect(calls('clipboard_query')).toHaveLength(2);
    const retry = within(screen.getByRole('alert')).getByRole('button', { name: 'Retry' });
    const quickPaste = () => fireEvent.keyDown(first, { key: '1', code: 'Digit1', metaKey: true });
    quickPaste();
    expect(calls('clipboard_record_use')).toHaveLength(0);
    await user.click(retry);
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
    expect(calls('clipboard_query')).toHaveLength(3);
    expect(calls('clipboard_record_update')).toHaveLength(1);
    expect(within(row('first')).getByRole('button', { name: 'Favorite' }).getAttribute('aria-pressed')).toBe('false');
    act(() => first.focus());
    quickPaste();
    await waitFor(() => expect(calls('clipboard_record_use').map(([, args]) => args)).toEqual([{ recordId: 'first', action: 'paste' }]));
  });

  it('keeps a failed write retryable when a silent search also fails', async () => {
    const hook = renderHook(() => useClipboard(true, 'Saved', false));
    await waitFor(() => expect(hook.result.current.page.records).toHaveLength(1));
    bridge.invoke.mockRejectedValueOnce(new Error('storage_unavailable'));
    await act(async () => hook.result.current.update('first', 'favorite'));
    queryHandler = async () => { throw new Error('clipboard_search_too_broad'); };
    await emit('clipboard_changed', 1);
    expect(hook.result.current.error).toBe(true);
    expect(hook.result.current.searchError).toBeNull();
    await act(async () => hook.result.current.refresh());
    expect(hook.result.current.searchError).toBe('tooBroad');
    expect(calls('clipboard_record_update')).toHaveLength(1);
  });

  it('clears capture errors after a successful retry and preserves them when retry fails', async () => {
    const user = userEvent.setup(); await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    await emit('clipboard_capture_error', 'storage_unavailable');
    queryHandler = async () => { throw new Error('storage_unavailable'); };
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.queryByRole('progressbar')).toBeNull());
    expect(screen.getByRole('alert')).toBeTruthy();
    queryHandler = undefined;
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('rejects a previous connection result across StrictMode cleanup and reconnect', async () => {
    let finishOld: (value: ClipboardPage) => void = () => {};
    let requests = 0;
    queryHandler = async () => ++requests === 1 ? new Promise((resolve) => { finishOld = resolve; }) : { ...page(), records: [{ ...records[0], id: 'new-connection' }] };
    const hook = renderHook(({ native }) => useClipboard(native, '', false), { initialProps: { native: true }, wrapper: StrictMode });
    await waitFor(() => expect(requests).toBe(1));
    hook.rerender({ native: false });
    hook.rerender({ native: true });
    await waitFor(() => expect(hook.result.current.page.records[0]?.id).toBe('new-connection'));
    await act(async () => finishOld(page()));
    expect(hook.result.current.page.records[0]?.id).toBe('new-connection');
    expect(bridge.listeners.get('clipboard_changed')?.size).toBe(1);
    hook.unmount();
    expect(bridge.listeners.get('clipboard_changed')?.size).toBe(0);
  });

  it.each([false, true])('hides on Escape from the window or search input even when pinned=%s', async (pinned) => {
    bridge.snapshot.window.clipboardPinned = pinned;
    const mounted = await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const focused = document.activeElement!;
    expect(fireEvent.keyDown(focused, { key: 'w', code: 'KeyW', metaKey: true })).toBe(false);
    expect(document.activeElement).toBe(focused);
    expect(calls('window_hide')).toHaveLength(0);
    fireEvent.keyDown(window, { key: 'Escape', isComposing: true });
    expect(calls('window_hide')).toHaveLength(0);
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(calls('window_hide')).toHaveLength(1);
    expect(calls('window_hide').at(-1)?.[1]).toEqual({ region: 'clipboard' });
    const search = screen.getByRole('searchbox'); search.focus();
    fireEvent.keyDown(search, { key: 'Escape' });
    expect(calls('window_hide')).toHaveLength(2);
    mounted.unmount();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(calls('window_hide')).toHaveLength(2);
  });

  it('hands focus to the current record before arrows move between rows', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox');
    const first = row('first').querySelector('.history-row-select')!;
    const pinned = row('pinned').querySelector('.history-row-select')!;
    // Opening starts on the first unpinned record; the search field is one Command-F away.
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: 'f', code: 'KeyF', metaKey: true });
    expect(document.activeElement).toBe(search);
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(pinned);
    fireEvent.keyDown(pinned, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(search);
    fireEvent.keyDown(search, { key: 'Escape' });
    expect(calls('window_hide').at(-1)?.[1]).toEqual({ region: 'clipboard' });
  });
});

describe('clipboard open target', () => {
  it('keeps Command-O for links without showing link actions in the text menu', async () => {
    records = records.map((record) => ({ ...record, kind: 'text', text: `https://example.test/${record.id}` }));
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => name === 'clipboard_open_target' ? 'link' : original(name, args));
    await mount('clipboard');
    const search = screen.getByRole('searchbox');
    await waitFor(() => expect(row('first').hasAttribute('data-keyboard-current')).toBe(true));
    search.focus();
    fireEvent.keyDown(search, { key: 'o', code: 'KeyO', metaKey: true });
    expect(calls('clipboard_open_target').at(-1)?.[1]).toMatchObject({ recordId: 'first', execute: true });
    fireEvent.contextMenu(row('pinned'));
    expect(screen.queryByRole('menuitem', { name: 'Open Link' })).toBeNull();
    expect(calls('clipboard_open_target')).toHaveLength(1);
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('groups file actions and sends only the context row ID for path paste or reveal', async () => {
    bridge.snapshot.window.clipboardDetailVisible = false;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => name === 'clipboard_open_target' ? 'files' : original(name, args));
    await mount('clipboard');
    await waitFor(() => expect(row('pinned')).toBeTruthy());
    fireEvent.contextMenu(row('pinned'));
    const menu = screen.getByRole('menu');
    const layout = [...menu.querySelectorAll('[role=menuitem], [role=separator]')].map((item) =>
      item.getAttribute('role') === 'separator' ? '/' : [item.querySelector('.record-menu-label')?.textContent, item.querySelector('kbd')?.textContent]);
    expect(layout).toEqual([
      ['Paste', '↵'], ['Paste as Path', '—'], '/', ['Show in Finder', '⌘O'], '/',
      ['Unfavorite', '⌘D'], ['Unpin', '⌥⌘P'], ['Note', '⌘N'], '/', ['Delete', '⌘⌫'],
    ]);
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Paste as Path' }));
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'pinned', action: 'pastePaths' }));
    expect(calls('clipboard_open_target')).toHaveLength(0);
    fireEvent.contextMenu(row('pinned'));
    fireEvent.keyDown(document.activeElement!, { key: 'o', code: 'KeyO', metaKey: true });
    expect(calls('clipboard_open_target').at(-1)?.[1]).toEqual({ recordId: 'pinned', execute: true, fileIndex: undefined });
    await waitFor(() => expect(calls('records_details_toggle').at(-1)?.[1]).toEqual({ recordId: 'pinned' }));
  });

  it('keeps a disabled shortcut inactive without removing the file reveal action', async () => {
    bridge.snapshot.clipboardControls.shortcuts.openTarget.enabled = false;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => name === 'clipboard_open_target' ? 'file' : original(name, args));
    await mount('clipboard');
    await waitFor(() => expect(row('pinned')).toBeTruthy());
    fireEvent.contextMenu(row('pinned'));
    const reveal = screen.getByRole('menuitem', { name: 'Show in Finder' });
    expect(reveal.querySelector('kbd')?.textContent).toBe('—');
    fireEvent.keyDown(reveal, { key: 'o', code: 'KeyO', metaKey: true });
    expect(calls('clipboard_open_target')).toHaveLength(0);
    fireEvent.click(reveal);
    expect(calls('clipboard_open_target').at(-1)?.[1]).toMatchObject({ recordId: 'pinned', execute: true });
  });

  it('exposes Paste as Path and Show in Finder for local image files while keeping pixel screenshots unchanged', async () => {
    records = [
      { ...records[0], id: 'local-img', kind: 'image', text: '', hasFilePaths: true, isPinned: false, isFavorite: false },
      { ...records[0], id: 'raw-img', kind: 'image', text: '', isPinned: false, isFavorite: false },
    ];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => name === 'clipboard_open_target' ? 'file' : original(name, args));
    await mount('clipboard');
    await waitFor(() => expect(row('local-img')).toBeTruthy());

    // Local image context menu includes Paste as Path and Show in Finder.
    fireEvent.contextMenu(row('local-img'));
    const localMenu = screen.getByRole('menu');
    expect(within(localMenu).getByRole('menuitem', { name: 'Paste as Path' })).toBeTruthy();
    expect(within(localMenu).getByRole('menuitem', { name: 'Show in Finder' })).toBeTruthy();
    fireEvent.click(within(localMenu).getByRole('menuitem', { name: 'Paste as Path' }));
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'local-img', action: 'pastePaths' }));

    // Cmd+O on local image dispatches clipboard_open_target.
    const localButton = row('local-img').querySelector<HTMLButtonElement>('.history-row-select')!;
    localButton.focus();
    fireEvent.keyDown(localButton, { key: 'o', code: 'KeyO', metaKey: true });
    await waitFor(() => expect(calls('clipboard_open_target').at(-1)?.[1]).toMatchObject({ recordId: 'local-img', execute: true }));

    // Pure screenshot context menu omits Paste as Path and Show in Finder, and Cmd+O is a no-op.
    const countBefore = calls('clipboard_open_target').length;
    fireEvent.contextMenu(row('raw-img'));
    const rawMenu = screen.getByRole('menu');
    expect(within(rawMenu).queryByRole('menuitem', { name: 'Paste as Path' })).toBeNull();
    expect(within(rawMenu).queryByRole('menuitem', { name: 'Show in Finder' })).toBeNull();
    fireEvent.pointerDown(document.body);
    const rawButton = row('raw-img').querySelector<HTMLButtonElement>('.history-row-select')!;
    rawButton.focus();
    fireEvent.keyDown(rawButton, { key: 'o', code: 'KeyO', metaKey: true });
    expect(calls('clipboard_open_target')).toHaveLength(countBefore);
  });

  it('exposes VoiceOver live announcements, pinned landmark, row metadata and focus restoration for menu and inline actions', async () => {
    records = [
      { ...records[1], id: 'pinned-file', kind: 'files', text: 'report.pdf', title: 'Quarterly report', isPinned: true, isFavorite: true },
      { ...records[0], id: 'first-text', kind: 'text', text: 'First entry', isPinned: false, isFavorite: false },
      { ...records[0], id: 'second-text', kind: 'text', text: 'Second entry', isPinned: false, isFavorite: false },
    ];
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(3));

    // Persistent polite live feedback region is mounted even before any message renders.
    const feedbackRegion = document.querySelector('.clipboard-panel .record-feedback-live');
    expect(feedbackRegion?.getAttribute('aria-live')).toBe('polite');
    expect(document.querySelector('.clipboard-panel .record-feedback')).toBeNull();

    // Pinned section has a region landmark and rows expose kind, note, pin and favorite state.
    expect(screen.getByRole('region', { name: 'Pinned records' })).toBeTruthy();
    const pinnedButton = within(row('pinned-file')).getByRole('button', { name: 'Clipboard: report.pdf' });
    expect(pinnedButton.getAttribute('aria-description')).toBe('Files, Note: Quarterly report, Pinned, Favorite');

    // Switching category filter announces active filter and count politely without stealing focus.
    expect(screen.getByRole('group', { name: 'Filter records' })).toBeTruthy();
    const liveFilter = document.querySelector<HTMLElement>('.clipboard-panel .history-footer .sr-only[aria-live="polite"]')!;
    expect(liveFilter.textContent).toBe('');
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    await waitFor(() => expect(liveFilter.textContent).toBe('Favorites, 1 record'));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    await waitFor(() => expect(liveFilter.textContent).toBe('All records, 3 records'));

    // Context-menu action returns focus to the record row instead of dropping to body.
    fireEvent.contextMenu(row('first-text'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Favorite' }));
    await waitFor(() => expect(document.activeElement).toBe(within(row('first-text')).getByRole('button', { name: 'Clipboard: First entry' })));

    // Inline delete button activation restores focus cleanly to the neighboring record.
    const deleteButton = within(row('first-text')).getByRole('button', { name: 'Delete clipboard record' });
    act(() => deleteButton.focus());
    fireEvent.click(deleteButton);
    fireEvent.keyDown(await screen.findByRole('alertdialog', { name: 'Delete this record?' }), { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(document.activeElement).toBe(within(row('second-text')).getByRole('button', { name: 'Clipboard: Second entry' }));
  });

  it('initiates native drag-out on dragstart, surfaces missing-file errors, and respects platform capability', async () => {
    const view = await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const selectButton = within(row('first')).getByRole('button', { name: /Clipboard: Saved text/ });
    expect(selectButton.getAttribute('draggable')).toBe('true');

    const dragEvent = new Event('dragstart', { bubbles: true, cancelable: true });
    selectButton.dispatchEvent(dragEvent);
    expect(dragEvent.defaultPrevented).toBe(true);
    await waitFor(() => expect(calls('clipboard_record_drag').at(-1)?.[1]).toEqual({ recordId: 'first' }));

    const previous = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) =>
      name === 'clipboard_record_drag' ? Promise.reject('clipboard_file_missing') : previous(name, args),
    );
    fireEvent.dragStart(selectButton);
    expect(await screen.findByText('The original file was moved or deleted and cannot be restored')).toBeTruthy();

    bridge.invoke.mockImplementation(async (name: string, args?: unknown) =>
      name === 'clipboard_record_drag' ? Promise.reject('cancelled') : previous(name, args),
    );
    fireEvent.dragStart(selectButton);
    await waitFor(() => expect(document.querySelector('.clipboard-panel .record-feedback')).toBeNull());
    view.unmount();

    bridge.snapshot.capabilities.clipboardDrag = false;
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const disabledButton = within(row('first')).getByRole('button', { name: /Clipboard: Saved text/ });
    expect(disabledButton.getAttribute('draggable')).toBe('false');
    const before = calls('clipboard_record_drag').length;
    fireEvent.dragStart(disabledButton);
    expect(calls('clipboard_record_drag')).toHaveLength(before);
  });

  it('renders L1 inline visual recognition for colors, links, emails, and code while preserving exact text', async () => {
    records = [
      { id: 'color-alpha', kind: 'text', text: 'rgba(255, 128, 0, 0.5)', date: '2026-11-24T00:05:00Z', isFavorite: false, isPinned: false },
      { id: 'link-row', kind: 'text', text: 'https://github.com/aihaipeng/nami', date: '2026-11-24T00:04:00Z', isFavorite: false, isPinned: false },
      { id: 'email-row', kind: 'text', text: 'hello@example.test', date: '2026-11-24T00:03:00Z', isFavorite: false, isPinned: false },
      { id: 'code-row', kind: 'text', text: '{"name":"nami","v":1}', date: '2026-11-24T00:02:00Z', isFavorite: false, isPinned: false },
      { id: 'plain-row', kind: 'text', text: 'Just an ordinary sentence', date: '2026-11-24T00:01:00Z', isFavorite: false, isPinned: false },
    ];
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(5));

    const colorPreview = row('color-alpha').querySelector<HTMLElement>('.history-text-preview')!;
    expect(colorPreview.dataset.contentKind).toBe('color');
    expect(colorPreview.textContent).toBe('rgba(255, 128, 0, 0.5)');
    const swatch = colorPreview.querySelector<HTMLElement>('.clipboard-color-swatch')!;
    expect(swatch.getAttribute('aria-hidden')).toBe('true');
    expect(swatch.dataset.alpha).toBe('true');
    expect(swatch.style.getPropertyValue('--swatch-color')).toBe('rgb(255 128 0 / 0.5)');

    const linkPreview = row('link-row').querySelector<HTMLElement>('.history-text-preview')!;
    expect(linkPreview.dataset.contentKind).toBe('link');
    expect(linkPreview.textContent).toBe('https://github.com/aihaipeng/nami');
    const linkIcon = linkPreview.querySelector('.clipboard-inline-icon');
    expect(linkIcon?.getAttribute('aria-hidden')).toBe('true');
    expect(linkIcon?.classList.contains('lucide-link')).toBe(true);
    expect(linkPreview.querySelector('.clipboard-link-muted')?.textContent).toBe('https://');
    expect(linkPreview.querySelector('.clipboard-link-host')?.textContent).toBe('github.com');
    expect(linkPreview.querySelector('.clipboard-link-rest')?.textContent).toBe('/aihaipeng/nami');

    const emailPreview = row('email-row').querySelector<HTMLElement>('.history-text-preview')!;
    expect(emailPreview.dataset.contentKind).toBe('email');
    expect(emailPreview.textContent).toBe('hello@example.test');
    expect(emailPreview.querySelector('.clipboard-inline-icon')?.getAttribute('aria-hidden')).toBe('true');
    expect(emailPreview.querySelector('.clipboard-link-host')?.textContent).toBe('hello');

    const codePreview = row('code-row').querySelector<HTMLElement>('.history-text-preview')!;
    expect(codePreview.dataset.contentKind).toBe('code');
    expect(codePreview.textContent).toBe('{"name":"nami","v":1}');

    const plainPreview = row('plain-row').querySelector<HTMLElement>('.history-text-preview')!;
    expect(plainPreview.dataset.contentKind).toBeUndefined();
    expect(plainPreview.querySelector('.clipboard-color-swatch')).toBeNull();
    expect(plainPreview.querySelector('.clipboard-inline-icon')).toBeNull();
    expect(plainPreview.textContent).toBe('Just an ordinary sentence');
  });

  it('keeps Space and custom preview shortcuts working after selecting, pinning, favoriting, filter, and body/scrollbar focus loss', async () => {
    bridge.snapshot.window.clipboardDetailVisible = false;
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const { shortcuts } = bridge.snapshot.clipboardControls;
    await emit('clipboard_controls_changed', {
      shortcuts: { ...shortcuts, preview: { enabled: true, binding: { code: 'Space', modifiers: [] } } },
    });

    const pressSpace = (target: Document | Element | Window, repeat = false) =>
      fireEvent.keyDown(target, { key: ' ', code: 'Space', repeat });

    // Leaving a focused record for document.body still toggles its details.
    row('first').querySelector<HTMLButtonElement>('.history-row-select')!.focus();
    (document.activeElement as HTMLElement | null)?.blur();
    expect(document.activeElement).toBe(document.body);
    expect(pressSpace(document.body)).toBe(false);
    expect(calls('records_details_toggle').at(-1)?.[1]).toEqual({ recordId: 'first' });

    // Clicking Pin moves the record to the pinned section and drops focus; Space still opens details for that record.
    const pinButton = within(row('first')).getByRole('button', { name: 'Pin record' });
    fireEvent.click(pinButton);
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'pin' }));
    (document.activeElement as HTMLElement | null)?.blur();
    expect(pressSpace(document.documentElement)).toBe(false);
    expect(calls('records_details_toggle').at(-1)?.[1]).toEqual({ recordId: 'first' });

    // Clicking Favorite or a footer filter button does not break Space, and Space on a focused action button does not re-trigger it.
    const favoriteButton = within(row('pinned')).getByRole('button', { name: 'Remove favorite' });
    const updatesBefore = calls('clipboard_record_update').length;
    act(() => favoriteButton.focus());
    expect(pressSpace(favoriteButton)).toBe(false);
    expect(calls('clipboard_record_update')).toHaveLength(updatesBefore);
    expect(calls('records_details_toggle').at(-1)?.[1]).toEqual({ recordId: 'pinned' });

    // Repeat Space prevents scrolling without firing a second toggle.
    const togglesBeforeRepeat = calls('records_details_toggle').length;
    expect(pressSpace(document.body, true)).toBe(false);
    expect(calls('records_details_toggle')).toHaveLength(togglesBeforeRepeat);

    // Typing Space in the search box still edits text normally.
    const search = screen.getByRole('searchbox');
    act(() => search.focus());
    expect(pressSpace(search)).toBe(true);
    expect(calls('records_details_toggle')).toHaveLength(togglesBeforeRepeat);

    // Even when details is open and a filter yields zero records, Space still collapses details.
    await windowState({ clipboardDetailVisible: true, rightExtent: bridge.snapshot.window.clipboardDetailWidth });
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    await waitFor(() => expect(listRows()).toHaveLength(0));
    (document.activeElement as HTMLElement | null)?.blur();
    expect(pressSpace(document.body)).toBe(false);
    expect(calls('records_details_toggle').at(-1)?.[1]).toBeUndefined();
  });

  it('ignores unbound Option+Return while plain Return still pastes the focused record', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    const select = row('first').querySelector<HTMLButtonElement>('.history-row-select')!;
    select.focus();
    await userEvent.setup().keyboard('{Alt>}{Enter}{/Alt}');
    expect(calls('clipboard_record_use')).toHaveLength(0);
    await userEvent.setup().keyboard('{Enter}');
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'activate' });
  });

  it('omits both copy commands even from a rich-text record menu', async () => {
    records[0] = { ...records[0], hasRichText: true };
    await mount('clipboard');
    await waitFor(() => expect(row('first')).toBeTruthy());
    fireEvent.contextMenu(row('first'));
    const menu = within(screen.getByRole('menu'));
    expect(menu.queryByRole('menuitem', { name: 'Copy' })).toBeNull();
    expect(menu.queryByRole('menuitem', { name: 'Copy as Plain Text' })).toBeNull();
    expect(menu.getByRole('menuitem', { name: 'Paste as Plain Text' })).toBeTruthy();
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('distinguishes Plain Text and Rich Text row headings while grouping both under the Text filter', async () => {
    records = [
      { ...records[0], id: 'plain-1', kind: 'text', text: 'Plain item', isPinned: false, isFavorite: false },
      { ...records[0], id: 'rich-1', kind: 'text', text: 'Rich item', hasRichText: true, isPinned: false, isFavorite: false },
      { ...records[0], id: 'img-1', kind: 'image', text: '', isPinned: false, isFavorite: false },
    ];
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(3));
    expect(row('plain-1').querySelector('.history-languages')?.textContent).toBe('Plain Text');
    expect(row('rich-1').querySelector('.history-languages')?.textContent).toBe('Rich Text');
    expect(row('img-1').querySelector('.history-languages')?.textContent).toBe('Image');

    fireEvent.click(screen.getByRole('button', { name: 'Text clipboard records' }));
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(row('plain-1').querySelector('.history-languages')?.textContent).toBe('Plain Text');
    expect(row('rich-1').querySelector('.history-languages')?.textContent).toBe('Rich Text');
  });

  it('auto-opens details after hovering a record for the configured delay and respects cancellation guards', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    vi.useFakeTimers();
    try {
      // Disabled by default: hovering never opens details.
      fireEvent.mouseMove(row('first'), { clientX: 40, clientY: 80 });
      await act(async () => { vi.advanceTimersByTime(1500); });
      expect(calls('records_details_toggle')).toHaveLength(0);

      // Enable auto-open preview with 600ms delay via live settings broadcast.
      bridge.snapshot.clipboardControls.hoverPreviewEnabled = true;
      bridge.snapshot.clipboardControls.hoverPreviewDelayMs = 600;
      await emit('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));

      // Hovering over row actions cancels preview instead of opening details.
      fireEvent.mouseMove(row('first'), { clientX: 42, clientY: 82 });
      await act(async () => { vi.advanceTimersByTime(400); });
      const actionButton = within(row('first')).getByRole('button', { name: 'Note' });
      fireEvent.mouseMove(actionButton, { clientX: 120, clientY: 82 });
      await act(async () => { vi.advanceTimersByTime(700); });
      expect(calls('records_details_toggle')).toHaveLength(0);

      // Switching rows resets the timer; staying on the second row past the delay opens details.
      fireEvent.mouseMove(row('first'), { clientX: 44, clientY: 82 });
      await act(async () => { vi.advanceTimersByTime(400); });
      fireEvent.pointerLeave(row('first'), { relatedTarget: row('pinned') });
      fireEvent.mouseMove(row('pinned'), { clientX: 44, clientY: 20 });
      await act(async () => { vi.advanceTimersByTime(500); });
      expect(calls('records_details_toggle')).toHaveLength(0);
      await act(async () => { vi.advanceTimersByTime(100); });
      expect(calls('records_details_toggle')).toHaveLength(1);
      expect(calls('records_details_toggle')[0]?.[1]).toEqual({ recordId: 'pinned' });

      // Manually closing details suppresses reopening in place until moving to another row.
      await windowState({ clipboardDetailVisible: true, clipboardDetailId: 'pinned' });
      await windowState({ clipboardDetailVisible: false });
      fireEvent.mouseMove(row('pinned'), { clientX: 46, clientY: 20 });
      await act(async () => { vi.advanceTimersByTime(800); });
      expect(calls('records_details_toggle')).toHaveLength(1);

      // Moving to another row lifts suppression and opens details after the delay.
      fireEvent.pointerLeave(row('pinned'), { relatedTarget: row('first') });
      fireEvent.mouseMove(row('first'), { clientX: 46, clientY: 84 });
      await act(async () => { vi.advanceTimersByTime(600); });
      expect(calls('records_details_toggle')).toHaveLength(2);
      expect(calls('records_details_toggle')[1]?.[1]).toEqual({ recordId: 'first' });

      // Close details again, then verify leaving rows, scrolling, or pressing ArrowUp cancels pending timers.
      await windowState({ clipboardDetailVisible: false });
      fireEvent.pointerLeave(row('first'), { relatedTarget: document.body });
      fireEvent.mouseMove(row('first'), { clientX: 48, clientY: 84 });
      await act(async () => { vi.advanceTimersByTime(300); });
      fireEvent.scroll(document.querySelector('.history-list')!);
      await act(async () => { vi.advanceTimersByTime(500); });
      expect(calls('records_details_toggle')).toHaveLength(2);

      fireEvent.mouseMove(row('first'), { clientX: 50, clientY: 84 });
      await act(async () => { vi.advanceTimersByTime(300); });
      row('first').querySelector<HTMLButtonElement>('.history-row-select')!.focus();
      fireEvent.keyDown(row('first').querySelector('.history-row-select')!, { key: 'ArrowUp', code: 'ArrowUp' });
      await act(async () => { vi.advanceTimersByTime(800); });
      expect(calls('records_details_toggle')).toHaveLength(2);

      // 0ms delay opens immediately upon real pointer movement.
      bridge.snapshot.clipboardControls.hoverPreviewDelayMs = 0;
      await emit('clipboard_controls_changed', structuredClone(bridge.snapshot.clipboardControls));
      fireEvent.mouseMove(row('pinned'), { clientX: 52, clientY: 22 });
      expect(calls('records_details_toggle')).toHaveLength(3);
      expect(calls('records_details_toggle')[2]?.[1]).toEqual({ recordId: 'pinned' });
    } finally {
      vi.useRealTimers();
    }
  });
});
