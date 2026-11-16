// @vitest-environment jsdom
import { act, fireEvent, render, renderHook, screen, waitFor, within } from '@testing-library/react';
import { StrictMode } from 'react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';
import { ClipboardNotePreview } from '../../src/clipboard/ClipboardNotePreview';
import { useClipboard } from '../../src/clipboard/useClipboard';
import { HistoryFooter } from '../../src/shared/HistoryFooter';
import { useRecordBrowser } from '../../src/shared/useRecordBrowser';
import type { ClipboardKind, ClipboardPage, ClipboardRecord } from '../../src/clipboard/useClipboard';

let records: ClipboardRecord[];
let queryHandler: ((args: { query: string; favoritesOnly: boolean; kind?: ClipboardKind; cursor: unknown }) => Promise<ClipboardPage>) | undefined;
const cursor = { pinned: false, createdAt: 1794650400000, sequence: 100 };
function page(query = '', favoritesOnly = false, kind?: ClipboardKind): ClipboardPage {
  const matches = records.filter((record) => (!favoritesOnly || record.isFavorite) && (!kind || record.kind === kind) && record.text.toLowerCase().includes(query.toLowerCase()));
  return { records: structuredClone(matches), cursor: null, hasMore: false, totalRecords: matches.length };
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
    if (name === 'clipboard_record_use') return { status: 'copied', copyStatus: 'current', code: null };
    return original(name, args);
  });
});

