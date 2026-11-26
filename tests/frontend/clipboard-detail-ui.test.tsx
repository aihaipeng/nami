// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';
import { ClipboardDetail, type ClipboardDetailData } from '../../src/clipboard/ClipboardDetail';
import { listen } from '@tauri-apps/api/event';
import { translator } from '../../src/shared/i18n';

let records: ClipboardDetailData[];
let fetchDetail: ((id: string) => Promise<ClipboardDetailData>) | undefined;
const pane = () => screen.getByRole('region', { name: 'Clipboard details' });
const row = (id: string) => document.querySelector<HTMLElement>(`.clipboard-panel [data-record-id="${id}"]`)!;
const first = () => records.find(({ id }) => id === 'first')!;
beforeEach(() => {
  fetchDetail = undefined;
  records = [
    { id: 'pinned', kind: 'text', title: '', text: 'Pinned text', date: '2026-11-16T10:00:00Z', isFavorite: false, isPinned: true, width: null, height: null, size: null, characterCount: 11, files: [] },
    { id: 'first', kind: 'text', title: '', text: 'Complete first text\n'.repeat(60), date: '2026-11-16T09:00:00Z', isFavorite: false, isPinned: false, sourceApp: { name: 'An application with its complete long name', iconDataUrl: 'data:image/png;base64,abc' }, width: null, height: null, size: null, characterCount: 1200, files: [] },
    { id: 'second', kind: 'text', title: '', text: 'Second text', date: '2026-11-16T08:00:00Z', isFavorite: false, isPinned: false, width: null, height: null, size: null, characterCount: 11, files: [] },
  ];
  bridge.snapshot.capabilities.nativeHistory = false;
  bridge.snapshot.capabilities.nativeChrome = false;
  bridge.snapshot.window.standaloneTranslationVisible = false;
  bridge.snapshot.window.clipboardVisible = true;
  bridge.snapshot.window.clipboardDetailVisible = false;
  bridge.snapshot.window.clipboardExtent = bridge.snapshot.window.clipboardWidth;
  bridge.snapshot.window.rightExtent = 0;
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    const id = (args as { recordId?: string })?.recordId;
    if (name === 'clipboard_query') {
      const page = { records: records.map(({ sourceApp, text, ...record }) => ({ ...record, text: text.slice(0, 120), sourceAppId: sourceApp ? 1 : null })), sources: { 1: first().sourceApp }, totalRecords: records.length, hasMore: false, cursor: null };
      return { page, alternate: { ...page, records: [], totalRecords: 0 } };
    }
    if (name === 'clipboard_record_detail') return fetchDetail ? fetchDetail(id!) : structuredClone(records.find((record) => record.id === id)!);
    if (name === 'clipboard_record_use') return { status: 'copied', copyStatus: 'current', code: null };
    if (name === 'clipboard_record_update') {
      const record = records.find((record) => record.id === id)!;
      const action = (args as { action: string }).action;
      if (action === 'favorite') record.isFavorite = !record.isFavorite;
      if (action === 'pin') record.isPinned = !record.isPinned;
      if (action === 'delete') records = records.filter((record) => record.id !== id);
      bridge.listeners.get('clipboard_changed')?.forEach((callback) => callback({ payload: 1 }));
    }
    return original(name, args);
  });
});
async function open() {
  await mount('translation');
  await waitFor(() => expect(row('first')).toBeTruthy());
  fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
}
function direct() {
  bridge.snapshot.window.clipboardDetailVisible = true;
  bridge.snapshot.window.rightExtent = bridge.snapshot.window.clipboardDetailWidth;
  bridge.snapshot.window.clipboardDetailId = 'first';
  return render(<ClipboardDetail snapshot={bridge.snapshot} native t={translator('en')} command={async (name, args) => { await bridge.invoke(name, args); }} activity={vi.fn()} />);
}

