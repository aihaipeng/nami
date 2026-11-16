// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';
import { bridge, calls, emit, mount, windowState } from './harness';
import { RecordActions } from '../../src/shared/RecordActions';
import { translator } from '../../src/shared/i18n';

const clipboardRecords = [
  { id: 'clip-first', kind: 'text', title: '', text: 'Saved text', date: '2026-11-15T12:00:00Z', isFavorite: false, isPinned: false, sourceAppId: null },
  { id: 'clip-favorite', kind: 'text', title: '', text: 'Favorite text', date: '2026-11-15T11:00:00Z', isFavorite: true, isPinned: false, sourceAppId: null },
];
let savedText = 'Saved text';
let failSave = false;
beforeEach(() => {
  savedText = 'Saved text'; failSave = false;
  bridge.snapshot.capabilities.nativeHistory = false;
  bridge.snapshot.capabilities.nativeChrome = false;
  bridge.snapshot.window.recordView = 'clipboard';
  bridge.snapshot.window.translationVisible = false;
  bridge.snapshot.window.translationExtent = 0;
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_query') {
      const { query, favoritesOnly } = args as { query: string; favoritesOnly: boolean };
      const page = (favorites: boolean) => {
        const records = clipboardRecords.filter((record) => (!favorites || record.isFavorite) && record.text.toLowerCase().includes(query.toLowerCase()));
        return { records, sources: {}, totalRecords: records.length, hasMore: false, cursor: null };
      };
      return { page: page(favoritesOnly), alternate: page(!favoritesOnly) };
    }
    if (name === 'clipboard_record_detail') return { title: '', text: savedText, kind: 'text' };
    if (name === 'clipboard_record_update') { if (failSave) throw new Error('save_failed'); savedText = (args as { text: string }).text; }
    return original(name, args);
  });
});

const panel = (kind: 'clipboard' | 'translation') => document.querySelector<HTMLElement>(kind === 'clipboard' ? '.clipboard-panel' : '#translation-history')!;
const search = (kind: 'clipboard' | 'translation') => within(panel(kind)).getByRole('searchbox', { hidden: true }) as HTMLInputElement;
async function switchView(kind: 'clipboard' | 'translation') {
  const toolbar = panel(bridge.snapshot.window.recordView === 'clipboard' ? 'clipboard' : 'translation').querySelector<HTMLElement>('.history-toolbar')!;
  fireEvent.click(within(toolbar).getByRole('button', { name: kind === 'clipboard' ? 'Clipboard' : 'Translations' }));
  await waitFor(() => expect(bridge.snapshot.window.recordView).toBe(kind));
}

