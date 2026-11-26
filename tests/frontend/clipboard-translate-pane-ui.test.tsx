// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount, windowState } from './harness';
import type { ClipboardDetailData } from '../../src/clipboard/ClipboardDetail';

let records: ClipboardDetailData[];
let pendingContext: { recordId: string; text: string; error: string | null } | null;

const listRow = (id: string) => document.querySelector<HTMLElement>(`.clipboard-panel [data-record-id="${id}"]`)!;
const detailPane = () => screen.getByRole('region', { name: 'Clipboard details' });
const translatePane = () => screen.getByRole('region', { name: 'Translation pane' });

beforeEach(() => {
  pendingContext = null;
  records = [
    { id: 'first', kind: 'text', title: '', text: 'Hello world', date: '2026-11-16T09:00:00Z', isFavorite: false, isPinned: false, width: null, height: null, size: null, characterCount: 11, files: [] },
    { id: 'second', kind: 'text', title: '', text: 'Good morning', date: '2026-11-16T08:00:00Z', isFavorite: false, isPinned: false, width: null, height: null, size: null, characterCount: 12, files: [] },
  ];
  bridge.snapshot.capabilities.nativeHistory = false;
  bridge.snapshot.capabilities.nativeChrome = false;
  bridge.snapshot.window.standaloneTranslationVisible = false;
  bridge.snapshot.window.clipboardVisible = true;
  bridge.snapshot.window.clipboardDetailVisible = false;
  bridge.snapshot.window.clipboardTranslationVisible = false;
  bridge.snapshot.window.clipboardTranslationRecordId = null;
  bridge.snapshot.window.clipboardExtent = bridge.snapshot.window.clipboardWidth;
  bridge.snapshot.window.rightExtent = 0;

  const deliver = (event: string, payload: unknown) => {
    bridge.listeners.get(event)?.forEach((callback) => callback({ payload }));
  };
  const original = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    const id = (args as { recordId?: string })?.recordId;
    if (name === 'clipboard_query') {
      const page = { records: records.map(({ text, ...rest }) => ({ ...rest, text, sourceAppId: null })), sources: {}, totalRecords: records.length, hasMore: false, cursor: null };
      return { page, alternate: { ...page, records: [], totalRecords: 0 } };
    }
    if (name === 'clipboard_record_detail') return structuredClone(records.find((r) => r.id === id)!);
    if (name === 'clipboard_translation_take') {
      const next = pendingContext;
      pendingContext = null;
      return next;
    }
    if (name === 'clipboard_record_process') {
      const { recordId, action } = args as { recordId: string; action: string };
      if (action === 'translate') {
        const state = bridge.snapshot.window;
        if (state.clipboardTranslationVisible && state.clipboardTranslationRecordId === recordId) {
          state.clipboardTranslationVisible = false;
          state.clipboardTranslationRecordId = null;
          state.rightExtent = state.clipboardDetailVisible ? state.clipboardDetailWidth : 0;
          deliver('window_state', structuredClone(state));
        } else {
          const record = records.find((r) => r.id === recordId)!;
          pendingContext = { recordId, text: record.text, error: null };
          state.clipboardTranslationVisible = true;
          state.clipboardTranslationRecordId = recordId;
          state.rightExtent = (state.clipboardDetailVisible ? state.clipboardDetailWidth : 0) + state.clipboardTranslationWidth;
          deliver('window_state', structuredClone(state));
          deliver('clipboard_translation_ready', null);
        }
      }
      return;
    }
    if (name === 'translation_run') {
      const request = args as { text: string; source: string; targets: string[]; retryEngine?: string };
      const output = await original(name, args) as { translation: Record<string, unknown>; [key: string]: unknown };
      const engine = request.retryEngine ?? bridge.snapshot.translationEngine;
      return {
        ...output,
        sourceLanguage: request.source === 'auto' ? 'en' : request.source,
        translation: { ...output.translation, engine },
      };
    }
    return original(name, args);
  });
});

