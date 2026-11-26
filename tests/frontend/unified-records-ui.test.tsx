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
  bridge.snapshot.window.standaloneTranslationVisible = false;
  bridge.snapshot.window.clipboardVisible = true;
  bridge.snapshot.window.clipboardDetailVisible = false;
  bridge.snapshot.window.clipboardTranslationVisible = false;
  bridge.snapshot.window.clipboardExtent = bridge.snapshot.window.clipboardWidth;
  bridge.snapshot.window.rightExtent = 0;
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

const panel = () => document.querySelector<HTMLElement>('.clipboard-panel')!;
const search = () => within(panel()).getByRole('searchbox', { hidden: true }) as HTMLInputElement;

describe('unified records workspace', () => {
  it('pins, toggles shortcut help, and focuses search in the clipboard view', async () => {
    await mount('translation');
    const toolbar = within(panel().querySelector<HTMLElement>('.history-toolbar')!);
    const help = () => within(panel()).queryByRole('dialog', { name: 'HotKey' });
    const key = (code: string, key: string, shiftKey = false, repeat = false) => fireEvent.keyDown(document.activeElement!, { code, key, metaKey: true, shiftKey, repeat });
    toolbar.getByRole('button', { name: 'Pin records' }).focus();
    key('KeyP', 'P', true);
    expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'clipboard', pinned: true });
    await windowState({ clipboardPinned: true });
    key('KeyP', 'P', true);
    expect(calls('window_pin').at(-1)?.[1]).toEqual({ region: 'clipboard', pinned: false });
    await windowState({ clipboardPinned: false });
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
    expect(document.activeElement).toBe(search());
  });

  it('honors modified direction and close keys after remapping a shared shortcut', async () => {
    const preferences = bridge.snapshot.translationPreferences;
    preferences.shortcuts.focusSearch = { code: 'ArrowDown', modifiers: ['meta'] };
    preferences.shortcuts.showShortcuts = { code: 'KeyW', modifiers: ['shift', 'meta'] };
    await mount('translation');
    const row = panel().querySelector<HTMLButtonElement>('.history-row-select')!;
    row.focus();
    fireEvent.keyDown(row, { key: 'ArrowDown', code: 'ArrowDown', metaKey: true });
    expect(document.activeElement).toBe(search());
    fireEvent.keyDown(search(), { key: 'W', code: 'KeyW', metaKey: true, shiftKey: true });
    expect(within(panel()).getByRole('dialog', { name: 'HotKey' })).toBeTruthy();
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('uses saved shared bindings and ignores disabled or composing shortcuts', async () => {
    const preferences = bridge.snapshot.translationPreferences;
    preferences.shortcuts.focusSearch = { code: 'KeyJ', modifiers: ['alt'] };
    preferences.disabledShortcuts = ['showShortcuts'];
    await mount('clipboard');
    const button = within(panel()).getByRole('button', { name: 'HotKey' });
    button.focus();
    fireEvent.keyDown(button, { key: 'k', code: 'KeyK', metaKey: true });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    fireEvent.keyDown(button, { key: 'f', code: 'KeyF', metaKey: true });
    fireEvent.keyDown(button, { key: 'j', code: 'KeyJ', altKey: true, isComposing: true });
    expect(document.activeElement).toBe(button);
    fireEvent.keyDown(button, { key: 'j', code: 'KeyJ', altKey: true });
    expect(document.activeElement).toBe(search());
  });

  it('deletes the targeted clipboard record on the first menu click', async () => {
    await mount('clipboard');
    const target = panel().querySelector('[data-record-id="clip-first"]')!;
    fireEvent.contextMenu(target);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    await waitFor(() => expect(calls('clipboard_record_update')).toHaveLength(1));
    expect(calls('clipboard_record_update')[0][1]).toEqual({ recordId: 'clip-first', action: 'delete' });
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

  it('uses the same outline favorite and active color in the clipboard footer and menu', async () => {
    await mount('clipboard');
    const pane = panel();
    const button = within(pane).getByRole('button', { name: 'Favorite clipboard records' });
    const icon = button.firstElementChild;
    fireEvent.click(button);
    expect(button.firstElementChild).toBe(icon);
    expect(icon?.getAttribute('data-symbol')).toBe('star');
    expect(button.classList.contains('record-favorite')).toBe(true);
    const row = await waitFor(() => {
      const element = pane.querySelector<HTMLElement>('[data-record-id="clip-favorite"]');
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

  it('clips ordinary clipboard records below the pinned region with one active scroll owner', async () => {
    await mount('clipboard');
    const pane = panel();
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

  it('uses standard arrow keys for navigation and category switching without hijacking search typing', async () => {
    await mount('clipboard');
    const input = search();
    input.focus();
    const key = (target: Element, key: string, code = key) => fireEvent.keyDown(target, { key, code });
    const firstRow = panel().querySelector<HTMLButtonElement>('.history-row-select')!;
    firstRow.focus();
    expect(key(firstRow, 'ArrowDown')).toBe(false);
    expect(document.activeElement).not.toBe(firstRow);
    expect(key(document.activeElement!, 'ArrowUp')).toBe(false);
    expect(document.activeElement).toBe(firstRow);
    expect(key(firstRow, 'ArrowUp')).toBe(false);
    expect(document.activeElement).toBe(input);
    const favorite = within(panel()).getByRole('button', { name: 'Favorite clipboard records' });
    expect(favorite.getAttribute('aria-pressed')).toBe('false');
    expect(key(input, 'ArrowRight')).toBe(true);
    expect(favorite.getAttribute('aria-pressed')).toBe('false');
    fireEvent.change(input, { target: { value: 'a' } });
    expect(key(input, 'Tab')).toBe(false);
    expect(favorite.getAttribute('aria-pressed')).toBe('true');
    expect(fireEvent.keyDown(input, { key: 'Tab', code: 'Tab', shiftKey: true })).toBe(false);
    expect(favorite.getAttribute('aria-pressed')).toBe('false');
  });

  it.each(['search', 'list'] as const)('opens clipboard with %s focus and hands off without changing the query', async (openingFocus) => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = openingFocus;
    const { shortcuts } = bridge.snapshot.clipboardControls;
    shortcuts.favorite.binding = { code: 'KeyF', modifiers: [] };
    await mount('clipboard');
    const input = search();
    const row = await waitFor(() => {
      const current = panel().querySelector<HTMLButtonElement>('[data-keyboard-current] .history-row-select');
      expect(current).not.toBeNull();
      return current!;
    });
    expect(document.activeElement).toBe(openingFocus === 'list' ? row : input);
    fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
    expect(document.activeElement).toBe(input);
    expect(fireEvent.keyDown(input, { key: 'f', code: 'KeyF' })).toBe(true);
    expect(document.activeElement).toBe(input);
    fireEvent.change(input, { target: { value: 'Saved' } });
    await waitFor(() => expect(panel().hasAttribute('data-focus-ready')).toBe(true));
    expect(fireEvent.keyDown(input, { key: 'ArrowDown', code: 'ArrowDown' })).toBe(false);
    expect(document.activeElement).toBe(row);
    expect(fireEvent.keyDown(row, { key: 'f', code: 'KeyF' })).toBe(false);
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: row.closest('article')?.dataset.recordId, action: 'favorite' }));
    fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
    expect(document.activeElement).toBe(input);
    expect(input.value).toBe('Saved');
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 'Saved'.length]);
  });

  it.each(['search', 'list'] as const)('keeps one %s keyboard and mouse target in clipboard view', async (openingFocus) => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = openingFocus;
    await mount('clipboard');
    const rows = () => [...panel().querySelectorAll<HTMLElement>('.history-unpinned .history-row:not([hidden])')];
    await waitFor(() => expect(rows().length).toBeGreaterThan(1));
    const [first, second] = rows();
    const button = (row: HTMLElement) => row.querySelector<HTMLButtonElement>('.history-row-select')!;
    const targets = () => [...panel().querySelectorAll('[data-record-target]')].map((row) => (row as HTMLElement).dataset.recordId);
    expect(document.activeElement).toBe(openingFocus === 'list' ? button(first) : search());
    expect(targets()).toEqual(openingFocus === 'list' ? [first.dataset.recordId] : []);
    fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
    expect(targets()).toEqual([second.dataset.recordId]);
    expect(document.activeElement).toBe(openingFocus === 'list' ? button(second) : search());
    if (openingFocus === 'search') fireEvent.keyDown(search(), { key: 'ArrowDown', code: 'ArrowDown' });
    expect(document.activeElement).toBe(button(second));
    fireEvent.keyDown(document.activeElement!, { key: 'c', code: 'KeyC', metaKey: true });
    await waitFor(() => expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: second.dataset.recordId, action: 'copy' }));
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
    expect(document.activeElement).toBe(button(first));
    fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
    expect(targets()).toEqual([first.dataset.recordId]);
    fireEvent.mouseMove(second, { clientX: 11, clientY: 100 });
    expect(targets()).toEqual([second.dataset.recordId]);
  });

  function actionStripRules() {
    const lines = (readFileSync('src/styles.css', 'utf8') as string).split('\n').map((line) => line.trim());
    const rules = lines.filter((line) => line.includes('.history-row-actions {') || /\.history-row:(hover|focus-within) \.history-row-actions/.test(line));
    expect(rules.length).toBeGreaterThanOrEqual(3);
    return rules.join('\n').replaceAll(':hover', '[data-hover]');
  }

  it('shows only the clipboard action strip of the keyboard record while the pointer rests on another', async () => {
    const style = document.createElement('style');
    style.textContent = actionStripRules();
    document.head.append(style);
    try {
      await mount('clipboard');
      const rows = () => [...panel().querySelectorAll<HTMLElement>('.history-unpinned .history-row:not([hidden])')];
      await waitFor(() => expect(rows().length).toBeGreaterThan(1));
      const [first, second] = rows();
      const button = (row: HTMLElement) => row.querySelector('.history-row-select')!;
      const strip = (row: HTMLElement) => getComputedStyle(row.querySelector('.history-row-actions')!);
      const shown = () => rows().filter((row) => strip(row).visibility === 'visible').map((row) => row.dataset.recordId);
      const fade = (row: HTMLElement) => `${strip(row).transitionProperty} ${strip(row).transitionDuration}`;
      await waitFor(() => expect(document.activeElement).toBe(button(first)));
      expect(shown()).toEqual([first.dataset.recordId]);
      expect(fade(second)).toBe('opacity, transform, visibility 0.15s, 0.15s, 0.15s');
      second.dataset.hover = '';
      fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
      expect(shown()).toEqual([second.dataset.recordId]);
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
      expect(document.activeElement).toBe(button(first));
      expect(shown()).toEqual([first.dataset.recordId]);
      expect(fade(second)).toBe('none 0s');
      expect(strip(first).transitionDuration).toBe('0.15s');
      fireEvent.mouseMove(second, { clientX: 10, clientY: 100 });
      expect(shown()).toEqual([first.dataset.recordId]);
      fireEvent.mouseMove(second, { clientX: 11, clientY: 100 });
      expect(shown()).toEqual([second.dataset.recordId]);
      fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
      expect(document.activeElement).toBe(search());
      expect(shown()).toEqual([second.dataset.recordId]);
      expect(fade(second)).toBe('opacity, transform, visibility 0.15s');
      delete second.dataset.hover;
      expect(shown()).toEqual([]);
    } finally { style.remove(); }
  });

  it('falls back to search when record-list focus is preferred but no records exist', async () => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = 'list';
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_query') {
        const empty = { records: [], sources: {}, totalRecords: 0, hasMore: false, cursor: null };
        return { page: empty, alternate: empty };
      }
      return original(name, args);
    });
    await mount('clipboard');
    await waitFor(() => expect(document.activeElement).toBe(search()));
  });

  it('opens clipboard on the first unpinned record by default and moves between it and search', async () => {
    expect(bridge.snapshot.clipboardControls.recordsOpenFocus).toBe('list');
    await mount('clipboard');
    const row = () => panel().querySelector<HTMLButtonElement>('[data-keyboard-current] .history-row-select');
    await waitFor(() => expect(document.activeElement).toBe(row()));
    expect(row()).toBe(panel().querySelector('.history-row:not([hidden]) .history-row-select'));
    fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
    expect(document.activeElement).toBe(search());
    fireEvent.keyDown(search(), { key: 'ArrowDown', code: 'ArrowDown' });
    expect(document.activeElement).toBe(row());
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
    expect(document.activeElement).toBe(search());
  });

  it('opens clipboard on the first unpinned record when pinned records exist', async () => {
    const pinned = clipboardRecords[1];
    pinned.isPinned = true;
    try {
      await mount('clipboard');
      const rows = () => [...panel().querySelectorAll<HTMLButtonElement>('.history-row:not([hidden]) .history-row-select')];
      const unpinned = rows().filter((button) => !button.closest('article')!.classList.contains('pinned'));
      const pinnedRows = rows().filter((button) => button.closest('article')!.classList.contains('pinned'));
      await waitFor(() => expect(document.activeElement).toBe(unpinned[0]));
      expect(pinnedRows.length).toBeGreaterThan(0);
      expect(document.activeElement!.closest('article')!.dataset.recordId).toBe('clip-first');
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
      expect(document.activeElement).toBe(pinnedRows[pinnedRows.length - 1]);
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp', code: 'ArrowUp' });
      expect(document.activeElement).toBe(search());
    } finally { pinned.isPinned = false; }
  });

  it('keeps the footer shortcut listener registered while the Command hint re-renders the panel', async () => {
    await mount('clipboard');
    const added = vi.spyOn(window, 'addEventListener');
    const removed = vi.spyOn(window, 'removeEventListener');
    fireEvent.keyDown(document.activeElement!, { key: 'Meta', code: 'MetaLeft', metaKey: true });
    expect(removed.mock.calls.filter(([type, , options]) => type === 'keydown' && options === true)).toHaveLength(0);
    expect(added.mock.calls.filter(([type, , options]) => type === 'keydown' && options === true)).toHaveLength(0);
    fireEvent.keyUp(document.activeElement!, { key: 'Meta', code: 'MetaLeft' });
    fireEvent.keyDown(document.activeElement!, { key: 'f', code: 'KeyF', metaKey: true });
    expect(document.activeElement).toBe(search());
  });

  it('waits for an in-flight refresh before focusing the first record on open', async () => {
    await mount('clipboard');
    const focused = (id: string) => panel().querySelector(`[data-record-id="${id}"] .history-row-select`);
    await waitFor(() => expect(document.activeElement).toBe(focused('clip-first')));
    await windowState({ clipboardVisible: false });
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
      await windowState({ clipboardVisible: true, recordsFocus: 1 });
      expect(document.activeElement).not.toBe(focused('clip-first'));
      const page = { records: [...clipboardRecords], sources: {}, totalRecords: clipboardRecords.length, hasMore: false, cursor: null };
      complete({ page, alternate: page });
      await waitFor(() => expect(document.activeElement).toBe(focused('clip-newest')));
    } finally { clipboardRecords.shift(); bridge.invoke.mockImplementation(original); }
  });

  it('opens on the first pinned record when only pinned records exist', async () => {
    clipboardRecords.forEach((record) => { record.isPinned = true; });
    try {
      await mount('clipboard');
      const first = () => panel().querySelector<HTMLButtonElement>('.history-row:not([hidden]) .history-row-select');
      await waitFor(() => expect(document.activeElement).toBe(first()));
      expect(panel().querySelector('.history-unpinned .history-row:not([hidden])')).toBeNull();
      expect(first()!.closest('article')!.classList.contains('pinned')).toBe(true);
      expect(document.activeElement).not.toBe(search());
    } finally { clipboardRecords.forEach((record) => { record.isPinned = false; }); }
  });

  it('cycles clipboard categories with tab and shift tab', async () => {
    await mount('clipboard');
    const tab = (shiftKey = false) => fireEvent.keyDown(search(), { key: 'Tab', code: 'Tab', shiftKey });
    const pressed = (label: string) => screen.getByRole('button', { name: label }).getAttribute('aria-pressed');
    expect(pressed('All clipboard records')).toBe('true');
    for (const label of ['Favorite clipboard records', 'Text clipboard records', 'Image clipboard records', 'File clipboard records']) {
      tab();
      expect(pressed(label)).toBe('true');
      expect(pressed('All clipboard records')).toBe('false');
    }
    tab();
    expect(pressed('File clipboard records')).toBe('false');
    expect(pressed('All clipboard records')).toBe('true');
    tab(true);
    expect(pressed('File clipboard records')).toBe('true');
    expect(pressed('All clipboard records')).toBe('false');
  });

  it('has one ordered toolbar without view-switcher buttons and centered footer filter groups', async () => {
    await mount('translation');
    const toolbar = panel().querySelector<HTMLElement>('.history-toolbar')!;
    expect(within(toolbar).getAllByRole('button').map((button) => button.getAttribute('aria-label'))).toEqual(['Pin records', 'Expand details']);
    expect(within(toolbar).queryByRole('button', { name: 'Clipboard' })).toBeNull();
    expect(within(toolbar).queryByRole('button', { name: 'Translations' })).toBeNull();
    const favorite = within(panel()).getByRole('button', { name: 'Favorite clipboard records' });
    expect(favorite.closest('.history-footer')).toBeTruthy();
    expect(toolbar.contains(favorite)).toBe(false);
    fireEvent.click(favorite);
    expect(favorite.getAttribute('aria-pressed')).toBe('true');
    await waitFor(() => expect(within(panel()).getByText('1 record')).toBeTruthy());
  });

  it('resets clipboard search and filters on session change', async () => {
    await mount('translation');
    fireEvent.change(search(), { target: { value: 'text' } });
    fireEvent.click(within(panel()).getByRole('button', { name: 'Favorite clipboard records' }));
    expect(search().value).toBe('text');
    await windowState({ recordsSession: 1 });
    expect(search().value).toBe('');
    expect(within(panel()).getByRole('button', { name: 'Favorite clipboard records', hidden: true }).getAttribute('aria-pressed')).toBe('false');
  });

  it('restores each filter scroll position and resets it when the search changes', async () => {
    await mount('translation');
    const list = panel().querySelector<HTMLElement>('.history-list')!;
    Object.defineProperties(list, { clientHeight: { configurable: true, value: 100 }, scrollHeight: { configurable: true, value: 1000 } });
    list.scrollTop = 260; fireEvent.scroll(list);
    const favorite = within(panel()).getByRole('button', { name: 'Favorite clipboard records' });
    fireEvent.click(favorite);
    expect(list.scrollTop).toBe(0);
    list.scrollTop = 70; fireEvent.scroll(list);
    fireEvent.click(favorite);
    expect(list.scrollTop).toBe(260);
    fireEvent.change(search(), { target: { value: 'Saved' } });
    expect(list.scrollTop).toBe(0);
  });

  it('routes macOS Copy to the focused clipboard row only when bound', async () => {
    const shortcuts = bridge.snapshot.clipboardControls.shortcuts;
    shortcuts.copyRecord.binding = { code: 'KeyC', modifiers: ['meta'] };
    await mount('clipboard');
    const selected = panel().querySelector<HTMLButtonElement>('.history-row-select')!;
    const input = search();
    input.focus();
    expect(fireEvent.copy(input)).toBe(true);
    expect(calls('clipboard_record_use')).toHaveLength(0);
    selected.focus();
    expect(fireEvent.copy(selected)).toBe(false);
    await waitFor(() => expect(calls('clipboard_record_use')).toHaveLength(1));
    expect(calls('clipboard_record_use')[0][1]).toMatchObject({ recordId: selected.closest('article')?.dataset.recordId, action: 'copy' });
  });

  it.each(['search', 'list'] as const)('restores the %s opening target on explicit clipboard entry without stealing focus during normal updates', async (openingFocus) => {
    bridge.snapshot.clipboardControls.recordsOpenFocus = openingFocus;
    await mount('translation');
    const target = () => openingFocus === 'list'
      ? panel().querySelector<HTMLButtonElement>('[data-record-id="clip-first"] .history-row-select')
      : search();
    expect(document.activeElement).toBe(target());
    const pin = within(panel()).getByRole('button', { name: 'Pin records' }); pin.focus();
    await windowState({ clipboardWidth: 420 });
    expect(document.activeElement).toBe(pin);
    await windowState({ recordsFocus: 1 });
    expect(document.activeElement).toBe(target());
  });

  it('keeps clipboard text unchanged and delegates clicked records to the configured activation', async () => {
    await mount('translation');
    await waitFor(() => expect(within(panel()).getByRole('button', { name: 'Clipboard: Saved text' })).toBeTruthy());
    fireEvent.click(within(panel()).getByRole('button', { name: 'Clipboard: Saved text' }));
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
    expect(itemsBox.dataset.virtualized).toBeUndefined();
    expect(itemsBox.querySelectorAll('.history-row')).toHaveLength(240);

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

    scroller.scrollTop = 180 * 72;
    fireEvent.scroll(scroller);
    expect(itemsBox.querySelectorAll('.history-row').length).toBeLessThan(60);
    expect(view.container.querySelector('[data-record-id="row-180"]')).not.toBeNull();
    expect(view.container.querySelector('[data-record-id="row-0"]')).toBeNull();
    expect(parseFloat(itemsBox.style.paddingTop)).toBeGreaterThan(10000);

    act(() => {
      expect(handleRef.current?.scrollToId('row-20')).toBe(true);
    });
    expect(view.container.querySelector('[data-record-id="row-20"]')).not.toBeNull();
    expect(view.container.querySelector('[data-record-id="row-180"]')).toBeNull();
  });

  it('virtualizes deep pages in Clipboard view', async () => {
    const deepClipboard = [
      { id: 'clip-pin', kind: 'text', title: '', text: 'Pinned clip', date: '2026-11-15T12:00:00Z', isFavorite: false, isPinned: true, sourceAppId: null },
      ...Array.from({ length: 220 }, (_, i) => ({
        id: `clip-${i}`, kind: 'text', title: '', text: `Deep clip ${i}`, date: '2026-11-15T11:00:00Z', isFavorite: false, isPinned: false, sourceAppId: null,
      })),
    ];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_query') {
        const page = { records: deepClipboard, sources: {}, totalRecords: deepClipboard.length, hasMore: false, cursor: null };
        return { page, alternate: { records: [], sources: {}, totalRecords: 0, hasMore: false, cursor: null } };
      }
      return original(name, args);
    });

    await mount('translation');
    await waitFor(() => expect(panel().querySelector('[data-record-id="clip-0"]')).not.toBeNull());
    const clipList = panel().querySelector<HTMLElement>('.history-list')!;
    const clipItems = panel().querySelector<HTMLElement>('.history-items')!;
    Object.defineProperties(clipList, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 220 * 72 },
    });
    fireEvent.scroll(clipList);
    expect(clipItems.dataset.virtualized).toBe('true');
    expect(clipItems.querySelectorAll('.history-row').length).toBeLessThan(60);
    expect(panel().querySelector('[data-record-id="clip-pin"]')).not.toBeNull();
    clipList.scrollTop = 160 * 72;
    fireEvent.scroll(clipList);
    expect(panel().querySelector('[data-record-id="clip-160"]')).not.toBeNull();
    expect(panel().querySelector('[data-record-id="clip-0"]')).toBeNull();
  });
});
