// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';

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
