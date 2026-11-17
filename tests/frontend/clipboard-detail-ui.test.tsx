// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';
import { ClipboardDetail, type ClipboardDetailData } from '../../src/clipboard/ClipboardDetail';
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
  bridge.snapshot.window.recordView = 'clipboard';
  bridge.snapshot.window.translationVisible = false;
  bridge.snapshot.window.translationExtent = 0;
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    const id = (args as { recordId?: string })?.recordId;
    if (name === 'clipboard_query') {
      const page = { records: records.map(({ sourceApp, text, ...record }) => ({ ...record, text: text.slice(0, 120), sourceAppId: sourceApp ? 1 : null })), sources: { 1: first().sourceApp }, totalRecords: records.length, hasMore: false, cursor: null };
      return { page, alternate: { ...page, records: [], totalRecords: 0 } };
    }
    if (name === 'clipboard_record_detail') return fetchDetail ? fetchDetail(id!) : structuredClone(records.find((record) => record.id === id)!);
    if (name === 'clipboard_record_has_text') return id === 'image-text';
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
  bridge.snapshot.window.translationVisible = true;
  bridge.snapshot.window.clipboardDetailId = 'first';
  return render(<ClipboardDetail snapshot={bridge.snapshot} native t={translator('en')} command={async (name, args) => { await bridge.invoke(name, args); }} activity={vi.fn()} />);
}