describe('clipboard details', () => {
  it('hides passive index status and keeps indexing and retry actions without extracting or copying', async () => {
    records[1] = { ...first(), kind: 'image', imageIndexStatus: 'unindexed' };
    const invoke = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    bridge.invoke.mockImplementation((name, args) => name === 'clipboard_record_process'
      ? new Promise<void>((resolve) => { finish = resolve; }) : invoke(name, args));
    direct();
    const index = await within(pane()).findByRole('button', { name: 'Index image text' });
    expect(within(pane()).queryByText('Image text not indexed')).toBeNull();
    fireEvent.click(index);
    expect(index.isConnected).toBe(false);
    expect(within(pane()).queryByText('Indexing image text…')).toBeNull();
    expect(calls('clipboard_record_process').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'index' });
    first().imageIndexStatus = 'failed';
    await act(async () => finish());
    await emit('clipboard_changed', 30);
    const retry = await within(pane()).findByRole('button', { name: 'Retry' });
    expect(within(pane()).queryByText('Could not index image text')).toBeNull();
    fireEvent.click(retry);
    first().imageIndexStatus = 'empty';
    await act(async () => finish());
    await emit('clipboard_changed', 31);
    await waitFor(() => expect(within(pane()).queryByRole('button', { name: 'Extract text' })).toBeNull());
    expect(within(pane()).queryByText('No text recognized')).toBeNull();
    first().imageIndexStatus = 'indexed';
    await emit('clipboard_changed', 32);
    expect(await within(pane()).findByRole('button', { name: 'Extract text' })).toBeTruthy();
    expect(within(pane()).queryByText('Image text is searchable')).toBeNull();
    expect(pane().querySelector('.clipboard-image-index-action')).toBeNull();
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });
  it('retains an OCR result during pointer movement and automatic focus until explicit keyboard browsing', async () => {
    await open();
    await windowState({ recordsBrowsing: false });
    const selections = calls('clipboard_details_select').length;
    fireEvent.mouseMove(row('second'), { clientY: 60 });
    fireEvent.focus(row('second').querySelector('button')!);
    expect(calls('clipboard_details_select')).toHaveLength(selections);
    expect(bridge.snapshot.window.clipboardDetailId).toBe('first');
    fireEvent.keyDown(row('first').querySelector('button')!, { key: 'ArrowDown' });
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('second'));
    expect(bridge.snapshot.window.recordsBrowsing).toBe(true);
    fireEvent.mouseMove(row('first'), { clientY: 30 });
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('first'));
  });

  it('keeps OCR image details unchanged on pointer entry and follows actual movement', async () => {
    records[2] = { ...records[2], id: 'image-text', kind: 'image', text: 'Synthetic OCR image' };
    await open();
    const selections = calls('clipboard_details_select').length;
    fireEvent.mouseEnter(row('image-text'));
    expect(calls('clipboard_details_select')).toHaveLength(selections);
    expect(bridge.snapshot.window.clipboardDetailId).toBe('first');
    fireEvent.mouseMove(row('image-text'));
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('image-text'));
    fireEvent.click(await within(pane()).findByRole('button', { name: 'Extract text' }));
    await waitFor(() => expect(calls('clipboard_record_process').at(-1)?.[1]).toEqual({ recordId: 'image-text', action: 'ocr' }));
    fireEvent.focus(row('first').querySelector('button')!);
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('first'));
  });

  it('keeps visual and accessible selection aligned when hover changes the detail', async () => {
    await open();
    fireEvent.focus(row('first').querySelector('button')!);
    fireEvent.mouseMove(row('second'));
    await waitFor(() => expect(row('second').classList.contains('selected')).toBe(true));
    expect(row('second').querySelector('button')?.getAttribute('aria-pressed')).toBe('true');
    expect(row('first').querySelector('button')?.getAttribute('aria-pressed')).toBe('false');
  });

  it('follows pointer movement across records after expanding details with Space and collapses on second Space', async () => {
    const { activation, shortcuts } = bridge.snapshot.clipboardControls;
    bridge.snapshot.clipboardControls = {
      ...bridge.snapshot.clipboardControls,
      activation,
      shortcuts: { ...shortcuts, preview: { enabled: true, binding: { code: 'Space', modifiers: [] } } },
    };
    await mount('translation');
    await waitFor(() => expect(row('first')).toBeTruthy());
    act(() => row('first').querySelector<HTMLButtonElement>('.history-row-select')!.focus());
    fireEvent.keyDown(document.activeElement!, { key: ' ', code: 'Space' });
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
    expect(bridge.snapshot.window.recordsBrowsing).toBe(true);

    fireEvent.mouseMove(row('second'), { clientX: 40, clientY: 80 });
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
    expect(bridge.snapshot.window.clipboardDetailId).toBe('second');

    fireEvent.mouseMove(row('pinned'), { clientX: 40, clientY: 20 });
    await waitFor(() => expect(within(pane()).getByText('Pinned text')).toBeTruthy());
    expect(bridge.snapshot.window.clipboardDetailId).toBe('pinned');

    fireEvent.keyDown(document.activeElement!, { key: ' ', code: 'Space' });
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailVisible).toBe(false));
  });

  it('follows pointer movement even when programmatic row focus does not emit a focus event', async () => {
    await open();
    const firstButton = row('first').querySelector<HTMLButtonElement>('.history-row-select')!;
    const secondButton = row('second').querySelector<HTMLButtonElement>('.history-row-select')!;
    act(() => firstButton.focus());
    const silentFocus = vi.spyOn(secondButton, 'focus').mockImplementation(() => {});
    try {
      fireEvent.mouseMove(row('second'), { clientX: 40, clientY: 90 });
      await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
      expect(bridge.snapshot.window.clipboardDetailId).toBe('second');
    } finally {
      silentFocus.mockRestore();
    }
  });

  it('keeps marks available during processing without releasing its processing guard', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: () => void;
    bridge.invoke.mockImplementation((name,args) => name === 'clipboard_record_process' ? new Promise<void>((resolve)=>{finish=resolve;}) : original(name,args));
    direct();
    const translate = await within(pane()).findByRole('button', {name:'Translate'});
    fireEvent.click(translate);
    expect(translate).toHaveProperty('disabled',true);
    expect(within(pane()).getByRole('button',{name:'Delete clipboard record'})).toHaveProperty('disabled',true);
    const favorite = within(pane()).getByRole('button',{name:'Favorite'});
    expect(favorite).toHaveProperty('disabled',false);
    expect(within(pane()).getByRole('button',{name:'Note'})).toHaveProperty('disabled',false);
    await act(async()=>fireEvent.click(favorite));
    expect(translate).toHaveProperty('disabled',true);
    fireEvent.click(translate);
    expect(calls('clipboard_record_process')).toHaveLength(1);
    await act(async()=>finish());
    expect(translate).toHaveProperty('disabled',false);
  });

  it('copies the open detail without hiding the window', async () => {
    await open();
    const button = await within(pane()).findByRole('button', { name: 'Copy' });
    fireEvent.click(button);
    await waitFor(() => expect(button.querySelector('.copy-success')).toBeTruthy());
    expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'copy' });
    expect(calls('window_hide')).toHaveLength(0);
    expect(bridge.snapshot.window.clipboardDetailVisible).toBe(true);
  });

  it('expands the first unpinned record, keeps full text and source name, and adds Note after text actions', async () => {
    await open();
    expect(bridge.snapshot.window.clipboardDetailId).toBe('first');
    expect(row('first').classList.contains('selected')).toBe(true);
    expect(row('pinned').classList.contains('selected')).toBe(false);
    expect(document.querySelector<HTMLElement>('.translation-panel')?.hidden).toBe(true);
    expect(within(pane()).getByText('An application with its complete long name')).toBeTruthy();
    expect(within(pane()).getByText('1,200')).toBeTruthy();
    expect(within(pane()).queryByText('Dimensions')).toBeNull();
    expect(within(pane()).queryByText('Size')).toBeNull();
    expect([...row('first').querySelectorAll('[data-action]')].map((button) => (button as HTMLElement).dataset.action)).toEqual(['copy', 'translate', 'note', 'favorite', 'pin', 'delete']);
    expect(within(pane()).queryByRole('button', { name: 'Extract text' })).toBeNull();
    expect(within(pane()).getByRole('button', { name: 'Note' }).querySelector('.history-symbol')?.getAttribute('style')).toBe(row('first').querySelector('[data-action=note] .history-symbol')?.getAttribute('style'));
  });

  it('follows hover and keyboard focus without pasting, keeps preview on leave, and preserves the keyboard target on details reopen', async () => {
    await open();
    fireEvent.mouseMove(row('second'));
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
    expect(row('second').classList.contains('selected')).toBe(true);
    fireEvent.mouseLeave(row('second'));
    expect(bridge.snapshot.window.clipboardDetailId).toBe('second');
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.focus(row('pinned').querySelector('button')!);
    await waitFor(() => expect(within(pane()).getByText('Pinned text')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Collapse details' }));
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailVisible).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('pinned'));
  });

  it('drops conditional detail actions immediately while sliding between records', async () => {
    records[2] = { ...records[2], kind: 'files', text: '[]', characterCount: null, size: null, files: [{ path: '/tmp/a.txt', name: 'a.txt', exists: true, isDir: false, isImage: false, iconDataUrl: null }] };
    await open();
    expect(within(pane()).getByRole('button', { name: 'Translate' })).toBeTruthy();
    fireEvent.mouseMove(row('second'));
    await waitFor(() => expect(within(pane()).getByText('a.txt')).toBeTruthy());
    expect(within(pane()).queryByRole('button', { name: 'Translate' })).toBeNull();
    expect(within(pane()).queryByRole('button', { name: 'Show QR Code' })).toBeNull();
    expect(within(pane()).queryByRole('button', { name: 'Show in Finder' })).toBeNull();
    expect(within(pane()).getByRole('button', { name: 'Show in Finder: a.txt' })).toBeTruthy();
    expect(pane().querySelectorAll('[data-action="delete"]')).toHaveLength(1);
    expect(pane().querySelectorAll('.history-action')).toHaveLength(5);
  });

  it('disables every retained detail action while a new record loads and delays the shared activity beam', async () => {
    let resolve!: (value: ClipboardDetailData) => void;
    fetchDetail = (id) => id === 'second' ? new Promise((done) => { resolve = done; }) : Promise.resolve(structuredClone(records.find((record) => record.id === id)!));
    await open();
    const stable = ['Note', 'Favorite', 'Pin record', 'Delete clipboard record'].map((name) => within(pane()).getByRole('button', { name }));
    const viewport = pane().querySelector('.history-viewport');
    fireEvent.mouseMove(row('second'));
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')).toBeNull());
    const beam = within(pane()).getByRole('progressbar', { name: 'Working' });
    expect(beam.getAttribute('data-visible')).toBe('false');
    for (const button of within(pane()).getAllByRole('button')) expect(button).toHaveProperty('disabled', true);
    await waitFor(() => expect(beam.getAttribute('data-visible')).toBe('true'));
    expect(pane().querySelector('.history-viewport')).toBe(viewport);
    expect(stable.map((button) => button.isConnected)).toEqual([true, true, true, true]);
    stable.forEach((button, index) => expect(within(pane()).getByRole('button', { name: ['Note', 'Favorite', 'Pin record', 'Delete clipboard record'][index] })).toBe(button));
    fireEvent.click(stable[0]);
    expect(calls('clipboard_note_open')).toHaveLength(0);
    await act(async () => resolve(structuredClone(records.find((record) => record.id === 'second')!)));
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
    expect(within(pane()).queryByRole('progressbar')).toBeNull();
    expect(within(pane()).getByRole('button', { name: 'Note' })).toHaveProperty('disabled', false);
  });

  it('ignores an old detail response after moving to another record or hiding', async () => {
    let resolve!: (value: ClipboardDetailData) => void;
    fetchDetail = (id) => id === 'first' ? new Promise((done) => { resolve = done; }) : Promise.resolve(records.find((record) => record.id === id)!);
    await mount('translation');
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.mouseMove(row('second'));
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
    await act(async () => resolve(first()));
    expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe('Second text');
    await windowState({ clipboardDetailVisible: false, rightExtent: 0 });
    expect(document.querySelector('.clipboard-detail-content')).toBeNull();
  });

  it('refreshes favorite and pin state from events and routes Note to the existing left editor', async () => {
    await open();
    const remove = within(pane()).getByRole('button', { name: 'Delete clipboard record' });
    fireEvent.click(within(pane()).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(pane()).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    expect(within(pane()).getByRole('button', { name: 'Delete clipboard record' })).toBe(remove);
    expect(remove).toHaveProperty('disabled', false);
    fireEvent.mouseMove(row('pinned'));
    await waitFor(() => expect(within(pane()).getByText('Pinned text')).toBeTruthy());
    expect(within(pane()).getByRole('button', { name: 'Delete clipboard record' })).toHaveProperty('disabled', false);
    fireEvent.click(within(pane()).getByRole('button', { name: 'Note' }));
    expect(await screen.findByRole('dialog', { name: 'Note' })).toBeTruthy();
    await waitFor(() => expect(pane().hasAttribute('inert')).toBe(true));
    expect(calls('clipboard_note_open').at(-1)?.[1]).toEqual({ recordId: 'pinned' });
    expect(screen.getByRole('textbox', { name: 'Content' })).toHaveProperty('value', 'Pinned text');
  });

  it('opens the existing editor when a note request arrives while the list is temporarily hidden', async () => {
    await mount('translation');
    await waitFor(() => expect(row('first')).toBeTruthy());
    await windowState({ clipboardVisible: false, clipboardDetailVisible: true, clipboardDetailId: 'first' });
    await emit('clipboard_note_requested', 'first');
    await windowState({ clipboardVisible: true });
    expect(await screen.findByRole('dialog', { name: 'Note' })).toBeTruthy();
    expect(screen.getByRole('textbox', { name: 'Content' })).toHaveProperty('value', first().text);
  });

  it.each([['image-unknown', 'unindexed', true], ['image-text', 'indexed', true], ['image-blank', 'empty', false]] as const)('uses stored status without probing %s', async (id, imageIndexStatus, text) => {
    records[1] = { ...first(), id, kind: 'image', text: 'Image', imageIndexStatus, width: 2400, height: 1600, characterCount: null, size: 1536 * 1024 };
    bridge.snapshot.window.clipboardDetailId = id;
    bridge.snapshot.window.clipboardDetailVisible = true;
    render(<ClipboardDetail snapshot={bridge.snapshot} native t={translator('en')} command={vi.fn()} activity={vi.fn()} />);
    await waitFor(() => expect(within(pane()).getByText('2400 × 1600')).toBeTruthy());
    await act(async () => {});
    expect(calls('clipboard_record_has_text')).toHaveLength(0);
    expect(calls('clipboard_record_process')).toHaveLength(0);
    expect(!!within(pane()).queryByRole('button', { name: 'Extract text' })).toBe(text);
    expect(!!within(pane()).queryByRole('button', { name: 'Translate' })).toBe(text);
    expect(within(pane()).queryByText('Characters')).toBeNull();
    expect(within(pane()).getByText('Size')).toBeTruthy();
    expect(within(pane()).getByText('1.5 MB')).toBeTruthy();
    const actions = [...pane().querySelectorAll('[data-action]')].map((element) => (element as HTMLElement).dataset.action);
    if (text) expect(actions.indexOf('note')).toBe(actions.indexOf('ocr') + 1);
  });

  it('hides image text actions when recognition finds nothing and shows no empty hint', async () => {
    records[1] = { ...first(), kind: 'image', imageIndexStatus: 'unindexed' };
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => {
      if (name === 'clipboard_record_process') throw 'ocr_no_text';
      return invoke(name, args);
    });
    direct();
    fireEvent.click(await within(pane()).findByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(within(pane()).queryByRole('button', { name: 'Extract text' })).toBeNull());
    expect(within(pane()).queryByRole('button', { name: 'Translate' })).toBeNull();
    expect(within(pane()).queryByText('No text recognized')).toBeNull();
    expect(within(pane()).queryByText('未识别到文字')).toBeNull();
    expect(calls('clipboard_record_process').map(([, args]) => args)).toEqual([{ recordId: 'first', action: 'translate' }]);
    expect(calls('clipboard_record_use')).toHaveLength(0);
  });

  it('shows every file and only the applicable size metadata', async () => {
    records[1] = { ...first(), kind: 'files', text: '[]', characterCount: null, size: 1536 * 1024, files: Array.from({ length: 5 }, (_, i) => ({ path: `/tmp/file-${i}.txt`, name: `file-${i}.txt`, exists: true, isDir: false, isImage: false, iconDataUrl: null })) };
    direct();
    await waitFor(() => expect(pane().querySelectorAll('.clipboard-file')).toHaveLength(5));
    expect(within(pane()).getByText('Size')).toBeTruthy();
    expect(within(pane()).getByText('1.5 MB')).toBeTruthy();
    expect(within(pane()).queryByText('Characters')).toBeNull();
    expect(within(pane()).queryByRole('button', { name: 'Extract text' })).toBeNull();
  });

  it('asks before deleting a pinned detail record and accepts Esc or Enter', async () => {
    first().isFavorite = true;
    direct();
    fireEvent.click(await within(pane()).findByRole('button', { name: 'Delete clipboard record' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete this record?' });
    expect(dialog.textContent).toContain('This record is favorited.');
    expect(within(dialog).getByRole('button', { name: 'Cancel' }).querySelector('kbd')?.textContent).toBe('Esc');
    expect(within(dialog).getByRole('button', { name: 'Delete' }).querySelector('kbd')?.textContent).toBe('↵');
    fireEvent.keyDown(dialog, { key: 'Escape', code: 'Escape' });
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(calls('clipboard_record_update')).toHaveLength(0);
    fireEvent.click(within(pane()).getByRole('button', { name: 'Delete clipboard record' }));
    fireEvent.keyDown(await screen.findByRole('alertdialog'), { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(calls('clipboard_record_update').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'delete' }));
  });

  it.each([
    ['Translate', 'clipboard_record_process', { recordId: 'first', action: 'translate' }],
    ['Pin record', 'clipboard_record_update', { recordId: 'first', action: 'pin' }],
    ['Delete clipboard record', 'clipboard_record_update', { recordId: 'first', action: 'delete' }],
  ] as const)('executes the detail %s action for the displayed complete record', async (label, name, args) => {
    direct();
    const button = await within(pane()).findByRole('button', { name: label });
    fireEvent.click(button);
    await waitFor(() => expect(calls(name).at(-1)?.[1]).toEqual(args));
  });

  it('shows the detail scrollbar on keyboard focus or scroll, then hides after idle', async () => {
    direct();
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
    const scroller = pane().querySelector('.history-list')!;
    Object.defineProperties(scroller, { clientHeight: { value: 100, configurable: true }, scrollHeight: { value: 1000, configurable: true } });
    fireEvent.scroll(scroller);
    const bar = within(pane()).getByRole('scrollbar');
    expect(bar.classList.contains('visible')).toBe(false);
    fireEvent.pointerMove(bar);
    expect(bar.classList.contains('visible')).toBe(false);
    fireEvent.focus(bar);
    expect(bar.classList.contains('visible')).toBe(true);
    fireEvent.blur(bar);
    await waitFor(() => expect(bar.classList.contains('visible')).toBe(false), { timeout: 1600 });
    fireEvent.wheel(scroller);
    expect(bar.classList.contains('visible')).toBe(true);
    await waitFor(() => expect(bar.classList.contains('visible')).toBe(false), { timeout: 1600 });
  });

  it('offers Retry after a transient detail error and recovers the selected record', async () => {
    fetchDetail = async () => { throw 'storage_worker'; };
    direct();
    expect(await within(pane()).findByRole('alert')).toHaveProperty('textContent', 'Unable to load this record. Try again.Retry');
    fetchDetail = async () => structuredClone(first());
    fireEvent.click(within(pane()).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
    expect(within(pane()).queryByRole('alert')).toBeNull();
    expect(within(pane()).queryByRole('progressbar')).toBeNull();
  });

  it('rejects a late Retry response after changing the selected record', async () => {
    await open();
    fetchDetail = async () => { throw new Error('storage_worker'); };
    await emit('clipboard_changed', 2);
    const retry = await within(pane()).findByRole('button', { name: 'Retry' });
    let resolve!: (value: ClipboardDetailData) => void;
    fetchDetail = (id) => id === 'first' ? new Promise((done) => { resolve = done; }) : Promise.resolve(structuredClone(records.find((record) => record.id === id)!));
    fireEvent.click(retry);
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.mouseMove(row('second'));
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
    await act(async () => resolve(structuredClone(first())));
    expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe('Second text');
  });

  it('reestablishes a failed detail subscription on Retry', async () => {
    const subscribe = vi.mocked(listen).getMockImplementation()!;
    let fail = true;
    vi.mocked(listen).mockImplementation((event, handler, options) => {
      if (event === 'clipboard_changed' && fail) {
        fail = false;
        return Promise.reject(new Error('subscription_failed'));
      }
      return subscribe(event, handler, options);
    });
    direct();
    fireEvent.click(await within(pane()).findByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
    expect(bridge.listeners.get('clipboard_changed')?.size).toBe(1);
  });

  it.each([new Error('not_found'), 'not_found'])('shows unavailable instead of stale content after deletion (%s), and handles Escape', async (error) => {
    direct();
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
    fetchDetail = async () => { throw error; };
    await emit('clipboard_changed', 2);
    expect(await within(pane()).findByRole('alert')).toHaveProperty('textContent', 'This record is unavailable.');
    expect(pane().querySelector('.clipboard-detail-content')).toBeNull();
    expect(within(pane()).queryByRole('button', { name: 'Retry' })).toBeNull();
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(calls('window_hide').at(-1)?.[1]).toEqual({ region: 'clipboard' });
  });
});

it('reveals only the chosen file and reports when it disappears after loading details', async () => {
  Object.assign(first(), { kind: 'files', files: ['/tmp/a.txt', '/tmp/b.txt'].map((path) => ({
    path, name: path.slice(5), exists: true, isDir: false, isImage: false, iconDataUrl: null,
  })) });
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name, args) => {
    if (name === 'clipboard_open_target') throw 'clipboard_file_missing';
    return original(name, args);
  });
  direct();
  fireEvent.click(await within(pane()).findByRole('button', { name: 'Show in Finder: b.txt' }));
  expect(calls('clipboard_open_target').at(-1)?.[1]).toEqual({ recordId: 'first', execute: true, fileIndex: 1 });
  expect(await within(pane()).findByText('The original file was moved or deleted and cannot be located')).toBeTruthy();
});

it('renders L1 visual cues in the detail pane without altering exact textContent', async () => {
  first().text = '  #0ea5e9  ';
  direct();
  await waitFor(() => expect(pane().querySelector<HTMLElement>('.clipboard-detail-text')?.dataset.contentKind).toBe('color'));
  const textNode = pane().querySelector<HTMLElement>('.clipboard-detail-text')!;
  expect(textNode.textContent).toBe('  #0ea5e9  ');
  const swatch = textNode.querySelector<HTMLElement>('.clipboard-color-swatch')!;
  expect(swatch.style.getPropertyValue('--swatch-color')).toBe('#0ea5e9');
  expect(swatch.getAttribute('aria-hidden')).toBe('true');

  first().text = 'const sum = (a: number, b: number) => a + b;';
  await emit('clipboard_changed', 2);
  await waitFor(() => expect(pane().querySelector<HTMLElement>('.clipboard-detail-text')?.dataset.contentKind).toBe('code'));
  expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text);
});

