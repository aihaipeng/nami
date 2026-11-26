// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { bridge, calls, emit, mount } from './harness';

describe('clipboard recording settings', () => {
  it('stores clipboard preview lines under General and defaults to three lines', async () => {
    await mount('settings');
    expect(screen.queryByRole('button', { name: /^Translation Preview Lines:/ })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard Preview Lines: 3' }));
    expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual(['1', '2', '3']);
    fireEvent.click(screen.getByRole('option', { name: '1' }));
    await screen.findByRole('button', { name: 'Clipboard Preview Lines: 1' });
    expect(calls('records_preview_lines_set').at(-1)?.[1]).toEqual({ lines: 1 });
  });

  it('saves the shared opening focus from General', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Focus When Opening Records: Record List' }));
    fireEvent.click(screen.getByRole('option', { name: 'Search Field' }));
    await screen.findByRole('button', { name: 'Focus When Opening Records: Search Field' });
    expect(calls('records_open_focus_set').at(-1)?.[1]).toEqual({ focus: 'search' });
  });

  it('restores only the shortcuts and enabled states in the selected group', async () => {
    const prefs = bridge.snapshot.translationPreferences;
    const records = bridge.snapshot.clipboardControls.shortcuts;
    bridge.snapshot.clipboardBinding = { code: 'KeyJ', modifiers: ['control', 'shift'] };
    bridge.snapshot.clipboardShortcutEnabled = false;
    prefs.shortcuts.openTranslation = { code: 'F8', modifiers: [] };
    prefs.shortcuts.focusSearch = { code: 'KeyJ', modifiers: ['alt'] };
    prefs.shortcuts.translate = null;
    prefs.selectionEnabled = false;
    prefs.disabledShortcuts = ['openTranslation', 'focusSearch', 'translate'];
    records.switchCategory = { binding: { code: 'KeyH', modifiers: ['alt'] }, enabled: false };
    records.note = { binding: null, enabled: false };
    records.preview = { binding: null, enabled: false };
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const restore = (name: string) => within(screen.getByRole('region', { name })).getByRole('button', { name: 'Restore Default Shortcuts' });

    fireEvent.click(restore('Window'));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Focus Input shortcut' }).textContent).toBe('⌘F'));
    expect(screen.getByRole('checkbox', { name: 'Focus Input' })).toHaveProperty('checked', true);
    expect(screen.getByRole('button', { name: 'Switch Group shortcut' }).textContent).toBe('Tab');
    expect(screen.getByRole('checkbox', { name: 'Switch Group' })).toHaveProperty('checked', true);
    expect(records.note).toEqual({ binding: null, enabled: false });
    expect(records.preview).toEqual({ binding: null, enabled: false });
    expect(prefs.shortcuts.translate).toBeNull();
    expect(prefs.selectionEnabled).toBe(false);
    expect(bridge.snapshot.clipboardShortcutEnabled).toBe(false);
    expect(prefs.disabledShortcuts).toEqual(['openTranslation', 'translate']);

    fireEvent.click(restore('Record Actions'));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Preview shortcut' }).textContent).toBe('Space'));
    expect(screen.getByRole('checkbox', { name: 'Add/Remove Note' })).toHaveProperty('checked', true);
    expect(records.note.binding?.code).toBe('KeyN');
    expect(records.preview).toEqual({ binding: { code: 'Space', modifiers: [] }, enabled: true });
    expect(prefs.shortcuts.translate).toBeNull();
    expect(prefs.selectionEnabled).toBe(false);

    fireEvent.click(restore('Translation'));
    await waitFor(() => expect(prefs.shortcuts.translate).toEqual({ code: 'Enter', modifiers: ['meta'] }));
    expect(screen.getByRole('checkbox', { name: 'Open Translation' })).toHaveProperty('checked', false);
    expect(prefs.selectionEnabled).toBe(false);
    expect(prefs.disabledShortcuts).toEqual(['openTranslation']);

    fireEvent.click(restore('Global'));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Open Clipboard History shortcut' }).textContent).toBe('⌃`'));
    expect(screen.getByRole('checkbox', { name: 'Open Clipboard History' })).toHaveProperty('checked', true);
    expect(screen.getByRole('checkbox', { name: 'Open Translation' })).toHaveProperty('checked', true);
    expect(screen.getByRole('checkbox', { name: 'Autofill Input' })).toHaveProperty('checked', true);
    expect(prefs.disabledShortcuts).toEqual([]);
  });

  it('shows the default Control and backquote shortcut', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    expect([...screen.getByRole('button', { name: 'Open Clipboard History shortcut' }).closest('.settings-shortcut-row')!.querySelectorAll('.keycaps kbd')].map((key) => key.textContent)).toEqual(['⌃', '`']);
  });

  it('uses text-only headings and plain action labels in clipboard and storage settings', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect(screen.getAllByRole('heading').every((heading) => !heading.querySelector('svg'))).toBe(true);
    expect(screen.getByRole('button', { name: 'Add App' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Storage' }));
    expect(screen.getAllByRole('heading').every((heading) => !heading.querySelector('svg'))).toBe(true);
    expect(screen.getByRole('button', { name: 'Manual Cleanup' })).toBeTruthy();
  });

  it('groups global, window, record, and translation shortcuts and keeps them out of the Clipboard settings page', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    expect(screen.getByRole('heading', { level: 1, name: 'HotKey' })).toBeTruthy();
    expect(screen.getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent)).toEqual(['Global', 'Window', 'Record Actions', 'Translation']);
    const rowTitle = (toggle: Element) => toggle.closest('.settings-shortcut-row')?.querySelector('.settings-field-content')?.textContent;
    const globalGroup = within(screen.getByRole('group', { name: 'Global' }));
    expect(globalGroup.getAllByRole<HTMLInputElement>('checkbox').map(rowTitle)).toEqual([
      'Open Clipboard History', 'Open Translation', 'Autofill Input',
    ]);
    expect(globalGroup.getAllByRole('checkbox').map((toggle) => toggle.closest('.settings-shortcut-row')?.querySelector('.keycaps')?.textContent)).toEqual([
      '⌃`', '⌘T', '⌘T',
    ]);
    const windowGroup = within(screen.getByRole('group', { name: 'Window' }));
    expect(windowGroup.getAllByRole<HTMLInputElement>('checkbox').map(rowTitle)).toEqual([
      'Focus Input', 'Switch Group', 'Pin / Unpin Window', 'Show Shortcuts', 'Open Settings',
    ]);
    expect(windowGroup.getAllByRole('checkbox').map((toggle) => toggle.closest('.settings-shortcut-row')?.querySelector('.keycaps')?.textContent)).toEqual([
      '⌘F', 'Tab', '⌘P', '⌘K', '⌘,',
    ]);
    for (const retired of ['Switch Record View', 'Open Translation History', 'Focus Record List', 'Navigate Up', 'Navigate Down', 'Previous Category', 'Next Category', 'Paste (Context Menu)']) {
      expect(screen.queryByRole('checkbox', { name: retired })).toBeNull();
    }
    const recordGroup = within(screen.getByRole('group', { name: 'Record Actions' }));
    const recordOrder = [
      'Preview', 'Copy', 'Paste as Plain Text', 'Favorite/Unfavorite', 'Pin/Unpin', 'Add/Remove Note',
      'Translate', 'Extract Text', 'Open Link, Email or Locate File', 'Save Image', 'Delete',
    ];
    expect(recordGroup.getAllByRole<HTMLInputElement>('checkbox').map(rowTitle)).toEqual(recordOrder);
    expect(recordGroup.getAllByRole('checkbox').map((toggle) => toggle.closest('.settings-shortcut-row')?.querySelector('.keycaps')?.textContent)).toEqual([
      'Space', '⌘C', '⌘↵', '⌘D', '⌥⌘P', '⌘N', '⌘T', '⇧⌘O', '⌘O', '⌘S', '⌘⌫',
    ]);
    for (const name of recordOrder) {
      expect(recordGroup.getByRole('button', { name: `${name} shortcut` })).toBeTruthy();
      expect(recordGroup.getByRole<HTMLInputElement>('checkbox', { name }).checked).toBe(true);
    }
    const translation = within(screen.getByRole('group', { name: 'Translation' }));
    expect(translation.getAllByRole<HTMLInputElement>('checkbox').map(rowTitle)).toEqual([
      'Translate', 'Copy Primary Translation', 'Copy Secondary Translation',
    ]);
    await emit('locale_changed', 'zh-CN');
    expect(screen.getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent)).toEqual(['全局', '窗口', '记录操作', '翻译']);
    expect(screen.getByRole('checkbox', { name: '打开翻译' })).toBeTruthy();
    await emit('locale_changed', 'en');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect(screen.queryByRole('button', { name: 'Open Clipboard History shortcut' })).toBeNull();
  });

  it('rejects a record binding already used by a window action and preserves both', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const windowGroup = within(screen.getByRole('group', { name: 'Window' }));
    const search = windowGroup.getByRole('button', { name: 'Focus Input shortcut' });
    const preview = screen.getByRole('button', { name: 'Preview shortcut' });
    fireEvent.click(preview);
    fireEvent.keyDown(window, { key: 'f', code: 'KeyF', metaKey: true });
    expect((await screen.findByRole('alert')).textContent).toContain('already assigned');
    expect(preview.textContent).toBe('Space');
    expect(search.textContent).toBe('⌘F');
    fireEvent.click(search);
    fireEvent.keyDown(window, { key: ' ', code: 'Space' });
    expect((await screen.findByRole('alertdialog')).textContent).toContain('already assigned');
    expect(search.textContent).toBe('⌘F');
  });

  it('switches the recorder between matching labels and releases capture on navigation', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const recordTranslate = within(screen.getByRole('group', { name: 'Record Actions' })).getByRole('button', { name: 'Translate shortcut' });
    const translation = within(screen.getByRole('group', { name: 'Translation' })).getByRole('button', { name: 'Translate shortcut' });
    await user.click(recordTranslate);
    expect(recordTranslate.getAttribute('aria-pressed')).toBe('true');
    await user.click(translation);
    expect(recordTranslate.getAttribute('aria-pressed')).toBe('false');
    expect(translation.getAttribute('aria-pressed')).toBe('true');
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', metaKey: true });
    await waitFor(() => expect(translation.textContent).toBe('⌘J'));
    expect(calls('clipboard_record_shortcut_set')).toHaveLength(0);
    expect(calls('translation_shortcut_set').at(-1)?.[1]).toEqual({ action: 'translate', binding: { code: 'KeyJ', modifiers: ['meta'] } });
    fireEvent.click(recordTranslate);
    expect(calls('translation_shortcut_capture').at(-1)?.[1]).toEqual({ active: true });
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect(calls('translation_shortcut_capture').at(-1)?.[1]).toEqual({ active: false });
    fireEvent.keyDown(window, { key: 'k', code: 'KeyK', altKey: true });
    expect(calls('clipboard_record_shortcut_set')).toHaveLength(0);
  });

  it('keeps the secondary copy shortcut when the second language is off', async () => {
    const prefs = bridge.snapshot.translationPreferences;
    const saved = { code: 'KeyU', modifiers: ['meta'] as const };
    prefs.shortcuts.copySecondary = { ...saved, modifiers: [...saved.modifiers] };
    prefs.secondaryLanguage = 'none';
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    expect(screen.getByRole('button', { name: 'Copy Secondary Translation shortcut' }).textContent).toBe('⌘U');
    await emit('translation_preferences_changed', { ...prefs, secondaryLanguage: 'fr' });
    expect(screen.getByRole('button', { name: 'Copy Secondary Translation shortcut' }).textContent).toBe('⌘U');
  });

  it('describes both operations for note, favorite, and pin in Simplified Chinese', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    await emit('locale_changed', 'zh-CN');
    expect(screen.getAllByRole('button', { name: '恢复默认快捷键' })).toHaveLength(4);
    for (const name of ['备注/取消备注', '收藏/取消收藏', '置顶/取消置顶']) {
      expect(screen.getByRole('checkbox', { name })).toBeTruthy();
      expect(screen.getByRole('button', { name: `${name} 快捷键` })).toBeTruthy();
    }
  });

  it.each([['Preview', 'preview', 'Space'], ['Add/Remove Note', 'note', '⌘N'], ['Copy', 'copyRecord', '⌘C'], ['Extract Text', 'ocr', '⇧⌘O'], ['Translate', 'translate', '⌘T'], ['Favorite/Unfavorite', 'favorite', '⌘D'], ['Delete', 'delete', '⌘⌫'], ['Paste as Plain Text', 'pastePlain', '⌘↵'], ['Save Image', 'saveImage', '⌘S'], ['Switch Group', 'switchCategory', 'Tab']])(
    'edits %s while unchecked and scopes Record Actions restore to its own group', async (title, action, defaultKeys) => {
      const user = userEvent.setup(); await mount('settings');
      await user.click(screen.getByRole('button', { name: 'HotKey' }));
      const inRecords = action !== 'switchCategory';
      const group = within(screen.getByRole('group', { name: inRecords ? 'Record Actions' : 'Window' }));
      const recorder = group.getByRole('button', { name: `${title} shortcut` });
      const toggle = group.getByRole<HTMLInputElement>('checkbox', { name: title });
      await user.click(toggle);
      expect(calls('clipboard_record_shortcut_set').at(-1)?.[1]).toEqual({ action, enabled: false });
      await user.click(recorder);
      fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', altKey: true });
      await waitFor(() => expect(recorder.textContent).toBe('⌥J'));
      expect(toggle.checked).toBe(false);
      await user.click(within(screen.getByRole('region', { name: 'Record Actions' })).getByRole('button', { name: 'Restore Default Shortcuts' }));
      await waitFor(() => expect(recorder.textContent).toBe(inRecords ? defaultKeys : '⌥J'));
      expect(toggle.checked).toBe(inRecords);
    });

  it.each([[' ', 'Space', false, 'Space'], ['F6', 'F6', false, 'F6'], ['J', 'KeyJ', true, '⇧J'], ['Escape', 'Escape', false, 'Escape'], ['Backspace', 'Backspace', false, '⌫']])('records %s in the preview recorder', async (key, code, shiftKey, label) => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const recorder = screen.getByRole('button', { name: 'Preview shortcut' });
    fireEvent.click(recorder);
    fireEvent.keyDown(window, { key, code, shiftKey });
    await waitFor(() => expect(recorder.textContent).toBe(label));
    expect(bridge.snapshot.clipboardControls.shortcuts.preview.binding).toEqual({ code, modifiers: shiftKey ? ['shift'] : [] });
  });

  it('waits for a main key during shortcut capture and omits the manual key picker', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const recorder = screen.getByRole('button', { name: 'Copy shortcut' });
    fireEvent.click(recorder);
    expect(screen.queryByRole('button', { name: 'Choose Keys' })).toBeNull();
    // A modifier-tap event emitted when WebView misses the main key must not replace the binding.
    fireEvent.keyDown(recorder, { key: 'Meta', code: 'MetaLeft' });
    expect(recorder.textContent).toBe('Press shortcut');
    expect(calls('clipboard_record_shortcut_set')).toHaveLength(0);
    fireEvent.keyDown(recorder, { key: 'c', code: 'KeyC', metaKey: true });
    await waitFor(() => {
      expect(recorder.textContent).toBe('⌘C');
      expect(recorder.getAttribute('aria-disabled')).toBeNull();
    });
    expect(calls('clipboard_record_shortcut_set')).toHaveLength(1);
    expect(bridge.snapshot.clipboardControls.shortcuts.copyRecord.binding).toEqual({ code: 'KeyC', modifiers: ['meta'] });
  });

  it('shows shortcut explanation tooltips on hover or click and dismisses them on mouse leave', async () => {
    bridge.snapshot.clipboardControls.shortcuts.delete.binding = { ...bridge.snapshot.clipboardBinding! };
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));

    // Only the 6 non-obvious actions (plus any conflicting row) render tip icons.
    expect(document.querySelectorAll('.settings-shortcut-tip')).toHaveLength(7);
    expect(screen.queryByRole('button', { name: 'Open Settings info' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Preview info' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Favorite/Unfavorite info' })).toBeNull();

    // Hover shows the tooltip (one sentence per line), clicking while hovered keeps it open, and mouse leave dismisses it.
    const openTip = screen.getByRole('button', { name: 'Open Translation info' });
    fireEvent.mouseEnter(openTip);
    const openTooltip = screen.getByRole('tooltip');
    expect(openTooltip.textContent).toContain('selected text takes priority');
    expect(openTooltip.querySelectorAll('.clipboard-tip-lines > div')).toHaveLength(2);
    fireEvent.click(openTip);
    expect(screen.getByRole('tooltip').textContent).toContain('selected text takes priority');
    fireEvent.mouseLeave(openTip);
    expect(screen.queryByRole('tooltip')).toBeNull();

    // Direct click opens the tooltip, mouse leave dismisses it, and clicking again toggles it off.
    const switchTip = screen.getByRole('button', { name: 'Switch Group info' });
    fireEvent.click(switchTip);
    expect(screen.getByRole('tooltip').textContent).toContain('Hold Shift to go backward');
    fireEvent.mouseLeave(switchTip);
    expect(screen.queryByRole('tooltip')).toBeNull();
    fireEvent.click(switchTip);
    expect(screen.getByRole('tooltip')).toBeTruthy();
    fireEvent.click(switchTip);
    expect(screen.queryByRole('tooltip')).toBeNull();

    // Global conflict surfaces a warning tooltip on otherwise plain items.
    const deleteTip = screen.getByRole('button', { name: 'Delete info' });
    expect(deleteTip.dataset.conflict).toBe('true');
    expect(deleteTip.closest('.settings-shortcut-name')?.querySelector('.settings-field-content')?.textContent).toBe('Delete');
    expect(deleteTip.closest('.settings-shortcut-row')?.children).toHaveLength(3);
    fireEvent.mouseEnter(deleteTip);
    expect(screen.getByRole('tooltip').textContent).toBe('Conflicts with an existing global shortcut.');
    fireEvent.mouseLeave(deleteTip);
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('keeps other shortcut rows and restore actions active while toggling a clipboard shortcut', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const originalInvoke = bridge.invoke.getMockImplementation()!;
    let finishSave!: () => void;
    bridge.invoke.mockImplementation((name: string, args?: unknown) => {
      if (name === 'clipboard_record_shortcut_set') {
        return new Promise<void>((resolve) => {
          finishSave = () => { void Promise.resolve(originalInvoke(name, args)).then(() => resolve()); };
        });
      }
      return originalInvoke(name, args);
    });

    const previewToggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Preview' });
    const previewRecorder = screen.getByRole('button', { name: 'Preview shortcut' });
    const copyToggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Copy' });
    const copyRecorder = screen.getByRole('button', { name: 'Copy shortcut' });
    const globalToggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Open Clipboard History' });
    const restoreButtons = screen.getAllByRole<HTMLButtonElement>('button', { name: 'Restore Default Shortcuts' });

    fireEvent.click(previewToggle);
    await waitFor(() => expect(previewToggle.getAttribute('aria-disabled')).toBe('true'));
    expect(previewRecorder.getAttribute('aria-disabled')).toBe('true');
    expect(copyToggle.getAttribute('aria-disabled')).toBeNull();
    expect(copyRecorder.getAttribute('aria-disabled')).toBeNull();
    expect(globalToggle.getAttribute('aria-disabled')).toBeNull();
    expect(restoreButtons.every((button) => !button.disabled)).toBe(true);

    finishSave();
    await waitFor(() => {
      expect(previewToggle.checked).toBe(false);
      expect(previewToggle.getAttribute('aria-disabled')).toBeNull();
    });
  });

  it('reuses a chord across clipboard-only record actions and Translation, but rejects it for a shared record action', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const group = (name: string) => within(screen.getByRole('group', { name }));
    const record = (button: HTMLElement, key: { key: string; code: string; metaKey: boolean }) => {
      fireEvent.click(button);
      fireEvent.keyDown(window, key);
    };
    const plain = group('Record Actions').getByRole('button', { name: 'Paste as Plain Text shortcut' });
    const translate = group('Translation').getByRole('button', { name: 'Translate shortcut' });
    record(translate, { key: 'j', code: 'KeyJ', metaKey: true });
    await waitFor(() => expect(translate.textContent).toBe('⌘J'));
    record(translate, { key: 'Enter', code: 'Enter', metaKey: true });
    await waitFor(() => expect(translate.textContent).toBe('⌘↵'));
    expect(plain.textContent).toBe('⌘↵');
    record(plain, { key: 'i', code: 'KeyI', metaKey: true });
    await waitFor(() => expect(plain.textContent).toBe('⌘I'));
    record(plain, { key: 'Enter', code: 'Enter', metaKey: true });
    await waitFor(() => expect(plain.textContent).toBe('⌘↵'));
    const preview = group('Record Actions').getByRole('button', { name: 'Preview shortcut' });
    record(preview, { key: 'Enter', code: 'Enter', metaKey: true });
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(preview.textContent).toBe('Space');
  });

  it('keeps panel shortcut changes separate and rejects a collision with another panel action', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const pin = screen.getByRole('button', { name: 'Pin/Unpin shortcut' });
    const toggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Pin/Unpin' });
    await user.click(toggle);
    await user.click(pin);
    fireEvent.keyDown(window, { key: 'd', code: 'KeyD', metaKey: true });
    expect(await screen.findByText('This key combination is already assigned to a shortcut in the same scope.')).toBeTruthy();
    expect(pin.textContent).toBe('⌥⌘P');
    await user.click(pin);
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', altKey: true });
    await waitFor(() => expect(pin.textContent).toBe('⌥J'));
    expect(toggle.checked).toBe(false);
    expect(calls('clipboard_record_shortcut_set').at(-1)?.[1]).toEqual({ action: 'pin', binding: { code: 'KeyJ', modifiers: ['alt'] } });
    expect(calls('clipboard_shortcut_set')).toHaveLength(0);
  });

  it('syncs the capture switch with menu bar events and preserves recording types', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const capture = screen.getByRole('region', { name: 'Clipboard Recording' });
    const master = within(capture).getByRole<HTMLInputElement>('checkbox', { name: 'Record Clipboard History' });
    expect(master.checked).toBe(false);
    expect(within(capture).getByRole('status').textContent).toBe('Recording paused. Resume from the menu bar.');
    await emit('clipboard_capture_changed', true);
    expect(master.checked).toBe(true);
    expect(within(capture).queryByRole('status')).toBeNull();
    expect(within(capture).getByRole('checkbox', { name: 'Images' }).getAttribute('aria-description')).toBe('Includes supported single image files.');
  });

  it('toggles content checkboxes only from the checkbox or keyboard and preserves them when recording resumes', async () => {
    const user = userEvent.setup();
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const image = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Images' });
    expect(image.checked).toBe(true);
    expect(image.matches(':disabled')).toBe(false);
    // Clicking the row label text must not accidentally toggle the checkbox.
    await user.click(screen.getByText('Images', { exact: true }));
    expect(image.checked).toBe(true);
    expect(calls('clipboard_recording_type_set')).toHaveLength(0);
    await user.click(image);
    await waitFor(() => { expect(image.checked).toBe(false); expect(image.hasAttribute('aria-disabled')).toBe(false); });
    expect(calls('clipboard_recording_type_set')[0][1]).toEqual({ kind: 'image', enabled: false });
    expect(screen.getByText('Recording paused. Resume from the menu bar.')).toBeTruthy();
    await emit('clipboard_capture_changed', true);
    expect(image.checked).toBe(false);
    fireEvent.click(screen.getByRole('checkbox', { name: 'Text' }));
    await waitFor(() => {
      const text = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Text' });
      expect(text.checked).toBe(false);
      expect(text.hasAttribute('aria-disabled')).toBe(false);
    });
    fireEvent.click(screen.getByRole('checkbox', { name: 'Files' }));
    await screen.findByText('No content types selected. New copies won’t be recorded.');
    image.focus();
    await user.keyboard(' ');
    expect(image.checked).toBe(true);
    expect(calls('clipboard_recording_type_set').at(-1)?.[1]).toEqual({ kind: 'image', enabled: true });
  });

  it('records, cancels, clears and restores the shortcut, retaining it on conflict', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const control = () => screen.getByRole('button', { name: /Open Clipboard History shortcut/ });
    const shortcuts = screen.getByRole('region', { name: 'Global' });
    const restore = within(shortcuts).getByRole('button', { name: 'Restore Default Shortcuts' });
    fireEvent.click(control());
    expect(within(control()).getByText('Press shortcut')).toBeTruthy();
    fireEvent.keyDown(window, { key: 't', code: 'KeyT', metaKey: true });
    expect((await within(shortcuts).findByRole('alert')).textContent).toBe('Already assigned to Open Translation.');
    expect(control().textContent).toBe('⌃`');
    fireEvent.click(control());
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(control().textContent).toBe('⌃⇧J'));
    expect(within(shortcuts).queryByRole('status')).toBeNull();
    fireEvent.click(control());
    fireEvent.click(control());
    expect(control().textContent).toBe('⌃⇧J');
    fireEvent.click(control());
    fireEvent.keyDown(window, { key: 'F6', code: 'F6' });
    await waitFor(() => expect(control().textContent).toBe('F6'));
    fireEvent.click(restore);
    await waitFor(() => expect(control().textContent).toBe('⌃`'));
    expect(within(shortcuts).queryByRole('status')).toBeNull();
    const clear = within(screen.getByRole('button', { name: 'Open Clipboard History shortcut' }).closest('.settings-shortcut-row')!).getByRole('button', { name: 'Clear shortcut' });
    clear.focus();
    fireEvent.click(clear);
    await waitFor(() => expect(control().textContent).toBe('Add Hotkey'));
    expect(within(shortcuts).queryByRole('status')).toBeNull();
    expect(document.activeElement).toBe(control());
  });

  it('clears and records a clipboard shortcut while it remains unchecked', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const checkbox = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Open Clipboard History' });
    const recorder = screen.getByRole<HTMLButtonElement>('button', { name: 'Open Clipboard History shortcut' });
    const clear = within(screen.getByRole('button', { name: 'Open Clipboard History shortcut' }).closest('.settings-shortcut-row')!).getByRole('button', { name: 'Clear shortcut' });
    await user.click(checkbox);
    expect(calls('clipboard_shortcut_set').at(-1)?.[1]).toEqual({ enabled: false });
    expect(checkbox.checked).toBe(false);
    expect(recorder.textContent).toBe('⌃`');
    expect(within(screen.getByRole('button', { name: 'Open Clipboard History shortcut' }).closest('.settings-shortcut-row')!).getByRole('button', { name: 'Clear shortcut' })).toBe(clear);
    await user.click(clear);
    expect(recorder.textContent).toBe('Add Hotkey');
    await user.click(recorder);
    expect(recorder.textContent).toBe('Press shortcut');
    fireEvent.keyDown(window, { key: 'k', code: 'KeyK', ctrlKey: true });
    await waitFor(() => expect(recorder.textContent).toBe('⌃K'));
    expect(checkbox.checked).toBe(false);
    await user.click(checkbox);
    expect(checkbox.checked).toBe(true);
    expect(recorder.textContent).toBe('⌃K');
  });

  it('disables recording on unsupported platforms and storage failure', async () => {
    bridge.snapshot.capabilities.clipboardWrite = false;
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect(screen.getByRole('checkbox', { name: 'Images' }).matches(':disabled')).toBe(true);
    expect(calls('clipboard_recording_type_set')).toHaveLength(0);
  });

  it('preserves a recording event delivered before the initial app snapshot', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'app_snapshot') {
        await emit('clipboard_capture_changed', true);
        return structuredClone(bridge.snapshot);
      }
      return invoke(name, args);
    });
    await mount('settings');
    fireEvent.click(await screen.findByRole('button', { name: 'Clipboard' }));
    expect(screen.queryByText('Recording paused. Resume from the menu bar.')).toBeNull();
  });
});

