// @vitest-environment jsdom
import { act, fireEvent, render, renderHook, screen, waitFor, within } from '@testing-library/react';
import { StrictMode } from 'react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, mount } from './harness';
import { ClipboardNotePreview } from '../../src/clipboard/ClipboardNotePreview';
import { useClipboard } from '../../src/clipboard/useClipboard';
import type { ClipboardPage, ClipboardRecord } from '../../src/clipboard/useClipboard';

let records: ClipboardRecord[];
let queryHandler: ((args: { query: string; favoritesOnly: boolean; cursor: unknown }) => Promise<ClipboardPage>) | undefined;
const cursor = { pinned: false, createdAt: 1794650400000, sequence: 100 };
function page(query = '', favoritesOnly = false): ClipboardPage {
  return { records: structuredClone(records.filter((record) => (!favoritesOnly || record.isFavorite) && record.text.toLowerCase().includes(query.toLowerCase()))), cursor: null, hasMore: false, totalRecords: records.length };
}
const row = (id: string) => document.querySelector<HTMLElement>(`[data-record-id="${id}"]`)!;
function scrollNearEnd() {
  const element = document.querySelector<HTMLElement>('.history-list')!;
  Object.defineProperties(element, { clientHeight: { value: 100, configurable: true }, scrollHeight: { value: 500, configurable: true } });
  element.scrollTop = 400;
  fireEvent.scroll(element);
}
const listRows = () => document.querySelectorAll('.history-row:not([hidden])');

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
      const request = args as { query: string; favoritesOnly: boolean; cursor: unknown };
      const encode = (page: ClipboardPage) => {
        const sources: Record<string, NonNullable<ClipboardRecord['sourceApp']>> = {};
        const records = page.records.map(({ sourceApp, ...record }) => {
          const id = sourceApp ? Object.keys(sources).find((key) => JSON.stringify(sources[key]) === JSON.stringify(sourceApp)) ?? String(Object.keys(sources).length + 1) : null;
          if (id) sources[id] = sourceApp!;
          return { ...record, sourceAppId: id ? Number(id) : null };
        });
        return { ...page, records, sources };
      };
      const result = queryHandler ? await queryHandler(request) : page(request.query, request.favoritesOnly);
      return { page: encode(result), alternate: request.cursor ? null : encode(page(request.query, !request.favoritesOnly)) };
    }
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

  it.each([false, true])('uses EcoPaste visibility events to dismiss drafts and restore defaults when pinned=%s', async (pinned) => {
    bridge.snapshot.window.clipboardPinned = pinned;
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'Saved' } });
    await waitFor(() => expect(listRows()).toHaveLength(1));
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    const title = await screen.findByRole('textbox', { name: 'Title' });
    fireEvent.change(title, { target: { value: 'Unsaved draft' } });
    await emit('window://visibility', { label: 'translation', visible: false });
    expect(screen.getByRole('dialog')).toBeTruthy();
    await emit('window://visibility', { label: 'clipboard', visible: false });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect((screen.getByRole('searchbox') as HTMLInputElement).value).toBe('');
    await emit('window://visibility', { label: 'clipboard', visible: true });
    await waitFor(() => expect(listRows()).toHaveLength(2));
    expect(screen.getByRole('button', { name: 'All clipboard records' }).getAttribute('aria-pressed')).toBe('true');
    expect(document.querySelector('.history-row.selected')).toBeNull();
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.contextMenu(row('first')); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    expect((await screen.findByRole('textbox', { name: 'Title' }) as HTMLInputElement).value).toBe('');
    await emit('window://visibility', { label: 'clipboard', visible: false });
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    fireEvent.click(screen.getByRole('button', { name: 'Keyboard shortcuts' }));
    await emit('window://visibility', { label: 'clipboard', visible: true });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button', { name: 'All clipboard records' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.contextMenu(row('first'));
    await emit('window://visibility', { label: 'clipboard', visible: false });
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
    await emit('window://visibility', { label: 'clipboard', visible: false });
    await act(async () => finish());
    await emit('window://visibility', { label: 'clipboard', visible: true });
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
    Object.defineProperty(document.querySelector('.history-viewport'), 'clientHeight', { value: 400 });
    scrollNearEnd();
    const scrollbar = screen.getByRole('scrollbar');
    const thumb = scrollbar.firstElementChild!;
    fireEvent.pointerDown(thumb, { pointerId: 9, button: 0, clientY: 300 });
    fireEvent.pointerMove(scrollbar, { pointerId: 9, clientY: 150 });
    const element = document.querySelector('.history-list')!;
    expect(element.scrollTop).toBe(212.5);
    expect(document.querySelector('.history-pinned')!.scrollTop).toBe(0);
    fireEvent.lostPointerCapture(scrollbar, { pointerId: 9 });
    fireEvent.pointerMove(scrollbar, { pointerId: 9, clientY: 0 });
    expect(element.scrollTop).toBe(212.5);
  });

  it('hides the scrollbar after inactivity, reveals it on scroll and keeps it visible during drag', async () => {
    const mounted = await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    vi.useFakeTimers();
    try {
      scrollNearEnd();
      const scrollbar = screen.getByRole('scrollbar');
      expect(scrollbar.classList.contains('visible')).toBe(true);
      act(() => vi.advanceTimersByTime(1000));
      expect(scrollbar.classList.contains('visible')).toBe(false);
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
      fireEvent.scroll(document.querySelector('.history-list')!);
      mounted.unmount();
      expect(vi.getTimerCount()).toBe(0);
    } finally { vi.useRealTimers(); }
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
    expect(calls('clipboard_query').at(-1)?.[1]).toEqual({ query: '', favoritesOnly: false, cursor: null });
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
    fireEvent.click(screen.getByRole('button', { name: 'All clipboard records' }));
    expect(listRows()).toHaveLength(2);
    fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
    expect(listRows()).toHaveLength(1);
    expect(calls('clipboard_query')).toHaveLength(queries);
    records[0].isFavorite = true;
    queryHandler = undefined;
    await emit('clipboard_changed', 8);
    await waitFor(() => expect(listRows()).toHaveLength(2));
    fireEvent.click(screen.getByRole('button', { name: 'All clipboard records' }));
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
    expect(row('pinned')).toBeNull();
  });

  it('prefetches favorites outside the first all page and keeps native image nodes mounted across switches', async () => {
    records = Array.from({ length: 125 }, (_, index) => ({ ...records[0], id: `record-${index}`, isFavorite: index === 124, sourceApp: { name: 'Synthetic', iconDataUrl: 'data:image/png;base64,c3ludGhldGlj' } }));
    queryHandler = async (request) => ({ ...page(request.query, request.favoritesOnly), records: records.slice(0, 100), hasMore: true, cursor });
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(100));
    const original = row('record-0'); const source = original.querySelector('img');
    expect(row('record-124').hidden).toBe(true);
    const queries = calls('clipboard_query').length;
    for (let index = 0; index < 8; index++) {
      fireEvent.click(screen.getByRole('button', { name: 'Favorite clipboard records' }));
      expect(listRows()).toHaveLength(1); expect(row('record-124').hidden).toBe(false);
      fireEvent.click(screen.getByRole('button', { name: 'All clipboard records' }));
      expect(listRows()).toHaveLength(100); expect(row('record-0')).toBe(original);
      expect(original.querySelector('img')).toBe(source);
    }
    expect(calls('clipboard_query')).toHaveLength(queries);
    expect(document.querySelectorAll('.history-row')).toHaveLength(101);
  });

  it('omits successful copy, paste and manual-paste footer messages', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    let status = 'copied';
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_record_use' ? { status, copyStatus: 'current', code: null } : original(name, args));
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    for (status of ['copied', 'dispatched', 'manual']) {
      await act(async () => fireEvent.click(within(row('first')).getByRole('button', { name: 'Copy' })));
      expect(document.querySelector('.clipboard-feedback')).toBeNull();
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

  it('keeps a newer search when an older result finishes late and refreshes current filters on capture', async () => {
    await mount('clipboard'); await waitFor(() => expect(listRows()).toHaveLength(2));
    let stale: (value: ClipboardPage) => void = () => {};
    queryHandler = async (request) => request.query === 'slow' ? new Promise((resolve) => { stale = resolve; }) : page(request.query, request.favoritesOnly);
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

  it('loads saved image previews by record ID and releases their URLs on unmount', async () => {
    records = [{ ...records[0], id: 'image', kind: 'image', text: '' }];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'clipboard_image' ? { bytes: [137, 80, 78, 71], mime: 'image/png' } : original(name, args));
    const create = vi.fn(() => 'blob:synthetic-image');
    const revoke = vi.fn();
    Object.defineProperties(URL, { createObjectURL: { configurable: true, value: create }, revokeObjectURL: { configurable: true, value: revoke } });
    const result = await mount('clipboard');
    await screen.findByRole('img', { name: 'Image' });
    expect(calls('clipboard_image').at(-1)?.[1]).toEqual({ recordId: 'image' });
    expect(create).toHaveBeenCalledTimes(1);
    result.unmount();
    expect(revoke).toHaveBeenCalledWith('blob:synthetic-image');
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