it('toggles details closed with the default preview key when focus is inside the detail pane', async () => {
  await open();
  const note = await within(pane()).findByRole('button', { name: 'Note' });
  act(() => note.focus());
  expect(fireEvent.keyDown(note, { key: ' ', code: 'Space' })).toBe(false);
  await waitFor(() => expect(screen.queryByRole('region', { name: 'Clipboard details' })).toBeNull());
  expect(document.activeElement).toBe(row('first').querySelector('.history-row-select'));
});

it('renders sanitized rich text preview by default, blocks script/external media/link navigation, and toggles to plain text', async () => {
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_record_detail') {
      const { recordId } = args as { recordId: string };
      const record = first();
      if (recordId === record.id) {
        return {
          ...record,
          width: null,
          height: null,
          size: null,
          characterCount: record.text.length,
          richTextFormat: 'html',
          richTextHtml: '<p>Hello <strong>Rich</strong></p><script>window.__xss=1</script><img src="https://evil.example/a.png"><a href="https://example.com">Docs</a><a href="javascript:alert(1)">Bad</a>',
          files: [],
        };
      }
    }
    return baseInvoke(name, args);
  });

  direct();
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-rich')).toBeTruthy());
  const rich = pane().querySelector<HTMLElement>('.clipboard-detail-rich')!;
  expect(rich.querySelector('strong')?.textContent).toBe('Rich');
  expect(rich.querySelector('script')).toBeNull();
  expect(rich.querySelector('img')).toBeNull();
  const links = [...rich.querySelectorAll('a')];
  expect(links[0].getAttribute('href')).toBe('https://example.com');
  expect(links[1].getAttribute('href')).toBeNull();
  expect(fireEvent.click(links[0])).toBe(false);
  expect(pane().querySelector('.clipboard-detail-metadata')?.textContent).toContain('Rich Text (HTML)');

  const modeGroup = within(pane()).getByRole('group', { name: 'Preview Format' });
  fireEvent.click(within(modeGroup).getByRole('button', { name: 'Plain' }));
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
  expect(pane().querySelector('.clipboard-detail-rich')).toBeNull();

  fireEvent.click(within(modeGroup).getByRole('button', { name: 'Rich' }));
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-rich')).toBeTruthy());
});

