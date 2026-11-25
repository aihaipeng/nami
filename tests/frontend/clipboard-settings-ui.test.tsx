// @vitest-environment jsdom
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
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
    prefs.shortcuts.focusSearch = { code: 'KeyJ', modifiers: ['alt'] };
    prefs.shortcuts.translate = null;
    prefs.selectionEnabled = false;
    prefs.disabledShortcuts = ['focusSearch', 'translate'];
    records.note = { binding: null, enabled: false };
    records.preview = { binding: null, enabled: false };
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const restore = (name: string) => within(screen.getByRole('region', { name })).getByRole('button', { name: 'Restore Default Shortcuts' });
    fireEvent.click(restore('General'));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Focus Input shortcut' }).textContent).toBe('⌘F'));
    expect(screen.getByRole('checkbox', { name: 'Focus Input' })).toHaveProperty('checked', true);
    expect(screen.getByRole('checkbox', { name: 'Add/Remove Note' })).toHaveProperty('checked', true);
    expect(records.note.binding?.code).toBe('KeyN');
    expect(records.preview).toEqual({ binding: { code: 'KeyY', modifiers: ['meta'] }, enabled: true });
    expect(prefs.shortcuts.translate).toBeNull();
    expect(prefs.selectionEnabled).toBe(false);
    expect(prefs.disabledShortcuts).toEqual(['translate']);
    fireEvent.click(restore('Translation'));
    await waitFor(() => expect(prefs.shortcuts.translate).toEqual({ code: 'Enter', modifiers: ['meta'] }));
    expect(screen.getByRole('checkbox', { name: 'Open Translation' })).toHaveProperty('checked', true);
    expect(prefs.disabledShortcuts).toEqual([]);
    expect(records.preview).toEqual({ binding: { code: 'KeyY', modifiers: ['meta'] }, enabled: true });
  });

  it('shows the default Control and backquote shortcut', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    expect([...screen.getByRole('button', { name: 'Open Clipboard History shortcut' }).closest('.settings-shortcut-row')!.querySelectorAll('.keycaps kbd')].map((key) => key.textContent)).toEqual(['⌃', '`']);
  });

  it('uses text-only headings and plain action labels in clipboard settings', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect(screen.getAllByRole('heading').every((heading) => !heading.querySelector('svg'))).toBe(true);
    expect(screen.getByRole('button', { name: 'Add App' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Clear Records' })).toBeTruthy();
  });

  it('groups global and clipboard shortcuts and keeps them out of the Clipboard settings page', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    expect(screen.getByRole('heading', { level: 1, name: 'HotKey' })).toBeTruthy();
    expect(screen.getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent)).toEqual(['General', 'Clipboard', 'Translation']);
    const general = within(screen.getByRole('group', { name: 'General' }));
    expect(general.getAllByRole<HTMLInputElement>('checkbox').map((toggle) => toggle.labels?.[0].textContent)).toEqual([
      'Focus Input', 'Preview', 'Copy', 'Favorite/Unfavorite', 'Pin/Unpin', 'Add/Remove Note', 'Delete', 'Pin / Unpin Window', 'Show Shortcuts', 'Open Settings',
    ]);
    for (const retired of ['Switch Record View', 'Open Translation History', 'Focus Record List', 'Navigate Up', 'Navigate Down', 'Previous Category', 'Next Category', 'Paste (Context Menu)']) {
      expect(screen.queryByRole('checkbox', { name: retired })).toBeNull();
    }
    const clipboard = within(screen.getByRole('group', { name: 'Clipboard' }));
    const clipboardOrder = ['Open Clipboard History', 'Translate', 'Open Link, Email or Locate File', 'Extract Text', 'Paste as Plain Text', 'Save Image'];
    expect(clipboard.getAllByRole<HTMLInputElement>('checkbox').map((toggle) => toggle.labels?.[0].textContent)).toEqual(clipboardOrder);
    for (const name of clipboardOrder) {
      expect(clipboard.getByRole('button', { name: `${name} shortcut` })).toBeTruthy();
      expect(clipboard.getByRole<HTMLInputElement>('checkbox', { name }).checked).toBe(true);
    }
    const translation = within(screen.getByRole('group', { name: 'Translation' }));
    expect(translation.getAllByRole<HTMLInputElement>('checkbox').map((toggle) => toggle.labels?.[0].textContent)).toEqual([
      'Autofill Input', 'Open Translation', 'Translate', 'Copy Primary Translation', 'Copy Secondary Translation',
    ]);
    await emit('locale_changed', 'zh-CN');
    expect(screen.getByRole('checkbox', { name: '打开翻译' })).toBeTruthy();
    await emit('locale_changed', 'en');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect(screen.queryByRole('button', { name: 'Open Clipboard History shortcut' })).toBeNull();
  });

  it('rejects a clipboard binding already used by a general action and preserves both', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const general = within(screen.getByRole('group', { name: 'General' }));
    const search = general.getByRole('button', { name: 'Focus Input shortcut' });
    const preview = screen.getByRole('button', { name: 'Preview shortcut' });
    fireEvent.click(preview);
    fireEvent.keyDown(window, { key: 'f', code: 'KeyF', metaKey: true });
    expect((await screen.findByRole('alert')).textContent).toContain('already assigned');
    expect(preview.textContent).toBe('⌘Y');
    expect(search.textContent).toBe('⌘F');
    fireEvent.click(search);
    fireEvent.keyDown(window, { key: 'y', code: 'KeyY', metaKey: true });
    expect((await screen.findByRole('alertdialog')).textContent).toContain('already assigned');
    expect(search.textContent).toBe('⌘F');
  });

  it('switches the recorder between matching labels and releases capture on navigation', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const clipboard = within(screen.getByRole('group', { name: 'Clipboard' })).getByRole('button', { name: 'Translate shortcut' });
    const translation = within(screen.getByRole('group', { name: 'Translation' })).getByRole('button', { name: 'Translate shortcut' });
    await user.click(clipboard);
    expect(clipboard.getAttribute('aria-pressed')).toBe('true');
    await user.click(translation);
    expect(clipboard.getAttribute('aria-pressed')).toBe('false');
    expect(translation.getAttribute('aria-pressed')).toBe('true');
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', metaKey: true });
    await waitFor(() => expect(translation.textContent).toBe('⌘J'));
    expect(calls('clipboard_record_shortcut_set')).toHaveLength(0);
    expect(calls('translation_shortcut_set').at(-1)?.[1]).toEqual({ action: 'translate', binding: { code: 'KeyJ', modifiers: ['meta'] } });
    fireEvent.click(clipboard);
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
    expect(screen.getAllByRole('button', { name: '恢复默认快捷键' })).toHaveLength(3);
    for (const name of ['备注/取消备注', '收藏/取消收藏', '置顶/取消置顶']) {
      expect(screen.getByRole('checkbox', { name })).toBeTruthy();
      expect(screen.getByRole('button', { name: `${name} 快捷键` })).toBeTruthy();
    }
  });

  it.each([['Preview', 'preview', '⌘Y'], ['Add/Remove Note', 'note', '⇧⌘N'], ['Copy', 'copyRecord', '⇧⌘C'], ['Extract Text', 'ocr', '⇧⌘O'], ['Translate', 'translate', '⌘T'], ['Favorite/Unfavorite', 'favorite', '⌥F'], ['Delete', 'delete', '⌘⌫'], ['Paste as Plain Text', 'pastePlain', '⌘↵'], ['Save Image', 'saveImage', '⌘S']])(
    'edits %s while unchecked and scopes Clipboard restore to its own group', async (title, action, defaultKeys) => {
      const user = userEvent.setup(); await mount('settings');
      await user.click(screen.getByRole('button', { name: 'HotKey' }));
      const local = ['ocr', 'translate', 'pastePlain', 'saveImage'].includes(action);
      const group = within(screen.getByRole('group', { name: local ? 'Clipboard' : 'General' }));
      const recorder = group.getByRole('button', { name: `${title} shortcut` });
      const toggle = group.getByRole<HTMLInputElement>('checkbox', { name: title });
      await user.click(toggle);
      expect(calls('clipboard_record_shortcut_set').at(-1)?.[1]).toEqual({ action, enabled: false });
      await user.click(recorder);
      fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', altKey: true });
      await waitFor(() => expect(recorder.textContent).toBe('⌥J'));
      expect(toggle.checked).toBe(false);
      await user.click(within(screen.getByRole('region', { name: 'Clipboard' })).getByRole('button', { name: 'Restore Default Shortcuts' }));
      await waitFor(() => expect(recorder.textContent).toBe(local ? defaultKeys : '⌥J'));
      expect(toggle.checked).toBe(local);
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

  it('waits for a main key and can explicitly save a combination or a modifier alone', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const recorder = screen.getByRole('button', { name: 'Copy shortcut' });
    fireEvent.click(recorder);
    // A modifier-tap event emitted when WebView misses the main key must not replace the binding.
    fireEvent.keyDown(recorder, { key: 'Meta', code: 'MetaLeft' });
    expect(recorder.textContent).toBe('Press shortcut');
    expect(calls('clipboard_record_shortcut_set')).toHaveLength(0);
    fireEvent.keyDown(recorder, { key: 'c', code: 'KeyC', metaKey: true });
    await waitFor(() => {
      expect(recorder.textContent).toBe('⌘C');
      expect(recorder.getAttribute('aria-disabled')).toBeNull();
    });

    function choose() {
      fireEvent.click(recorder);
      fireEvent.click(screen.getByRole('button', { name: 'Choose Keys' }));
      return within(screen.getByRole('dialog', { name: 'Choose Keys' }));
    }
    let manual = choose();
    fireEvent.change(manual.getByRole('combobox', { name: 'Key' }), { target: { value: 'KeyC' } });
    fireEvent.click(manual.getByRole('checkbox', { name: '⌘ Command' }));
    fireEvent.click(manual.getByRole('button', { name: 'Save' }));
    await waitFor(() => {
      expect(calls('clipboard_record_shortcut_set')).toHaveLength(2);
      expect(recorder.getAttribute('aria-disabled')).toBeNull();
    });
    expect(bridge.snapshot.clipboardControls.shortcuts.copyRecord.binding).toEqual({ code: 'KeyC', modifiers: ['meta'] });

    manual = choose();
    fireEvent.change(manual.getByRole('combobox', { name: 'Key' }), { target: { value: 'MetaLeft' } });
    fireEvent.click(manual.getByRole('button', { name: 'Save' }));
    await waitFor(() => {
      expect(recorder.textContent).toBe('Left ⌘');
      expect(recorder.getAttribute('aria-disabled')).toBeNull();
    });
    expect(bridge.snapshot.clipboardControls.shortcuts.copyRecord.binding).toEqual({ code: 'MetaLeft', modifiers: [] });
    fireEvent.click(choose().getByRole('button', { name: 'Cancel' }));
    expect(recorder.textContent).toBe('Left ⌘');
    expect(calls('clipboard_record_shortcut_set')).toHaveLength(3);
  });

  it('reuses a chord across Clipboard and Translation, but rejects it for a General action', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const group = (name: string) => within(screen.getByRole('group', { name }));
    const record = (button: HTMLElement, key: { key: string; code: string; metaKey: boolean }) => {
      fireEvent.click(button);
      fireEvent.keyDown(window, key);
    };
    const plain = group('Clipboard').getByRole('button', { name: 'Paste as Plain Text shortcut' });
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
    const preview = group('General').getByRole('button', { name: 'Preview shortcut' });
    record(preview, { key: 'Enter', code: 'Enter', metaKey: true });
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(preview.textContent).toBe('⌘Y');
  });

  it('keeps panel shortcut changes separate and rejects a collision with another panel action', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const pin = screen.getByRole('button', { name: 'Pin/Unpin shortcut' });
    const toggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Pin/Unpin' });
    await user.click(toggle);
    await user.click(pin);
    fireEvent.keyDown(window, { key: 'f', code: 'KeyF', altKey: true });
    expect(await screen.findByText('This key combination is already assigned to a shortcut in the same scope.')).toBeTruthy();
    expect(pin.textContent).toBe('⌥P');
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

  it('toggles content checkboxes from the row or keyboard and preserves them when recording resumes', async () => {
    const user = userEvent.setup();
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const image = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Images' });
    expect(image.checked).toBe(true);
    expect(image.matches(':disabled')).toBe(false);
    await user.click(screen.getByText('Images', { exact: true }));
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
    const shortcuts = screen.getByRole('region', { name: 'Clipboard' });
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
      if (name === 'records_storage_info') return { bytes: 4000, limit: 500 };
      if (name === 'records_cleanup_preview') return { count: 4, token: 'preview-token', ...(!(args as { policy: unknown }).policy && { categories: [1, 2, 2] }) };
      if (name === 'records_cleanup_apply') return { removed: (args as { categories?: number }).categories === 6 ? 3 : 4, mediaCleanupFailed: false };
      return invoke(name, args);
    });
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    await screen.findByText('4 kB');
  }

  it('groups apps, pasteboard types, and text patterns under one Ignore Rules heading with compact chips and on-demand inputs', async () => {
    await openSettings();
    for (const name of ['Clipboard Recording', 'Ignore Rules', 'History & Storage', 'Search', 'Behavior']) {
      expect(screen.getByRole('region', { name })).toBeTruthy();
    }
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

  it('defaults to 500, has no age rule, and confirms a lower custom count before removing items', async () => {
    await openSettings();
    const limit = screen.getByRole<HTMLInputElement>('textbox', { name: 'History Limit' });
    expect(limit.value).toBe('500');
    expect(screen.queryByText('Keep History For')).toBeNull();
    expect(limit.getAttribute('aria-description')).toBe('Maximum number of unprotected items to keep.');
    for (const value of ['0', '5.5', '1000001']) {
      fireEvent.change(limit, { target: { value } });
      expect(screen.queryByRole('button', { name: 'Save' })).toBeNull();
    }
    fireEvent.change(limit, { target: { value: '137' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('permanently removes 4 unprotected items');
    expect(calls('records_cleanup_preview')[0][1]).toEqual({ policy: { limit: 137 } });
    const cancel = within(dialog).getByRole('button', { name: 'Cancel' });
    expect(cancel).toBe(document.activeElement);
    expect(cancel.textContent).toBe('CancelEsc');
    fireEvent.keyDown(cancel, { key: 'Escape' });
    expect(limit.value).toBe('500');
    expect(calls('records_cleanup_apply')).toHaveLength(0);
  });

  it('saves a clipboard limit without dialog when the preview affects no items', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    let limit = 500;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      const request = args as { policy?: { limit: number } };
      if (name === 'records_storage_info') return { bytes: 1900, limit };
      if (name === 'records_cleanup_preview') return { count: 0, token: 'empty-preview' };
      if (name === 'records_cleanup_apply') { limit = request.policy!.limit; return { removed: 0, mediaCleanupFailed: false }; }
      return invoke(name, args);
    });
    fireEvent.change(screen.getByRole('textbox', { name: 'History Limit' }), { target: { value: '713' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls('records_cleanup_apply')[0][1]).toEqual({ policy: { limit: 713 }, token: 'empty-preview' }));
    expect(screen.queryByRole('alertdialog')).toBeNull();
  });

  it('confirms changing the clipboard time limit without changing its count limit', async () => {
    await openSettings();
    const label = 'Unprotected Item Retention: No Time Limit';
    fireEvent.click(screen.getByRole('button', { name: label }));
    fireEvent.click(screen.getByRole('option', { name: '7 Days' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Change history retention?' });
    expect(dialog.textContent).toContain('4 unprotected items');
    expect(calls('records_cleanup_apply')).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Apply & Remove' }));
    await waitFor(() => expect(calls('records_cleanup_apply').at(-1)?.[1]).toEqual({ policy: { limit: 500, maxAgeDays: 7 }, token: 'preview-token' }));
  });

  it('requires confirmation for recent cleanup and sends only the previewed cutoff', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    const recentSince = Date.now() - 900_000;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => name === 'records_cleanup_preview' ? { count: 2, token: 'recent-token', recentSince } : invoke(name, args));
    fireEvent.click(screen.getByRole('button', { name: 'Review' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Clear the last 15 minutes?' });
    expect(dialog.textContent).toContain('2 unprotected items');
    expect(within(dialog).queryByRole('checkbox')).toBeNull();
    expect(calls('records_cleanup_preview').at(-1)?.[1]).toEqual({ policy: null, recent: true });
    expect(calls('records_cleanup_apply')).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Clear Records' }));
    await waitFor(() => expect(calls('records_cleanup_apply').at(-1)?.[1]).toEqual({ policy: null, token: 'recent-token', recentSince }));
  });

  it('clears only the selected clipboard categories by keyboard and refreshes its size', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'records_storage_info') return { bytes: 2000, limit: 500 };
      return invoke(name, args);
    });
    const trigger = screen.getByRole('button', { name: 'Clear Records' });
    expect(trigger.parentElement?.contains(screen.getByRole('heading', { name: 'History & Storage' }))).toBe(true);
    fireEvent.click(trigger);
    const dialog = await screen.findByRole('alertdialog', { name: 'Clear Records' });
    const confirm = within(dialog).getByRole<HTMLButtonElement>('button', { name: 'Clear Records' });
    const checkbox = (name: RegExp) => within(dialog).getByRole<HTMLInputElement>('checkbox', { name });
    const options = within(dialog).getAllByRole<HTMLInputElement>('checkbox');
    expect(options).toHaveLength(3);
    expect(options.every((item) => !item.checked)).toBe(true);
    expect(dialog.querySelector('p')).toBeNull();
    expect(dialog.hasAttribute('aria-describedby')).toBe(false);
    expect(confirm.disabled).toBe(true);
    expect(confirm.textContent).toBe('Clear Records↵');
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
    await screen.findByText('2 kB');
    expect(screen.queryByText('4 kB')).toBeNull();
    expect(within(screen.getByRole('region', { name: 'History & Storage' })).queryByRole('status')).toBeNull();
    expect(calls('records_cleanup_apply')[0][1]).toEqual({ policy: null, categories: 6, token: 'preview-token' });
    expect(calls('records_storage_info').length).toBeGreaterThanOrEqual(2);
  });

  it('refreshes storage and reports an image cleanup failure after records were removed', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'records_cleanup_apply') return { removed: 1, mediaCleanupFailed: true };
      if (name === 'records_storage_info') return { bytes: 2000, limit: 500 };
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Clear Records' }));
    const dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('checkbox', { name: /^Ordinary/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Clear Records' }));
    await screen.findByText('2 kB');
    expect(screen.getByRole('alert').textContent).toContain('some local image copies');
  });

  it('does not enable confirmation when the clipboard history is empty', async () => {
    await openSettings();
    bridge.invoke.mockResolvedValueOnce({ count: 0, token: 'empty-preview', categories: [0, 0, 0] });
    fireEvent.click(screen.getByRole('button', { name: 'Clear Records' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.querySelector('p')).toBeNull();
    expect(within(dialog).getAllByRole('checkbox').every((item) => (item as HTMLInputElement).disabled)).toBe(true);
    expect(within(dialog).getByRole<HTMLButtonElement>('button', { name: 'Clear Records' }).disabled).toBe(true);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(calls('records_cleanup_apply')).toHaveLength(0);
  });

  it('requires another preview if the history changed and never retries deletion on its own', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'records_cleanup_apply') throw 'clipboard_cleanup_changed';
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Clear Records' }));
    const dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('checkbox', { name: /^Ordinary/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Clear Records' }));
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

  it('previews and applies clipboard storage space limits while preserving existing retention fields', async () => {
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
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const storageLimit = await screen.findByRole('button', { name: 'Storage Limit: No Limit' });
    fireEvent.click(storageLimit);
    fireEvent.click(screen.getByRole('option', { name: '500 MB' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Change history retention?' });
    fireEvent.click(within(dialog).getByRole('button', { name: /Apply & Remove/i }));
    await screen.findByRole('button', { name: 'Storage Limit: 500 MB' });
    expect(calls('records_cleanup_apply').at(-1)?.[1]).toEqual({
      policy: { limit: 500, maxAgeDays: 7, maxStorageMb: 500 },
      token: 'storage-limit-token',
    });
  });

  it('edits pre-populated pasteboard types inline and toggles Universal Clipboard and sensitive credential presets', async () => {
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

    // Inline edit a pre-populated type.
    fireEvent.click(within(ignoreRules).getByRole('button', { name: 'Edit com.agilebits.onepassword' }));
    const editInput = within(ignoreRules).getByRole<HTMLInputElement>('textbox', { name: 'Edit com.agilebits.onepassword' });
    fireEvent.change(editInput, { target: { value: 'com.custom.vault' } });
    fireEvent.click(within(ignoreRules).getByRole('button', { name: 'Save' }));
    await within(ignoreRules).findByText('com.custom.vault');
    expect(calls('clipboard_ignored_type_update').at(-1)?.[1]).toEqual({
      oldPasteboardType: 'com.agilebits.onepassword',
      newPasteboardType: 'com.custom.vault',
    });

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
});