describe('clipboard privacy, retention and behavior', () => {
  const excludedApps: { bundleId: string; name: string; iconDataUrl?: string }[] = [
    { bundleId: 'com.apple.Passwords', name: 'Passwords' },
    { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' },
  ];
  const defaultBreakdown = {
    text: { bytes: 1500, count: 12 },
    image: { bytes: 2000, count: 2 },
    files: { bytes: 500, count: 1 },
  };
  async function openSettings(apps = excludedApps) {
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_options_get') return {
        excludedApps: apps,
        activation: 'paste',
        pasteAsPlainText: false,
      };
      if (name === 'clipboard_excluded_app_pick') return [{ bundleId: 'com.apple.Passwords', name: 'Passwords' }, { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' }, { bundleId: 'com.example.synthetic', name: 'Synthetic App' }];
      if (name === 'clipboard_excluded_app_remove') return [{ bundleId: 'com.apple.Passwords', name: 'Passwords' }, { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' }];
      if (name === 'records_storage_info') return { bytes: 4000, limit: 500, autoCleanupIntervalHours: 1, breakdown: defaultBreakdown };
      if (name === 'storage_location_info') {
        return {
          dataDir: '/Users/synthetic/Library/Application Support/dev.local.nami',
          defaultDataDir: '/Users/synthetic/Library/Application Support/dev.local.nami',
          isCustomDataDir: false,
          logDir: '/Users/synthetic/Library/Logs/dev.local.nami',
        };
      }
      if (name === 'records_cleanup_preview') return { count: 4, token: 'preview-token', ...(!(args as { policy: unknown }).policy && { categories: [1, 2, 2] }) };
      if (name === 'records_cleanup_apply') {
        const categories = (args as { categories?: number }).categories;
        return { removed: categories === 6 ? 3 : 4, reclaimedBytes: categories === 6 ? 2_000_000 : 1_500_000, mediaCleanupFailed: false };
      }
      return invoke(name, args);
    });
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    await waitFor(() => expect(calls('clipboard_options_get').length).toBeGreaterThan(0));
    await screen.findByRole('region', { name: 'Ignore Rules' });
  }
  async function openStorageSettings() {
    await openSettings();
    fireEvent.click(screen.getByRole('button', { name: 'Storage' }));
    await screen.findByText('4 kB');
  }

  it('groups apps, pasteboard types, and text patterns under one Ignore Rules heading with compact chips and on-demand inputs', async () => {
    await openSettings();
    for (const name of ['Clipboard Recording', 'Ignore Rules', 'Search', 'Behavior']) {
      expect(screen.getByRole('region', { name })).toBeTruthy();
    }
    expect(screen.queryByRole('region', { name: 'Retention & Cleanup' })).toBeNull();
    const heading = screen.getByRole('heading', { level: 2, name: 'Clipboard Recording' });
    expect(heading.parentElement?.querySelector('[role=switch]')).toBeNull();
    const types = screen.getByRole('group', { name: 'Content types' });
    expect(within(types).getAllByRole('checkbox')).toHaveLength(3);
    expect(within(types).queryByRole('switch')).toBeNull();
    const ignoreRules = screen.getByRole('region', { name: 'Ignore Rules' });
    expect(within(ignoreRules).getAllByRole('heading')).toHaveLength(1);
    expect(ignoreRules.querySelectorAll('.settings-group')).toHaveLength(1);
    expect(ignoreRules.querySelectorAll('.clipboard-ignore-block')).toHaveLength(4);
    expect(ignoreRules.querySelectorAll('.clipboard-tip-trigger')).toHaveLength(0);
    expect(within(ignoreRules).queryAllByRole('textbox')).toHaveLength(0);
    expect(ignoreRules.textContent).not.toContain('Checks the active app only.');
    expect(ignoreRules.textContent).not.toContain('Built-in concealed');
    expect(ignoreRules.textContent).not.toContain('Matches newly copied');
    expect(within(ignoreRules).getByRole('button', { name: 'Add App' }).getAttribute('aria-description')).toBe('Checks the active app only.');
    const addType = within(ignoreRules).getByRole('button', { name: 'Add Pasteboard Type' });
    fireEvent.click(addType);
    const typeInput = within(ignoreRules).getByRole<HTMLInputElement>('textbox', { name: 'Pasteboard Types' });
    expect(typeInput.placeholder).toBe('org.example.SecretType');
    expect(document.activeElement).toBe(typeInput);
    fireEvent.keyDown(typeInput, { key: 'Escape' });
    expect(within(ignoreRules).queryByRole('textbox', { name: 'Pasteboard Types' })).toBeNull();
    expect(document.activeElement).toBe(addType);
    const addRegex = within(ignoreRules).getByRole('button', { name: 'Add Text Pattern' });
    fireEvent.click(addRegex);
    const regexInput = within(ignoreRules).getByRole<HTMLInputElement>('textbox', { name: 'Text Patterns' });
    expect(regexInput.placeholder).toBe('^sk-[A-Za-z0-9]+$');
    fireEvent.blur(regexInput);
    expect(within(ignoreRules).queryByRole('textbox', { name: 'Text Patterns' })).toBeNull();
    await emit('locale_changed', 'zh-CN');
    expect(screen.getByRole('heading', { level: 2, name: '忽略规则' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '添加应用' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '添加粘贴板类型' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '添加文本正则' })).toBeTruthy();
    await emit('locale_changed', 'en');
  });

  it('does not expose the retired automatic image indexing toggle', async () => {
    await openSettings();
    expect(screen.queryByRole('region', { name: 'Index image text automatically' })).toBeNull();
    expect(screen.queryByRole('checkbox', { name: 'Index image text automatically' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'About Image Text Indexing' })).toBeNull();
    expect(calls('clipboard_auto_image_index_set')).toHaveLength(0);
  });

  it('omits empty app lists and renders no extra hint tooltips when app names are unique', async () => {
    await openSettings([]);
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(screen.queryByRole('list', { name: 'Apps' })).toBeNull();
    const ignoreRules = screen.getByRole('region', { name: 'Ignore Rules' });
    expect(ignoreRules.querySelector('.clipboard-tip-trigger')).toBeNull();
  });

  it('shows the native icon next to the app name and falls back when the icon is absent or fails to load', async () => {
    const icon = 'data:image/png;base64,c3ludGhldGlj';
    await openSettings([{ ...excludedApps[0], iconDataUrl: icon }, excludedApps[1]]);
    const list = screen.getByRole('list', { name: 'Apps' });
    const known = screen.getByText('Passwords').closest('li')!;
    const missing = screen.getByText('Keychain Access').closest('li')!;
    const image = known.querySelector('img')!;
    expect(image.getAttribute('src')).toBe(icon);
    expect(image.alt).toBe('');
    expect(missing.querySelector('svg.clipboard-excluded-icon')).toBeTruthy();
    fireEvent.error(image);
    expect(known.querySelector('svg.clipboard-excluded-icon')).toBeTruthy();
    expect(within(list).getByText('Passwords')).toBeTruthy();
  });

  it('shows and manages excluded apps directly even when recording is off', async () => {
    await openSettings();
    const list = await screen.findByRole('list', { name: 'Apps' });
    expect(list.textContent).toContain('Passwords');
    expect(list.textContent).toContain('Keychain Access');
    expect(list.textContent).not.toContain('com.apple.Passwords');
    expect(screen.getByText('Passwords').title).toBe('com.apple.Passwords');
    expect(screen.queryByRole('dialog', { name: 'Apps' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Add App' }));
    await screen.findByText('Synthetic App');
    fireEvent.click(screen.getByRole('button', { name: 'Remove Synthetic App' }));
    await waitFor(() => expect(screen.queryByText('Synthetic App')).toBeNull());
    expect(calls('clipboard_excluded_app_remove')[0][1]).toEqual({ bundleId: 'com.example.synthetic' });
  });

  it('discloses ambiguous app identifiers on demand with keyboard-dismissible tips and removes by exact identifier', async () => {
    await openSettings([...excludedApps, { bundleId: 'com.example.passwords', name: 'Passwords' }]);
    for (const bundleId of ['com.apple.Passwords', 'com.example.passwords']) {
      expect(screen.queryByText(bundleId)).toBeNull();
      const tip = screen.getByRole('button', { name: `About Passwords (${bundleId})` });
      fireEvent.click(tip);
      const panel = screen.getByRole('tooltip');
      expect(within(panel).getByText(bundleId)).toBeTruthy();
      expect(within(panel).queryByRole('button')).toBeNull();
      expect(tip.getAttribute('aria-describedby')).toBe(panel.id);
      expect(document.activeElement).toBe(tip);
      fireEvent.keyDown(tip, { key: 'Escape' });
      expect(screen.queryByRole('tooltip')).toBeNull();
      fireEvent.click(tip);
      fireEvent.pointerDown(screen.getByRole('button', { name: 'Add App' }));
      expect(screen.queryByRole('tooltip')).toBeNull();
    }
    fireEvent.click(screen.getByRole('button', { name: 'Remove Passwords (com.example.passwords)' }));
    await waitFor(() => expect(screen.getAllByText('Passwords')).toHaveLength(1));
    expect(calls('clipboard_excluded_app_remove')[0][1]).toEqual({ bundleId: 'com.example.passwords' });
    expect(screen.queryByText('com.apple.keychainaccess')).toBeNull();
  });

  it('defaults to 500 items, 0 days, 1 hr cleanup interval, auto-saves after 2s inactivity without Save button, and confirms a lower custom count', async () => {
    await openStorageSettings();
    const limit = screen.getByRole<HTMLInputElement>('textbox', { name: 'Max History Items' });
    const age = screen.getByRole<HTMLInputElement>('textbox', { name: 'Retention Period' });
    const interval = screen.getByRole<HTMLInputElement>('textbox', { name: 'Auto Cleanup Interval' });
    expect(limit.value).toBe('500');
    expect(age.value).toBe('0');
    expect(interval.value).toBe('1');
    expect(screen.queryByText('Keep History For')).toBeNull();
    expect(limit.getAttribute('aria-description')).toContain('0 means unlimited');

    vi.useFakeTimers();
    try {
      for (const value of ['-1', '5.5', '1000001']) {
        fireEvent.change(limit, { target: { value } });
        expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
        expect(limit.getAttribute('aria-invalid')).toBe('true');
      }
      await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
      expect(calls('records_cleanup_preview')).toHaveLength(0);

      fireEvent.change(limit, { target: { value: '13' } });
      expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(1500); });
      expect(calls('records_cleanup_preview')).toHaveLength(0);

      // Continuing to type resets the 2-second inactivity timer.
      fireEvent.change(limit, { target: { value: '137' } });
      await act(async () => { await vi.advanceTimersByTimeAsync(1999); });
      expect(calls('records_cleanup_preview')).toHaveLength(0);
      await act(async () => { await vi.advanceTimersByTimeAsync(1); });

      const dialog = screen.getByRole('alertdialog');
      expect(dialog.textContent).toContain('permanently removes 4 unprotected items');
      expect(calls('records_cleanup_preview')[0][1]).toEqual({ policy: { limit: 137, autoCleanupIntervalHours: 1 } });
      const cancel = within(dialog).getByRole('button', { name: 'Cancel' });
      expect(cancel).toBe(document.activeElement);
      expect(cancel.textContent).toBe('CancelEsc');
      fireEvent.keyDown(cancel, { key: 'Escape' });
      expect(limit.value).toBe('500');
      expect(calls('records_cleanup_apply')).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('auto-saves unlimited count (0) and auto cleanup interval after 2s inactivity without dialog when preview affects no items', async () => {
    await openStorageSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    let limit = 500;
    let autoCleanupIntervalHours = 1;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      const request = args as { policy?: { limit: number; autoCleanupIntervalHours: number } };
      if (name === 'records_storage_info') return { bytes: 1900, limit, autoCleanupIntervalHours, breakdown: defaultBreakdown };
      if (name === 'records_cleanup_preview') return { count: 0, token: 'empty-preview' };
      if (name === 'records_cleanup_apply') {
        limit = request.policy!.limit;
        autoCleanupIntervalHours = request.policy!.autoCleanupIntervalHours;
        return { removed: 0, mediaCleanupFailed: false };
      }
      return invoke(name, args);
    });
    vi.useFakeTimers();
    try {
      fireEvent.change(screen.getByRole('textbox', { name: 'Max History Items' }), { target: { value: '0' } });
      expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
      expect(calls('records_cleanup_apply')[0][1]).toEqual({
        policy: { limit: 0, autoCleanupIntervalHours: 1 },
        token: 'empty-preview',
      });
      expect(screen.queryByRole('alertdialog')).toBeNull();

      fireEvent.change(screen.getByRole('textbox', { name: 'Auto Cleanup Interval' }), { target: { value: '6' } });
      expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
      expect(calls('records_cleanup_apply').at(-1)?.[1]).toEqual({
        policy: { limit: 0, autoCleanupIntervalHours: 6 },
        token: 'empty-preview',
      });
      expect(screen.queryByRole('alertdialog')).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('confirms changing custom retention period days without changing count limit', async () => {
    await openStorageSettings();
    const age = screen.getByRole<HTMLInputElement>('textbox', { name: 'Retention Period' });
    fireEvent.change(age, { target: { value: '14' } });
    expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
    fireEvent.submit(age.closest('form')!);
    const dialog = await screen.findByRole('alertdialog', { name: 'Change history retention?' });
    expect(dialog.textContent).toContain('4 unprotected items');
    expect(calls('records_cleanup_apply')).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Apply & Remove' }));
    await waitFor(() => expect(calls('records_cleanup_apply').at(-1)?.[1]).toEqual({
      policy: { limit: 500, maxAgeDays: 14, autoCleanupIntervalHours: 1 },
      token: 'preview-token',
    }));
  });

  it('runs manual cleanup for selected record categories by keyboard, shows unified summary toast, and refreshes storage', async () => {
    await openStorageSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'records_storage_info') return { bytes: 2000, limit: 500, autoCleanupIntervalHours: 1, breakdown: { text: { bytes: 1000, count: 5 }, image: { bytes: 1000, count: 1 }, files: { bytes: 0, count: 0 } } };
      return invoke(name, args);
    });
    expect(screen.queryByText('Clear Last 15 Minutes')).toBeNull();
    const trigger = screen.getByRole('button', { name: 'Manual Cleanup' });
    expect(trigger.parentElement?.contains(screen.getByRole('heading', { name: 'Retention & Cleanup' }))).toBe(true);
    fireEvent.click(trigger);
    const dialog = await screen.findByRole('alertdialog', { name: 'Manual Cleanup' });
    const confirm = within(dialog).getByRole<HTMLButtonElement>('button', { name: 'Clean Up' });
    const checkbox = (name: RegExp) => within(dialog).getByRole<HTMLInputElement>('checkbox', { name });
    const options = within(dialog).getAllByRole<HTMLInputElement>('checkbox');
    expect(options).toHaveLength(3);
    expect(options.every((item) => !item.checked)).toBe(true);
    expect(dialog.querySelector('p')).toBeNull();
    expect(dialog.hasAttribute('aria-describedby')).toBe(false);
    expect(confirm.disabled).toBe(true);
    expect(confirm.textContent).toBe('Clean Up↵');
    fireEvent.keyDown(within(dialog).getByRole('button', { name: 'Cancel' }), { key: 'Enter' });
    expect(calls('records_cleanup_apply')).toHaveLength(0);
    fireEvent.click(checkbox(/^Favorites/));
    fireEvent.click(checkbox(/^Pinned/));
    expect(confirm.disabled).toBe(false);
    expect(calls('records_cleanup_apply')).toHaveLength(0);
    // WebKit can focus the modal itself when clicking its contents.
    dialog.tabIndex = -1;
    dialog.focus();
    expect(document.activeElement).toBe(dialog);
    fireEvent.keyDown(dialog, { key: 'Enter' });
    await waitFor(() => expect(document.querySelector('.storage-usage-total')?.textContent).toBe('2 kB'));
    expect(screen.queryByText('4 kB')).toBeNull();
    expect(document.querySelector('.storage-toast')?.textContent).toBe('Cleaned up 3 items, freed ~2 MB');
    expect(within(screen.getByRole('region', { name: 'Retention & Cleanup' })).queryByRole('status')).toBeNull();
    expect(calls('records_cleanup_apply')[0][1]).toEqual({ policy: null, categories: 6, token: 'preview-token' });
    expect(calls('records_storage_info').length).toBeGreaterThanOrEqual(2);
  });

  it('refreshes storage and reports an image cleanup failure after records were removed', async () => {
    await openStorageSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'records_cleanup_apply') return { removed: 1, reclaimedBytes: 512_000, mediaCleanupFailed: true };
      if (name === 'records_storage_info') return { bytes: 2000, limit: 500, breakdown: { text: { bytes: 2000, count: 5 }, image: { bytes: 0, count: 0 }, files: { bytes: 0, count: 0 } } };
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Manual Cleanup' }));
    const dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('checkbox', { name: /^Ordinary/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Clean Up' }));
    await waitFor(() => expect(document.querySelector('.storage-usage-total')?.textContent).toBe('2 kB'));
    expect(document.querySelector('.storage-toast')?.textContent).toBe('Cleaned up 1 item, freed ~512 kB');
    expect(screen.getByRole('alert').textContent).toContain('some local image copies');
  });

  it('does not enable confirmation when the clipboard history is empty', async () => {
    await openStorageSettings();
    bridge.invoke.mockResolvedValueOnce({ count: 0, token: 'empty-preview', categories: [0, 0, 0] });
    fireEvent.click(screen.getByRole('button', { name: 'Manual Cleanup' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.querySelector('p')).toBeNull();
    const boxes = within(dialog).getAllByRole<HTMLInputElement>('checkbox');
    expect(boxes).toHaveLength(3);
    expect(boxes.every((item) => item.disabled)).toBe(true);
    expect(within(dialog).getByRole<HTMLButtonElement>('button', { name: 'Clean Up' }).disabled).toBe(true);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(calls('records_cleanup_apply')).toHaveLength(0);
  });

  it('requires another preview if the history changed and never retries deletion on its own', async () => {
    await openStorageSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'records_cleanup_apply') throw 'clipboard_cleanup_changed';
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Manual Cleanup' }));
    const dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('checkbox', { name: /^Ordinary/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Clean Up' }));
    await screen.findByText(/History changed/);
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(calls('records_cleanup_preview')).toHaveLength(1);
    expect(calls('records_cleanup_apply')).toHaveLength(1);
  });

  it('offers four mutually exclusive search rules and keeps all scopes active without extra scope checkboxes', async () => {
    await openSettings();
    expect(screen.queryByRole('group', { name: 'Search Scope' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Search Rule: Exact Match' }));
    expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual(['Exact Match', 'Fuzzy Match', 'Regular Expression', 'Mixed Match']);
    fireEvent.click(screen.getByRole('option', { name: 'Fuzzy Match' }));
    const fuzzy = await screen.findByRole('button', { name: 'Search Rule: Fuzzy Match' });
    expect(calls('clipboard_search_set').at(-1)?.[1]).toEqual({ search: { mode: 'fuzzy', body: true, note: true, imageText: true } });
    fireEvent.click(fuzzy);
    fireEvent.click(screen.getByRole('option', { name: 'Regular Expression' }));
    await screen.findByRole('button', { name: 'Search Rule: Regular Expression' });
    expect(calls('clipboard_search_set').at(-1)?.[1]).toEqual({ search: { mode: 'regex', body: true, note: true, imageText: true } });
    fireEvent.click(screen.getByRole('button', { name: 'Search Rule: Regular Expression' }));
    fireEvent.click(screen.getByRole('option', { name: 'Mixed Match' }));
    await screen.findByRole('button', { name: 'Search Rule: Mixed Match' });
    expect(calls('clipboard_search_set').at(-1)?.[1]).toEqual({ search: { mode: 'mixed', body: true, note: true, imageText: true } });
  });

  it('toggles clipboard recording in Settings, manages custom UTI/regex ignore rules, and configures launch at login and preview lines under General', async () => {
    const types: string[] = [];
    const regexps: string[] = [];
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_ignored_type_add') {
        const value = (args as { pasteboardType: string }).pasteboardType;
        if (value.includes(' ')) throw 'clipboard_invalid_pasteboard_type';
        types.push(value);
        return [...types];
      }
      if (name === 'clipboard_ignored_type_remove') {
        const value = (args as { pasteboardType: string }).pasteboardType;
        const idx = types.indexOf(value);
        if (idx >= 0) types.splice(idx, 1);
        return [...types];
      }
      if (name === 'clipboard_ignored_regex_add') {
        const value = (args as { pattern: string }).pattern;
        if (value === '[invalid') throw 'clipboard_invalid_regex';
        regexps.push(value);
        return [...regexps];
      }
      if (name === 'clipboard_ignored_regex_remove') {
        const value = (args as { pattern: string }).pattern;
        const idx = regexps.indexOf(value);
        if (idx >= 0) regexps.splice(idx, 1);
        return [...regexps];
      }
      return invoke(name, args);
    });

    const recordToggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Record Clipboard History' });
    expect(recordToggle.checked).toBe(false);
    fireEvent.click(recordToggle);
    await waitFor(() => expect(calls('clipboard_capture_set').at(-1)?.[1]).toEqual({ enabled: true }));
    expect(recordToggle.checked).toBe(true);

    const ignoreRules = screen.getByRole('region', { name: 'Ignore Rules' });
    const addType = within(ignoreRules).getByRole('button', { name: 'Add Pasteboard Type' });
    fireEvent.click(addType);
    const typeInput = within(ignoreRules).getByRole('textbox', { name: 'Pasteboard Types' });
    const typeSection = within(typeInput.closest('.clipboard-ignore-block')! as HTMLElement);
    fireEvent.change(typeInput, { target: { value: 'bad type' } });
    fireEvent.click(typeSection.getByRole('button', { name: 'Add' }));
    await typeSection.findByRole('alert');
    fireEvent.change(typeInput, { target: { value: 'org.example.SecretType' } });
    fireEvent.click(typeSection.getByRole('button', { name: 'Add' }));
    await typeSection.findByText('org.example.SecretType');
    const removeType = typeSection.getByRole('button', { name: 'Remove org.example.SecretType' });
    removeType.focus();
    fireEvent.click(removeType);
    await waitFor(() => expect(typeSection.queryByText('org.example.SecretType')).toBeNull());
    expect(document.activeElement).toBe(addType);

    const addRegex = within(ignoreRules).getByRole('button', { name: 'Add Text Pattern' });
    fireEvent.click(addRegex);
    const regexInput = within(ignoreRules).getByRole('textbox', { name: 'Text Patterns' });
    const regexSection = within(regexInput.closest('.clipboard-ignore-block')! as HTMLElement);
    fireEvent.change(regexInput, { target: { value: '[invalid' } });
    fireEvent.click(regexSection.getByRole('button', { name: 'Add' }));
    await regexSection.findByRole('alert');
    fireEvent.change(regexInput, { target: { value: '^sk-[0-9]+$' } });
    fireEvent.click(regexSection.getByRole('button', { name: 'Add' }));
    await regexSection.findByText('^sk-[0-9]+$');
    const removeRegex = regexSection.getByRole('button', { name: 'Remove ^sk-[0-9]+$' });
    removeRegex.focus();
    fireEvent.click(removeRegex);
    await waitFor(() => expect(regexSection.queryByText('^sk-[0-9]+$')).toBeNull());
    expect(document.activeElement).toBe(addRegex);

    fireEvent.click(screen.getByRole('button', { name: 'General' }));
    const launch = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Launch at Login' });
    expect(launch.checked).toBe(false);
    fireEvent.click(launch);
    await waitFor(() => expect(calls('launch_at_login_set').at(-1)?.[1]).toEqual({ enabled: true }));
    expect(launch.checked).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Clipboard Preview Lines: 3' }));
    fireEvent.click(screen.getByRole('option', { name: '1' }));
    await screen.findByRole('button', { name: 'Clipboard Preview Lines: 1' });
    expect(calls('records_preview_lines_set').at(-1)?.[1]).toEqual({ lines: 1 });
  });

  it('saves automatic paste format independently of the row activation behavior', async () => {
    await openSettings();
    fireEvent.click(screen.getByRole('button', { name: 'Automatic Paste Format: Keep Original Formatting' }));
    fireEvent.click(screen.getByRole('option', { name: 'Paste as Plain Text' }));
    const plain = await screen.findByRole('button', { name: 'Automatic Paste Format: Paste as Plain Text' });
    expect(calls('clipboard_behavior_set').at(-1)?.[1]).toEqual({ pasteAsPlainText: true });
    expect(bridge.snapshot.clipboardControls.activation).toBe('paste');
    expect(screen.getByRole('button', { name: 'When Activating an Item: Paste into Previous App' })).toBeTruthy();
    fireEvent.click(plain);
    fireEvent.click(screen.getByRole('option', { name: 'Keep Original Formatting' }));
    await screen.findByRole('button', { name: 'Automatic Paste Format: Keep Original Formatting' });
    expect(calls('clipboard_behavior_set').at(-1)?.[1]).toEqual({ pasteAsPlainText: false });
  });

  it('saves Copy Only as the row activation behavior', async () => {
    await openSettings();
    fireEvent.click(screen.getByRole('button', { name: 'When Activating an Item: Paste into Previous App' }));
    fireEvent.click(screen.getByRole('option', { name: 'Copy Only' }));
    await screen.findByRole('button', { name: 'When Activating an Item: Copy Only' });
    expect(calls('clipboard_behavior_set')[0][1]).toEqual({ activation: 'copy' });
  });

  it('previews and applies custom MB clipboard storage space limits while preserving existing retention fields', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    let storedPolicy: { limit: number; maxAgeDays?: number; maxStorageMb?: number } = { limit: 500, maxAgeDays: 7 };
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_options_get') return { excludedApps, activation: 'paste', pasteAsPlainText: false };
      if (name === 'records_storage_info') return { bytes: 8000, ...storedPolicy };
      if (name === 'records_cleanup_preview') return { count: 2, token: 'storage-limit-token' };
      if (name === 'records_cleanup_apply') {
        const next = (args as { policy?: typeof storedPolicy }).policy;
        if (next) storedPolicy = next;
        return { removed: 2, mediaCleanupFailed: false };
      }
      return invoke(name, args);
    });
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Storage' }));
    const storageLimit = await screen.findByRole<HTMLInputElement>('textbox', { name: 'Storage Limit' });
    expect(storageLimit.value).toBe('0');
    expect(storageLimit.closest('.settings-history-limit')?.querySelector('.settings-input-unit')?.textContent).toBe('MB');
    const storageLimitTip = screen.getByRole('button', { name: 'Storage Limit info' });
    fireEvent.mouseEnter(storageLimitTip);
    expect(screen.getByRole('tooltip').textContent).toContain('0 means no limit');
    fireEvent.mouseLeave(storageLimitTip);
    expect(screen.queryByRole('tooltip')).toBeNull();

    for (const invalid of ['-1', '12.5', '100001']) {
      fireEvent.change(storageLimit, { target: { value: invalid } });
      expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
      expect(storageLimit.getAttribute('aria-invalid')).toBe('true');
    }

    vi.useFakeTimers();
    try {
      fireEvent.change(storageLimit, { target: { value: '300' } });
      expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
      const dialog = screen.getByRole('alertdialog', { name: 'Change history retention?' });
      fireEvent.click(within(dialog).getByRole('button', { name: /Apply & Remove/i }));
      await act(async () => { await Promise.resolve(); });
      expect(storageLimit.value).toBe('300');
      expect(calls('records_cleanup_apply').at(-1)?.[1]).toEqual({
        policy: { limit: 500, maxAgeDays: 7, maxStorageMb: 300, autoCleanupIntervalHours: 1 },
        token: 'storage-limit-token',
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('edits pre-populated pasteboard types inline with 2s inactivity auto-save and toggles privacy presets', async () => {
    const invoke = bridge.invoke.getMockImplementation()!;
    let types = [
      'Pasteboard generator type',
      'com.agilebits.onepassword',
      'com.typeit4me.clipping',
      'de.petermaurer.TransientPasteboardType',
      'net.antelle.keeweb',
    ];
    let ignoreSecrets = false;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_options_get') {
        return { excludedApps, ignoredPasteboardTypes: [...types], ignoredRegexps: [], ignoreSecrets, activation: 'paste', pasteAsPlainText: false };
      }
      if (name === 'clipboard_ignored_type_update') {
        const { oldPasteboardType, newPasteboardType } = args as { oldPasteboardType: string; newPasteboardType: string };
        types = types.map((entry) => entry === oldPasteboardType ? newPasteboardType : entry);
        return [...types];
      }
      if (name === 'clipboard_ignored_type_add') {
        const { pasteboardType } = args as { pasteboardType: string };
        if (!types.includes(pasteboardType)) types = [...types, pasteboardType];
        return [...types];
      }
      if (name === 'clipboard_ignored_type_remove') {
        const { pasteboardType } = args as { pasteboardType: string };
        types = types.filter((entry) => entry !== pasteboardType);
        return [...types];
      }
      if (name === 'clipboard_ignore_secrets_set') {
        ignoreSecrets = Boolean((args as { enabled: boolean }).enabled);
        return ignoreSecrets;
      }
      return invoke(name, args);
    });

    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const ignoreRules = await screen.findByRole('region', { name: 'Ignore Rules' });
    await within(ignoreRules).findByText('Pasteboard generator type');
    await within(ignoreRules).findByText('com.agilebits.onepassword');

    // Inline edit a pre-populated type without a Save button; auto-saves after 2s inactivity.
    vi.useFakeTimers();
    try {
      fireEvent.click(within(ignoreRules).getByRole('button', { name: 'Edit com.agilebits.onepassword' }));
      const editInput = within(ignoreRules).getByRole<HTMLInputElement>('textbox', { name: 'Edit com.agilebits.onepassword' });
      expect(within(ignoreRules).queryByRole('button', { name: 'Save' })).toBeNull();
      fireEvent.change(editInput, { target: { value: 'com.custom.vault' } });
      expect(within(ignoreRules).queryByRole('button', { name: 'Save' })).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(1999); });
      expect(calls('clipboard_ignored_type_update')).toHaveLength(0);
      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(within(ignoreRules).getByText('com.custom.vault')).toBeTruthy();
      expect(calls('clipboard_ignored_type_update').at(-1)?.[1]).toEqual({
        oldPasteboardType: 'com.agilebits.onepassword',
        newPasteboardType: 'com.custom.vault',
      });
    } finally {
      vi.useRealTimers();
    }

    // Toggle Universal Clipboard adds/removes com.apple.is-remote-clipboard.
    const universal = within(ignoreRules).getByRole<HTMLInputElement>('checkbox', { name: 'Ignore Universal Clipboard' });
    expect(universal.checked).toBe(false);
    fireEvent.click(universal);
    await waitFor(() => expect(universal.checked).toBe(true));
    expect(calls('clipboard_ignored_type_add').at(-1)?.[1]).toEqual({ pasteboardType: 'com.apple.is-remote-clipboard' });
    await within(ignoreRules).findByText('com.apple.is-remote-clipboard');
    fireEvent.click(universal);
    await waitFor(() => expect(universal.checked).toBe(false));
    expect(calls('clipboard_ignored_type_remove').at(-1)?.[1]).toEqual({ pasteboardType: 'com.apple.is-remote-clipboard' });

    // Toggle built-in sensitive credential interception.
    const secrets = within(ignoreRules).getByRole<HTMLInputElement>('checkbox', { name: 'Ignore Sensitive Credentials' });
    expect(secrets.checked).toBe(false);
    fireEvent.click(secrets);
    await waitFor(() => expect(secrets.checked).toBe(true));
    expect(calls('clipboard_ignore_secrets_set').at(-1)?.[1]).toEqual({ enabled: true });
  });

  it('toggles auto-open preview, shows range tooltip, and auto-saves valid preview delay after 2s inactivity', async () => {
    await openSettings();
    const behavior = screen.getByRole('region', { name: 'Behavior' });
    const toggle = within(behavior).getByRole<HTMLInputElement>('checkbox', { name: 'Auto-open Preview' });
    const delay = within(behavior).getByRole<HTMLInputElement>('textbox', { name: 'Preview Delay' });
    const tip = within(behavior).getByRole('button', { name: 'Preview Delay info' });
    expect(toggle.checked).toBe(false);
    expect(delay.value).toBe('1000');
    expect(delay.disabled).toBe(true);

    // Range tip sits beside the Preview Delay label and opens on hover/click.
    expect(tip.closest('.settings-shortcut-name')?.querySelector('label')?.textContent).toBe('Preview Delay');
    fireEvent.mouseEnter(tip);
    expect(screen.getByRole('tooltip').textContent).toContain('0–5000 ms');
    fireEvent.mouseLeave(tip);
    expect(screen.queryByRole('tooltip')).toBeNull();

    fireEvent.click(toggle);
    await waitFor(() => expect(toggle.checked).toBe(true));
    expect(calls('clipboard_behavior_set').at(-1)?.[1]).toEqual({ hoverPreviewEnabled: true });
    expect(delay.disabled).toBe(false);

    // Editing never renders a Save button and waits 2s of inactivity before saving.
    vi.useFakeTimers();
    try {
      const beforeEdits = calls('clipboard_behavior_set').length;
      fireEvent.change(delay, { target: { value: '0' } });
      expect(within(behavior).queryByRole('button', { name: 'Save' })).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(1999); });
      expect(calls('clipboard_behavior_set')).toHaveLength(beforeEdits);
      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(calls('clipboard_behavior_set').at(-1)?.[1]).toEqual({ hoverPreviewDelayMs: 0 });

      fireEvent.change(delay, { target: { value: '45' } });
      await act(async () => { await vi.advanceTimersByTimeAsync(1200); });
      fireEvent.change(delay, { target: { value: '450' } });
      await act(async () => { await vi.advanceTimersByTimeAsync(1999); });
      expect(calls('clipboard_behavior_set').at(-1)?.[1]).toEqual({ hoverPreviewDelayMs: 0 });
      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(calls('clipboard_behavior_set').at(-1)?.[1]).toEqual({ hoverPreviewDelayMs: 450 });

      // Out-of-range values mark the input invalid without saving or shifting layout, and blur restores the last saved value.
      const countBeforeInvalid = calls('clipboard_behavior_set').length;
      fireEvent.change(delay, { target: { value: '5001' } });
      expect(delay.getAttribute('aria-invalid')).toBe('true');
      expect(within(behavior).queryByRole('alert')).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
      expect(calls('clipboard_behavior_set')).toHaveLength(countBeforeInvalid);
      fireEvent.blur(delay);
      expect(delay.value).toBe('450');
      expect(delay.getAttribute('aria-invalid')).toBeNull();
    } finally {
      vi.useRealTimers();
    }

    // Disabling keeps the saved delay visible while locking the field.
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle.checked).toBe(false));
    expect(calls('clipboard_behavior_set').at(-1)?.[1]).toEqual({ hoverPreviewEnabled: false });
    expect(delay.value).toBe('450');
    expect(delay.disabled).toBe(true);
  });

  it('uses consistent hover tips across settings pages and never toggles checkboxes from row text clicks', async () => {
    await openSettings();
    // Clicking row text in Clipboard settings does not toggle checkboxes.
    const autoPreview = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Auto-open Preview' });
    fireEvent.click(screen.getByText('Auto-open Preview'));
    expect(autoPreview.checked).toBe(false);
    const universal = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Ignore Universal Clipboard' });
    fireEvent.click(screen.getByText('Ignore Universal Clipboard'));
    expect(universal.checked).toBe(false);

    // Unified info tips expose concise one-sentence-per-line explanations on hover and dismiss on mouse leave.
    for (const [tipName, snippet, expectedLines] of [
      ['Images info', 'single image files', 1],
      ['Files info', 'Original files', 2],
      ['Apps info', 'active app only', 1],
      ['Search Rule info', 'Mixed tries Exact', 3],
      ['Automatic Paste Format info', 'auto-pasted text only', 2],
    ] as const) {
      const tip = screen.getByRole('button', { name: tipName });
      fireEvent.mouseEnter(tip);
      const panel = screen.getByRole('tooltip');
      expect(panel.textContent).toContain(snippet);
      if (expectedLines > 1) expect(panel.querySelectorAll('.clipboard-tip-lines > div')).toHaveLength(expectedLines);
      fireEvent.mouseLeave(tip);
      expect(screen.queryByRole('tooltip')).toBeNull();
    }

    // Storage page renders the 5-minute tip without a manual refresh button, per-type breakdown, stacked proportion bar, and live updates.
    fireEvent.click(screen.getByRole('button', { name: 'Storage' }));
    await screen.findByText('4 kB');
    expect(screen.queryByRole('button', { name: 'Refresh Storage Estimate' })).toBeNull();
    const storageTip = screen.getByRole('button', { name: 'Estimated Storage info' });
    fireEvent.mouseEnter(storageTip);
    expect(screen.getByRole('tooltip').textContent).toContain('every 5 minutes');
    fireEvent.mouseLeave(storageTip);
    expect(screen.queryByRole('tooltip')).toBeNull();

    // Storage Location keeps only Data Directory and Log Directory (no standalone Clear Cache row), with multiline tips.
    const locationRegion = screen.getByRole('region', { name: 'Storage Location' });
    expect(locationRegion.textContent).not.toContain('Runtime logs for troubleshooting');
    expect(within(locationRegion).queryByRole('button', { name: 'Clear Cache' })).toBeNull();
    for (const [tipName, snippet, minLines] of [
      ['Log Directory info', 'Runtime logs', 1],
      ['Storage Limit info', '0 means no limit', 2],
      ['Auto Cleanup Interval info', 'stale thumbnails', 3],
    ] as const) {
      const tip = screen.getByRole('button', { name: tipName });
      fireEvent.mouseEnter(tip);
      const panel = screen.getByRole('tooltip');
      expect(panel.textContent).toContain(snippet);
      if (minLines > 1) expect(panel.querySelectorAll('.clipboard-tip-lines > div').length).toBeGreaterThanOrEqual(minLines);
      fireEvent.mouseLeave(tip);
      expect(screen.queryByRole('tooltip')).toBeNull();
    }

    const usageRegion = screen.getByRole('region', { name: 'Storage Usage' });
    expect(usageRegion.querySelectorAll('.storage-usage-segment')).toHaveLength(3);
    expect(within(usageRegion).getByText('12 items')).toBeTruthy();
    expect(within(usageRegion).getByText('1.5 kB')).toBeTruthy();
    expect(within(usageRegion).getByText('2 items')).toBeTruthy();
    expect(within(usageRegion).getByText('2 kB')).toBeTruthy();
    expect(within(usageRegion).getByText('1 item')).toBeTruthy();
    expect(within(usageRegion).getByText('500 byte')).toBeTruthy();

    // Periodic 5-minute storage updates from the tray loop sync total and breakdown live.
    await emit('records_storage_changed', {
      bytes: 12_400_000,
      breakdown: {
        text: { bytes: 2_400_000, count: 20 },
        image: { bytes: 9_000_000, count: 3 },
        files: { bytes: 1_000_000, count: 1 },
      },
    });
    expect(within(usageRegion).getByText('12.4 MB')).toBeTruthy();
    expect(within(usageRegion).getByText('20 items')).toBeTruthy();
    expect(within(usageRegion).getByText('9 MB')).toBeTruthy();

    // General, Translation, HotKey, and Privacy & data share the same rules.
    fireEvent.click(screen.getByRole('button', { name: 'General' }));
    const launch = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Launch at Login' });
    fireEvent.click(screen.getByText('Launch at Login'));
    expect(launch.checked).toBe(false);

    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const preview = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Preview' });
    fireEvent.click(screen.getByText('Preview'));
    expect(preview.checked).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Privacy & data' }));
    expect(screen.queryByText(/Not granted; required/)).toBeNull();
    const accessTip = screen.getByRole('button', { name: 'Accessibility info' });
    fireEvent.mouseEnter(accessTip);
    expect(screen.getByRole('tooltip').textContent).toContain('selection translation and automatic paste');
    fireEvent.mouseLeave(accessTip);
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('opens directly to Storage from initialSettingsPage and navigates on settings_page_requested', async () => {
    bridge.snapshot.initialSettingsPage = 'storage';
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'records_storage_info') return { bytes: 4000, limit: 500, breakdown: defaultBreakdown };
      return invoke(name, args);
    });
    await mount('settings');
    expect(screen.getByRole('heading', { level: 1, name: 'Storage' })).toBeTruthy();
    await screen.findByText('4 kB');
    fireEvent.click(screen.getByRole('button', { name: 'General' }));
    expect(screen.getByRole('heading', { level: 1, name: 'General' })).toBeTruthy();
    await emit('settings_page_requested', 'storage');
    expect(screen.getByRole('heading', { level: 1, name: 'Storage' })).toBeTruthy();
  });

  it('manages storage data directory and log directory with clear feedback and error handling', async () => {
    await openStorageSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'storage_open_data_dir' || name === 'storage_open_log_dir') return undefined;
      if (name === 'storage_data_dir_pick') {
        return {
          dataDir: '/Users/synthetic/NamiData',
          defaultDataDir: '/Users/synthetic/Library/Application Support/dev.local.nami',
          isCustomDataDir: true,
          logDir: '/Users/synthetic/Library/Logs/dev.local.nami',
        };
      }
      if (name === 'storage_data_dir_reset') {
        return {
          dataDir: '/Users/synthetic/Library/Application Support/dev.local.nami',
          defaultDataDir: '/Users/synthetic/Library/Application Support/dev.local.nami',
          isCustomDataDir: false,
          logDir: '/Users/synthetic/Library/Logs/dev.local.nami',
        };
      }
      return invoke(name, args);
    });

    const locationRegion = screen.getByRole('region', { name: 'Storage Location' });
    const retentionRegion = screen.getByRole('region', { name: 'Retention & Cleanup' });
    expect(locationRegion.querySelectorAll('.clipboard-setting-icon')).toHaveLength(0);
    expect(retentionRegion.querySelectorAll('.clipboard-setting-icon')).toHaveLength(0);
    expect(locationRegion.querySelectorAll('.settings-field')).toHaveLength(2);
    expect(within(locationRegion).queryByRole('button', { name: 'Clear Cache' })).toBeNull();
    const locationItems = locationRegion.querySelectorAll('.storage-location-item');
    expect(locationItems).toHaveLength(2);
    expect(locationItems[0].querySelector('.storage-location-detail .storage-location-path')?.textContent).toBe('/Users/synthetic/Library/Application Support/dev.local.nami');
    expect(locationItems[0].querySelectorAll('.storage-location-detail .storage-location-actions button')).toHaveLength(3);
    expect(locationItems[1].querySelector('.storage-location-detail .storage-location-path')?.textContent).toBe('/Users/synthetic/Library/Logs/dev.local.nami');
    expect(locationItems[1].querySelectorAll('.storage-location-detail .storage-location-actions button')).toHaveLength(1);
    const resetBtn = within(locationRegion).getByRole<HTMLButtonElement>('button', { name: 'Reset to Default' });
    expect(resetBtn.disabled).toBe(true);

    // Open data & log folders via icon buttons.
    fireEvent.click(within(locationRegion).getByRole('button', { name: 'Open Folder' }));
    await waitFor(() => expect(calls('storage_open_data_dir')).toHaveLength(1));
    const openLogBtn = within(locationRegion).getByRole('button', { name: 'Open Log Folder' });
    expect(openLogBtn.textContent).toBe('');
    expect(openLogBtn.querySelector('svg')).toBeTruthy();
    fireEvent.click(openLogBtn);
    await waitFor(() => expect(calls('storage_open_log_dir')).toHaveLength(1));

    // Change data folder -> enables Reset to Default and updates displayed path.
    fireEvent.click(within(locationRegion).getByRole('button', { name: 'Change Folder' }));
    await within(locationRegion).findByText('/Users/synthetic/NamiData');
    expect(within(locationRegion).getByRole('status').textContent).toContain('Data directory updated and migrated.');
    expect(resetBtn.disabled).toBe(false);

    // Reset back to default folder.
    fireEvent.click(resetBtn);
    await within(locationRegion).findByText('/Users/synthetic/Library/Application Support/dev.local.nami');
    expect(within(locationRegion).getByRole('status').textContent).toContain('Restored default data directory.');
    expect(resetBtn.disabled).toBe(true);
    expect(calls('records_storage_info').every(([, args]) => args === undefined)).toBe(true);

    // Migration error surfaces specific alert.
    bridge.invoke.mockImplementationOnce(() => Promise.reject('storage_dir_not_empty'));
    fireEvent.click(within(locationRegion).getByRole('button', { name: 'Change Folder' }));
    expect((await within(locationRegion).findByRole('alert')).textContent).toContain('already contains Nami data files');
  });
});