it('displays exact Rich Text subtype labels (HTML, RTF, HTML + RTF) in the detail metadata footer', async () => {
  let format: 'html' | 'rtf' | 'html+rtf' | undefined = 'html';
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_record_detail') {
      const record = first();
      return {
        ...record,
        richTextFormat: format,
        richTextHtml: format ? '<p>Styled</p>' : undefined,
        files: [],
      };
    }
    return baseInvoke(name, args);
  });

  direct();
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-metadata')?.textContent).toContain('Rich Text (HTML)'));

  format = 'rtf';
  await emit('clipboard_changed', 2);
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-metadata')?.textContent).toContain('Rich Text (RTF)'));

  format = 'html+rtf';
  await emit('clipboard_changed', 3);
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-metadata')?.textContent).toContain('Rich Text (HTML + RTF)'));

  format = undefined;
  await emit('clipboard_changed', 4);
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-metadata')?.textContent).not.toContain('Format'));
});

it('renders L2 color, JSON, timestamp, and QR code views in details without modifying the stored record', async () => {
  // Color card + one-click HEX/RGB/HSL copy using shared icon-button and record-copied feedback
  first().text = '#0ea5e9';
  direct();
  const copyRgb = await within(pane()).findByRole('button', { name: 'Copy RGB' });
  expect(pane().querySelector('.clipboard-smart-color')).toBeTruthy();
  expect(copyRgb.classList.contains('icon-button')).toBe(true);
  expect(copyRgb.querySelector('.history-symbol[data-symbol="doc.on.doc"]')).toBeTruthy();
  fireEvent.click(copyRgb);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: 'rgb(14, 165, 233)' }));
  await waitFor(() => expect(copyRgb.classList.contains('record-copied')).toBe(true));
  expect(copyRgb.querySelector('.copy-success')).toBeTruthy();

  // JSON format / minify / copy
  first().text = '{"b":2,"a":1}';
  await emit('clipboard_changed', 2);
  const formatBtn = await within(pane()).findByRole('button', { name: 'Format' });
  fireEvent.click(formatBtn);
  expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe('{\n  "b": 2,\n  "a": 1\n}');
  const copyJson = within(pane()).getByRole('button', { name: 'Copy JSON' });
  expect(copyJson.classList.contains('icon-button')).toBe(true);
  fireEvent.click(copyJson);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '{\n  "b": 2,\n  "a": 1\n}' }));
  expect(first().text).toBe('{"b":2,"a":1}');

  // Timestamp local + ISO copy + relative time
  first().text = '1700000000';
  await emit('clipboard_changed', 3);
  const copyIso = await within(pane()).findByRole('button', { name: 'Copy ISO' });
  expect(within(pane()).getByText('Relative')).toBeTruthy();
  fireEvent.click(copyIso);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '2023-11-14T22:13:20.000Z' }));

  // Local QR generator toggle integrated into RecordActions
  first().text = 'https://example.test/docs';
  await emit('clipboard_changed', 4);
  const qrToggle = await within(pane()).findByRole('button', { name: 'Show QR Code' });
  expect(qrToggle.closest('.history-row-actions')).toBeTruthy();
  expect(qrToggle.classList.contains('icon-button')).toBe(true);
  fireEvent.click(qrToggle);
  expect(pane().dataset.qrView).toBe('true');
  expect(within(pane()).getByRole('img', { name: 'QR Code' })).toBeTruthy();
  const hideQr = within(pane()).getByRole('button', { name: 'Hide QR Code' });
  expect(hideQr.classList.contains('record-active')).toBe(true);
  fireEvent.click(hideQr);
  expect(pane().dataset.qrView).toBeUndefined();
  expect(within(pane()).queryByRole('img', { name: 'QR Code' })).toBeNull();

  // Ordinary long plain text shows no extra smart cards or QR toggle
  first().text = 'Plain text without structured data. '.repeat(12);
  await emit('clipboard_changed', 5);
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
  expect(pane().querySelector('.clipboard-smart-card')).toBeNull();
  expect(pane().querySelector('.clipboard-smart-bar')).toBeNull();
  expect(within(pane()).queryByRole('button', { name: 'Show QR Code' })).toBeNull();
});