describe('clipboard details', () => {
  it('keeps visual and accessible selection aligned when hover changes the detail', async () => {
    await open();
    fireEvent.focus(row('first').querySelector('button')!);
    fireEvent.mouseEnter(row('second'));
    await waitFor(() => expect(row('second').classList.contains('selected')).toBe(true));
    expect(row('second').querySelector('button')?.getAttribute('aria-pressed')).toBe('true');
    expect(row('first').querySelector('button')?.getAttribute('aria-pressed')).toBe('false');
  });

  it.each([
    ['clipboard_file_missing', 'not_written', 'The original file was moved or deleted and cannot be restored'],
    ['clipboard_image_missing', 'not_written', 'The saved image is unavailable and cannot be restored'],
    [null, 'superseded', 'Clipboard content changed. Copy the record again.'],
  ] as const)('preserves actionable copy feedback for %s / %s', async (code, copyStatus, message) => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => name === 'clipboard_record_use' ? {status: 'failed', copyStatus, code} : original(name,args));
    direct();
    fireEvent.click(await within(pane()).findByRole('button', {name:'Copy'}));
    expect(await within(pane()).findByText(message)).toBeTruthy();
    expect(pane().querySelector('.copy-success')).toBeNull();
    expect(document.querySelector('.global-error')).toBeNull();
  });

  it.each([
    ['clipboard_file_missing', 'The original file was moved or deleted and cannot be restored'],
    ['clipboard_image_missing', 'The saved image is unavailable and cannot be restored'],
    ['clipboard_write', 'Content was not copied. Try again.'],
  ])('maps a rejected detail copy %s through the same feedback as the list', async (code, message) => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name,args)=> { if (name === 'clipboard_record_use') throw new Error(code); return original(name,args); });
    direct();
    fireEvent.click(await within(pane()).findByRole('button',{name:'Copy'}));
    expect(await within(pane()).findByText(message)).toBeTruthy();
    expect(pane().querySelector('.copy-success')).toBeNull();
  });

  it('drops late copy failures after the detail identity changes', async () => {
    let reject!: (error: Error) => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name,args)=> name === 'clipboard_record_use' ? new Promise((_,fail)=>{reject=fail;}) : original(name,args));
    await open();
    fireEvent.click(within(pane()).getByRole('button',{name:'Copy'}));
    fireEvent.mouseEnter(row('second'));
    await waitFor(()=>expect(within(pane()).getByText('Second text')).toBeTruthy());
    await act(async()=>reject(new Error('clipboard_file_missing')));
    expect(pane().querySelector('.record-feedback')).toBeNull();
    expect(pane().querySelector('.copy-success')).toBeNull();
  });

  it('keeps a newer copy result when an older processing task fails', async () => {
    let reject!: (error: Error) => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name,args)=> name === 'clipboard_record_process' ? new Promise((_,fail)=>{reject=fail;}) : original(name,args));
    direct();
    fireEvent.click(await within(pane()).findByRole('button',{name:'Translate'}));
    await act(async()=>fireEvent.click(within(pane()).getByRole('button',{name:'Copy'})));
    await act(async()=>reject(new Error('old_failure')));
    expect(pane().querySelector('.record-feedback')).toBeNull();
    expect(within(pane()).getByRole('button',{name:'Translate'})).toHaveProperty('disabled',false);
    expect(pane().querySelector('.copy-success')).toBeTruthy();
  });

  it('keeps marks and copy available during processing without releasing its processing guard', async () => {
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
    await act(async()=>fireEvent.click(within(pane()).getByRole('button',{name:'Copy'})));
    expect(translate).toHaveProperty('disabled',true);
    fireEvent.click(translate);
    expect(calls('clipboard_record_process')).toHaveLength(1);
    await act(async()=>finish());
    expect(translate).toHaveProperty('disabled',false);
  });

  it('confirms detail copy inline without lighting the page', async () => {
    await mount('translation');
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    const detail = await screen.findByRole('region', { name: 'Clipboard details' });
    await waitFor(() => expect(within(detail).getByRole('button', { name: 'Copy' })).toBeTruthy());
    const original = bridge.invoke.getMockImplementation()!;
    let finish!: (value: unknown) => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => name === 'clipboard_record_use' ? new Promise((resolve) => { finish = resolve; }) : original(name, args));
    const button = within(detail).getByRole('button', { name: 'Copy' });
    fireEvent.click(button);
    expect(within(detail).queryByRole('progressbar')).toBeNull();
    await act(async () => finish({ status: 'copied', copyStatus: 'current', code: null }));
    expect(button.querySelector('.copy-success')).toBeTruthy();
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

  it('follows hover and keyboard focus without pasting, keeps preview on leave, and resets to first on reopen', async () => {
    await open();
    fireEvent.mouseEnter(row('second'));
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
    expect(row('second').classList.contains('selected')).toBe(true);
    fireEvent.mouseLeave(row('second'));
    expect(bridge.snapshot.window.clipboardDetailId).toBe('second');
    expect(calls('clipboard_record_use')).toHaveLength(0);
    fireEvent.focus(row('pinned').querySelector('button')!);
    await waitFor(() => expect(within(pane()).getByText('Pinned text')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Collapse details' }));
    await waitFor(() => expect(bridge.snapshot.window.translationVisible).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(bridge.snapshot.window.clipboardDetailId).toBe('first'));
  });

  it('drops conditional detail actions immediately while sliding between records', async () => {
    records[2] = { ...records[2], kind: 'files', text: '[]', characterCount: null, size: null, files: [{ path: '/tmp/a.txt', name: 'a.txt', exists: true, isDir: false, isImage: false, iconDataUrl: null }] };
    await open();
    expect(within(pane()).getByRole('button', { name: 'Translate' })).toBeTruthy();
    fireEvent.mouseEnter(row('second'));
    await waitFor(() => expect(within(pane()).getByText('a.txt')).toBeTruthy());
    expect(within(pane()).queryByRole('button', { name: 'Translate' })).toBeNull();
    expect(pane().querySelectorAll('[data-action="delete"]')).toHaveLength(1);
    expect(pane().querySelectorAll('.history-action')).toHaveLength(5);
  });

  it('does not show a loading beam while the next record detail is still loading', async () => {
    let resolve!: (value: ClipboardDetailData) => void;
    fetchDetail = (id) => id === 'second' ? new Promise((done) => { resolve = done; }) : Promise.resolve(structuredClone(records.find((record) => record.id === id)!));
    await open();
    const stable = ['Note', 'Favorite', 'Pin record', 'Delete clipboard record'].map((name) => within(pane()).getByRole('button', { name }));
    fireEvent.mouseEnter(row('second'));
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')).toBeNull());
    expect(pane().querySelector('[role="progressbar"]')).toBeNull();
    expect(stable.map((button) => button.isConnected)).toEqual([true, true, true, true]);
    stable.forEach((button, index) => expect(within(pane()).getByRole('button', { name: ['Note', 'Favorite', 'Pin record', 'Delete clipboard record'][index] })).toBe(button));
    fireEvent.click(stable[0]);
    expect(calls('clipboard_note_open')).toHaveLength(0);
    await act(async () => resolve(structuredClone(records.find((record) => record.id === 'second')!)));
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
  });

  it('ignores an old detail response after moving to another record or hiding', async () => {
    let resolve!: (value: ClipboardDetailData) => void;
    fetchDetail = (id) => id === 'first' ? new Promise((done) => { resolve = done; }) : Promise.resolve(records.find((record) => record.id === id)!);
    await mount('translation');
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(resolve).toBeTypeOf('function'));
    fireEvent.mouseEnter(row('second'));
    await waitFor(() => expect(within(pane()).getByText('Second text')).toBeTruthy());
    await act(async () => resolve(first()));
    expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe('Second text');
    await windowState({ translationVisible: false });
    expect(document.querySelector('.clipboard-detail-content')).toBeNull();
  });

  it('refreshes favorite and pin state from events and routes Note to the existing left editor', async () => {
    await open();
    const remove = within(pane()).getByRole('button', { name: 'Delete clipboard record' });
    fireEvent.click(within(pane()).getByRole('button', { name: 'Favorite' }));
    await waitFor(() => expect(within(pane()).getByRole('button', { name: 'Remove favorite' })).toBeTruthy());
    expect(within(pane()).getByRole('button', { name: 'Delete clipboard record' })).toBe(remove);
    expect(remove).toHaveProperty('disabled', true);
    fireEvent.click(remove);
    expect(calls('clipboard_record_update').filter(([,args]) => (args as {action:string}).action === 'delete')).toHaveLength(0);
    fireEvent.mouseEnter(row('pinned'));
    await waitFor(() => expect(within(pane()).getByText('Pinned text')).toBeTruthy());
    expect(within(pane()).getByRole('button', { name: 'Delete clipboard record' })).toHaveProperty('disabled', true);
    fireEvent.click(within(pane()).getByRole('button', { name: 'Note' }));
    expect(await screen.findByRole('dialog', { name: 'Note' })).toBeTruthy();
    await waitFor(() => expect(pane().hasAttribute('inert')).toBe(true));
    expect(calls('clipboard_note_open').at(-1)?.[1]).toEqual({ recordId: 'pinned' });
    expect(screen.getByRole('textbox', { name: 'Content' })).toHaveProperty('value', 'Pinned text');
  });

  it('opens the existing editor when a note request arrives while the list is temporarily hidden', async () => {
    await mount('translation');
    await waitFor(() => expect(row('first')).toBeTruthy());
    await windowState({ historyVisible: false, translationVisible: true, clipboardDetailId: 'first' });
    await emit('clipboard_note_requested', 'first');
    await windowState({ historyVisible: true });
    expect(await screen.findByRole('dialog', { name: 'Note' })).toBeTruthy();
    expect(screen.getByRole('textbox', { name: 'Content' })).toHaveProperty('value', first().text);
  });

  it.each([['image-text', true], ['image-blank', false]] as const)('only exposes recognized image actions for %s', async (id, text) => {
    records[1] = { ...first(), id, kind: 'image', text: 'Image', width: 2400, height: 1600, characterCount: null, size: 1536 * 1024 };
    bridge.snapshot.window.clipboardDetailId = id;
    bridge.snapshot.window.translationVisible = true;
    render(<ClipboardDetail snapshot={bridge.snapshot} native t={translator('en')} command={vi.fn()} activity={vi.fn()} />);
    await waitFor(() => expect(within(pane()).getByText('2400 × 1600')).toBeTruthy());
    await waitFor(() => expect(calls('clipboard_record_has_text').length).toBeGreaterThan(0));
    await act(async () => {});
    expect(!!within(pane()).queryByRole('button', { name: 'Extract text' })).toBe(text);
    expect(!!within(pane()).queryByRole('button', { name: 'Translate' })).toBe(text);
    expect(within(pane()).queryByText('Characters')).toBeNull();
    expect(within(pane()).getByText('Size')).toBeTruthy();
    expect(within(pane()).getByText('1.5 MB')).toBeTruthy();
    const actions = [...pane().querySelectorAll('[data-action]')].map((element) => (element as HTMLElement).dataset.action);
    if (text) expect(actions.indexOf('note')).toBe(actions.indexOf('ocr') + 1);
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

  it.each([
    ['Copy', 'clipboard_record_use', { recordId: 'first', action: 'copy' }],
    ['Translate', 'clipboard_record_process', { recordId: 'first', action: 'translate' }],
    ['Pin record', 'clipboard_record_update', { recordId: 'first', action: 'pin' }],
    ['Delete clipboard record', 'clipboard_record_update', { recordId: 'first', action: 'delete' }],
  ] as const)('executes the detail %s action for the displayed complete record', async (label, name, args) => {
    direct();
    const button = await within(pane()).findByRole('button', { name: label });
    fireEvent.click(button);
    if (label === 'Delete clipboard record') {
      expect(calls(name)).toHaveLength(0);
      fireEvent.click(button);
    }
    await waitFor(() => expect(calls(name).at(-1)?.[1]).toEqual(args));
  });

  it('reports a failed copy and restores the action buttons', async () => {
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name, args) => name === 'clipboard_record_use' ? { status: 'failed', copyStatus: 'not_written', code: 'clipboard_write' } : original(name, args));
    direct();
    fireEvent.click(await within(pane()).findByRole('button', { name: 'Copy' }));
    expect(await within(pane()).findByText('Content was not copied. Try again.')).toBeTruthy();
    expect(within(pane()).getByRole('button', { name: 'Copy' })).toHaveProperty('disabled', false);
  });

  it('keeps the scrollbar hidden until user scroll, then hides after idle', async () => {
    direct();
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
    const scroller = pane().querySelector('.history-list')!;
    Object.defineProperties(scroller, { clientHeight: { value: 100, configurable: true }, scrollHeight: { value: 1000, configurable: true } });
    fireEvent.scroll(scroller);
    const bar = within(pane()).getByRole('scrollbar');
    expect(bar.classList.contains('visible')).toBe(false);
    fireEvent.pointerMove(bar); fireEvent.focus(bar);
    expect(bar.classList.contains('visible')).toBe(false);
    fireEvent.blur(bar);
    expect(bar.classList.contains('visible')).toBe(false);
    fireEvent.wheel(scroller);
    expect(bar.classList.contains('visible')).toBe(true);
    await waitFor(() => expect(bar.classList.contains('visible')).toBe(false), { timeout: 1600 });
  });

  it('shows unavailable instead of stale content after deletion, and handles Escape', async () => {
    direct();
    await waitFor(() => expect(pane().querySelector('.clipboard-detail-text')?.textContent).toBe(first().text));
    fetchDetail = async () => { throw new Error('not_found'); };
    await emit('clipboard_changed', 2);
    expect(await within(pane()).findByRole('alert')).toHaveProperty('textContent', 'This record is unavailable.');
    expect(pane().querySelector('.clipboard-detail-content')).toBeNull();
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(calls('window_hide').at(-1)?.[1]).toEqual({ region: 'translation' });
  });
});