describe('clipboard attached translate pane', () => {
  it('supports independent 3-pane toggling ([List | Translate], [List | Detail | Translate], collapse Detail)', async () => {
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());

    // List-only + Translate -> [List | Translate]
    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(translatePane()).toBeTruthy());
    expect(screen.queryByRole('region', { name: 'Clipboard details' })).toBeNull();
    await waitFor(() => expect(within(translatePane()).getByText('Synthetic translation 1')).toBeTruthy());
    expect(calls('translation_run').at(-1)?.[1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['en', 'zh-CN'],
      scope: 'clipboard',
    });

    // Expand details -> [List | Detail | Translate] with split divider
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(detailPane()).toBeTruthy());
    expect(translatePane()).toBeTruthy();
    const divider = screen.getByRole('separator', { name: 'Resize details pane' });
    fireEvent.keyDown(divider, { key: 'ArrowRight' });
    expect(calls('clipboard_detail_width_set').at(-1)?.[1]).toEqual({ width: 504 });

    // Collapse details -> [List | Translate], translate pane stays open with its result
    fireEvent.click(screen.getByRole('button', { name: 'Collapse details' }));
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Clipboard details' })).toBeNull());
    expect(translatePane()).toBeTruthy();
    expect(within(translatePane()).getByText('Synthetic translation 1')).toBeTruthy();
  });

  it('keeps Translate button pressed in list and detail, toggles off on same record, and switches on another record', async () => {
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(detailPane()).toBeTruthy());

    const rowTranslate = () => within(listRow('first')).getByRole('button', { name: 'Translate' });
    const detailTranslate = () => within(detailPane()).getByRole('button', { name: 'Translate' });
    expect(rowTranslate().getAttribute('aria-pressed')).toBe('false');
    expect(detailTranslate().getAttribute('aria-pressed')).toBe('false');

    // Open translate for first record
    fireEvent.click(rowTranslate());
    await waitFor(() => expect(translatePane()).toBeTruthy());
    expect(listRow('first').dataset.translating).toBe('true');
    expect(rowTranslate().getAttribute('aria-pressed')).toBe('true');
    expect(rowTranslate().classList.contains('record-active')).toBe(true);
    expect(detailTranslate().getAttribute('aria-pressed')).toBe('true');
    expect(detailTranslate().classList.contains('record-active')).toBe(true);

    // Click second record's Translate -> switches target and pressed indicator
    fireEvent.click(within(listRow('second')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]).toMatchObject({ text: 'Good morning', scope: 'clipboard' });
    expect(listRow('first').dataset.translating).toBeUndefined();
    expect(listRow('second').dataset.translating).toBe('true');
    expect(within(listRow('second')).getByRole('button', { name: 'Translate' }).getAttribute('aria-pressed')).toBe('true');

    // Clicking second record's Translate again closes translate pane
    fireEvent.click(within(listRow('second')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Translation pane' })).toBeNull());
    expect(listRow('second').dataset.translating).toBeUndefined();
  });

  it('does not retranslate when hovering or navigating clipboard records while translate pane is open', async () => {
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    await waitFor(() => expect(detailPane()).toBeTruthy());

    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(1));

    // Hover and arrow-navigate to second record: Detail updates, Translate stays on first
    fireEvent.mouseMove(listRow('second'), { clientX: 40, clientY: 80 });
    await waitFor(() => expect(within(detailPane()).getByText('Good morning')).toBeTruthy());
    expect(calls('translation_run')).toHaveLength(1);
    expect(bridge.snapshot.window.clipboardTranslationRecordId).toBe('first');
    expect(within(detailPane()).getByRole('button', { name: 'Translate' }).getAttribute('aria-pressed')).toBe('false');
  });

  it('shows full language names in top tabs, dynamic language metadata row with Auto-Detect prefix, switches languages with targeted retranslation, and keeps source read-only', async () => {
    const user = userEvent.setup();
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());

    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(within(translatePane()).getByText('Synthetic translation 1')).toBeTruthy());

    // Legacy language picker footer and arrow separator between tabs are removed
    expect(translatePane().querySelector('.clipboard-translate-footer')).toBeNull();
    expect(translatePane().querySelector('.clipboard-translate-tab-arrow')).toBeNull();

    // Metadata shows Characters, active tab language, then Service with its icon
    const metadata = translatePane().querySelector<HTMLElement>('.clipboard-detail-metadata')!;
    const metadataLabels = () => Array.from(metadata.children, (row) => row.firstElementChild?.textContent);
    expect(metadataLabels()).toEqual(['Characters', 'Primary', 'Service']);
    expect(within(metadata).getByText('Microsoft Edge Translate')).toBeTruthy();
    expect(metadata.querySelector<HTMLImageElement>('img.history-origin')?.getAttribute('src')).toBeTruthy();
    expect(within(metadata).getByText('Characters')).toBeTruthy();
    expect(within(metadata).getByText('23')).toBeTruthy();
    expect(within(metadata).getByText('Primary')).toBeTruthy();
    expect(within(metadata).getByText('English')).toBeTruthy();
    expect(within(metadata).queryByText('Retranslate')).toBeNull();

    // Top tabs show full language names (detected source English, primary English, secondary Chinese (Simplified))
    const tabs = within(translatePane()).getByRole('group', { name: 'Translation view' });
    const sourceTab = () => tabs.querySelector<HTMLButtonElement>('[data-tab="source"]')!;
    const primaryTab = () => tabs.querySelector<HTMLButtonElement>('[data-tab="primary"]')!;
    const secondaryTab = () => tabs.querySelector<HTMLButtonElement>('[data-tab="secondary"]')!;
    expect(sourceTab().textContent).toBe('English');
    expect(primaryTab().textContent).toBe('English');
    expect(primaryTab().getAttribute('aria-pressed')).toBe('true');
    expect(secondaryTab().textContent).toBe('Chinese (Simplified)');

    // Language switch and engine switch buttons are placed to the left of Copy button
    const actionItems = Array.from(translatePane().querySelectorAll<HTMLElement>('.clipboard-detail-toolbar .history-action'));
    expect(actionItems.map((el) => el.dataset.action)).toEqual(['language', 'engine', 'copy']);

    // Copy Primary
    const copyBtn = within(translatePane()).getByRole('button', { name: 'Copy Primary Translation' });
    await user.click(copyBtn);
    await waitFor(() => expect(calls('translation_copy').at(-1)?.[1]).toEqual({ scope: 'clipboard', part: 'primary' }));
    expect(copyBtn.classList.contains('record-copied')).toBe(true);

    // Switch Primary language -> updates Primary tab & metadata row immediately, retranslates only Primary (reuseTargetIndex: 0)
    await user.click(within(translatePane()).getByRole('button', { name: 'Primary language: English' }));
    expect(screen.getByRole('dialog', { name: 'Primary language' })).toBeTruthy();
    await user.click(screen.getByRole('option', { name: 'Japanese' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(primaryTab().textContent).toBe('Japanese');
    expect(within(metadata).getByText('Primary')).toBeTruthy();
    expect(within(metadata).getByText('Japanese')).toBeTruthy();
    expect(calls('translation_run')[1][1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['ja', 'zh-CN'],
      scope: 'clipboard',
      reuseTargetIndex: 0,
    });

    // Switch to Secondary -> language row follows Secondary, copy, then switch Secondary language (reuseTargetIndex: 1)
    await user.click(secondaryTab());
    expect(within(translatePane()).getByText('Synthetic translation 2')).toBeTruthy();
    expect(metadataLabels()).toEqual(['Characters', 'Secondary', 'Service']);
    expect(within(metadata).getByText('Chinese (Simplified)')).toBeTruthy();
    await user.click(within(translatePane()).getByRole('button', { name: 'Copy Secondary Translation' }));
    await waitFor(() => expect(calls('translation_copy').at(-1)?.[1]).toEqual({ scope: 'clipboard', part: 'secondary' }));

    await user.click(within(translatePane()).getByRole('button', { name: 'Secondary language: Chinese (Simplified)' }));
    await user.click(screen.getByRole('option', { name: 'French' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(3));
    expect(secondaryTab().textContent).toBe('French');
    expect(within(metadata).getByText('Secondary')).toBeTruthy();
    expect(within(metadata).getByText('French')).toBeTruthy();
    expect(calls('translation_run')[2][1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['ja', 'fr'],
      scope: 'clipboard',
      reuseTargetIndex: 1,
    });

    // Switch to Source tab -> read-only source display (no textarea/retranslate), 3rd row shows Original + Auto-Detect · English
    await user.click(sourceTab());
    expect(translatePane().querySelector('textarea')).toBeNull();
    expect(within(translatePane()).getByText('Hello world')).toBeTruthy();
    expect(within(metadata).getByText('11')).toBeTruthy();
    expect(within(metadata).getByText('Original')).toBeTruthy();
    expect(within(metadata).getByText('Auto-Detect · English')).toBeTruthy();
    expect(within(metadata).queryByText('Retranslate')).toBeNull();

    // Pressing Cmd+Return does not retranslate
    fireEvent.keyDown(translatePane(), { key: 'Enter', code: 'Enter', metaKey: true });
    expect(calls('translation_run')).toHaveLength(3);

    // Change Source language to German -> updates Source tab & metadata row immediately (manual, no Auto-Detect prefix) and retranslates BOTH targets
    await user.click(within(translatePane()).getByRole('button', { name: 'Source language: English' }));
    await user.click(screen.getByRole('option', { name: 'German' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(4));
    expect(sourceTab().textContent).toBe('German');
    expect(within(metadata).getByText('Original')).toBeTruthy();
    expect(within(metadata).getByText('German')).toBeTruthy();
    expect(calls('translation_run')[3][1]).toEqual({
      text: 'Hello world',
      source: 'de',
      targets: ['ja', 'fr'],
      scope: 'clipboard',
    });

    // Disable secondary language from Secondary tab -> Secondary tab disappears and falls back to Primary
    await user.click(secondaryTab());
    await user.click(within(translatePane()).getByRole('button', { name: 'Secondary language: French' }));
    await user.click(screen.getByRole('option', { name: 'None' }));
    await waitFor(() => expect(tabs.querySelector('[data-tab="secondary"]')).toBeNull());
    expect(primaryTab().getAttribute('aria-pressed')).toBe('true');
    expect(primaryTab().textContent).toBe('Japanese');
    expect(within(metadata).getByText('Primary')).toBeTruthy();
    expect(within(metadata).getByText('Japanese')).toBeTruthy();
  });

  it('closes dropdown first on Escape, then closes only attached translate pane before hiding clipboard window', async () => {
    const user = userEvent.setup();
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());

    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(translatePane()).toBeTruthy());

    // Open top toolbar language picker -> Escape closes only picker
    await user.click(within(translatePane()).getByRole('button', { name: 'Primary language: English' }));
    expect(screen.getByRole('dialog', { name: 'Primary language' })).toBeTruthy();
    fireEvent.keyDown(screen.getByRole('dialog', { name: 'Primary language' }), { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Primary language' })).toBeNull();
    expect(calls('clipboard_translation_close')).toHaveLength(0);
    expect(calls('window_hide')).toHaveLength(0);

    // Next Escape closes only attached translate pane
    fireEvent.keyDown(translatePane(), { key: 'Escape' });
    await waitFor(() => expect(calls('clipboard_translation_close')).toHaveLength(1));
    expect(calls('window_hide')).toHaveLength(0);
    expect(screen.queryByRole('region', { name: 'Translation pane' })).toBeNull();

    // Escape again hides clipboard window
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(calls('window_hide').at(-1)?.[1]).toEqual({ region: 'clipboard' });
  });

  it('closes a focused service menu on Command-W without closing or restarting the translation', async () => {
    const user = userEvent.setup();
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());
    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    const trigger = within(await screen.findByRole('region', { name: 'Translation pane' })).getByRole('button', { name: /translation service/i });
    await user.click(trigger);
    const menu = screen.getByRole('dialog', { name: 'Translation Service' });
    fireEvent.keyDown(document.activeElement ?? menu, { key: 'w', code: 'KeyW', metaKey: true });
    expect(screen.queryByRole('dialog', { name: 'Translation Service' })).toBeNull();
    expect(translatePane()).toBeTruthy();
    expect(document.activeElement).toBe(trigger);
    expect(calls('clipboard_translation_close')).toHaveLength(0);
    expect(calls('window_hide')).toHaveLength(0);
  });

  it('uses the preview key from the attached translation without moving focus or starting another translation', async () => {
    bridge.snapshot.clipboardControls.shortcuts.preview = { enabled: true, binding: { code: 'KeyY', modifiers: ['meta'] } };
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());
    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    const pane = await screen.findByRole('region', { name: 'Translation pane' });
    await within(pane).findByText('Synthetic translation 1');
    const tab = pane.querySelector<HTMLButtonElement>('[data-tab="primary"]')!;
    act(() => tab.focus());
    const preview = () => fireEvent.keyDown(tab, { key: 'y', code: 'KeyY', metaKey: true });
    preview();
    await waitFor(() => expect(within(detailPane()).getByText('Hello world')).toBeTruthy());
    preview();
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Clipboard details' })).toBeNull());
    expect(document.activeElement).toBe(tab);
    expect(calls('translation_run')).toHaveLength(1);
  });

  it('returns to the list when a focused right-hand panel closes', async () => {
    bridge.snapshot.clipboardControls.shortcuts.preview = { enabled: true, binding: { code: 'KeyY', modifiers: ['meta'] } };
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Expand details' }));
    const detail = await screen.findByRole('region', { name: 'Clipboard details' });
    const note = await within(detail).findByRole('button', { name: 'Note' });
    await waitFor(() => expect((note as HTMLButtonElement).disabled).toBe(false));
    act(() => note.focus());
    fireEvent.keyDown(note, { key: 'y', code: 'KeyY', metaKey: true });
    const row = listRow('first').querySelector('.history-row-select');
    await waitFor(() => expect(document.activeElement).toBe(row));
    expect(screen.queryByRole('region', { name: 'Clipboard details' })).toBeNull();

    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    const pane = await screen.findByRole('region', { name: 'Translation pane' });
    const tab = pane.querySelector<HTMLButtonElement>('[data-tab="primary"]')!;
    act(() => tab.focus());
    fireEvent.keyDown(tab, { key: 'Escape' });
    await waitFor(() => expect(document.activeElement).toBe(row));
    expect(screen.queryByRole('region', { name: 'Translation pane' })).toBeNull();
    fireEvent.keyDown(row!, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(listRow('second').querySelector('.history-row-select'));
  });

  it('opens engine dropdown to temporarily switch translation service and retranslate both targets, and displays Edge when Google falls back', async () => {
    const user = userEvent.setup();
    bridge.snapshot.translationEngine = 'google';
    bridge.snapshot.translationPreferences.engine = 'google';
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());

    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(within(translatePane()).getByText('Synthetic translation 1')).toBeTruthy());

    const metadata = () => translatePane().querySelector<HTMLElement>('.clipboard-detail-metadata')!;
    expect(within(metadata()).getByText('Google Translate')).toBeTruthy();

    // Click engine switch button (to the left of Copy) -> opens dropdown with official service options and icons
    await user.click(within(translatePane()).getByRole('button', { name: 'Translation Service: Google Translate' }));
    const engineDialog = screen.getByRole('dialog', { name: 'Translation Service' });
    expect(within(engineDialog).getByRole('option', { name: 'Microsoft Edge Translate' }).querySelector('img.language-option-icon')).toBeTruthy();
    expect(within(engineDialog).getByRole('option', { name: 'Google Translate' }).querySelector('img.language-option-icon')).toBeTruthy();

    // Selecting Microsoft Edge Translate temporarily switches and retranslates both targets without changing global default
    await user.click(within(engineDialog).getByRole('option', { name: 'Microsoft Edge Translate' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(screen.queryByRole('dialog', { name: 'Translation Service' })).toBeNull();
    expect(within(metadata()).getByText('Microsoft Edge Translate')).toBeTruthy();
    expect(calls('translation_run')[1][1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['en', 'zh-CN'],
      scope: 'clipboard',
      retryEngine: 'edge',
    });
    expect(calls('translation_preferences_set')).toHaveLength(0);

    // Selecting already-active engine is a no-op
    await user.click(within(translatePane()).getByRole('button', { name: 'Translation Service: Microsoft Edge Translate' }));
    await user.click(screen.getByRole('option', { name: 'Microsoft Edge Translate' }));
    expect(calls('translation_run')).toHaveLength(2);

    // Selecting Google Translate switches back and retranslates both targets
    await user.click(within(translatePane()).getByRole('button', { name: 'Translation Service: Microsoft Edge Translate' }));
    await user.click(screen.getByRole('option', { name: 'Google Translate' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(3));
    expect(within(metadata()).getByText('Google Translate')).toBeTruthy();
    expect(calls('translation_run')[2][1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['en', 'zh-CN'],
      scope: 'clipboard',
    });

    // Switch back to Edge temporarily, then translate another record -> resets to default Google Translate
    await user.click(within(translatePane()).getByRole('button', { name: 'Translation Service: Google Translate' }));
    await user.click(screen.getByRole('option', { name: 'Microsoft Edge Translate' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(4));
    expect(within(metadata()).getByText('Microsoft Edge Translate')).toBeTruthy();

    // Simulate backend Google -> Edge silent fallback on second record
    const prevImpl = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'translation_run') {
        const result = await prevImpl(name, args) as { translation: Record<string, unknown>; [key: string]: unknown };
        return { ...result, translations: ['Edge fallback 1', 'Edge fallback 2'], translation: { ...result.translation, engine: 'edge' } };
      }
      return prevImpl(name, args);
    });
    fireEvent.click(within(listRow('second')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(within(translatePane()).getByText('Edge fallback 1')).toBeTruthy());
    expect(calls('translation_run')[4][1]).toEqual({
      text: 'Good morning',
      source: 'auto',
      targets: ['en', 'zh-CN'],
      scope: 'clipboard',
    });
    expect(within(translatePane()).queryByRole('alert')).toBeNull();
    expect(within(metadata()).getByText('Microsoft Edge Translate')).toBeTruthy();
  });

  it('switches a clipboard session to Apple and leaves an unsupported target isolated', async () => {
    bridge.snapshot.capabilities.appleTranslation = true;
    bridge.snapshot.translationEngine = 'google';
    bridge.snapshot.translationPreferences.engine = 'google';
    bridge.snapshot.translationPreferences.sourceLanguage = 'eo';
    bridge.snapshot.translationPreferences.secondaryLanguage = 'eo';
    const user = userEvent.setup();
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());
    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(within(translatePane()).getByText('Synthetic translation 1')).toBeTruthy());
    await user.click(within(translatePane()).getByRole('button', { name: 'Translation Service: Google Translate' }));
    await user.click(screen.getByRole('option', { name: 'Apple Translate' }));
    await waitFor(() => expect(calls('translation_run').at(-1)?.[1]).toEqual({
      text: 'Hello world', source: 'auto', targets: ['en'], scope: 'clipboard', retryEngine: 'apple',
    }));
    await waitFor(() => expect(within(translatePane().querySelector<HTMLElement>('.clipboard-detail-metadata')!).getByText('Apple Translate')).toBeTruthy());
    await user.click(within(translatePane()).getByRole('button', { name: 'Esperanto (unavailable)' }));
    expect(within(translatePane()).getByText(/not supported/)).toBeTruthy();
    expect(calls('translation_preferences_set')).toHaveLength(0);
  });

  it('keeps supported targets working when switching to Edge with a Google-only language, and retranslates on Settings engine change', async () => {
    const user = userEvent.setup();
    bridge.snapshot.translationEngine = 'google';
    bridge.snapshot.translationPreferences.engine = 'google';
    bridge.snapshot.translationPreferences.sourceLanguage = 'eo';
    bridge.snapshot.translationPreferences.primaryLanguage = 'zh-CN';
    bridge.snapshot.translationPreferences.secondaryLanguage = 'eo';
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());

    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(within(translatePane()).getByText('Synthetic translation 1')).toBeTruthy());

    // Switching temporarily to Edge falls back unsupported manual source to auto and still translates supported primary (zh-CN)
    await user.click(within(translatePane()).getByRole('button', { name: 'Translation Service: Google Translate' }));
    await user.click(screen.getByRole('option', { name: 'Microsoft Edge Translate' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(2));
    expect(calls('translation_run')[1][1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['zh-CN'],
      scope: 'clipboard',
      retryEngine: 'edge',
    });
    expect(within(translatePane()).getByText('Synthetic translation 1')).toBeTruthy();
    expect(within(translatePane()).queryByText(/not supported/)).toBeNull();

    // Unsupported secondary tab shows languageNotSupported; switching it to an Edge-only language works locally without persisting invalid global preference
    await user.click(within(translatePane()).getByRole('button', { name: /Esperanto/ }));
    expect(within(translatePane()).getByText(/not supported/)).toBeTruthy();
    await user.click(within(translatePane()).getByRole('button', { name: /Secondary language:/ }));
    await user.click(screen.getByRole('option', { name: 'Chinese (Literary)' }));
    await waitFor(() => expect(calls('translation_run')).toHaveLength(3));
    expect(calls('translation_run')[2][1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['zh-CN', 'lzh'],
      scope: 'clipboard',
      reuseTargetIndex: 1,
      retryEngine: 'edge',
    });
    expect(calls('translation_preferences_set')).toHaveLength(0);

    // Changing global default engine from Settings while pane is open automatically retranslates instead of blanking
    bridge.snapshot.translationEngine = 'edge';
    bridge.snapshot.translationPreferences.engine = 'edge';
    await emit('translation_engine_changed', 'edge');
    await waitFor(() => expect(calls('translation_run')).toHaveLength(4));
    expect(calls('translation_run')[3][1]).toEqual({
      text: 'Hello world',
      source: 'auto',
      targets: ['zh-CN', 'lzh'],
      scope: 'clipboard',
    });
  });

  it('cancels pending clipboard translation and resets state when session ends', async () => {
    await mount('translation');
    await waitFor(() => expect(listRow('first')).toBeTruthy());
    fireEvent.click(within(listRow('first')).getByRole('button', { name: 'Translate' }));
    await waitFor(() => expect(translatePane()).toBeTruthy());
    const before = calls('translation_cancel').length;
    await windowState({ clipboardTranslationVisible: false, clipboardTranslationRecordId: null, recordsSession: 1 });
    expect(calls('translation_cancel').length).toBeGreaterThan(before);
    expect(calls('translation_cancel').at(-1)?.[1]).toEqual({ scope: 'clipboard' });
  });
});