it('displays locally detected QR codes on images and opens links only on explicit click', async () => {
  records[1] = { ...first(), id: 'qr-img', kind: 'image', text: 'Image', imageIndexStatus: 'indexed', width: 400, height: 400, characterCount: null, size: 12_000 };
  bridge.snapshot.window.clipboardDetailId = 'qr-img';
  bridge.snapshot.window.clipboardDetailVisible = true;
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_image_qr_codes') return ['https://example.test/from-qr', 'plain-qr-payload'];
    if (name === 'clipboard_text_copy' || name === 'clipboard_open_url') return null;
    return baseInvoke(name, args);
  });
  render(<ClipboardDetail snapshot={bridge.snapshot} native t={translator('en')} command={vi.fn()} activity={vi.fn()} />);
  expect(await within(pane()).findByText('https://example.test/from-qr')).toBeTruthy();
  expect(within(pane()).getByText('plain-qr-payload')).toBeTruthy();
  expect(calls('clipboard_open_url')).toHaveLength(0);
  const openButtons = within(pane()).getAllByRole('button', { name: 'Open Link' });
  expect(openButtons).toHaveLength(1);
  fireEvent.click(openButtons[0]);
  await waitFor(() => expect(calls('clipboard_open_url').at(-1)?.[1]).toEqual({ url: 'https://example.test/from-qr' }));
  fireEvent.click(within(pane()).getAllByRole('button', { name: 'Copy QR Code' })[1]);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: 'plain-qr-payload' }));
});

