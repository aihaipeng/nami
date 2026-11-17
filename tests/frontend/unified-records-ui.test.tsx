// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';
import { bridge, calls, emit, mount, windowState } from './harness';
import { createRef } from 'react';
import { HistoryList } from '../../src/shared/HistoryList';
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
    expect(source().value).toBe('Synthetic image OCR content. Please translate this image.');
    fireEvent.mouseMove(panel('translation').querySelector('[data-record-id="sample-01"]')!);
    expect(source().value).toBe('Synthetic image OCR content. Please translate this image.');
    fireEvent.click(panel('translation').querySelector('[data-record-id="sample-01"] .history-row-select')!);
    await waitFor(() => expect(source().value).toBe(bridge.history.records.find(({ id }) => id === 'sample-01')!.text));
    fireEvent.mouseMove(panel('translation').querySelector('[data-record-id="sample-03"]')!);
    await waitFor(() => expect(source().value).toBe(bridge.history.records.find(({ id }) => id === 'sample-03')!.text));
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

  it('arms deletion on the first click, deletes on the second and requires fresh confirmation afterwards', () => {
    const onUpdate = vi.fn();
    render(<RecordActions recordId="first" t={translator('en')} native favorite={false} pinned={false} canCopy copyLabel="Copy" deleteLabel="Delete" onCopy={vi.fn()} onUpdate={onUpdate} />);
    const button = screen.getByRole('button', { name: 'Delete' });
    expect(button.classList.contains('delete-armed')).toBe(false);
    fireEvent.click(button);
    expect(onUpdate).not.toHaveBeenCalled();
    expect(button.classList.contains('delete-armed')).toBe(true);
    expect(button.getAttribute('aria-description')).toBe('Click again to delete');
    fireEvent.click(button);
    expect(onUpdate).toHaveBeenCalledExactlyOnceWith('delete');
    expect(button.classList.contains('delete-armed')).toBe(false);
    fireEvent.click(button);
    expect(onUpdate).toHaveBeenCalledTimes(1);
  });

  it.each(['outside', 'escape', 'blur', 'window-blur', 'record', 'busy', 'protected'] as const)('cancels delete confirmation on %s', (reset) => {
    const onUpdate = vi.fn();
    const props = { recordId: 'first', t: translator('en'), native: true, favorite: false, pinned: false, canCopy: true, copyLabel: 'Copy', deleteLabel: 'Delete', onCopy: vi.fn(), onUpdate, retainDelete: true };
    const view = render(<RecordActions {...props} />);
    const button = screen.getByRole('button', { name: 'Delete' });
    fireEvent.click(button);
    expect(onUpdate).not.toHaveBeenCalled();
    if (reset === 'outside') fireEvent.pointerDown(document.body);
    if (reset === 'escape') fireEvent.keyDown(button, { key: 'Escape' });
    if (reset === 'blur') fireEvent.blur(button);
    if (reset === 'window-blur') fireEvent(window, new Event('blur'));
    if (reset === 'record') view.rerender(<RecordActions {...props} recordId="second" />);
    if (reset === 'busy') { view.rerender(<RecordActions {...props} busy />); view.rerender(<RecordActions {...props} />); }
    if (reset === 'protected') { view.rerender(<RecordActions {...props} favorite />); view.rerender(<RecordActions {...props} />); }
    expect(button.classList.contains('delete-armed')).toBe(false);
    fireEvent.click(button);
    expect(onUpdate).not.toHaveBeenCalled();
  });

  it.each(['clipboard', 'translation'] as const)('requires two menu clicks before deleting a %s record and resets after reopening', async (kind) => {
    await mount('clipboard');
    if (kind === 'translation') await switchView(kind);
    const id = kind === 'clipboard' ? 'clip-first' : 'sample-03';
    const target = panel(kind).querySelector(`[data-record-id="${id}"]`)!;
    const command = kind === 'clipboard' ? 'clipboard_record_update' : 'history_record_update';
    fireEvent.contextMenu(target);
    const first = screen.getByRole('menuitem', { name: 'Delete' });
    fireEvent.click(first);
    expect(calls(command)).toHaveLength(0);
    expect(first.classList.contains('delete-armed')).toBe(true);
    expect(screen.getByRole('menu')).toBeTruthy();
    fireEvent.keyDown(first, { key: 'Escape' });
    fireEvent.contextMenu(target);
    const next = screen.getByRole('menuitem', { name: 'Delete' });
    expect(next.classList.contains('delete-armed')).toBe(false);
    fireEvent.click(next);
    expect(calls(command)).toHaveLength(0);
    fireEvent.click(next);
    await waitFor(() => expect(calls(command)).toHaveLength(1));
    expect(calls(command)[0][1]).toEqual({ recordId: id, action: 'delete' });
  });

  it('uses the same confirmation for keyboard Enter and Space', async () => {
    const onUpdate = vi.fn();
    const user = userEvent.setup();
    render(<RecordActions recordId="keyboard" t={translator('en')} native favorite={false} pinned={false} canCopy copyLabel="Copy" deleteLabel="Delete" onCopy={vi.fn()} onUpdate={onUpdate} />);
    screen.getByRole('button', { name: 'Delete' }).focus();
    await user.keyboard('{Enter}');
    expect(onUpdate).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Delete' }).classList.contains('delete-armed')).toBe(true);
    await user.keyboard(' ');
    expect(onUpdate).toHaveBeenCalledExactlyOnceWith('delete');
  });

  it.each(['clipboard', 'translation'] as const)('discards menu confirmation when right-clicking another %s record', async (kind) => {
    await mount('clipboard');
    if (kind === 'translation') await switchView(kind);
    const first = panel(kind).querySelector(`[data-record-id="${kind === 'clipboard' ? 'clip-first' : 'sample-03'}"]`)!;
    const second = panel(kind).querySelector(`[data-record-id="${kind === 'clipboard' ? 'clip-favorite' : 'sample-02'}"]`)!;
    fireEvent.contextMenu(first);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    fireEvent.contextMenu(second);
    expect(screen.getByRole('menuitem', { name: 'Delete' }).classList.contains('delete-armed')).toBe(false);
    fireEvent.contextMenu(first);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    expect(calls(kind === 'clipboard' ? 'clipboard_record_update' : 'history_record_update')).toHaveLength(0);
    expect(screen.getByRole('menuitem', { name: 'Delete' }).classList.contains('delete-armed')).toBe(true);
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
    const favorite = screen.getByRole('menuitem', { name: 'Remove favorite' });
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

  it('moves the loading beam at 240 pixels per second across either window width', () => {
    const box = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 400 } as DOMRect);
    const view = render(<ActivityBeam label="Working" />);
    expect(view.container.firstElementChild?.getAttribute('style')).toContain('--beam-duration: 2.1666666666666665s');
    expect(view.container.firstElementChild?.getAttribute('style')).toContain('--beam-distance: 400px');
    view.unmount();
    box.mockReturnValue({ width: 1000 } as DOMRect);
    const wide = render(<ActivityBeam label="Working" />);
    expect(wide.container.firstElementChild?.getAttribute('style')).toContain('--beam-duration: 5.083333333333333s');
    box.mockRestore();
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
    expect(onUpdate).not.toHaveBeenCalled();
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
    expect(icons(translationRow)).toEqual(icons(clipboardRow).filter(([symbol]) => symbol !== 'languages' && symbol !== 'note'));
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
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(true));
    fireEvent.contextMenu(row);
    await user.click(screen.getByRole('menuitem', { name: 'Retranslate' }));
    expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Source text' }));
  });

  it('opens translation details on the first unpinned record and follows hover or focus', async () => {
    await mount('translation');
    await switchView('translation');
    const row = (id: string) => panel('translation').querySelector<HTMLElement>(`[data-record-id="${id}"]`)!;
    const source = () => screen.getByRole('textbox', { name: 'Source text' }) as HTMLTextAreaElement;
    await waitFor(() => expect(row('sample-03')).toBeTruthy());
    fireEvent.mouseMove(row('sample-04'));
    expect(calls('history_sample_select')).toHaveLength(0);
    fireEvent.click(within(panel('translation')).getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(source().value).toBe('The next train leaves in ten minutes.'));
    expect(bridge.history.selectedId).toBe('sample-03');
    expect(row('sample-03').classList.contains('selected')).toBe(true);
    expect(row('sample-01').classList.contains('selected')).toBe(false);
    expect(screen.getByRole('region', { name: 'Primary language' }).textContent).toBe('下一班列车将在十分钟后出发。');
    fireEvent.mouseMove(row('sample-04'));
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
    fireEvent.mouseMove(row('sample-04'));
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
    expect([...actions.querySelectorAll('.history-symbol')].map((icon) => (icon as HTMLElement).dataset.symbol)).toEqual(['doc.on.doc', 'star', 'pin', 'trash']);
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

  it('restores search focus on explicit clipboard entry without stealing it during normal updates', async () => {
    await mount('translation');
    expect(document.activeElement).toBe(search('clipboard'));
    const pin = within(panel('clipboard')).getByRole('button', { name: 'Pin records' }); pin.focus();
    await windowState({ historyWidth: 420 });
    expect(document.activeElement).toBe(pin);
    await windowState({ recordsFocus: 1 });
    expect(document.activeElement).toBe(search('clipboard'));
  });

  it('keeps clipboard text unchanged and preserves click-to-paste', async () => {
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
});