describe('clipboard type filters', () => {
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
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    await waitFor(() => expect(ids()).toEqual(['picture']));
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    await waitFor(() => expect(listRows()).toHaveLength(3));
    for (const name of ['Text clipboard records', 'Image clipboard records', 'File clipboard records', 'Favorite clipboard records']) expect(pressed(name)).toBe('false');
    expect(document.querySelector('.history-record-count')?.textContent).toBe('3 records');
  });

  it('places mutually exclusive, cancellable type buttons after Favorites without changing translation controls', () => {
    const onKindChange = vi.fn();
    const view = render(<HistoryFooter total={42} clipboard onFavorites={vi.fn()} kind="text" onKindChange={onKindChange} primaryModifier="meta" />);
    const filters = view.container.querySelector('.history-filters')!;
    expect(within(filters as HTMLElement).getAllByRole('button').map((button) => button.getAttribute('aria-label'))).toEqual(['Favorite clipboard records', 'Text clipboard records', 'Image clipboard records', 'File clipboard records']);
    expect(screen.getByRole('button', { name: 'Text clipboard records' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Text clipboard records' }));
    expect(onKindChange).toHaveBeenLastCalledWith(null);
    fireEvent.click(screen.getByRole('button', { name: 'Image clipboard records' }));
    expect(onKindChange).toHaveBeenLastCalledWith('image');
    fireEvent.click(screen.getByRole('button', { name: 'File clipboard records' }));
    expect(onKindChange).toHaveBeenLastCalledWith('files');
    view.rerender(<HistoryFooter total={5} onFavorites={vi.fn()} primaryModifier="meta" />);
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
  it('shows translation and OCR for eligible records regardless of favorite status in copy / translate / OCR order', async () => {
    records = [
      { ...records[0], isFavorite: true },
      { ...records[0], id: 'ordinary' },
      { ...records[0], id: 'blank', text: '   ', isFavorite: true },
      { ...records[0], id: 'text-image', kind: 'image', isFavorite: true },
      { ...records[0], id: 'empty-image', kind: 'image', isFavorite: true },
      { ...records[0], id: 'ordinary-image', kind: 'image' },
      { ...records[1] },
    ];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_has_text' ? ['text-image', 'ordinary-image'].includes((args as { recordId: string }).recordId) : original(name, args));
    await mount('clipboard');
    await waitFor(() => expect(within(row('text-image')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    const actions = (id: string) => within(row(id)).getAllByRole('button').filter((button) => button.closest('.history-row-actions')).map((button) => button.getAttribute('aria-label'));
    expect(actions('first')).toEqual(['Copy', 'Translate', 'Note', 'Remove favorite', 'Pin record']);
    expect(actions('text-image')).toEqual(['Copy', 'Translate', 'Extract text', 'Note', 'Remove favorite', 'Pin record']);
    expect(actions('ordinary')).toEqual(['Copy', 'Translate', 'Note', 'Favorite', 'Pin record', 'Delete clipboard record']);
    expect(actions('ordinary-image')).toEqual(['Copy', 'Translate', 'Extract text', 'Note', 'Favorite', 'Pin record', 'Delete clipboard record']);
    for (const id of ['blank', 'empty-image', 'pinned']) {
      expect(within(row(id)).queryByRole('button', { name: 'Translate' })).toBeNull();
      expect(within(row(id)).queryByRole('button', { name: 'Extract text' })).toBeNull();
    }
    expect(calls('clipboard_record_has_text').map(([, args]) => args)).toEqual([{ recordId: 'text-image' }, { recordId: 'empty-image' }, { recordId: 'ordinary-image' }, { recordId: 'pinned' }]);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(within(row('first')).getByRole('button', { name: 'Translate' })).toBeTruthy();
    expect(calls('clipboard_record_has_text').filter(([, args]) => (args as { recordId: string }).recordId !== 'pinned')).toHaveLength(3);
  });

  it('offers image text actions for a single image file without treating ordinary file paths as text', async () => {
    records = ['image-file', 'blank-file', 'document-file', 'multiple-files'].map((id) => ({ ...records[0], id, kind: 'files' }));
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_has_text' ? (args as { recordId: string }).recordId === 'image-file' : original(name, args));
    await mount('clipboard');
    await waitFor(() => expect(within(row('image-file')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    expect(within(row('image-file')).getByRole('button', { name: 'Translate' })).toBeTruthy();
    for (const id of ['blank-file', 'document-file', 'multiple-files']) {
      expect(within(row(id)).queryByRole('button', { name: 'Translate' })).toBeNull();
      expect(within(row(id)).queryByRole('button', { name: 'Extract text' })).toBeNull();
    }
    for (const label of ['Translate', 'Extract text']) await act(async () => fireEvent.click(within(row('image-file')).getByRole('button', { name: label })));
    expect(calls('clipboard_record_process').map(([, args]) => args)).toEqual([{ recordId: 'image-file', action: 'translate' }, { recordId: 'image-file', action: 'ocr' }]);
  });

  it('rechecks image files after reopening and removes actions when the referenced file changes or disappears', async () => {
    records = [{ ...records[0], id: 'image-file', kind: 'files' }];
    let hasText = true, missing = false;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_has_text') { if (missing) throw new Error('clipboard_file_missing'); return hasText; }
      return original(name, args);
    });
    await mount('clipboard');
    await waitFor(() => expect(within(row('image-file')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    const switchView = async (view: 'translation' | 'clipboard') => {
      fireEvent.click(within(document.querySelector<HTMLElement>(view === 'translation' ? '.clipboard-panel .history-toolbar' : '#translation-history .history-toolbar')!).getByRole('button', { name: view === 'translation' ? 'Translations' : 'Clipboard' }));
      await waitFor(() => expect(bridge.snapshot.window.recordView).toBe(view));
    };
    await switchView('translation'); hasText = false; await switchView('clipboard');
    await waitFor(() => expect(within(row('image-file')).queryByRole('button', { name: 'Extract text' })).toBeNull());
    await switchView('translation'); hasText = true; await switchView('clipboard');
    await waitFor(() => expect(within(row('image-file')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    await switchView('translation'); missing = true; await switchView('clipboard');
    await waitFor(() => expect(within(row('image-file')).queryByRole('button', { name: 'Translate' })).toBeNull());
    expect(calls('clipboard_record_has_text')).toHaveLength(4);
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

  it('serializes image detection, stops after leaving the view, and retries the discarded result on reopening', async () => {
    records = [1, 2].map((index) => ({ ...records[0], id: `picture-${index}`, kind: 'image', isFavorite: true }));
    let finish: (hasText: boolean) => void = () => {};
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_has_text' ? new Promise<boolean>((resolve) => { finish = resolve; }) : original(name, args));
    await mount('clipboard'); await waitFor(() => expect(calls('clipboard_record_has_text')).toHaveLength(1));
    expect(calls('clipboard_record_has_text')[0][1]).toEqual({ recordId: 'picture-1' });
    fireEvent.click(within(document.querySelector<HTMLElement>('.clipboard-panel .history-toolbar')!).getByRole('button', { name: 'Translations' }));
    await waitFor(() => expect(bridge.snapshot.window.recordView).toBe('translation'));
    await act(async () => finish(true));
    expect(calls('clipboard_record_has_text')).toHaveLength(1);
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_has_text' ? true : original(name, args));
    fireEvent.click(within(document.querySelector<HTMLElement>('#translation-history .history-toolbar')!).getByRole('button', { name: 'Clipboard' }));
    await waitFor(() => expect(calls('clipboard_record_has_text')).toHaveLength(3));
    expect(calls('clipboard_record_has_text').map(([, args]) => args)).toEqual([{ recordId: 'picture-1' }, { recordId: 'picture-1' }, { recordId: 'picture-2' }]);
    expect(within(row('picture-2')).getByRole('button', { name: 'Extract text' })).toBeTruthy();
  });

  it('does not cache recognition failures and retries them when the clipboard view is reopened', async () => {
    records = [{ ...records[0], id: 'picture', kind: 'image', isFavorite: true }];
    let fail = true;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_has_text') { if (fail) throw new Error('ocr_failed'); return true; }
      return original(name, args);
    });
    await mount('clipboard'); await waitFor(() => expect(calls('clipboard_record_has_text')).toHaveLength(1));
    expect(within(row('picture')).queryByRole('button', { name: 'Extract text' })).toBeNull();
    fail = false;
    fireEvent.click(within(document.querySelector<HTMLElement>('.clipboard-panel .history-toolbar')!).getByRole('button', { name: 'Translations' }));
    await waitFor(() => expect(bridge.snapshot.window.recordView).toBe('translation'));
    fireEvent.click(within(document.querySelector<HTMLElement>('#translation-history .history-toolbar')!).getByRole('button', { name: 'Clipboard' }));
    await waitFor(() => expect(within(row('picture')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    expect(calls('clipboard_record_has_text')).toHaveLength(2);
  });

  it('dispatches text/image translation and OCR by record ID without invoking copy or paste', async () => {
    records = [{ ...records[0] }, { ...records[0], id: 'picture', kind: 'image' }];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_has_text' ? true : original(name, args));
    await mount('clipboard'); await waitFor(() => expect(within(row('picture')).getByRole('button', { name: 'Extract text' })).toBeTruthy());
    for (const [id, label] of [['first', 'Translate'], ['picture', 'Translate'], ['picture', 'Extract text']]) {
      await act(async () => fireEvent.click(within(row(id)).getByRole('button', { name: label })));
    }
    expect(calls('clipboard_record_process').map(([, args]) => args)).toEqual([{ recordId: 'first', action: 'translate' }, { recordId: 'picture', action: 'translate' }, { recordId: 'picture', action: 'ocr' }]);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(document.querySelector('.record-feedback')).toBeNull();
  });

  it.each([['text', 'Translate', 'Unable to open this text for translation. Try again.'], ['image', 'Extract text', 'Unable to extract and copy text. Try again.']] as const)('reports %s processing failures in English and allows another attempt', async (kind, label, message) => {
    records = [{ ...records[0], kind, isFavorite: true }];
    let fail = true;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_has_text') return true;
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
      if (name === 'clipboard_record_has_text') return true;
      if (name === 'clipboard_record_process') return new Promise<void>((resolve) => { finish = resolve; });
      return original(name, args);
    });
    await mount('clipboard');
    const process = await within(row('first') ?? document.body).findByRole('button', { name: 'Extract text' }) as HTMLButtonElement;
    fireEvent.click(process);
    await act(async () => fireEvent.click(within(row('first')).getByRole('button', { name: 'Copy' })));
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
    fireEvent.click(within(document.querySelector<HTMLElement>('.clipboard-panel .history-toolbar')!).getByRole('button', { name: 'Translations' }));
    await waitFor(() => expect(bridge.snapshot.window.recordView).toBe('translation'));
    await act(async () => reject(new Error('late_failure')));
    fireEvent.click(within(document.querySelector<HTMLElement>('#translation-history .history-toolbar')!).getByRole('button', { name: 'Clipboard' }));
    await waitFor(() => expect(bridge.snapshot.window.recordView).toBe('clipboard'));
    expect(document.querySelector('.record-feedback')).toBeNull();
    expect((within(row('first')).getByRole('button', { name: 'Translate' }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe('database clipboard history', () => {
  it('opens a five-action record menu without pasting, with icons and protected red delete', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.contextMenu(row('first'), { clientX: 190, clientY: 200 });
    const menu = screen.getByRole('menu');
    const items = within(menu).getAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual(['Paste', 'Favorite', 'Pin record', 'Note', 'Delete']);
    expect(items.every((item) => item.children.length === 2)).toBe(true);
    expect(items[4].classList.contains('clipboard-menu-delete')).toBe(true);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    expect(document.activeElement).toBe(items[0]);
    fireEvent.keyDown(items[0], { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);
    fireEvent.click(items[1]);
    await waitFor(() => expect(records[0].isFavorite).toBe(true));
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.contextMenu(row('pinned'));
    expect((within(screen.getByRole('menu')).getByRole('menuitem', { name: 'Delete' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('pastes by menu record ID and dismisses on outside pointer or scroll', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.contextMenu(row('first'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Paste' }));
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'paste' }));
    fireEvent.contextMenu(row('first'));
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.contextMenu(row('first'));
    fireEvent.scroll(document.querySelector('.history-list')!);
    expect(screen.queryByRole('menu')).toBeNull();
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
    fireEvent.change(content, { target: { value: 'Edited original text' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
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
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Title' }), { key: 'Escape' });
    expect(screen.getByRole('dialog')).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(calls('window_hide')).toHaveLength(0);
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
    expect(document.querySelector('.history-row.selected')).toBeNull();
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    expect((await screen.findByRole('textbox', { name: 'Title' }) as HTMLInputElement).value).toBe('');
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    fireEvent.click(screen.getByRole('button', { name: 'Keyboard shortcuts' }));
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
    expect(document.querySelector('.history-row.selected')).toBeNull();
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
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(100));
    scrollNearEnd(); await waitFor(() => expect(listRows()).toHaveLength(102));
    const favorite = screen.getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite); expect(listRows()).toHaveLength(1);
    fireEvent.click(favorite); expect(listRows()).toHaveLength(102);
    expect(document.querySelector('.history-list')!.scrollTop).toBe(400);
    expect(row('deep-101')).toBeTruthy();
  });

  it('updates a saved note immediately without discarding deep pages or the current scroll position', async () => {
    const sample = records[0];
    records = Array.from({ length: 102 }, (_, index) => ({ ...sample, id: `note-${index}`, text: `Deep note ${index}`, isFavorite: index === 101 }));
    queryHandler = async ({ cursor: next, favoritesOnly }) => ({ records: favoritesOnly ? records.filter((record) => record.isFavorite) : next ? records.slice(100) : records.slice(0, 100), cursor: favoritesOnly || next ? null : cursor, hasMore: !favoritesOnly && !next, totalRecords: favoritesOnly ? 1 : records.length });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(100));
    scrollNearEnd(); await waitFor(() => expect(listRows()).toHaveLength(102));
    fireEvent.mouseEnter(row('note-101'));
    fireEvent.contextMenu(row('note-101'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Content' })).toHaveProperty('value', 'Deep note 101'));
    fireEvent.change(screen.getByRole('textbox', { name: 'Title' }), { target: { value: 'Updated deep title' } });
    fireEvent.change(screen.getByRole('textbox', { name: 'Content' }), { target: { value: 'Updated full note content' } });
    const queried = calls('clipboard_query').length;
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Note' })).toBeNull());
    expect(listRows()).toHaveLength(102);
    expect(within(row('note-101')).getByText('Updated deep title')).toBeTruthy();
    fireEvent.mouseEnter(row('note-101'));
    await waitFor(() => expect(within(row('note-101')).getByText('Updated full note content')).toBeTruthy());
    expect(calls('clipboard_query')).toHaveLength(queried);
    expect(document.querySelector('.history-list')!.scrollTop).toBe(400);
    const favorite = screen.getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite);
    expect(within(row('note-101')).getByText('Updated deep title')).toBeTruthy();
    fireEvent.click(favorite);
    expect(listRows()).toHaveLength(102);
    expect(document.querySelector('.history-list')!.scrollTop).toBe(400);
  });

  it('removes an edited record from cached search results and updates the matching count without replacing browsed pages', async () => {
    queryHandler = async ({ query, favoritesOnly }) => {
      const matching = records.filter((record) => (!favoritesOnly || record.isFavorite) && record.text.toLowerCase().includes(query.toLowerCase()));
      return { records: matching, cursor: null, hasMore: false, totalRecords: matching.length };
    };
    const { result } = renderHook(() => useClipboard(true, 'Saved', false, () => true));
    await waitFor(() => expect(result.current.page.records).toHaveLength(1));
    const queried = calls('clipboard_query').length;
    act(() => result.current.applyEdit('first', { title: 'Saved title is not indexed', text: 'Different original content' }));
    expect(result.current.page.records).toHaveLength(0);
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
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(100));
    scrollNearEnd(); await waitFor(() => expect(listRows()).toHaveLength(102));
    const list = document.querySelector<HTMLElement>('.history-list')!;
    let physicalTop = 400;
    Object.defineProperties(list, {
      scrollHeight: { configurable: true, get: () => list.querySelectorAll('.history-row:not([hidden])').length * 20 },
      scrollTop: { configurable: true, get: () => Math.max(0, Math.min(physicalTop, list.scrollHeight - list.clientHeight)), set: (value: number) => { physicalTop = Math.max(0, Math.min(value, list.scrollHeight - list.clientHeight)); } },
    });
    list.scrollTop = 1920; fireEvent.scroll(list);
    fireEvent.click(within(row('deep-101')).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(row('deep-101')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    const favorite = screen.getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite); await waitFor(() => expect(listRows()).toHaveLength(1));
    fireEvent.click(favorite);
    expect(listRows()).toHaveLength(102);
    expect(row('deep-101')).toBeTruthy();
    expect(list.scrollTop).toBe(1920);
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
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(100));
    scrollNearEnd(); await waitFor(() => expect(listRows()).toHaveLength(102));
    const before = calls('clipboard_query').length;
    fireEvent.click(within(row('deep-101')).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(row('deep-101')).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    expect(listRows()).toHaveLength(102);
    expect(calls('clipboard_query')).toHaveLength(before);
    expect(document.querySelector('.history-list')!.scrollTop).toBe(400);
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

  it('toggles shortcut help, closes it before hiding the card, and dismisses outside', async () => {
    const user = userEvent.setup(); await mount('clipboard');
    const keyboard = screen.getByRole('button', { name: 'Keyboard shortcuts' });
    await user.click(keyboard);
    expect(screen.getByRole('dialog', { name: 'Keyboard shortcuts' }).textContent).toContain('Paste selection');
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

  it('subscribes before loading real records and separates protected pinned rows', async () => {
    await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(bridge.listen.mock.invocationCallOrder[bridge.listen.mock.calls.findIndex(([name]) => name === 'clipboard_changed')]).toBeLessThan(bridge.invoke.mock.invocationCallOrder[bridge.invoke.mock.calls.findIndex(([name]) => name === 'clipboard_query')]);
    expect(row('pinned').parentElement?.className).toContain('history-pinned');
    expect(within(row('pinned')).queryByRole('button', { name: 'Delete clipboard record' })).toBeNull();
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
    expect(fallback.style.background).toBe('rgb(255, 255, 255)');
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

  it('moves a copied or clicked record to the top of its pin group', async () => {
    records = [
      { id: 'plain-new', kind: 'text', text: 'Newer text', date: '2020-01-02T00:00:00Z', isFavorite: false, isPinned: false },
      { id: 'plain-old', kind: 'text', text: 'Older text', date: '2020-01-01T00:00:00Z', isFavorite: false, isPinned: false },
      { id: 'pin-new', kind: 'files', text: '/tmp/newer.txt', date: '2020-01-02T00:00:00Z', isFavorite: true, isPinned: true },
      { id: 'pin-old', kind: 'files', text: '/tmp/older.txt', date: '2020-01-01T00:00:00Z', isFavorite: true, isPinned: true },
    ];
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['pin-new', 'pin-old', 'plain-new', 'plain-old']));
    await act(async () => fireEvent.click(within(row('pin-old')).getByRole('button', { name: 'Copy' })));
    expect(ids()).toEqual(['pin-old', 'pin-new', 'plain-new', 'plain-old']);
    await act(async () => fireEvent.click(within(row('pin-new')).getByRole('button', { name: 'Clipboard: /tmp/newer.txt' })));
    expect(ids()).toEqual(['pin-new', 'pin-old', 'plain-new', 'plain-old']);
    await act(async () => fireEvent.click(within(row('plain-old')).getByRole('button', { name: 'Clipboard: Older text' })));
    expect(ids()).toEqual(['pin-new', 'pin-old', 'plain-old', 'plain-new']);
  });

  it('keeps a newly copied image ahead of an image copied earlier', async () => {
    records = [
      { id: 'img-b', kind: 'image', text: '', date: '2020-01-02T00:00:00.000Z', isFavorite: true, isPinned: false },
      { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: true, isPinned: false },
    ];
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['img-b', 'img-a']));
    await act(async () => fireEvent.click(within(row('img-a')).getByRole('button', { name: 'Copy' })));
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

  it('keeps the clicked record first while the returned page is still older', async () => {
    records = [
      { id: 'img-b', kind: 'image', text: '', date: '2020-01-02T00:00:00.000Z', isFavorite: false, isPinned: false },
      { id: 'img-a', kind: 'image', text: '', date: '2020-01-01T00:00:00.000Z', isFavorite: false, isPinned: false },
    ];
    const ids = () => [...listRows()].map((element) => (element as HTMLElement).dataset.recordId);
    await mount('clipboard');
    await waitFor(() => expect(ids()).toEqual(['img-b', 'img-a']));
    await act(async () => fireEvent.click(within(row('img-a')).getByRole('button', { name: 'Copy' })));
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
    await act(async () => fireEvent.click(within(row('img-a')).getByRole('button', { name: 'Copy' })));
    refreshed = true;
    await emit('clipboard_changed', 5);
    await waitFor(() => expect(within(row('img-b')).getByRole('button', { name: 'Clipboard: refreshed' })).toBeTruthy());
    expect(ids()).toEqual(['img-a', 'img-b']);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(ids()).toEqual(['img-new', 'img-b', 'img-a']);
  });

  it('still moves a copied record to the top when closing the panel ends the session first', async () => {
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
    fireEvent.click(within(row('plain-old')).getByRole('button', { name: 'Copy' }));
    await windowState({ recordsSession: (bridge.snapshot.window.recordsSession ?? 0) + 1 });
    await act(async () => finish({ status: 'copied', copyStatus: 'current', code: null }));
    expect(ids()).toEqual(['plain-old', 'plain-new']);
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

  it('omits successful copy, paste and manual-paste footer messages', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    let status = 'copied';
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_use' ? { status, copyStatus: 'current', code: null } : original(name, args));
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    for (status of ['copied', 'dispatched', 'manual']) {
      await act(async () => fireEvent.click(within(row('first')).getByRole('button', { name: 'Copy' })));
      expect(document.querySelector('.record-feedback')).toBeNull();
    }
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

  it('uses record IDs for full copy and paste and persists favorite, pin, and delete actions', async () => {
    const user = userEvent.setup(); await mount('clipboard');
    await waitFor(() => expect(listRows()).toHaveLength(2));
    await user.click(within(row('first')).getByRole('button', { name: 'Copy' }));
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'copy' });
    await user.click(within(row('first')).getByRole('button', { name: 'Clipboard: Saved text' }));
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'paste' });
    await user.click(within(row('first')).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(row('first')).queryByRole('button', { name: 'Delete clipboard record' })).toBeNull());
    await user.click(within(row('first')).getByRole('button', { name: 'Remove favorite' }));
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Delete clipboard record' })).toBeTruthy());
    await user.click(within(row('first')).getByRole('button', { name: 'Pin record' }));
    await waitFor(() => expect(row('first').parentElement?.className).toContain('history-pinned'));
    await user.click(within(row('first')).getByRole('button', { name: 'Unpin record' }));
    await waitFor(() => expect(within(row('first')).getByRole('button', { name: 'Delete clipboard record' })).toBeTruthy());
    await user.click(within(row('first')).getByRole('button', { name: 'Delete clipboard record' }));
    await waitFor(() => expect(row('first')).toBeNull());
  });

  it('retains default first-page row instances through search and restores them immediately without another query', async () => {
    const sample = records[0];
    records = Array.from({ length: 100 }, (_, index) => ({ ...sample, id: `home-${index}`, text: `Home text ${index}` }));
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(100));
    const first = row('home-0'); const last = row('home-99');
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Home text 99' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    expect(row('home-0')).toBe(first);
    expect(first.hidden).toBe(true);
    expect(row('home-99')).toBe(last);
    const queries = calls('clipboard_query').length;
    fireEvent.click(screen.getByRole('button', { name: 'Clear clipboard search' }));
    expect(listRows()).toHaveLength(100);
    expect(row('home-0')).toBe(first);
    expect(row('home-99')).toBe(last);
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
    queryHandler = async (request) => request.cursor ? { ...page(), records: [records[0], { ...records[0], id: 'second' }], hasMore: false } : { ...page(), cursor, hasMore: true };
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
      { path: '/synthetic/missing.png', name: 'missing.png', exists: false, isDir: false, isImage: true, iconDataUrl: null },
    ];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_files' ? entries : original(name, args));
    await mount('clipboard');
    await screen.findByText('report.pdf');
    expect(row('first').querySelectorAll('.clipboard-file')).toHaveLength(3);
    expect(row('first').querySelector('[title="/synthetic/report.pdf"] img')?.getAttribute('src')).toBe(entries[0].iconDataUrl);
    expect(screen.getByText('folder').parentElement?.title).toBe('/synthetic/folder');
    expect(screen.getByText('missing.png').parentElement?.dataset.missing).toBe('true');
    expect(screen.queryByText('truncated path summary')).toBeNull();
    expect(calls('clipboard_image')).toHaveLength(0);
  });

  it('previews only a single existing image file and falls back to its filename when decoding fails', async () => {
    records = [{ ...records[0], kind: 'files' }];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_files') return [{ path: '/synthetic/image.png', name: 'image.png', exists: true, isDir: false, isImage: true, iconDataUrl: null }];
      if (name === 'clipboard_image') throw new Error('invalid image');
      return original(name, args);
    });
    await mount('clipboard');
    await screen.findByText('image.png');
    expect(calls('clipboard_image').at(-1)?.[1]).toEqual({ recordId: 'first', maxPixels: 1024 });
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

  it('requests Retina-sized previews, keeps the old bitmap while resizing, and caps pixel work', async () => {
    const previousWidth = window.innerWidth, previousScale = window.devicePixelRatio;
    records = [{ ...records[0], id: 'retina-image', kind: 'image', text: '' }];
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: (image: { bytes: number[]; mime: string }) => void;
    let pending = false;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name !== 'clipboard_image') return original(name, args);
      if (pending) return new Promise((resolve) => { finish = resolve; });
      return { bytes: [137, 80, 78, 71], mime: 'image/png' };
    });
    const create = vi.fn().mockReturnValueOnce('blob:retina-first').mockReturnValue('blob:retina-upgraded');
    const revoke = vi.fn();
    Object.defineProperties(URL, { createObjectURL: { configurable: true, value: create }, revokeObjectURL: { configurable: true, value: revoke } });
    Object.defineProperties(window, { innerWidth: { configurable: true, value: 420 }, devicePixelRatio: { configurable: true, value: 2 } });
    try {
      const view = await mount('clipboard');
      const image = await screen.findByRole('img', { name: 'Image' });
      expect(calls('clipboard_image').at(-1)?.[1]).toEqual({ recordId: 'retina-image', maxPixels: 1024 });
      pending = true;
      Object.defineProperty(window, 'innerWidth', { value: 600 });
      fireEvent(window, new Event('resize'));
      await waitFor(() => expect(calls('clipboard_image').at(-1)?.[1]).toEqual({ recordId: 'retina-image', maxPixels: 1280 }));
      expect(image.getAttribute('src')).toBe('blob:retina-first');
      expect(revoke).not.toHaveBeenCalled();
      await act(async () => finish({ bytes: [137, 80, 78, 71], mime: 'image/png' }));
      expect(image.getAttribute('src')).toBe('blob:retina-upgraded');
      expect(revoke).toHaveBeenCalledWith('blob:retina-first');
      Object.defineProperty(window, 'innerWidth', { value: 10000 });
      fireEvent(window, new Event('resize'));
      await waitFor(() => expect(calls('clipboard_image').at(-1)?.[1]).toEqual({ recordId: 'retina-image', maxPixels: 2048 }));
      const count = calls('clipboard_image').length;
      view.unmount();
      fireEvent(window, new Event('resize'));
      expect(calls('clipboard_image')).toHaveLength(count);
      await act(async () => finish({ bytes: [137, 80, 78, 71], mime: 'image/png' }));
      expect(create).toHaveBeenCalledTimes(2);
    } finally {
      Object.defineProperties(window, { innerWidth: { configurable: true, value: previousWidth }, devicePixelRatio: { configurable: true, value: previousScale } });
    }
  });

  it('upgrades only intersecting images and reuses the cached bitmap when reentering at the same size', async () => {
    records = [{ ...records[0], id: 'visible-image', kind: 'image', text: '' }];
    const previousObserver = globalThis.IntersectionObserver, previousWidth = window.innerWidth;
    let intersect!: IntersectionObserverCallback;
    const disconnect = vi.fn();
    vi.stubGlobal('IntersectionObserver', class {
      constructor(callback: IntersectionObserverCallback) { intersect = callback; }
      observe() {} disconnect = disconnect;
    });
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_image' ? { bytes: [137, 80, 78, 71], mime: 'image/png' } : original(name, args));
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: vi.fn(() => 'blob:visible') });
    const enter = (value: boolean) => act(() => intersect([{ isIntersecting: value }] as IntersectionObserverEntry[], {} as IntersectionObserver));
    try {
      const view = await mount('clipboard');
      await waitFor(() => expect(listRows()).toHaveLength(1));
      enter(true);
      await screen.findByRole('img', { name: 'Image' });
      expect(calls('clipboard_image')).toHaveLength(1);
      enter(false);
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1500 });
      fireEvent(window, new Event('resize'));
      expect(calls('clipboard_image')).toHaveLength(1);
      expect(screen.getByRole('img', { name: 'Image' }).getAttribute('src')).toBe('blob:visible');
      enter(true);
      await waitFor(() => expect(calls('clipboard_image').at(-1)?.[1]).toEqual({ recordId: 'visible-image', maxPixels: 1536 }));
      await act(async () => {});
      const count = calls('clipboard_image').length;
      enter(false); enter(true);
      await act(async () => {});
      expect(calls('clipboard_image')).toHaveLength(count);
      view.unmount();
      expect(disconnect).toHaveBeenCalled();
    } finally {
      vi.stubGlobal('IntersectionObserver', previousObserver);
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: previousWidth });
    }
  });

  it('loads saved image previews by record ID and releases their URLs on unmount', async () => {
    records = [{ ...records[0], id: 'image', kind: 'image', text: '' }];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_image' ? { bytes: [137, 80, 78, 71], mime: 'image/png' } : original(name, args));
    const create = vi.fn(() => 'blob:synthetic-image');
    const revoke = vi.fn();
    Object.defineProperties(URL, { createObjectURL: { configurable: true, value: create }, revokeObjectURL: { configurable: true, value: revoke } });
    const result = await mount('clipboard');
    await screen.findByRole('img', { name: 'Image' });
    expect(calls('clipboard_image').at(-1)?.[1]).toEqual({ recordId: 'image', maxPixels: 1024 });
    expect(create).toHaveBeenCalledTimes(1);
    result.unmount();
    expect(revoke).toHaveBeenCalledWith('blob:synthetic-image');
  });

  it('uses shared nonblocking feedback for a rejected copy without adding a global error', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_record_use') throw new Error('clipboard_write');
      return original(name, args);
    });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.click(within(row('first')).getByRole('button', { name: 'Copy' }));
    const feedback = await screen.findByText('Content was not copied. Try again.');
    expect(feedback.className).toBe('record-feedback');
    expect(feedback.closest('.clipboard-panel')).toBeTruthy();
    expect(document.querySelector('.global-error')).toBeNull();
    await windowState({ recordsSession: 1 });
    expect(screen.queryByText('Content was not copied. Try again.')).toBeNull();
  });

  it('disables native copy and paste when the platform does not implement them', async () => {
    bridge.snapshot.capabilities.clipboardWrite = false;
    bridge.snapshot.capabilities.autoPaste = false;
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    expect((within(row('first')).getByRole('button', { name: 'Copy' }) as HTMLButtonElement).disabled).toBe(true);
    expect((within(row('first')).getByRole('button', { name: 'Clipboard: Saved text' }) as HTMLButtonElement).disabled).toBe(true);
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

  it('shows a useful empty state and respects platform clipboard capabilities', async () => {
    records = []; await mount('clipboard');
    await screen.findByText('Enable clipboard recording to keep copied content on this device');
    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(calls('window_show').at(-1)?.[1]).toEqual({ target: 'settings' });
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
    await user.click(within(row('first')).getByRole('button', { name: 'Copy' }));
    expect(screen.getByRole('status').textContent).toBe('The saved image is unavailable and cannot be restored');
    failure = 'clipboard_file_missing';
    await user.click(within(row('first')).getByRole('button', { name: 'Copy' }));
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

  it('moves keyboard focus through rows and hides with Escape', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    const search = screen.getByRole('searchbox'); search.focus();
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(within(row('pinned')).getByRole('button', { name: 'Clipboard: /tmp/synthetic.txt' }));
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(within(row('first')).getByRole('button', { name: 'Clipboard: Saved text' }));
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(calls('window_hide').at(-1)?.[1]).toEqual({ region: 'clipboard' });
  });
});