it('shows detail toolbar open-target actions for text links and paths while local files reveal inside their cards', async () => {
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_open_target') {
      const req = args as { execute: boolean; fileIndex?: number };
      if (!req.execute) {
        if (first().text.startsWith('https://')) return 'link';
        if (first().text.startsWith('/tmp/')) return 'file';
        return null;
      }
      return 'file';
    }
    return baseInvoke(name, args);
  });

  // Single existing local file reveals inside its unified card and does not show a toolbar open button.
  Object.assign(first(), {
    kind: 'files',
    text: '["/tmp/photo.png"]',
    files: [{ path: '/tmp/photo.png', name: 'photo.png', exists: true, isDir: false, isImage: true, iconDataUrl: null }],
  });
  direct();
  const revealFileBtn = await within(pane()).findByRole('button', { name: 'Show in Finder: photo.png' });
  expect(revealFileBtn.closest('.history-row-actions')).toBeNull();
  expect(within(pane()).queryByRole('button', { name: 'Show in Finder' })).toBeNull();
  expect(revealFileBtn.classList.contains('icon-button')).toBe(true);
  fireEvent.click(revealFileBtn);
  await waitFor(() => expect(calls('clipboard_open_target').at(-1)?.[1]).toEqual({ recordId: 'first', execute: true, fileIndex: 0 }));

  // Missing single file disables its card reveal button and still shows no toolbar open button.
  Object.assign(first(), {
    files: [{ path: '/tmp/missing.png', name: 'missing.png', exists: false, isDir: false, isImage: true, iconDataUrl: null }],
  });
  await emit('clipboard_changed', 2);
  const missingRevealBtn = await within(pane()).findByRole('button', { name: 'Show in Finder: missing.png' });
  expect((missingRevealBtn as HTMLButtonElement).disabled).toBe(true);
  expect(within(pane()).queryByRole('button', { name: 'Show in Finder' })).toBeNull();

  // Openable text link shows Open Link in RecordActions.
  Object.assign(first(), { kind: 'text', text: 'https://example.test/open', files: [] });
  await emit('clipboard_changed', 3);
  const openLinkBtn = await within(pane()).findByRole('button', { name: 'Open Link' });
  expect(openLinkBtn.closest('.history-row-actions')).toBeTruthy();
  fireEvent.click(openLinkBtn);
  await waitFor(() => expect(calls('clipboard_open_target').at(-1)?.[1]).toEqual({ recordId: 'first', execute: true, fileIndex: undefined }));

  // Openable plain-text absolute path still shows Show in Finder in RecordActions.
  Object.assign(first(), { kind: 'text', text: '/tmp/plain-path.txt', files: [] });
  await emit('clipboard_changed', 4);
  const openPathBtn = await within(pane()).findByRole('button', { name: 'Show in Finder' });
  expect(openPathBtn.closest('.history-row-actions')).toBeTruthy();
});

it('renders URL, DateTime, JWT, and decoded text smart cards with consistent icon copy actions', async () => {
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_text_copy') return null;
    return baseInvoke(name, args);
  });

  // URL card: Clean URL + Decoded URL + query param copy
  first().text = 'https://example.test/wiki/%E4%B8%AD%E6%96%87?id=42&utm_source=share&q=%E6%B5%8B%E8%AF%95';
  direct();
  const copyClean = await within(pane()).findByRole('button', { name: 'Copy Clean URL' });
  expect(copyClean.classList.contains('icon-button')).toBe(true);
  fireEvent.click(copyClean);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({
    text: 'https://example.test/wiki/%E4%B8%AD%E6%96%87?id=42&q=%E6%B5%8B%E8%AF%95',
  }));
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Decoded' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({
    text: 'https://example.test/wiki/中文?id=42&utm_source=share&q=测试',
  }));
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy q' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '测试' }));
  const paramsList = pane().querySelector('.clipboard-smart-params');
  expect(paramsList).toBeTruthy();
  expect(getComputedStyle(paramsList!).overflowY).not.toBe('auto');

  // DateTime card: ISO UTC -> Local, Unix (s), Unix (ms), Relative
  first().text = '2026-11-25T00:22:55Z';
  await emit('clipboard_changed', 2);
  const copyUnixSec = await within(pane()).findByRole('button', { name: 'Copy Unix (s)' });
  expect(within(pane()).getByText('Relative')).toBeTruthy();
  fireEvent.click(copyUnixSec);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: String(Date.UTC(2026, 10, 25, 0, 22, 55) / 1000) }));

  // JWT card: status badge, Payload/Header toggle, copy active JSON
  const b64u = (obj: object) => {
    const bytes = new TextEncoder().encode(JSON.stringify(obj));
    let bin = '';
    for (const byte of bytes) bin += String.fromCharCode(byte);
    return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
  };
  first().text = `${b64u({ alg: 'HS256', typ: 'JWT' })}.${b64u({ sub: 'u_1', exp: 1600000000 })}.signature_part`;
  await emit('clipboard_changed', 3);
  expect(await within(pane()).findByText('Expired')).toBeTruthy();
  expect(within(pane()).getByText('HS256')).toBeTruthy();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Header' }));
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Header' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({
    text: '{\n  "alg": "HS256",\n  "typ": "JWT"\n}',
  }));

  // Base64 JSON decoded card: defaults to formatted JSON and toggles to original
  first().text = 'eyJ1c2VyIjoibmFtaSIsInJvbGUiOiJhZG1pbiJ9';
  await emit('clipboard_changed', 4);
  expect(await within(pane()).findByText('Base64')).toBeTruthy();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Base64' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({
    text: '{\n  "user": "nami",\n  "role": "admin"\n}',
  }));
  fireEvent.click(within(pane()).getByRole('button', { name: 'Original' }));
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Base64' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({
    text: '{"user":"nami","role":"admin"}',
  }));

  // Unicode escape decoded card
  first().text = '\\u4f60\\u597d\\u4e16\\u754c';
  await emit('clipboard_changed', 5);
  expect(await within(pane()).findByText('Unescaped')).toBeTruthy();
  expect(within(pane()).getByText('你好世界')).toBeTruthy();
  // Smart cards rely on the outer detail HistoryList scrollbar rather than nested scroll containers.
  const codeBlock = pane().querySelector('.clipboard-smart-code');
  expect(codeBlock).toBeTruthy();
  expect(getComputedStyle(codeBlock!).overflowY).not.toBe('auto');
});

