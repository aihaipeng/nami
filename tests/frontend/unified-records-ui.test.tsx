// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';
import { bridge, calls, emit, mount, windowState } from './harness';
import { createRef } from 'react';
// @ts-expect-error Vitest runs in Node; the app's TypeScript configuration includes browser types only.
import { readFileSync } from 'node:fs';
import { HistoryList, type HistoryListHandle } from '../../src/shared/HistoryList';
import { ActivityBeam } from '../../src/shared/ActivityBeam';
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
  it('pins, toggles shortcut help, and focuses search in the active record view', async () => {
    await mount('translation');
    for (const kind of ['clipboard', 'translation'] as const) {
      if (kind === 'translation') await switchView(kind);
      const toolbar = within(panel(kind).querySelector<HTMLElement>('.history-toolbar')!);
      const help = () => within(panel(kind)).queryByRole('dialog', { name: 'HotKey' });
      const key = (code: string, key: string, shiftKey = false, repeat = false) => fireEvent.keyDown(document.activeElement!, { code, key, metaKey: true, shiftKey, repeat });
      toolbar.getByRole('button', { name: 'Pin records' }).focus();
      key('KeyP', 'P', true);
      expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'history', pinned: true });
      await windowState({ historyPinned: true });
      key('KeyP', 'P', true);
      expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'history', pinned: false });
      await windowState({ historyPinned: false });
      key('KeyK', 'k');
      expect(help()?.textContent).toContain('Focus Input');
      expect(screen.getAllByRole('dialog', { name: 'HotKey' })).toHaveLength(1);
      key('KeyK', 'k', false, true);
      expect(help()).toBeTruthy();
      key('KeyK', 'k');
      expect(help()).toBeNull();
      key('KeyK', 'k');
      fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
      expect(help()).toBeNull();
      expect(calls('window_hide')).toHaveLength(0);
      key('KeyF', 'f');
      expect(document.activeElement).toBe(search(kind));
    }
  });

  it('honors modified direction and close keys after remapping a shared shortcut', async () => {
    const preferences = bridge.snapshot.translationPreferences;
    preferences.shortcuts.focusSearch = { code: 'ArrowDown', modifiers: ['meta'] };
    preferences.shortcuts.showShortcuts = { code: 'KeyW', modifiers: ['shift', 'meta'] };
    await mount('translation');
    for (const kind of ['clipboard', 'translation'] as const) {
      if (kind === 'translation') await switchView(kind);
      const row = panel(kind).querySelector<HTMLButtonElement>('.history-row-select')!;
      row.focus();
      fireEvent.keyDown(row, { key: 'ArrowDown', code: 'ArrowDown', metaKey: true });
      expect(document.activeElement).toBe(search(kind));
      if (kind === 'translation') {
        fireEvent.keyDown(search(kind), { key: 'ArrowDown', code: 'ArrowDown', metaKey: true });
        expect(document.activeElement).toBe(search(kind));
      }
      fireEvent.keyDown(search(kind), { key: 'W', code: 'KeyW', metaKey: true, shiftKey: true });
      expect(within(panel(kind)).getByRole('dialog', { name: 'HotKey' })).toBeTruthy();
      expect(calls('window_hide')).toHaveLength(0);
    }
  });

  it('uses saved shared bindings and ignores disabled or composing shortcuts', async () => {
    const preferences = bridge.snapshot.translationPreferences;
    preferences.shortcuts.focusSearch = { code: 'KeyJ', modifiers: ['alt'] };
    preferences.disabledShortcuts = ['showShortcuts'];
    await mount('clipboard');
    const button = within(panel('clipboard')).getByRole('button', { name: 'HotKey' });
    button.focus();
    fireEvent.keyDown(button, { key: 'k', code: 'KeyK', metaKey: true });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    fireEvent.keyDown(button, { key: 'f', code: 'KeyF', metaKey: true });
    fireEvent.keyDown(button, { key: 'j', code: 'KeyJ', altKey: true, isComposing: true });
    expect(document.activeElement).toBe(button);
    fireEvent.keyDown(button, { key: 'j', code: 'KeyJ', altKey: true });
    expect(document.activeElement).toBe(search('clipboard'));
  });

  it('keeps an image translation when the newly revealed history row enters a stationary pointer', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    let pending: { text: string; error: null } | null = null;
    let finish!: () => void;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_translation_take') { const context = pending; pending = null; return context; }
      if (name === 'translation_run') await new Promise<void>((resolve) => { finish = resolve; });
      return original(name, args);
    });
    await mount('translation');
    await switchView('translation');
    fireEvent.click(panel('translation').querySelector('[data-record-id="sample-01"] .history-row-select')!);
    const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
    await waitFor(() => expect(source().value).toBe(bridge.history.records.find(({ id }) => id === 'sample-01')!.text));
    await switchView('clipboard');
    expect(bridge.listeners.get('history_refocus')?.size).toBe(1);
    pending = { text: 'Synthetic image OCR content. Please translate this image.', error: null };
    await windowState({ recordView: 'translation', translationVisible: true, recordsBrowsing: false });
    await emit('clipboard_translation_ready', null);
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    expect(source().value).toBe('Synthetic image OCR content. Please translate this image.');
    const selections = calls('history_sample_select').length;
    fireEvent.mouseMove(panel('translation').querySelector('[data-record-id="sample-01"]')!);
    fireEvent.focus(panel('translation').querySelector('[data-record-id="sample-01"] .history-row-select')!);
    expect(calls('history_sample_select')).toHaveLength(selections);
    fireEvent.mouseEnter(panel('translation').querySelector('[data-record-id="sample-01"]')!);
    expect(calls('history_sample_select')).toHaveLength(selections);
    expect(source().value).toBe('Synthetic image OCR content. Please translate this image.');
    await act(async () => finish());
    await waitFor(() => expect(bridge.history.records.some(({ text }) => text === source().value)).toBe(true));
    const newest = panel('translation').querySelector('[data-record-id="translation-test"] .history-row-select');
    expect(document.activeElement).toBe(newest);
    expect(newest?.getAttribute('aria-pressed')).toBe('true');
    expect(newest?.closest('[data-record-id]')?.hasAttribute('data-keyboard-current')).toBe(true);
    expect(source().value).toBe('Synthetic image OCR content. Please translate this image.');
    fireEvent.mouseMove(panel('translation').querySelector('[data-record-id="sample-01"]')!);
    expect(source().value).toBe('Synthetic image OCR content. Please translate this image.');
    fireEvent.click(panel('translation').querySelector('[data-record-id="sample-01"] .history-row-select')!);
    await waitFor(() => expect(source().value).toBe(bridge.history.records.find(({ id }) => id === 'sample-01')!.text));
    fireEvent.mouseMove(panel('translation').querySelector('[data-record-id="sample-03"]')!, { clientY: 90 });
    await waitFor(() => expect(source().value).toBe(bridge.history.records.find(({ id }) => id === 'sample-03')!.text));
  });

  it('saves a clipboard translation without stealing focus from a record opened while waiting', async () => {
    const text = 'Clipboard translation that finishes after browsing.';
    let pending = false;
    let finish!: () => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_translation_take') {
        if (!pending) return null;
        pending = false;
        return { text, error: null };
      }
      if (name === 'translation_run') await new Promise<void>((resolve) => { finish = resolve; });
      return original(name, args);
    });
    await mount('translation');
    await switchView('clipboard');
    await windowState({ recordView: 'translation', translationVisible: true, recordsBrowsing: false });
    pending = true;
    await emit('clipboard_translation_ready', null);
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));
    const old = panel('translation').querySelector<HTMLButtonElement>('[data-record-id="sample-03"] .history-row-select')!;
    old.focus(); fireEvent.click(old);
    const originalText = bridge.history.records.find(({ id }) => id === 'sample-03')!.text;
    const source = screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Source text' });
    await waitFor(() => expect(source.value).toBe(originalText));
    await act(async () => finish());
    await waitFor(() => expect(bridge.history.records.some((record) => record.text === text)).toBe(true));
    expect(document.activeElement).toBe(old);
    expect(bridge.history.selectedId).toBe('sample-03');
    expect(source.value).toBe(originalText);
  });

  it('creates a new image translation after deleting its previous result without restoring an unrelated record', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    const text = 'Synthetic image text translated again after deletion.';
    let pending: { text: string; error: null } | null = null;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_translation_take') { const context = pending; pending = null; return context; }
      return original(name, args);
    });
    await mount('translation');
    const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
    for (let attempt = 0; attempt < 2; attempt++) {
      pending = { text, error: null };
      await windowState({ recordView: 'translation', translationVisible: true });
      await emit('clipboard_translation_ready', null);
      if (attempt === 1) fireEvent.mouseEnter(panel('translation').querySelector('[data-record-id="sample-01"]')!);
      await waitFor(() => expect(calls('translation_run')).toHaveLength(attempt + 1));
      await waitFor(() => expect(source().value).toBe(text));
      const id = attempt === 0 ? 'translation-test' : 'translation-test-2';
      await waitFor(() => expect(bridge.history.selectedId).toBe(id));
      expect(bridge.history.records.filter((record) => record.text === text).map((record) => record.id)).toEqual([id]);
      expect(calls('translation_run').at(-1)?.[1]).toMatchObject({ text, source: 'auto', targets: ['en', 'zh-CN'] });
      if (attempt === 0) {
        const row = panel('translation').querySelector(`[data-record-id="${id}"]`)!;
        const remove = within(row as HTMLElement).getByRole('button', { name: 'Delete history record' });
        fireEvent.click(remove);
        fireEvent.click(remove);
        await waitFor(() => expect(bridge.history.records.some((record) => record.id === id)).toBe(false));
        await switchView('clipboard');
      }
    }
    expect(bridge.history.records.some(({ id }) => id === 'translation-test')).toBe(false);
  });

  it.each(['clipboard', 'translation'] as const)('deletes the targeted %s record on the first menu click', async (kind) => {
    await mount('clipboard');
    if (kind === 'translation') await switchView(kind);
    const id = kind === 'clipboard' ? 'clip-first' : 'sample-03';
    const target = panel(kind).querySelector(`[data-record-id="${id}"]`)!;
    const command = kind === 'clipboard' ? 'clipboard_record_update' : 'history_record_update';
    fireEvent.contextMenu(target);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    await waitFor(() => expect(calls(command)).toHaveLength(1));
    expect(calls(command)[0][1]).toEqual({ recordId: id, action: 'delete' });
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it.each(['{Enter}', ' '])('deletes on the first button activation with %s', async (key) => {
    const onUpdate = vi.fn();
    const user = userEvent.setup();
    render(<RecordActions recordId="keyboard" t={translator('en')} native favorite={false} pinned={false} deleteLabel="Delete" onUpdate={onUpdate} />);
    screen.getByRole('button', { name: 'Delete' }).focus();
    await user.keyboard(key);
    expect(onUpdate).toHaveBeenCalledExactlyOnceWith('delete');
  });

  it('keeps one outline favorite icon and changes only its color state', () => {
    const props = { t: translator('en'), native: true, pinned: false, canCopy: true, copyLabel: 'Copy', deleteLabel: 'Delete', onCopy: vi.fn(), onUpdate: vi.fn() };
    const view = render(<RecordActions {...props} favorite={false} />);
    const icon = screen.getByRole('button', { name: 'Favorite' }).firstElementChild;
    view.rerender(<RecordActions {...props} favorite />);
    const favorite = screen.getByRole('button', { name: 'Remove favorite' });
    expect(favorite.firstElementChild).toBe(icon);
    expect(favorite.classList.contains('record-favorite')).toBe(true);
    expect(icon?.getAttribute('data-symbol')).toBe('star');
    view.rerender(<RecordActions {...props} favorite={false} />);
    expect(screen.getByRole('button', { name: 'Favorite' }).firstElementChild).toBe(icon);
    expect(screen.getByRole('button', { name: 'Favorite' }).classList.contains('record-favorite')).toBe(false);
  });

  it.each(['clipboard', 'translation'] as const)('uses the same outline favorite and active color in the %s footer and menu', async (kind) => {
    await mount('clipboard');
    if (kind === 'translation') await switchView(kind);
    const pane = panel(kind);
    const button = within(pane).getByRole('button', { name: kind === 'clipboard' ? 'Favorite clipboard records' : 'Favorite history' });
    const icon = button.firstElementChild;
    fireEvent.click(button);
    expect(button.firstElementChild).toBe(icon);
    expect(icon?.getAttribute('data-symbol')).toBe('star');
    expect(button.classList.contains('record-favorite')).toBe(true);
    const row = await waitFor(() => {
      const element = pane.querySelector<HTMLElement>(kind === 'clipboard' ? '[data-record-id="clip-favorite"]' : '[data-record-id="sample-04"]');
      expect(element).toBeTruthy(); return element!;
    });
    fireEvent.contextMenu(row);
    const favorite = screen.getByRole('menuitem', { name: 'Unfavorite' });
    expect(favorite.querySelector('[data-symbol="star"]')).toBeTruthy();
    expect(favorite.classList.contains('record-favorite')).toBe(true);
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    fireEvent.click(button);
    expect(button.firstElementChild).toBe(icon);
    expect(button.classList.contains('record-favorite')).toBe(false);
  });

  it('shows copy success only after confirmation, ignores repeats and restores the same-size copy after one second', async () => {
    let finish!: (success: boolean) => void;
    const onCopy = vi.fn(() => new Promise<boolean>((resolve) => { finish = resolve; }));
    render(<RecordActions t={translator('en')} native favorite={false} pinned={false} canCopy copyLabel="Copy" deleteLabel="Delete" onCopy={onCopy} onUpdate={vi.fn()} />);
    const button = screen.getByRole('button', { name: 'Copy' });
    fireEvent.click(button); fireEvent.click(button);
    expect(onCopy).toHaveBeenCalledTimes(1);
    expect(button.querySelector('.copy-success')).toBeNull();
    vi.useFakeTimers();
    try {
      await act(async () => finish(true));
      expect(button.classList.contains('record-copied')).toBe(true);
      expect(button.querySelector('.copy-success')).toBeTruthy();
      fireEvent.click(button);
      expect(onCopy).toHaveBeenCalledTimes(1);
      act(() => vi.advanceTimersByTime(999));
      expect(button.querySelector('.copy-success')).toBeTruthy();
      act(() => vi.advanceTimersByTime(1));
      expect(button.querySelector('.copy-success')).toBeNull();
      expect(button.querySelector('[data-symbol="doc.on.doc"]')).toBeTruthy();
    } finally { vi.useRealTimers(); }
    fireEvent.click(button);
    await act(async () => finish(false));
    expect(button.querySelector('.copy-success')).toBeNull();
  });

  it.each([{ favorite: true, pinned: false }, { favorite: false, pinned: true }])('keeps a protected detail delete in place but disabled: %j', (marks) => {
    const update = vi.fn();
    render(<RecordActions t={translator('en')} native {...marks} canCopy copyLabel="Copy" deleteLabel="Delete" onCopy={vi.fn()} onUpdate={update} retainDelete />);
    const button = screen.getByRole('button', { name: 'Delete' });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(button);
    expect(update).not.toHaveBeenCalled();
  });

  it('blocks conflicting deletion during processing while keeping independent actions available', () => {
    render(<RecordActions t={translator('en')} native favorite={false} pinned={false} canCopy copyLabel="Copy" deleteLabel="Delete" onCopy={vi.fn()} onUpdate={vi.fn()} onNote={vi.fn()} onTranslate={vi.fn()} onOcr={vi.fn()} busy />);
    for (const label of ['Delete', 'Translate', 'Extract text']) expect((screen.getByRole('button', { name: label }) as HTMLButtonElement).disabled).toBe(true);
    for (const label of ['Copy', 'Note', 'Favorite', 'Pin record']) expect((screen.getByRole('button', { name: label }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('does not carry pending copy feedback to the next detail record', async () => {
    let finish!: (success: boolean) => void;
    const props = { t: translator('en'), native: true, favorite: false, pinned: false, canCopy: true, copyLabel: 'Copy', deleteLabel: 'Delete', onCopy: () => new Promise<boolean>((resolve) => { finish = resolve; }), onUpdate: vi.fn(), retainDelete: true };
    const view = render(<RecordActions {...props} recordId="first" />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    view.rerender(<RecordActions {...props} recordId="second" />);
    await act(async () => finish(true));
    expect(view.container.querySelector('.copy-success')).toBeNull();
  });

  it('suppresses very short activity and cancels its delayed reveal', () => {
    vi.useFakeTimers();
    try {
      const view = render(<ActivityBeam label="Working" active />);
      const track = view.container.firstElementChild!;
      expect(track.getAttribute('data-visible')).toBe('false');
      act(() => vi.advanceTimersByTime(100));
      view.rerender(<ActivityBeam label="Working" active={false} />);
      act(() => vi.advanceTimersByTime(500));
      expect(track.getAttribute('data-visible')).toBe('false');
      expect(screen.queryByRole('progressbar')).toBeNull();
    } finally { vi.useRealTimers(); }
  });

  it('reveals a whole-line glow after 180ms and preserves its node for fade-out', () => {
    vi.useFakeTimers();
    try {
      const view = render(<ActivityBeam label="Working" active />);
      const track = view.container.firstElementChild!;
      act(() => vi.advanceTimersByTime(179));
      expect(track.getAttribute('data-visible')).toBe('false');
      act(() => vi.advanceTimersByTime(1));
      expect(track.getAttribute('data-visible')).toBe('true');
      expect(track.getAttribute('style')).toBeNull();
      view.rerender(<ActivityBeam label="Working" active={false} />);
      expect(view.container.firstElementChild).toBe(track);
      expect(track.getAttribute('data-visible')).toBe('false');
      expect(track.getAttribute('aria-hidden')).toBe('true');
      expect(screen.queryByRole('progressbar')).toBeNull();
      view.rerender(<ActivityBeam label="Working" active />);
      expect(track.getAttribute('data-visible')).toBe('false');
      act(() => vi.advanceTimersByTime(180));
      expect(track.getAttribute('data-visible')).toBe('true');
    } finally { vi.useRealTimers(); }
  });

  it('cleans up the reveal timer when its media placeholder unmounts', () => {
    vi.useFakeTimers();
    try {
      const view = render(<ActivityBeam label="Working" />);
      expect(vi.getTimerCount()).toBe(1);
      view.unmount();
      expect(vi.getTimerCount()).toBe(0);
    } finally { vi.useRealTimers(); }
  });

  it('opens on click, closes on the next click and treats double-click clicks the same way', async () => {
    await mount('clipboard'); await switchView('translation');
    const button = panel('translation').querySelector<HTMLElement>('[data-record-id="sample-03"] .history-row-select')!;
    fireEvent.click(button, { detail: 1 });
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
    fireEvent.click(button, { detail: 2 }); fireEvent.doubleClick(button, { detail: 2 });
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(false));
    expect(calls('history_sample_select')).toHaveLength(1);
    fireEvent.click(button, { detail: 1 });
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
    const next = panel('translation').querySelector<HTMLElement>('[data-record-id="sample-04"] .history-row-select')!;
    fireEvent.click(next);
    await waitFor(() => expect(bridge.history.selectedId).toBe('sample-04'));
    expect(bridge.snapshot.window.translationVisible).toBe(true);
  });

  it('keeps a hovered next record open on its first real click and closes on its second click', async () => {
    const user = userEvent.setup();
    await mount('clipboard'); await switchView('translation');
    const button = (id: string) => panel('translation').querySelector<HTMLElement>(`[data-record-id="${id}"] .history-row-select`)!;
    await user.click(button('sample-03'));
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
    await user.click(button('sample-04'));
    expect(bridge.history.selectedId).toBe('sample-04');
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    await user.click(button('sample-04'));
    expect(bridge.snapshot.window.translationVisible).toBe(false);
    await act(async () => { fireEvent.click(button('sample-03')); fireEvent.click(button('sample-03')); });
    expect(bridge.snapshot.window.translationVisible).toBe(false);
  });

  it('keeps a protected delete button for its exit animation, blocks it immediately and can reverse the exit', () => {
    const onUpdate = vi.fn();
    const props = { t: translator('en'), native: true, pinned: false, canCopy: true, copyLabel: 'Copy', deleteLabel: 'Delete', onCopy: vi.fn(), onUpdate };
    const view = render(<RecordActions {...props} favorite={false} />);
    expect(view.container.querySelector('[data-action=pin]')?.getAttribute('style') ?? '').not.toMatch(/translate/i);
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

  it.each(['clipboard', 'translation'] as const)('clips %s ordinary records below the pinned region with one active scroll owner', async (kind) => {
    await mount('clipboard');
    if (kind === 'translation') await switchView(kind);
    const pane = panel(kind);
    const scroller = pane.querySelector('.history-list')!;
    expect(pane.querySelector('.history-pinned')?.parentElement).toBe(scroller.parentElement);
    expect(scroller.contains(pane.querySelector('.history-pinned'))).toBe(false);
    expect(pane.querySelector('.history-items')?.parentElement).toBe(scroller);
    expect(pane.querySelector('.history-viewport')?.querySelectorAll('.history-list')).toHaveLength(1);
  });

  it('preserves row instances across oversized pins, uses one scroll owner and maps the thumb to the full track', () => {
    const list = createRef<HTMLDivElement>(); const pinnedList = createRef<HTMLDivElement>();
    const onNearEnd = vi.fn();
    const view = render(<HistoryList list={list} pinnedList={pinnedList} pinned={[<article key="p" data-testid="p">Pinned</article>]} onNearEnd={onNearEnd}><article data-testid="r">Ordinary</article></HistoryList>);
    const viewport = view.container.querySelector<HTMLElement>('.history-viewport')!;
    const frame = view.container.querySelector<HTMLElement>('.history-list-frame')!;
    const content = view.container.querySelector<HTMLElement>('.history-unpinned')!;
    let pinnedHeight = 80;
    Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 200 });
    Object.defineProperty(pinnedList.current, 'offsetHeight', { configurable: true, get: () => pinnedHeight });
    Object.defineProperties(content, { clientHeight: { configurable: true, value: 120 }, scrollHeight: { configurable: true, value: 400 } });
    Object.defineProperties(frame, { clientHeight: { configurable: true, value: 200 }, scrollHeight: { configurable: true, value: 480 } });
    const pinnedRow = screen.getByTestId('p'); const ordinaryRow = screen.getByTestId('r');
    fireEvent.scroll(content);
    expect(list.current).toBe(content);
    expect(view.container.querySelector<HTMLElement>('.history-scrollbar-thumb')!.style.height).toBe('60px');
    fireEvent.wheel(pinnedRow, { deltaY: 3, deltaMode: 1 });
    expect(content.scrollTop).toBe(48);
    pinnedHeight = 240; fireEvent.scroll(content);
    expect(list.current).toBe(frame);
    expect(frame.scrollTop).toBe(48);
    expect(frame.contains(pinnedRow)).toBe(true);
    expect(view.container.querySelectorAll('.history-list')).toHaveLength(1);
    fireEvent.scroll(frame);
    expect(onNearEnd.mock.calls.at(-1)?.[0]).toBe(frame);
    frame.scrollTop = 100;
    pinnedHeight = 80; fireEvent.scroll(frame);
    expect(list.current).toBe(content);
    expect(content.scrollTop).toBe(100);
    expect(frame.scrollTop).toBe(0);
    expect(screen.getByTestId('p')).toBe(pinnedRow);
    expect(screen.getByTestId('r')).toBe(ordinaryRow);
  });

  it('uses standard arrow keys for navigation and category switching in both views without hijacking search typing', async () => {
    await mount('clipboard');
    for (const kind of ['clipboard', 'translation'] as const) {
      if (kind === 'translation') await switchView(kind);
      const input = search(kind);
      input.focus();
      const key = (target: Element, key: string, code = key) => fireEvent.keyDown(target, { key, code });
      const firstRow = panel(kind).querySelector<HTMLButtonElement>('.history-row-select')!;
      firstRow.focus();
      expect(key(firstRow, 'ArrowDown')).toBe(false);
      expect(document.activeElement).not.toBe(firstRow);
      expect(key(document.activeElement!, 'ArrowUp')).toBe(false);
      expect(document.activeElement).toBe(firstRow);
      expect(key(firstRow, 'ArrowUp')).toBe(false);
      expect(document.activeElement).toBe(input);
      const favorite = within(panel(kind)).getByRole('button', { name: /Favorite (clipboard records|history)/ });
      expect(favorite.getAttribute('aria-pressed')).toBe('false');
      expect(key(input, 'ArrowRight')).toBe(false);
      expect(favorite.getAttribute('aria-pressed')).toBe('true');
      expect(key(input, 'ArrowLeft')).toBe(false);
      expect(favorite.getAttribute('aria-pressed')).toBe('false');
    }
  });

  it.each(['search', 'list'] as const)('opens both views with %s focus and hands off without changing the query', async (openingFocus) => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = openingFocus;
    const { shortcuts } = bridge.snapshot.clipboardControls;
    shortcuts.favorite.binding = { code: 'KeyF', modifiers: [] };
    await mount('clipboard');
    for (const kind of ['clipboard', 'translation'] as const) {
      if (kind === 'translation') await switchView(kind);
      const input = search(kind);
      const row = await waitFor(() => {
        const current = panel(kind).querySelector<HTMLButtonElement>('[data-keyboard-current] .history-row-select');
        expect(current).not.toBeNull();
        return current!;
      });
      expect(document.activeElement).toBe(openingFocus === 'list' ? row : input);
      fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
      expect(document.activeElement).toBe(input);
      expect(fireEvent.keyDown(input, { key: 'f', code: 'KeyF' })).toBe(true);
      expect(document.activeElement).toBe(input);
      const query = kind === 'clipboard' ? 'Saved' : row.getAttribute('aria-label')!.slice('Translation: '.length);
      fireEvent.change(input, { target: { value: query } });
      await waitFor(() => expect(panel(kind).hasAttribute('data-focus-ready')).toBe(true));
      expect(fireEvent.keyDown(input, { key: 'ArrowDown', code: 'ArrowDown' })).toBe(false);
      expect(document.activeElement).toBe(row);
      expect(fireEvent.keyDown(row, { key: 'f', code: 'KeyF' })).toBe(false);
      await waitFor(() => expect(calls(kind === 'clipboard' ? 'clipboard_record_update' : 'history_record_update').at(-1)?.[1]).toEqual({ recordId: row.closest('article')?.dataset.recordId, action: 'favorite' }));
      fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
      expect(document.activeElement).toBe(input);
      expect(input.value).toBe(query);
      expect([input.selectionStart, input.selectionEnd]).toEqual([0, query.length]);
    }
  });

  it.each(['search', 'list'] as const)('keeps one %s keyboard and mouse target in both record views', async (openingFocus) => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = openingFocus;
    await mount('clipboard');
    for (const kind of ['clipboard', 'translation'] as const) {
      if (kind === 'translation') await switchView(kind);
      // Opening and the arrow keys below start in the ordinary list, below any pinned records.
      const rows = () => [...panel(kind).querySelectorAll<HTMLElement>('.history-unpinned .history-row:not([hidden])')];
      await waitFor(() => expect(rows().length).toBeGreaterThan(1));
      const [first, second] = rows();
      const button = (row: HTMLElement) => row.querySelector<HTMLButtonElement>('.history-row-select')!;
      const targets = () => [...panel(kind).querySelectorAll('[data-record-target]')].map((row) => (row as HTMLElement).dataset.recordId);
      expect(document.activeElement).toBe(openingFocus === 'list' ? button(first) : search(kind));
      expect(targets()).toEqual(openingFocus === 'list' ? [first.dataset.recordId] : []);
      fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
      expect(targets()).toEqual([second.dataset.recordId]);
      expect(document.activeElement).toBe(openingFocus === 'list' ? button(second) : search(kind));
      fireEvent.keyDown(document.activeElement!, { key: 'C', code: 'KeyC', metaKey: true, shiftKey: true });
      await waitFor(() => expect(calls(kind === 'clipboard' ? 'clipboard_record_use' : 'translation_sample_copy').at(-1)?.[1]).toEqual(kind === 'clipboard'
        ? { recordId: second.dataset.recordId, action: 'copy' }
        : { sampleId: second.dataset.recordId, part: 'source' }));
      if (openingFocus === 'search') fireEvent.keyDown(search(kind), { key: 'ArrowDown', code: 'ArrowDown' });
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
      expect(document.activeElement).toBe(button(first));
      fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
      expect(targets()).toEqual([first.dataset.recordId]);
      fireEvent.mouseMove(second, { clientX: 11, clientY: 100 });
      expect(targets()).toEqual([second.dataset.recordId]);
    }
  });

  // jsdom has no pointer, so the real hover selectors run against a stand-in attribute.
  function actionStripRules() {
    const lines = (readFileSync('src/styles.css', 'utf8') as string).split('\n').map((line) => line.trim());
    const rules = lines.filter((line) => line.includes('.history-row-actions {') || /\.history-row:(hover|focus-within) \.history-row-actions/.test(line));
    expect(rules.length).toBeGreaterThanOrEqual(3);
    return rules.join('\n').replaceAll(':hover', '[data-hover]');
  }

  it.each(['clipboard', 'translation'] as const)('shows only the %s action strip of the keyboard record while the pointer rests on another', async (kind) => {
    const style = document.createElement('style');
    style.textContent = actionStripRules();
    document.head.append(style);
    try {
      await mount('clipboard');
      if (kind === 'translation') await switchView(kind);
      const rows = () => [...panel(kind).querySelectorAll<HTMLElement>('.history-unpinned .history-row:not([hidden])')];
      await waitFor(() => expect(rows().length).toBeGreaterThan(1));
      const [first, second] = rows();
      const button = (row: HTMLElement) => row.querySelector('.history-row-select')!;
      const strip = (row: HTMLElement) => getComputedStyle(row.querySelector('.history-row-actions')!);
      const shown = () => rows().filter((row) => strip(row).visibility === 'visible').map((row) => row.dataset.recordId);
      const fade = (row: HTMLElement) => `${strip(row).transitionProperty} ${strip(row).transitionDuration}`;
      await waitFor(() => expect(document.activeElement).toBe(button(first)));
      expect(shown()).toEqual([first.dataset.recordId]);
      // A record neither hovered nor focused keeps its ordinary fade-out.
      expect(fade(second)).toBe('opacity, transform, visibility 0.15s, 0.15s, 0.15s');
      second.dataset.hover = '';
      fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
      expect(shown()).toEqual([second.dataset.recordId]);
      // The pointer stays on the second record: the keyboard record alone keeps its actions.
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
      expect(document.activeElement).toBe(button(first));
      expect(shown()).toEqual([first.dataset.recordId]);
      // The resting record gives way on the spot: a fading strip stays visible and clickable.
      expect(fade(second)).toBe('none 0s');
      expect(strip(first).transitionDuration).toBe('0.15s');
      fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
      expect(shown()).toEqual([first.dataset.recordId]);
      fireEvent.mouseMove(second, { clientX: 11, clientY: 100 });
      expect(shown()).toEqual([second.dataset.recordId]);
      // No record holds focus: the resting pointer reveals its own record with its ordinary fade.
      fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
      expect(document.activeElement).toBe(search(kind));
      expect(shown()).toEqual([second.dataset.recordId]);
      expect(fade(second)).toBe('opacity, transform, visibility 0.15s');
      delete second.dataset.hover;
      expect(shown()).toEqual([]);
    } finally { style.remove(); }
  });

  it('falls back to search when record-list focus is preferred but no records exist', async () => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = 'list';
    bridge.history.records = [];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_query') {
        const empty = { records: [], sources: {}, totalRecords: 0, hasMore: false, cursor: null };
        return { page: empty, alternate: empty };
      }
      return original(name, args);
    });
    await mount('clipboard');
    await waitFor(() => expect(document.activeElement).toBe(search('clipboard')));
    await switchView('translation');
    await waitFor(() => expect(document.activeElement).toBe(search('translation')));
    expect(panel('translation').querySelector('.history-row-select')).toBeNull();
  });

  it('opens both views on the first unpinned record by default and moves between it and search', async () => {
    expect(bridge.snapshot.clipboardControls.recordsOpenFocus).toBe('list');
    bridge.history.records.forEach((record) => { record.isPinned = false; });
    await mount('clipboard');
    for (const kind of ['clipboard', 'translation'] as const) {
      if (kind === 'translation') await switchView(kind);
      const row = () => panel(kind).querySelector<HTMLButtonElement>('[data-keyboard-current] .history-row-select');
      await waitFor(() => expect(document.activeElement).toBe(row()));
      expect(row()).toBe(panel(kind).querySelector('.history-row:not([hidden]) .history-row-select'));
      expect(calls('translation_focus_input')).toHaveLength(0);
      fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
      expect(document.activeElement).toBe(search(kind));
      fireEvent.keyDown(search(kind), { key: 'ArrowDown', code: 'ArrowDown' });
      expect(document.activeElement).toBe(row());
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
      expect(document.activeElement).toBe(search(kind));
    }
  });

  it('opens both views on the first unpinned record when pinned records exist', async () => {
    const pinned = clipboardRecords[1];
    pinned.isPinned = true;
    try {
      await mount('clipboard');
      for (const kind of ['clipboard', 'translation'] as const) {
        if (kind === 'translation') await switchView(kind);
        const rows = () => [...panel(kind).querySelectorAll<HTMLButtonElement>('.history-row:not([hidden]) .history-row-select')];
        const unpinned = rows().filter((button) => !button.closest('article')!.classList.contains('pinned'));
        const pinnedRows = rows().filter((button) => button.closest('article')!.classList.contains('pinned'));
        await waitFor(() => expect(document.activeElement).toBe(unpinned[0]));
        expect(pinnedRows.length).toBeGreaterThan(0);
        expect(document.activeElement!.closest('article')!.dataset.recordId).toBe(kind === 'clipboard' ? 'clip-first' : bridge.history.records.find((record) => !record.isPinned)!.id);
        // Pinned rows stay above the first unpinned record for arrow keys as well as the pointer.
        fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
        expect(document.activeElement).toBe(pinnedRows[pinnedRows.length - 1]);
        for (let index = pinnedRows.length - 1; index > 0; index -= 1) fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
        expect(document.activeElement).toBe(pinnedRows[0]);
        fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
        expect(document.activeElement).toBe(search(kind));
      }
    } finally { pinned.isPinned = false; }
  });

  it('keeps the footer shortcut listener registered while the Command hint re-renders the panel', async () => {
    await mount('clipboard');
    const added = vi.spyOn(window, 'addEventListener');
    const removed = vi.spyOn(window, 'removeEventListener');
    // Holding Command shows the numbered paste hints. WebKit flushes that render between listeners, and a
    // listener removed during a dispatch never runs for the same key, so the footer must not re-register.
    fireEvent.keyDown(document.activeElement!, { key: 'Meta', code: 'MetaLeft', metaKey: true });
    expect(removed.mock.calls.filter(([type, , options]) => type === 'keydown' && options === true)).toHaveLength(0);
    expect(added.mock.calls.filter(([type, , options]) => type === 'keydown' && options === true)).toHaveLength(0);
    fireEvent.keyUp(document.activeElement!, { key: 'Meta', code: 'MetaLeft' });
    fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
    expect(document.activeElement).toBe(search('clipboard'));
  });

  it('waits for an in-flight refresh before focusing the first record on open', async () => {
    await mount('clipboard');
    const focused = (id: string) => panel('clipboard').querySelector(`[data-record-id="${id}"] .history-row-select`);
    await waitFor(() => expect(document.activeElement).toBe(focused('clip-first')));
    await windowState({ historyVisible: false });
    // WebKit drops focus from the hidden view; jsdom keeps it on the old row.
    (document.activeElement as HTMLElement | null)?.blur();
    const original = bridge.invoke.getMockImplementation()!;
    const before = calls('clipboard_query').length;
    let complete!: (value: unknown) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'clipboard_query'
      ? new Promise<unknown>((resolve) => { complete = resolve; })
      : original(name, args));
    clipboardRecords.unshift({ id: 'clip-newest', kind: 'text', title: '', text: 'Newest text', date: '2026-11-16T12:00:00Z', isFavorite: false, isPinned: false, sourceAppId: null });
    try {
      await emit('clipboard_changed', 1);
      await waitFor(() => expect(calls('clipboard_query').length).toBeGreaterThan(before));
      await windowState({ historyVisible: true, recordsFocus: 1 });
      // A copy that arrived while hidden is still being loaded: do not focus the stale first record.
      expect(document.activeElement).not.toBe(focused('clip-first'));
      const page = { records: [...clipboardRecords], sources: {}, totalRecords: clipboardRecords.length, hasMore: false, cursor: null };
      complete({ page, alternate: page });
      await waitFor(() => expect(document.activeElement).toBe(focused('clip-newest')));
    } finally { clipboardRecords.shift(); bridge.invoke.mockImplementation(original); }
  });

  it('returns to the last keyboard record when switching views and opens on the first record when reopened', async () => {
    await mount('clipboard');
    const row = (kind: 'clipboard' | 'translation') => panel(kind).querySelector<HTMLButtonElement>('[data-keyboard-current] .history-row-select');
    const down = () => fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown', code: 'ArrowDown' });
    await waitFor(() => expect(row('clipboard')?.closest('article')?.dataset.recordId).toBe('clip-first'));
    down();
    expect(row('clipboard')?.closest('article')?.dataset.recordId).toBe('clip-favorite');
    await switchView('translation');
    expect(row('translation')?.closest('article')?.dataset.recordId).toBe('sample-03');
    down();
    expect(row('translation')?.closest('article')?.dataset.recordId).toBe('sample-04');
    expect(document.activeElement).toBe(row('translation'));
    await switchView('clipboard');
    expect(document.activeElement).toBe(row('clipboard'));
    expect(row('clipboard')?.closest('article')?.dataset.recordId).toBe('clip-favorite');
    await switchView('translation');
    expect(document.activeElement).toBe(row('translation'));
    expect(row('translation')?.closest('article')?.dataset.recordId).toBe('sample-04');
    await windowState({ historyVisible: false, recordsSession: 1 });
    await windowState({ historyVisible: true, recordsFocus: 1, recordsSession: 2 });
    expect(document.activeElement).toBe(row('translation'));
    expect(row('translation')?.closest('article')?.dataset.recordId).toBe('sample-03');
  });

  it('opens both views on the first pinned record when only pinned records exist', async () => {
    const pinned = [...clipboardRecords, ...bridge.history.records];
    pinned.forEach((record) => { record.isPinned = true; });
    try {
      await mount('clipboard');
      for (const kind of ['clipboard', 'translation'] as const) {
        if (kind === 'translation') await switchView(kind);
        const first = () => panel(kind).querySelector<HTMLButtonElement>('.history-row:not([hidden]) .history-row-select');
        await waitFor(() => expect(document.activeElement).toBe(first()));
        expect(panel(kind).querySelector('.history-unpinned .history-row:not([hidden])')).toBeNull();
        expect(first()!.closest('article')!.classList.contains('pinned')).toBe(true);
        expect(document.activeElement).not.toBe(search(kind));
      }
    } finally { pinned.forEach((record) => { record.isPinned = false; }); }
  });

  it('cycles categories per view and switches record views with Tab', async () => {
    await mount('clipboard');
    const arrow = (key: string) => fireEvent.keyDown(search(bridge.snapshot.window.recordView === 'translation' ? 'translation' : 'clipboard'), { key });
    for (const label of ['Favorite clipboard records', 'Text clipboard records', 'Image clipboard records', 'File clipboard records']) {
      arrow('ArrowRight');
      expect(screen.getByRole('button', { name: label }).getAttribute('aria-pressed')).toBe('true');
    }
    arrow('ArrowRight');
    expect(screen.getByRole('button', { name: 'File clipboard records' }).getAttribute('aria-pressed')).toBe('false');
    arrow('ArrowLeft');
    expect(screen.getByRole('button', { name: 'File clipboard records' }).getAttribute('aria-pressed')).toBe('true');
    arrow('Tab');
    await waitFor(() => expect(bridge.snapshot.window.recordView).toBe('translation'));
    arrow('ArrowLeft');
    expect(screen.getByRole('button', { name: 'Favorite history' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.change(search('translation'), { target: { value: 'query' } });
    expect(arrow('ArrowRight')).toBe(true);
    expect(screen.getByRole('button', { name: 'Favorite history' }).getAttribute('aria-pressed')).toBe('true');
    expect(fireEvent.keyDown(search('translation'), { key: 'Tab', shiftKey: true })).toBe(true);
    arrow('Tab');
    await waitFor(() => expect(bridge.snapshot.window.recordView).toBe('clipboard'));
    const { activation, shortcuts } = bridge.snapshot.clipboardControls;
    await emit('clipboard_controls_changed', { activation, shortcuts: { ...shortcuts, switchView: { ...shortcuts.switchView, enabled: false } } });
    expect(arrow('Tab')).toBe(true);
    expect(bridge.snapshot.window.recordView).toBe('clipboard');
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
    expect(document.activeElement).toBe(translationPane.querySelector('[data-record-id="sample-03"] .history-row-select'));
    const translationRow = translationPane.querySelector<HTMLElement>('[data-record-id="sample-03"]')!;
    expect(icons(translationRow)).toEqual(icons(clipboardRow).filter(([symbol]) => symbol !== 'languages'));
    expect(translationRow.querySelector('[data-action=copy]')).toBeTruthy();
    expect(clipboardRow.querySelector('[data-action=copy]')).toBeTruthy();
    expect(clipboardRow.querySelector('[data-action=translate] .history-symbol')?.getAttribute('data-symbol')).toBe('languages');
    expect(translationRow.querySelector('[data-action=translate]')).toBeNull();
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

  it.each(['clipboard', 'translation'] as const)('routes macOS Copy to the focused %s row only when bound', async (kind) => {
    const shortcuts = bridge.snapshot.clipboardControls.shortcuts;
    shortcuts.copyRecord.binding = { code: 'KeyC', modifiers: ['meta'] };
    await mount('clipboard');
    if (kind === 'translation') await switchView('translation');
    const selected = panel(kind).querySelector<HTMLButtonElement>('.history-row-select')!;
    const input = search(kind);
    const command = kind === 'clipboard' ? 'clipboard_record_use' : 'translation_sample_copy';
    input.focus();
    expect(fireEvent.copy(input)).toBe(true);
    expect(calls(command)).toHaveLength(0);
    selected.focus();
    expect(fireEvent.copy(selected)).toBe(false);
    await waitFor(() => expect(calls(command)).toHaveLength(1));
    expect(calls(command)[0][1]).toMatchObject(kind === 'clipboard'
      ? { recordId: selected.closest('article')?.dataset.recordId, action: 'copy' }
      : { sampleId: selected.closest('article')?.dataset.recordId, part: 'source' });
  });

  it('toggles translation history from the records WebView with the current binding', async () => {
    bridge.snapshot.capabilities.selection = false;
    await mount('clipboard'); await switchView('translation');
    const preferences = bridge.snapshot.translationPreferences;
    const key = { key: '/', code: 'Slash', metaKey: true };
    expect(fireEvent.keyDown(search('translation'), key)).toBe(false);
    expect(calls('history_toggle')).toHaveLength(1);
    await emit('translation_preferences_changed', { ...preferences, disabledShortcuts: ['toggleHistory'] });
    expect(fireEvent.keyDown(search('translation'), key)).toBe(true);
    await emit('translation_preferences_changed', { ...preferences, shortcuts: { ...preferences.shortcuts, toggleHistory: { code: 'KeyJ', modifiers: [] } } });
    expect(fireEvent.keyDown(search('translation'), { key: 'j', code: 'KeyJ' })).toBe(true);
    const row = panel('translation').querySelector('.history-row-select')!;
    expect(fireEvent.keyDown(row, { key: 'j', code: 'KeyJ' })).toBe(false);
    expect(calls('history_toggle')).toHaveLength(2);
  });

  it('previews the focused translation with the shared shortcut and closes on the next press', async () => {
    await mount('clipboard');
    await switchView('translation');
    const row = panel('translation').querySelectorAll<HTMLButtonElement>('.history-row-select')[1];
    row.focus();
    const id = row.closest('article')!.dataset.recordId;
    const press = (target: Element, key = 'y', code = 'KeyY', metaKey = true, repeat = false) => fireEvent.keyDown(target, { key, code, metaKey, repeat });
    press(row);
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
    expect(calls('history_sample_select').at(-1)?.[1]).toEqual({ sampleId: id, retranslate: false });
    press(row, 'y', 'KeyY', true, true);
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    press(row);
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(false));
    const { activation, shortcuts } = bridge.snapshot.clipboardControls;
    await emit('clipboard_controls_changed', { activation, shortcuts: { ...shortcuts, preview: { enabled: true, binding: { code: 'Space', modifiers: [] } } } });
    const input = search('translation');
    input.focus();
    expect(press(input, ' ', 'Space', false)).toBe(true);
    expect(bridge.snapshot.window.translationVisible).toBe(false);
    expect(press(row, ' ', 'Space', false)).toBe(false);
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
  });

  it('uses the last translation target for Space when focus leaves the row, without scrolling', async () => {
    await mount('clipboard'); await switchView('translation');
    const { activation, shortcuts } = bridge.snapshot.clipboardControls;
    await emit('clipboard_controls_changed', { activation, shortcuts: { ...shortcuts, preview: { enabled: true, binding: { code: 'Space', modifiers: [] } } } });
    const history = panel('translation');
    const row = history.querySelectorAll<HTMLButtonElement>('.history-row-select')[1];
    act(() => row.focus());
    await waitFor(() => expect(row.closest('article')?.hasAttribute('data-keyboard-current')).toBe(true));
    row.blur();
    expect(document.activeElement).toBe(document.body);
    const press = (target: Element, repeat = false) => fireEvent.keyDown(target, { key: ' ', code: 'Space', repeat });
    expect(press(document.body)).toBe(false);
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
    expect(calls('history_sample_select').at(-1)?.[1]).toMatchObject({ sampleId: row.closest('article')!.dataset.recordId });
    const scrollbar = history.querySelector('.history-list')!;
    expect(press(scrollbar, true)).toBe(false);
    expect(calls('records_details_toggle')).toHaveLength(0);
    expect(press(scrollbar)).toBe(false);
    expect(calls('records_details_toggle')).toHaveLength(1);
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(false));
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
    const list = pane.querySelector<HTMLElement>('.history-list')!;
    const row = pane.querySelector<HTMLElement>('[data-record-id="sample-03"]')!;
    const focused = row.querySelector<HTMLButtonElement>('.history-row-select')!;
    expect(document.activeElement).toBe(focused);
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 1000 } });
    list.getBoundingClientRect = () => ({ top: 50, bottom: 150 } as DOMRect);
    row.getBoundingClientRect = () => ({ top: 20, bottom: 100 } as DOMRect);
    list.scrollTop = 100;
    await windowState({ translationVisible: true, translationExtent: 920 });
    await emit('history_sample_selected', { sampleId: 'sample-03', retranslate: false });
    expect(list.scrollTop).toBe(66);
    expect(document.activeElement).toBe(focused);
    fireEvent.scroll(list);
    expect(within(pane).getByRole('scrollbar').classList.contains('visible')).toBe(false);
    await switchView('clipboard');
    list.scrollTop = 100;
    await emit('history_sample_selected', { sampleId: 'sample-04', retranslate: false });
    expect(list.scrollTop).toBe(100);
  });

  it('keeps row focus when opening details and moves it to the new row after retranslation', async () => {
    const user = userEvent.setup();
    await mount('translation'); await switchView('translation');
    const row = panel('translation').querySelector<HTMLElement>('[data-record-id="sample-03"]')!;
    const button = row.querySelector<HTMLButtonElement>('.history-row-select')!;
    await user.click(button);
    expect(document.activeElement).toBe(button);
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
    fireEvent.contextMenu(row);
    await user.click(screen.getByRole('menuitem', { name: 'Translate' }));
    await waitFor(() => expect(document.activeElement?.closest('[data-record-id]')?.getAttribute('data-record-id')).toBe('translation-test'));
  });

  it('opens translation details on the first unpinned record and follows hover or focus', async () => {
    await mount('translation');
    await switchView('translation');
    const row = (id: string) => panel('translation').querySelector<HTMLElement>(`[data-record-id="${id}"]`)!;
    const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
    await waitFor(() => expect(row('sample-03')).toBeTruthy());
    fireEvent.mouseMove(row('sample-04'), { clientY: 80 });
    expect(calls('history_sample_select')).toHaveLength(0);
    fireEvent.click(within(panel('translation')).getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(source().value).toBe('The next train leaves in ten minutes.'));
    expect(bridge.history.selectedId).toBe('sample-03');
    expect(row('sample-03').classList.contains('selected')).toBe(true);
    expect(row('sample-01').classList.contains('selected')).toBe(false);
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('下一班列车将在十分钟后出发。');
    fireEvent.mouseMove(row('sample-04'), { clientY: 81 });
    await waitFor(() => expect(source().value).toBe('Please send me the updated design before Friday.'));
    expect(row('sample-04').classList.contains('selected')).toBe(true);
    fireEvent.mouseLeave(row('sample-04'));
    expect(bridge.history.selectedId).toBe('sample-04');
    expect(source().value).toBe('Please send me the updated design before Friday.');
    fireEvent.focus(row('sample-05').querySelector('button')!);
    await waitFor(() => expect(source().value).toContain('A thoughtful tool stays out of the way.'));
    fireEvent.click(within(panel('translation')).getByRole('button', { name: 'Collapse details' }));
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(false));
    const selects = calls('history_sample_select').length;
    fireEvent.mouseMove(row('sample-04'), { clientY: 82 });
    expect(calls('history_sample_select')).toHaveLength(selects);
    fireEvent.click(within(panel('translation')).getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(bridge.history.selectedId).toBe('sample-03'));
    expect(source().value).toBe('The next train leaves in ten minutes.');
  });

  it('keeps open details visible when switching between clipboard and translation records', async () => {
    await mount('translation');
    await switchView('translation');
    expect(bridge.snapshot.window.translationVisible).toBe(false);
    const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
    fireEvent.click(panel('translation').querySelector<HTMLElement>('[data-record-id="sample-04"] button')!);
    await waitFor(() => expect(source().value).toBe('Please send me the updated design before Friday.'));
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    const selects = calls('history_sample_select').length;
    await switchView('clipboard');
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('clip-first'));
    expect(panel('clipboard').querySelector('[data-record-id="clip-first"]')?.classList.contains('selected')).toBe(true);
    expect(calls('history_sample_select')).toHaveLength(selects);
    await switchView('translation');
    expect(bridge.snapshot.window.translationVisible).toBe(true);
    expect(bridge.history.selectedId).toBe('sample-04');
    expect(source().value).toBe('Please send me the updated design before Friday.');
    expect(panel('translation').querySelector('[data-record-id="sample-04"]')?.classList.contains('selected')).toBe(true);
    expect(bridge.snapshot.window.clipboardDetailId).toBe('clip-first');
    expect(within(panel('translation')).getByRole('button', { name: 'Collapse details' })).toBeTruthy();
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
    expect([...actions.querySelectorAll('.history-symbol')].map((icon) => (icon as HTMLElement).dataset.symbol)).toEqual(['doc.on.doc', 'note', 'star', 'pin', 'trash']);
  });

  it('has one ordered toolbar, mutually exclusive modes and centered footer filter groups', async () => {
    await mount('translation');
    const toolbar = panel('clipboard').querySelector<HTMLElement>('.history-toolbar')!;
    expect(within(toolbar).getAllByRole('button').map((button) => button.getAttribute('aria-label'))).toEqual(['Clipboard', 'Translations', 'Pin records', 'Expand details']);
    expect((within(toolbar).getByRole('button', { name: 'Expand details' }) as HTMLButtonElement).disabled).toBe(false);
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

  it.each(['search', 'list'] as const)('restores the %s opening target on explicit clipboard entry without stealing focus during normal updates', async (openingFocus) => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = openingFocus;
    await mount('translation');
    const target = () => openingFocus === 'list'
      ? panel('clipboard').querySelector<HTMLButtonElement>('[data-record-id="clip-first"] .history-row-select')
      : search('clipboard');
    expect(document.activeElement).toBe(target());
    const pin = within(panel('clipboard')).getByRole('button', { name: 'Pin records' }); pin.focus();
    await windowState({ historyWidth: 420 });
    expect(document.activeElement).toBe(pin);
    await windowState({ recordsFocus: 1 });
    expect(document.activeElement).toBe(target());
  });

  it('keeps clipboard text unchanged and delegates clicked records to the configured activation', async () => {
    await mount('translation');
    await waitFor(() => expect(within(panel('clipboard')).getByRole('button', { name: 'Clipboard: Saved text' })).toBeTruthy());
    fireEvent.click(within(panel('clipboard')).getByRole('button', { name: 'Clipboard: Saved text' }));
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'clip-first', action: 'activate' });
    expect(calls('translation_run')).toHaveLength(0);
  });

  it.each(['Cancel', 'Discard', 'Save'] as const)('protects an unsaved note before global navigation: %s', async (choice) => {
    await mount('clipboard');
    const row = await screen.findByRole('button', { name: 'Clipboard: Saved text' });
    fireEvent.contextMenu(row.closest('article')!); fireEvent.click(screen.getByRole('menuitem', { name: 'Note' }));
    const input = await screen.findByRole('textbox', { name: 'Content' });
    fireEvent.change(input, { target: { value: 'Changed content' } });
    await waitFor(() => expect(calls('records_editor_set').at(-1)?.[1]).toEqual({ dirty: true, open: true }));
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

  it('virtualizes deep pages in HistoryList while keeping pinned rows, scroll padding, and scrollToId accurate', () => {
    const listRef = createRef<HTMLDivElement>();
    const pinnedRef = createRef<HTMLDivElement>();
    const handleRef = createRef<HistoryListHandle>();
    const items = Array.from({ length: 240 }, (_, index) => ({ id: `row-${index}`, text: `Deep record ${index}` }));
    const pinned = [<article key="pin-0" data-record-id="pin-0" className="history-row pinned">Pinned 0</article>];
    const renderItem = (item: { id: string; text: string }) => (
      <article key={item.id} data-record-id={item.id} className="history-row">
        <button type="button" className="history-row-select">{item.text}</button>
      </article>
    );
    const view = render(
      <HistoryList
        list={listRef}
        pinnedList={pinnedRef}
        handleRef={handleRef}
        pinned={pinned}
        virtualItems={{ items, itemKey: (item) => item.id, renderItem }}
        onNearEnd={() => {}}
      />,
    );
    const scroller = view.container.querySelector<HTMLElement>('.history-list')!;
    const itemsBox = view.container.querySelector<HTMLElement>('.history-items')!;
    // Viewport height 0 in JSDOM does not virtualize yet.
    expect(itemsBox.dataset.virtualized).toBeUndefined();
    expect(itemsBox.querySelectorAll('.history-row')).toHaveLength(240);

    // Give the scroller a real viewport height and full content height so deep-page virtualization activates.
    Object.defineProperties(scroller, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 240 * 72 },
    });
    fireEvent.scroll(scroller);

    expect(itemsBox.dataset.virtualized).toBe('true');
    const mountedAtTop = itemsBox.querySelectorAll('.history-row').length;
    expect(mountedAtTop).toBeGreaterThan(0);
    expect(mountedAtTop).toBeLessThan(60);
    expect(view.container.querySelector('[data-record-id="pin-0"]')).not.toBeNull();
    expect(view.container.querySelector('[data-record-id="row-0"]')).not.toBeNull();
    expect(view.container.querySelector('[data-record-id="row-200"]')).toBeNull();
    expect(parseFloat(itemsBox.style.paddingTop)).toBe(0);
    expect(parseFloat(itemsBox.style.paddingBottom)).toBeGreaterThan(10000);

    // Scroll deep to row 180: row 180 mounts, row 0 unmounts, paddingTop grows.
    scroller.scrollTop = 180 * 72;
    fireEvent.scroll(scroller);
    expect(itemsBox.querySelectorAll('.history-row').length).toBeLessThan(60);
    expect(view.container.querySelector('[data-record-id="row-180"]')).not.toBeNull();
    expect(view.container.querySelector('[data-record-id="row-0"]')).toBeNull();
    expect(parseFloat(itemsBox.style.paddingTop)).toBeGreaterThan(10000);

    // Programmatic scrollToId jumps to an off-screen row and mounts it into the virtual window.
    act(() => {
      expect(handleRef.current?.scrollToId('row-20')).toBe(true);
    });
    expect(view.container.querySelector('[data-record-id="row-20"]')).not.toBeNull();
    expect(view.container.querySelector('[data-record-id="row-180"]')).toBeNull();
  });

  it('virtualizes deep pages in both Clipboard and HistoryPanel views and jumps to off-screen selection', async () => {
    const deepClipboard = [
      { id: 'clip-pin', kind: 'text', title: '', text: 'Pinned clip', date: '2026-11-15T12:00:00Z', isFavorite: false, isPinned: true, sourceAppId: null },
      ...Array.from({ length: 220 }, (_, i) => ({
        id: `clip-${i}`, kind: 'text', title: '', text: `Deep clip ${i}`, date: '2026-11-15T11:00:00Z', isFavorite: false, isPinned: false, sourceAppId: null,
      })),
    ];
    const deepHistory = [
      { id: 'hist-pin', text: 'Pinned hist', title: '', sourceLanguage: 'en', targetLanguages: ['zh-CN'], translations: ['置顶'], date: '2026-11-15T12:00:00Z', isFavorite: false, isPinned: true, sourceApp: null, targetEngines: ['edge'] },
      ...Array.from({ length: 220 }, (_, i) => ({
        id: `hist-${i}`, text: `Deep hist ${i}`, title: '', sourceLanguage: 'en', targetLanguages: ['zh-CN'], translations: [`译文 ${i}`], date: '2026-11-15T11:00:00Z', isFavorite: false, isPinned: false, sourceApp: null, targetEngines: ['edge'],
      })),
    ];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_query') {
        const page = { records: deepClipboard, sources: {}, totalRecords: deepClipboard.length, hasMore: false, cursor: null };
        return { page, alternate: { records: [], sources: {}, totalRecords: 0, hasMore: false, cursor: null } };
      }
      if (name === 'history_snapshot') {
        return { revision: 10, records: deepHistory, sources: {}, totalRecords: deepHistory.length, hasMore: false, selectedId: null };
      }
      return original(name, args);
    });

    await mount('translation');
    await waitFor(() => expect(panel('clipboard').querySelector('[data-record-id="clip-0"]')).not.toBeNull());
    const clipList = panel('clipboard').querySelector<HTMLElement>('.history-list')!;
    const clipItems = panel('clipboard').querySelector<HTMLElement>('.history-items')!;
    Object.defineProperties(clipList, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 220 * 72 },
    });
    fireEvent.scroll(clipList);
    expect(clipItems.dataset.virtualized).toBe('true');
    expect(clipItems.querySelectorAll('.history-row').length).toBeLessThan(60);
    expect(panel('clipboard').querySelector('[data-record-id="clip-pin"]')).not.toBeNull();
    clipList.scrollTop = 160 * 72;
    fireEvent.scroll(clipList);
    expect(panel('clipboard').querySelector('[data-record-id="clip-160"]')).not.toBeNull();
    expect(panel('clipboard').querySelector('[data-record-id="clip-0"]')).toBeNull();

    await switchView('translation');
    await waitFor(() => expect(panel('translation').querySelector('[data-record-id="hist-0"]')).not.toBeNull());
    const histList = panel('translation').querySelector<HTMLElement>('.history-list')!;
    const histItems = panel('translation').querySelector<HTMLElement>('.history-items')!;
    Object.defineProperties(histList, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 220 * 72 },
    });
    fireEvent.scroll(histList);
    expect(histItems.dataset.virtualized).toBe('true');
    expect(histItems.querySelectorAll('.history-row').length).toBeLessThan(60);
    expect(panel('translation').querySelector('[data-record-id="hist-pin"]')).not.toBeNull();
    histList.scrollTop = 160 * 72;
    fireEvent.scroll(histList);
    expect(panel('translation').querySelector('[data-record-id="hist-160"]')).not.toBeNull();
    expect(panel('translation').querySelector('[data-record-id="hist-0"]')).toBeNull();

    // Selecting an off-screen translation record scrolls it back into the virtual window.
    await windowState({ translationVisible: true, translationExtent: 560 });
    await emit('history_changed', { revision: 11, records: deepHistory, sources: {}, totalRecords: deepHistory.length, hasMore: false, selectedId: 'hist-10' });
    await waitFor(() => expect(panel('translation').querySelector('[data-record-id="hist-10"]')).not.toBeNull());
  });
});