describe('unified records workspace', () => {
  it('keeps a protected delete button for its exit animation, blocks it immediately and can reverse the exit', () => {
    const onUpdate = vi.fn();
    const props = { t: translator('en'), native: true, pinned: false, canCopy: true, copyLabel: 'Copy', deleteLabel: 'Delete', onCopy: vi.fn(), onUpdate };
    const view = render(<RecordActions {...props} favorite={false} />);
    const button = screen.getByRole('button', { name: 'Delete' }) as HTMLButtonElement;
    view.rerender(<RecordActions {...props} favorite />);
    expect(screen.queryByRole('button', { name: 'Delete' })).toBeNull();
    expect(view.container.querySelector('[data-action=delete] button')).toBe(button);
    expect(button.disabled).toBe(true);
    expect(button.getAttribute('aria-hidden')).toBe('true');
    fireEvent.click(button);
    expect(onUpdate).not.toHaveBeenCalled();
    view.rerender(<RecordActions {...props} favorite={false} />);
    expect(screen.getByRole('button', { name: 'Delete' })).toBe(button);
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    expect(onUpdate).toHaveBeenCalledExactlyOnceWith('delete');
  });

  it.each(['clipboard', 'translation'] as const)('uses a single full-height %s scroll owner for pinned and ordinary records', async (kind) => {
    await mount('clipboard');
    if (kind === 'translation') await switchView(kind);
    const pane = panel(kind);
    const scroller = pane.querySelector('.history-list')!;
    expect(pane.querySelector('.history-pinned')?.parentElement).toBe(scroller);
    expect(pane.querySelector('.history-items')?.parentElement).toBe(scroller);
    expect(pane.querySelector('.history-viewport')?.querySelectorAll('.history-list')).toHaveLength(1);
  });

  it('uses the same React record workspace in the macOS child WebView and retains both panes', async () => {
    bridge.snapshot.capabilities.nativeHistory = true;
    bridge.history.records[2].date = clipboardRecords[0].date;
    await mount('clipboard');
    const workspace = document.querySelector('.records-workspace')!;
    const clipboardPane = panel('clipboard');
    const translationPane = panel('translation');
    const clipboardRow = clipboardPane.querySelector<HTMLElement>('[data-record-id="clip-first"]')!;
    const icons = (row: HTMLElement) => [...row.querySelectorAll<HTMLElement>('.history-row-actions .history-symbol')].map((icon) => [icon.dataset.symbol, icon.style.getPropertyValue('--symbol-url')]);
    await switchView('translation');
    expect(workspace.getAttribute('data-record-view')).toBe('translation');
    expect(clipboardPane.closest<HTMLElement>('.record-view')!.hidden).toBe(true);
    expect(translationPane.closest<HTMLElement>('.record-view')!.hidden).toBe(false);
    expect(document.activeElement).toBe(search('translation'));
    const translationRow = translationPane.querySelector<HTMLElement>('[data-record-id="sample-03"]')!;
    expect(icons(translationRow)).toEqual(icons(clipboardRow));
    expect(translationRow.querySelector('time')?.textContent).toBe(clipboardRow.querySelector('time')?.textContent);
    expect(translationRow.querySelector('.history-origin')?.getAttribute('style')).toBe(clipboardRow.querySelector('.history-origin')?.getAttribute('style'));
    fireEvent.contextMenu(translationRow);
    expect(screen.getByRole('menu').classList.contains('record-context-menu')).toBe(true);
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(calls('window_hide')).toHaveLength(0);
    await switchView('clipboard');
    expect(panel('clipboard')).toBe(clipboardPane);
    expect(panel('translation')).toBe(translationPane);
  });

  it.each(['clipboard', 'translation'] as const)('keeps %s scrollbars hidden during restoration and reveals them only for user scrolling', async (kind) => {
    await mount('clipboard');
    if (kind === 'translation') await switchView(kind);
    const pane = panel(kind);
    const list = pane.querySelector<HTMLElement>('.history-list')!;
    const viewport = pane.querySelector<HTMLElement>('.history-viewport')!;
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 1000 } });
    Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 100 });
    list.scrollTop = 260; fireEvent.scroll(list);
    const scrollbar = within(pane).getByRole('scrollbar');
    expect(scrollbar.classList.contains('visible')).toBe(false);
    fireEvent.wheel(list);
    expect(scrollbar.classList.contains('visible')).toBe(true);
    await switchView(kind === 'clipboard' ? 'translation' : 'clipboard');
    await switchView(kind);
    expect(list.scrollTop).toBe(260);
    fireEvent.scroll(list);
    expect(scrollbar.classList.contains('visible')).toBe(false);
    vi.useFakeTimers();
    try {
      fireEvent.pointerMove(scrollbar, { pointerId: 3 });
      expect(scrollbar.classList.contains('visible')).toBe(true);
      act(() => vi.advanceTimersByTime(1000));
      expect(scrollbar.classList.contains('visible')).toBe(false);
    } finally { vi.useRealTimers(); }
  });

  it('keeps translation keyboard navigation in the record pane until Return and respects IME before hiding', async () => {
    await mount('clipboard'); await switchView('translation');
    const input = search('translation');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    const selected = document.activeElement as HTMLButtonElement;
    expect(selected.classList.contains('history-row-select')).toBe(true);
    expect(calls('history_sample_select')).toHaveLength(0);
    fireEvent.keyDown(selected, { key: 'ArrowDown' });
    expect(calls('history_sample_select')).toHaveLength(0);
    fireEvent.click(document.activeElement!);
    expect(calls('history_sample_select')).toHaveLength(1);
    fireEvent.keyDown(window, { key: 'Escape', isComposing: true });
    expect(calls('window_hide')).toHaveLength(0);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(calls('window_hide').at(-1)?.[1]).toEqual({ region: 'translation' });
  });

  it('reveals a newly selected translation without stealing row focus or showing the scrollbar', async () => {
    await mount('clipboard'); await switchView('translation');
    const pane = panel('translation');
    const input = search('translation');
    const list = pane.querySelector<HTMLElement>('.history-list')!;
    const row = pane.querySelector<HTMLElement>('[data-record-id="sample-03"]')!;
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 1000 } });
    list.getBoundingClientRect = () => ({ top: 50, bottom: 150 } as DOMRect);
    row.getBoundingClientRect = () => ({ top: 20, bottom: 100 } as DOMRect);
    list.scrollTop = 100;
    await windowState({ translationVisible: true, translationExtent: 920 });
    await emit('history_sample_selected', { sampleId: 'sample-03', retranslate: false });
    expect(list.scrollTop).toBe(66);
    expect(document.activeElement).toBe(input);
    fireEvent.scroll(list);
    expect(within(pane).getByRole('scrollbar').classList.contains('visible')).toBe(false);
    await switchView('clipboard');
    list.scrollTop = 100;
    await emit('history_sample_selected', { sampleId: 'sample-04', retranslate: false });
    expect(list.scrollTop).toBe(100);
  });

  it('keeps row focus when opening details and focuses the translation input only for retranslation', async () => {
    const user = userEvent.setup();
    await mount('translation'); await switchView('translation');
    const row = panel('translation').querySelector<HTMLElement>('[data-record-id="sample-03"]')!;
    const button = row.querySelector<HTMLButtonElement>('.history-row-select')!;
    await user.click(button);
    expect(document.activeElement).toBe(button);
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    fireEvent.contextMenu(row);
    await user.click(screen.getByRole('menuitem', { name: 'Retranslate' }));
    expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Source text' }));
  });

  it('uses the same search layout and detail artwork in both record views', async () => {
    await mount('translation');
    const clipboardSearch = search('clipboard');
    const clear = clipboardSearch.parentElement!.querySelector<HTMLButtonElement>('button')!;
    expect(clear.hidden).toBe(true);
    fireEvent.change(clipboardSearch, { target: { value: 'text' } });
    expect(clipboardSearch.parentElement!.querySelector('button')).toBe(clear);
    expect(clear.hidden).toBe(false);
    fireEvent.click(clear);
    expect(document.activeElement).toBe(clipboardSearch);
    expect(clear.hidden).toBe(true);
    const clipboardIcon = panel('clipboard').querySelector<HTMLElement>('.records-details .history-symbol')!;
    expect(clipboardIcon.dataset.symbol).toBe('chevron.right');
    await switchView('translation');
    expect(panel('translation').querySelector('.history-search')?.className).toBe(clipboardSearch.parentElement!.className);
    const translationIcon = panel('translation').querySelector<HTMLElement>('.records-details .history-symbol')!;
    expect(translationIcon.style.getPropertyValue('--symbol-url')).toBe(clipboardIcon.style.getPropertyValue('--symbol-url'));
    fireEvent.click(within(panel('translation')).getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(translationIcon.dataset.symbol).toBe('chevron.left'));
    expect(panel('translation').querySelector('.records-details')?.getAttribute('aria-expanded')).toBe('true');
    const actions = panel('translation').querySelector<HTMLElement>('.history-row:not(.pinned) .history-row-actions')!;
    expect([...actions.querySelectorAll('.history-symbol')].map((icon) => (icon as HTMLElement).dataset.symbol)).toEqual(['doc.on.doc', 'star', 'pin', 'trash']);
  });

  it('has one ordered toolbar, mutually exclusive modes and centered footer favorites', async () => {
    await mount('translation');
    const toolbar = panel('clipboard').querySelector<HTMLElement>('.history-toolbar')!;
    expect(within(toolbar).getAllByRole('button').map((button) => button.getAttribute('aria-label'))).toEqual(['Clipboard', 'Translations', 'Pin records', 'Details unavailable']);
    expect((within(toolbar).getByRole('button', { name: 'Details unavailable' }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(toolbar).getByRole('button', { name: 'Clipboard' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(within(toolbar).getByRole('button', { name: 'Clipboard' }));
    expect(calls('records_view_set')).toHaveLength(0);
    const favorite = within(panel('clipboard')).getByRole('button', { name: 'Favorite clipboard records' });
    expect(favorite.closest('.history-footer')).toBeTruthy();
    expect(toolbar.contains(favorite)).toBe(false);
    fireEvent.click(favorite);
    expect(favorite.getAttribute('aria-pressed')).toBe('true');
    await waitFor(() => expect(within(panel('clipboard')).getByText('1 record')).toBeTruthy());
    await switchView('translation');
    expect(within(panel('translation')).getByRole('button', { name: 'Translations' }).getAttribute('aria-pressed')).toBe('true');
    expect(within(panel('translation')).getByRole('button', { name: 'Expand details' })).toBeTruthy();
  });

  it('retains independent searches and filters across mode changes, resetting only at full session end', async () => {
    await mount('translation');
    fireEvent.change(search('clipboard'), { target: { value: 'text' } });
    fireEvent.click(within(panel('clipboard')).getByRole('button', { name: 'Favorite clipboard records' }));
    await switchView('translation');
    expect(search('translation').value).toBe('');
    fireEvent.change(search('translation'), { target: { value: 'morning' } });
    await switchView('clipboard');
    expect(search('clipboard').value).toBe('text');
    expect(within(panel('clipboard')).getByRole('button', { name: 'Favorite clipboard records' }).getAttribute('aria-pressed')).toBe('true');
    await switchView('translation');
    expect(search('translation').value).toBe('morning');
    await windowState({ historyVisible: false, historyExtent: 0, translationVisible: true });
    await windowState({ historyVisible: true, historyExtent: 360 });
    expect(search('translation').value).toBe('morning');
    await windowState({ recordsSession: 1 });
    expect(search('translation').value).toBe('');
    expect(search('clipboard').value).toBe('');
    expect(within(panel('clipboard')).getByRole('button', { name: 'Favorite clipboard records', hidden: true }).getAttribute('aria-pressed')).toBe('false');
  });

  it('restores each filter scroll position and resets it when the search changes', async () => {
    await mount('translation');
    const list = panel('clipboard').querySelector<HTMLElement>('.history-list')!;
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 1000 } });
    list.scrollTop = 260; fireEvent.scroll(list);
    const favorite = within(panel('clipboard')).getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite);
    expect(list.scrollTop).toBe(0);
    list.scrollTop = 70; fireEvent.scroll(list);
    fireEvent.click(favorite);
    expect(list.scrollTop).toBe(260);
    await switchView('translation'); await switchView('clipboard');
    expect(list.scrollTop).toBe(260);
    fireEvent.change(search('clipboard'), { target: { value: 'Saved' } });
    expect(list.scrollTop).toBe(0);
  });

  it('restores search focus on explicit clipboard entry without stealing it during normal updates', async () => {
    await mount('translation');
    expect(document.activeElement).toBe(search('clipboard'));
    const pin = within(panel('clipboard')).getByRole('button', { name: 'Pin records' }); pin.focus();
    await windowState({ historyWidth: 420 });
    expect(document.activeElement).toBe(pin);
    await windowState({ recordsFocus: 1 });
    expect(document.activeElement).toBe(search('clipboard'));
  });

  it('keeps clipboard text unchanged when details are unavailable and preserves click-to-paste', async () => {
    await mount('translation');
    await waitFor(() => expect(within(panel('clipboard')).getByRole('button', { name: 'Clipboard: Saved text' })).toBeTruthy());
    fireEvent.click(within(panel('clipboard')).getByRole('button', { name: 'Clipboard: Saved text' }));
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'clip-first', action: 'paste' });
    expect(calls('translation_run')).toHaveLength(0);
  });

  it.each(['Cancel', 'Discard', 'Save'] as const)('protects an unsaved note before global navigation: %s', async (choice) => {
    await mount('clipboard');
    const row = await screen.findByRole('button', { name: 'Clipboard: Saved text' });
    fireEvent.contextMenu(row.closest('article')!); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    const input = await screen.findByRole('textbox', { name: 'Content' });
    fireEvent.change(input, { target: { value: 'Changed content' } });
    await waitFor(() => expect(calls('records_editor_set').at(-1)?.[1]).toEqual({ dirty: true }));
    await emit('records_navigation_requested', null);
    const confirmation = screen.getByRole('alertdialog', { name: 'Save changes?' });
    fireEvent.click(within(confirmation).getByRole('button', { name: choice }));
    await waitFor(() => expect(calls('records_navigation_resolve').at(-1)?.[1]).toEqual({ proceed: choice !== 'Cancel' }));
    if (choice === 'Cancel') expect(screen.getByRole('textbox', { name: 'Content' })).toHaveProperty('value', 'Changed content');
    else expect(screen.queryByRole('dialog', { name: 'Note' })).toBeNull();
    expect(savedText).toBe(choice === 'Save' ? 'Changed content' : 'Saved text');
  });

  it('keeps edits and pending navigation when saving fails', async () => {
    failSave = true;
    await mount('clipboard');
    fireEvent.contextMenu((await screen.findByRole('button', { name: 'Clipboard: Saved text' })).closest('article')!);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    fireEvent.change(await screen.findByRole('textbox', { name: 'Content' }), { target: { value: 'Retained edit' } });
    await emit('records_navigation_requested', null);
    fireEvent.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(within(screen.getByRole('alertdialog')).getByRole('alert').textContent).toContain('preserved'));
    expect(calls('records_navigation_resolve')).toHaveLength(0);
    expect((screen.getByRole('textbox', { name: 'Content', hidden: true }) as HTMLTextAreaElement).value).toBe('Retained edit');
  });
});