it('shows unified file+path cards for single files, local images, and multi-file groups without toolbar file reveal buttons', async () => {
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name, args) => {
    if (name === 'clipboard_text_copy') return true;
    if (name === 'clipboard_open_target') return 'file';
    if (name === 'clipboard_image_qr_codes') return [];
    if (name === 'clipboard_image') return 'data:image/png;base64,iVBORw0KGgo=';
    return original(name, args);
  });

  // Single regular file: unified file+path card with reveal on file row and copy on path row; no top toolbar open button.
  const record = first();
  record.kind = 'files';
  record.text = JSON.stringify(['/Users/apple/Downloads/report.pdf']);
  record.files = [{ path: '/Users/apple/Downloads/report.pdf', name: 'report.pdf', exists: true, isDir: false, isImage: false, iconDataUrl: null }];
  direct();
  const copyReport = await within(pane()).findByRole('button', { name: 'Copy report.pdf' });
  expect(within(pane()).queryByRole('button', { name: 'Show in Finder' })).toBeNull();
  expect(pane().querySelector('.clipboard-files')).toBeNull();
  const singleBlocks = pane().querySelectorAll('.clipboard-smart-file-item');
  expect(singleBlocks).toHaveLength(1);
  fireEvent.click(within(singleBlocks[0] as HTMLElement).getByRole('button', { name: 'Show in Finder: report.pdf' }));
  await waitFor(() => expect(calls('clipboard_open_target').at(-1)?.[1]).toEqual({ recordId: 'first', execute: true, fileIndex: 0 }));
  fireEvent.click(copyReport);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '/Users/apple/Downloads/report.pdf' }));

  // Local single image (kind === 'image' with original local file entry): keeps ImagePreview + compact Path card with Open left of Copy; no top toolbar open button.
  record.kind = 'image';
  record.text = '';
  record.files = [{ path: '/Users/apple/Pictures/photo.png', name: 'photo.png', exists: true, isDir: false, isImage: true, iconDataUrl: null }];
  await emit('clipboard_changed', 2);
  const revealImgBtn = await within(pane()).findByRole('button', { name: 'Show in Finder: photo.png' });
  const copyImgBtn = within(pane()).getByRole('button', { name: 'Copy photo.png' });
  expect(pane().querySelector('img')).toBeTruthy();
  expect(within(pane()).getByText('Path')).toBeTruthy();
  expect(within(pane()).queryByRole('button', { name: 'Show in Finder' })).toBeNull();
  expect(revealImgBtn.compareDocumentPosition(copyImgBtn) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  fireEvent.click(revealImgBtn);
  await waitFor(() => expect(calls('clipboard_open_target').at(-1)?.[1]).toEqual({ recordId: 'first', execute: true, fileIndex: 0 }));
  fireEvent.click(copyImgBtn);
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '/Users/apple/Pictures/photo.png' }));

  // Missing local image file disables card reveal button but keeps path copyable.
  record.files = [{ path: '/Users/apple/Pictures/photo.png', name: 'photo.png', exists: false, isDir: false, isImage: true, iconDataUrl: null }];
  await emit('clipboard_changed', 3);
  const missingImgBtn = await within(pane()).findByRole('button', { name: 'Show in Finder: photo.png' });
  expect((missingImgBtn as HTMLButtonElement).disabled).toBe(true);
  expect(within(pane()).getByRole('button', { name: 'Copy photo.png' })).toBeTruthy();

  // Pure pixel screenshot (kind === 'image' with empty files): no file card, no Show in Finder.
  record.files = [];
  await emit('clipboard_changed', 4);
  await waitFor(() => expect(pane().querySelector('.clipboard-smart-files')).toBeNull());
  expect(within(pane()).queryByRole('button', { name: 'Show in Finder' })).toBeNull();

  // Multiple files: each file+path is a self-contained block with per-file reveal and copy buttons, and no Folder row.
  record.kind = 'files';
  record.text = JSON.stringify(['/Users/apple/Desktop/a.txt', '/Users/apple/Desktop/b.txt']);
  record.files = [
    { path: '/Users/apple/Desktop/a.txt', name: 'a.txt', exists: true, isDir: false, isImage: false, iconDataUrl: null },
    { path: '/Users/apple/Desktop/b.txt', name: 'b.txt', exists: false, isDir: false, isImage: false, iconDataUrl: null },
  ];
  await emit('clipboard_changed', 5);
  await within(pane()).findByRole('button', { name: 'Copy b.txt' });
  expect(within(pane()).queryByText('Folder')).toBeNull();
  expect(pane().querySelector('.clipboard-files')).toBeNull();
  const blocks = pane().querySelectorAll('.clipboard-smart-file-item');
  expect(blocks).toHaveLength(2);
  const revealA = within(blocks[0] as HTMLElement).getByRole('button', { name: 'Show in Finder: a.txt' });
  expect((revealA as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(revealA);
  await waitFor(() => expect(calls('clipboard_open_target').at(-1)?.[1]).toEqual({ recordId: 'first', execute: true, fileIndex: 0 }));
  const revealB = within(blocks[1] as HTMLElement).getByRole('button', { name: 'Show in Finder: b.txt' });
  expect((revealB as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(within(blocks[1] as HTMLElement).getByRole('button', { name: 'Copy b.txt' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '/Users/apple/Desktop/b.txt' }));
});

it('renders inline SVG/data-URI preview toggle, Cron schedule, safe math, and radix/chmod cards', async () => {
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_text_copy') return true;
    return baseInvoke(name, args);
  });

  // Inline SVG: defaults to Preview rendered inside <img>, toggles to Source
  const svgSource = '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/></svg>';
  first().text = svgSource;
  direct();
  const svgImg = await within(pane()).findByRole('img', { name: 'SVG Preview' }) as HTMLImageElement;
  expect(svgImg.src.startsWith('data:image/svg+xml;charset=utf-8,')).toBe(true);
  expect(pane().querySelector('.clipboard-smart-meta')?.textContent).toContain('SVG ·');
  expect(pane().querySelector('.clipboard-detail-text')).toBeNull();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Source' }));
  expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(svgSource);
  expect(within(pane()).queryByRole('img', { name: 'SVG Preview' })).toBeNull();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Preview' }));
  const reloadedImg = await within(pane()).findByRole('img', { name: 'SVG Preview' });
  fireEvent.error(reloadedImg);
  expect(await within(pane()).findByText('Unable to preview this image data')).toBeTruthy();

  // data:image/png;base64,... URI preview
  first().text = 'data:image/png;base64,iVBORw0KGgo=';
  await emit('clipboard_changed', 2);
  const pngImg = await within(pane()).findByRole('img', { name: 'PNG Preview' }) as HTMLImageElement;
  expect(pngImg.src).toBe('data:image/png;base64,iVBORw0KGgo=');
  expect(pane().querySelector('.clipboard-smart-meta')?.textContent).toBe('PNG · 8 B');

  // Cron expression: natural description + 3 future runs with one-click copy
  first().text = '*/15 9-18 * * 1-5';
  await emit('clipboard_changed', 3);
  expect(await within(pane()).findByText('Every 15 minutes, 09:00–18:59, Mon–Fri')).toBeTruthy();
  expect(within(pane()).getByText('Next 1')).toBeTruthy();
  expect(within(pane()).getByText('Next 2')).toBeTruthy();
  expect(within(pane()).getByText('Next 3')).toBeTruthy();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Schedule' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({
    text: 'Every 15 minutes, 09:00–18:59, Mon–Fri',
  }));
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Next 1' }));
  await waitFor(() => expect((calls('clipboard_text_copy').at(-1)?.[1] as { text: string }).text).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/));

  // Safe arithmetic: Result + Formatted with one-click copy
  first().text = '(1920 * 1080) / 1024';
  await emit('clipboard_changed', 4);
  expect(await within(pane()).findByText('2,025')).toBeTruthy();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Result' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '2025' }));
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Formatted' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '2,025' }));

  // Radix & Unix permissions: 0xFF and chmod 755
  first().text = '0xFF';
  await emit('clipboard_changed', 5);
  expect(await within(pane()).findByText('0b11111111')).toBeTruthy();
  expect(within(pane()).queryByText('Permissions')).toBeNull();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy DEC' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '255' }));

  first().text = 'chmod 755';
  await emit('clipboard_changed', 6);
  expect(await within(pane()).findByText('rwxr-xr-x')).toBeTruthy();
  fireEvent.click(within(pane()).getByRole('button', { name: 'Copy Permissions' }));
  await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: 'rwxr-xr-x' }));
});

it('aligns detail key-value rows on a shared grid with centered keys and left-aligned wrapping values for long labels and results', async () => {
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_text_copy') return true;
    return baseInvoke(name, args);
  });

  const longKey = 'very_long_tracking_parameter_name_for_alignment_test';
  const longVal = 'https://example.test/very/long/value/'.repeat(6);
  first().text = `https://example.test/page?${longKey}=${encodeURIComponent(longVal)}&utm_source=share&id=1`;
  direct();

  await within(pane()).findByRole('button', { name: 'Copy Clean URL' });
  const urlGrid = pane().querySelector('.clipboard-smart-url > .clipboard-smart-rows')!;
  expect(urlGrid).toBeTruthy();
  // Top section (Clean URL) and bottom section (params) share one grid container so columns align across the divider.
  expect(urlGrid.querySelectorAll('.clipboard-smart-section')).toHaveLength(2);
  expect(urlGrid.querySelector('.clipboard-smart-section.clipboard-smart-divider.clipboard-smart-params')).toBeTruthy();

  const keyNodes = [...urlGrid.querySelectorAll<HTMLElement>('.clipboard-smart-key')];
  expect(keyNodes.map((node) => node.getAttribute('title'))).toContain(longKey);
  expect(keyNodes.map((node) => node.getAttribute('title'))).toContain('Clean URL');

  const valueNodes = [...urlGrid.querySelectorAll<HTMLElement>('.clipboard-smart-value')];
  expect(valueNodes.map((node) => node.getAttribute('title'))).toContain(longVal);

  // Footer metadata retains label/value structure.
  expect(pane().querySelector('.clipboard-detail-metadata > div > span:first-child')?.textContent).toBe('Application');

  // Cron schedule and next runs also share one grid container across the divider.
  first().text = '*/15 9-18 * * 1-5';
  await emit('clipboard_changed', 2);
  await within(pane()).findByText('Next 1');
  const cronGrid = pane().querySelector('.clipboard-smart-cron > .clipboard-smart-rows')!;
  expect(cronGrid.querySelector('.clipboard-smart-section.clipboard-smart-divider')).toBeTruthy();
  expect(cronGrid.querySelectorAll('.clipboard-smart-row')).toHaveLength(4);
  expect(cronGrid.querySelector('.clipboard-smart-key')?.getAttribute('title')).toBe('Schedule');
});

it('copies plain text from the detail toolbar after switching a rich-text record to Plain view', async () => {
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_record_detail') {
      const record = first();
      return {
        ...record,
        richTextFormat: 'html',
        richTextHtml: '<p><strong>Styled</strong> text</p>',
        files: [],
      };
    }
    return baseInvoke(name, args);
  });

  direct();
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-rich')).toBeTruthy());

  const richCopy = within(pane()).getByRole('button', { name: 'Copy' });
  await act(async () => { fireEvent.click(richCopy); });
  expect(richCopy.querySelector('.copy-success')).toBeTruthy();
  expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'copy' });

  const modeGroup = within(pane()).getByRole('group', { name: 'Preview Format' });
  fireEvent.click(within(modeGroup).getByRole('button', { name: 'Plain' }));
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));

  const plainCopy = within(pane()).getByRole('button', { name: 'Copy as Plain Text' });
  expect(plainCopy.querySelector('.copy-success')).toBeNull();
  await act(async () => { fireEvent.click(plainCopy); });
  expect(plainCopy.querySelector('.copy-success')).toBeTruthy();
  expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'copyPlain' });

  fireEvent.click(within(modeGroup).getByRole('button', { name: 'Rich' }));
  await waitFor(() => expect(pane().querySelector('.clipboard-detail-rich')).toBeTruthy());
  const richCopyAgain = within(pane()).getByRole('button', { name: 'Copy' });
  expect(richCopyAgain.querySelector('.copy-success')).toBeNull();
  await act(async () => { fireEvent.click(richCopyAgain); });
  expect(calls('clipboard_record_use').at(-1)?.[1]).toEqual({ recordId: 'first', action: 'copy' });
});

it('extracts a 48x48 offscreen image palette below single images alongside QR and path cards and copies HEX on click', async () => {
  const baseInvoke = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === 'clipboard_image') return { bytes: [137, 80, 78, 71], mime: 'image/png' };
    if (name === 'clipboard_image_qr_codes') return ['https://example.test/img-qr'];
    if (name === 'clipboard_text_copy') return true;
    return baseInvoke(name, args);
  });

  const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:nami-test-image');
  const revokeObjectURL = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
  let sampledCanvasSize: [number, number] | null = null;
  const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    sampledCanvasSize = [this.width, this.height];
    const pixelCount = this.width * this.height;
    const data = new Uint8ClampedArray(pixelCount * 4);
    const palette: Array<[number, number, number]> = [
      [30, 41, 59],
      [56, 189, 248],
      [129, 140, 248],
      [244, 114, 182],
      [250, 204, 21],
    ];
    for (let i = 0; i < pixelCount; i += 1) {
      const color = palette[i % palette.length];
      data[i * 4] = color[0];
      data[i * 4 + 1] = color[1];
      data[i * 4 + 2] = color[2];
      data[i * 4 + 3] = 255;
    }
    return {
      drawImage: vi.fn(),
      getImageData: vi.fn(() => ({ data })),
    } as unknown as CanvasRenderingContext2D;
  });

  try {
    Object.assign(first(), {
      kind: 'image',
      text: '',
      width: 1920,
      height: 1080,
      size: 204800,
      characterCount: null,
      files: [{ path: '/Users/apple/Desktop/shot.png', name: 'shot.png', exists: true, isDir: false, isImage: true, iconDataUrl: null }],
    });
    direct();

    const img = await waitFor(() => {
      const node = pane().querySelector<HTMLImageElement>('.clipboard-image-preview img');
      expect(node).toBeTruthy();
      return node!;
    });
    Object.defineProperties(img, {
      naturalWidth: { value: 1920, configurable: true },
      naturalHeight: { value: 1080, configurable: true },
    });
    fireEvent.load(img);

    const paletteGroup = await within(pane()).findByRole('group', { name: 'Color Palette' });
    expect(sampledCanvasSize).toEqual([48, 27]);
    const swatches = within(paletteGroup).getAllByRole('button');
    expect(swatches).toHaveLength(5);

    // Ordering under single image: ImagePreview -> Color Palette -> QR -> Compact Path card
    const qrCard = pane().querySelector('.clipboard-smart-qr-list')!;
    const pathCard = pane().querySelector('.clipboard-smart-files')!;
    expect(img.compareDocumentPosition(paletteGroup) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(paletteGroup.compareDocumentPosition(qrCard) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(qrCard.compareDocumentPosition(pathCard) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    const skySwatch = within(paletteGroup).getByRole('button', { name: 'Copy #38BDF8' });
    fireEvent.click(skySwatch);
    await waitFor(() => expect(calls('clipboard_text_copy').at(-1)?.[1]).toEqual({ text: '#38BDF8' }));
    await waitFor(() => expect(skySwatch.classList.contains('record-copied')).toBe(true));
    expect(skySwatch.querySelector('.copy-success')).toBeTruthy();

    // Switching back to a text record clears the extracted image palette.
    Object.assign(first(), { kind: 'text', text: 'Plain text record', files: [] });
    await emit('clipboard_changed', 2);
    await waitFor(() => expect(pane().querySelector('.clipboard-smart-palette')).toBeNull());
  } finally {
    getContext.mockRestore();
    createObjectURL.mockRestore();
    revokeObjectURL.mockRestore();
  }
});
