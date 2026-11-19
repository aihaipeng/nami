// @vitest-environment jsdom
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount } from './harness';

describe('clipboard recording settings', () => {
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
      'Open Clipboard History', 'Open Translation', 'Pin / Unpin Window', 'Show Shortcuts', 'Focus Search',
    ]);
    const clipboard = within(screen.getByRole('group', { name: 'Clipboard' }));
    expect(clipboard.getAllByRole('checkbox')).toHaveLength(7);
    for (const name of ['Preview', 'Add/Edit Note', 'Extract Text', 'Translate', 'Favorite/Unfavorite', 'Pin/Unpin', 'Delete']) {
      expect(clipboard.getByRole('button', { name: `${name} shortcut` })).toBeTruthy();
      expect(clipboard.getByRole<HTMLInputElement>('checkbox', { name }).checked).toBe(true);
    }
    const translation = within(screen.getByRole('group', { name: 'Translation' }));
    expect(translation.getAllByRole<HTMLInputElement>('checkbox').map((toggle) => toggle.labels?.[0].textContent)).toEqual([
      'Translate', 'Copy Primary Translation', 'Copy Secondary Translation', 'Actions',
    ]);
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    expect(screen.queryByRole('button', { name: 'Open Clipboard History shortcut' })).toBeNull();
  });

  it('rejects a clipboard binding already used by a general action and preserves both', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const general = within(screen.getByRole('group', { name: 'General' }));
    const search = general.getByRole('button', { name: 'Focus Search shortcut' });
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

  it('describes both operations for note, favorite, and pin in Simplified Chinese', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    await emit('locale_changed', 'zh-CN');
    for (const name of ['添加/编辑备注', '收藏/取消收藏', '置顶/取消置顶']) {
      expect(screen.getByRole('checkbox', { name })).toBeTruthy();
      expect(screen.getByRole('button', { name: `${name} 快捷键` })).toBeTruthy();
    }
  });

  it.each([['Preview', 'preview', '⌘Y'], ['Add/Edit Note', 'note', '⇧⌘N'], ['Extract Text', 'ocr', '⇧⌘O'], ['Translate', 'translate', '⇧⌘T'], ['Favorite/Unfavorite', 'favorite', '⌥F'], ['Delete', 'delete', '⌘⌫']])(
    'edits %s while unchecked and restores the default for the section', async (title, action, defaultKeys) => {
      const user = userEvent.setup(); await mount('settings');
      await user.click(screen.getByRole('button', { name: 'HotKey' }));
      const clipboard = within(screen.getByRole('group', { name: 'Clipboard' }));
      const recorder = clipboard.getByRole('button', { name: `${title} shortcut` });
      const toggle = clipboard.getByRole<HTMLInputElement>('checkbox', { name: title });
      await user.click(toggle);
      expect(calls('clipboard_record_shortcut_set').at(-1)?.[1]).toEqual({ action, enabled: false });
      await user.click(recorder);
      fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', altKey: true });
      await waitFor(() => expect(recorder.textContent).toBe('⌥J'));
      expect(toggle.checked).toBe(false);
      await user.click(screen.getByRole('button', { name: 'Restore Clipboard Defaults' }));
      await waitFor(() => expect(recorder.textContent).toBe(defaultKeys));
      expect(toggle.checked).toBe(true);
    });

  it.each([[' ', 'Space'], ['F6', 'F6']])('rejects an unmodified %s in the preview recorder', async (key, code) => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const recorder = screen.getByRole('button', { name: 'Preview shortcut' });
    fireEvent.click(recorder);
    fireEvent.keyDown(window, { key, code });
    expect((await screen.findByRole('alert')).textContent).toBe('Use Command, Control, or Option with a key.');
    expect(recorder.textContent).toBe('⌘Y');
  });

  it('keeps panel shortcut changes separate and rejects a collision with another panel action', async () => {
    const user = userEvent.setup(); await mount('settings');
    await user.click(screen.getByRole('button', { name: 'HotKey' }));
    const pin = screen.getByRole('button', { name: 'Pin/Unpin shortcut' });
    const toggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'Pin/Unpin' });
    await user.click(toggle);
    await user.click(pin);
    fireEvent.keyDown(window, { key: 'f', code: 'KeyF', altKey: true });
    expect(await screen.findByText('This key combination is already assigned in Clipboard History or globally.')).toBeTruthy();
    expect(pin.textContent).toBe('⌥P');
    await user.click(pin);
    fireEvent.keyDown(window, { key: 'j', code: 'KeyJ', altKey: true });
    await waitFor(() => expect(pin.textContent).toBe('⌥J'));
    expect(toggle.checked).toBe(false);
    expect(calls('clipboard_record_shortcut_set').at(-1)?.[1]).toEqual({ action: 'pin', binding: { code: 'KeyJ', modifiers: ['alt'] } });
    expect(calls('clipboard_shortcut_set')).toHaveLength(0);
  });

  it('offers no master switch and reports recording state from the menu bar', async () => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    const capture = screen.getByRole('region', { name: 'Clipboard Recording' });
    expect(within(capture).queryByRole('checkbox', { name: 'Clipboard Recording' })).toBeNull();
    expect(within(capture).getByRole('status').textContent).toBe('Recording paused. Resume from the menu bar.');
    await emit('clipboard_capture_changed', true);
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
    const shortcuts = screen.getByRole('region', { name: 'General' });
    const restore = within(screen.getByRole('region', { name: 'Clipboard' })).getByRole('button', { name: 'Restore Clipboard Defaults' });
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
    fireEvent.keyDown(window, { key: 'Escape', code: 'Escape' });
    expect(control().textContent).toBe('⌃⇧J');
    fireEvent.click(control());
    fireEvent.keyDown(window, { key: 'Backspace', code: 'Backspace' });
    await waitFor(() => expect(control().textContent).toBe('Add Hotkey'));
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
        historyPolicy: { days: null, limit: null }, activation: 'paste',
      };
      if (name === 'clipboard_excluded_app_pick') return [{ bundleId: 'com.apple.Passwords', name: 'Passwords' }, { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' }, { bundleId: 'com.example.synthetic', name: 'Synthetic App' }];
      if (name === 'clipboard_excluded_app_remove') return [{ bundleId: 'com.apple.Passwords', name: 'Passwords' }, { bundleId: 'com.apple.keychainaccess', name: 'Keychain Access' }];
      if (name === 'clipboard_storage_info') return { bytes: 4000, unprotectedCount: 4 };
      if (name === 'clipboard_cleanup_preview') return { count: 4, token: 'preview-token', ...(!(args as { policy: unknown }).policy && { categories: [1, 2, 2] }) };
      if (name === 'clipboard_cleanup_apply') return { removed: (args as { categories?: number }).categories === 6 ? 3 : 4, mediaCleanupFailed: false };
      return invoke(name, args);
    });
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    await screen.findByText('4 kB');
  }

  it('gives recording and exclusion separate headings and limits recording controls to content types', async () => {
    await openSettings();
    for (const name of ['Clipboard Recording', 'Excluded Apps', 'History & Storage', 'Behavior']) {
      expect(screen.getByRole('region', { name })).toBeTruthy();
    }
    const heading = screen.getByRole('heading', { level: 2, name: 'Clipboard Recording' });
    expect(heading.parentElement?.querySelector('[role=switch]')).toBeNull();
    const types = screen.getByRole('group', { name: 'Content types' });
    expect(within(types).getAllByRole('checkbox')).toHaveLength(3);
    expect(within(types).queryByRole('switch')).toBeNull();
    expect(screen.getByRole('heading', { level: 2, name: 'Excluded Apps' })).toBeTruthy();
  });

  it('opens a one-line tip without a confirmation button and keeps keyboard focus on its trigger', async () => {
    await openSettings([]);
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(screen.queryByRole('list', { name: 'Excluded Apps' })).toBeNull();
    const tip = screen.getByRole('button', { name: 'About Excluded Apps' });
    fireEvent.click(tip);
    const panel = screen.getByRole('tooltip');
    expect(panel.textContent).toBe('Checks the active app only.');
    expect(within(panel).queryByRole('button')).toBeNull();
    expect(tip.getAttribute('aria-describedby')).toBe(panel.id);
    expect(document.activeElement).toBe(tip);
    fireEvent.keyDown(tip, { key: 'Escape' });
    expect(screen.queryByRole('tooltip')).toBeNull();
    fireEvent.click(tip);
    fireEvent.pointerDown(screen.getByRole('button', { name: 'Add App' }));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('shows the native icon next to the app name and falls back when the icon is absent or fails to load', async () => {
    const icon = 'data:image/png;base64,c3ludGhldGlj';
    await openSettings([{ ...excludedApps[0], iconDataUrl: icon }, excludedApps[1]]);
    const list = screen.getByRole('list', { name: 'Excluded Apps' });
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
    const list = await screen.findByRole('list', { name: 'Excluded Apps' });
    expect(list.textContent).toContain('Passwords');
    expect(list.textContent).toContain('Keychain Access');
    expect(list.textContent).not.toContain('com.apple.Passwords');
    expect(screen.getByText('Passwords').title).toBe('com.apple.Passwords');
    expect(screen.queryByRole('dialog', { name: 'Excluded Apps' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Add App' }));
    await screen.findByText('Synthetic App');
    fireEvent.click(screen.getByRole('button', { name: 'Remove Synthetic App' }));
    await waitFor(() => expect(screen.queryByText('Synthetic App')).toBeNull());
    expect(calls('clipboard_excluded_app_remove')[0][1]).toEqual({ bundleId: 'com.example.synthetic' });
  });

  it('discloses ambiguous app identifiers on demand and still removes by the exact identifier', async () => {
    await openSettings([...excludedApps, { bundleId: 'com.example.passwords', name: 'Passwords' }]);
    for (const bundleId of ['com.apple.Passwords', 'com.example.passwords']) {
      expect(screen.queryByText(bundleId)).toBeNull();
      fireEvent.click(screen.getByRole('button', { name: `About Passwords (${bundleId})` }));
      const panel = screen.getByRole('tooltip');
      expect(within(panel).getByText(bundleId)).toBeTruthy();
      fireEvent.click(screen.getByRole('button', { name: `About Passwords (${bundleId})` }));
    }
    fireEvent.click(screen.getByRole('button', { name: 'Remove Passwords (com.example.passwords)' }));
    await waitFor(() => expect(screen.getAllByText('Passwords')).toHaveLength(1));
    expect(calls('clipboard_excluded_app_remove')[0][1]).toEqual({ bundleId: 'com.example.passwords' });
    expect(screen.queryByText('com.apple.keychainaccess')).toBeNull();
  });

  it('moves retention help to the control and preserves the deletion warning at confirmation', async () => {
    await openSettings();
    const control = () => screen.getByRole('button', { name: /^Keep History For:/ });
    expect(screen.getByText('Estimated Storage')).toBeTruthy();
    expect(screen.queryByText('Based on when an item was last copied or reused.')).toBeNull();
    expect(control().getAttribute('aria-description')).toBe('Based on when an item was last copied or reused.');
    fireEvent.click(control());
    fireEvent.click(screen.getByRole('option', { name: '7 days' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('permanently removes 4 unprotected clipboard items');
    expect(dialog.textContent).toContain('Favorites, pinned items, and the system clipboard are unaffected.');
    expect(document.activeElement?.textContent).toBe('Cancel');
    expect(calls('clipboard_cleanup_preview')[0][1]).toEqual({ policy: { days: 7, limit: null } });
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(control().textContent).toBe('Forever');
    expect(document.activeElement).toBe(control());
    expect(calls('clipboard_cleanup_apply')).toHaveLength(0);
  });

  it('saves a retention policy without confirmation when the preview affects no items', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_cleanup_preview') return { count: 0, token: 'empty-preview' };
      if (name === 'clipboard_cleanup_apply') return { removed: 0, mediaCleanupFailed: false };
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'History Limit: Unlimited' }));
    fireEvent.click(screen.getByRole('option', { name: '500' }));
    await screen.findByRole('button', { name: 'History Limit: 500' });
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(calls('clipboard_cleanup_apply')[0][1]).toEqual({ policy: { days: null, limit: 500 }, token: 'empty-preview' });
  });

  it('opens cleanup from the heading and clears selected categories and refreshes size without a deletion count', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_storage_info') return { bytes: 2000 };
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
    fireEvent.click(checkbox(/^Favorites/));
    fireEvent.click(checkbox(/^Pinned/));
    expect(confirm.disabled).toBe(false);
    expect(calls('clipboard_cleanup_apply')).toHaveLength(0);
    fireEvent.click(confirm);
    await screen.findByText('2 kB');
    expect(screen.queryByText('4 kB')).toBeNull();
    expect(within(screen.getByRole('region', { name: 'History & Storage' })).queryByRole('status')).toBeNull();
    expect(calls('clipboard_cleanup_apply')[0][1]).toEqual({ policy: null, categories: 6, token: 'preview-token' });
    expect(calls('clipboard_storage_info')).toHaveLength(2);
  });

  it('refreshes storage and reports an image cleanup failure after records were removed', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_cleanup_apply') return { removed: 1, mediaCleanupFailed: true };
      if (name === 'clipboard_storage_info') return { bytes: 2000 };
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
    expect(calls('clipboard_cleanup_apply')).toHaveLength(0);
  });

  it('requires another preview if the history changed and never retries deletion on its own', async () => {
    await openSettings();
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'clipboard_cleanup_apply') throw 'clipboard_cleanup_changed';
      return invoke(name, args);
    });
    fireEvent.click(screen.getByRole('button', { name: 'Clear Records' }));
    const dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('checkbox', { name: /^Ordinary/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Clear Records' }));
    await screen.findByText(/Clipboard history changed/);
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(calls('clipboard_cleanup_preview')).toHaveLength(1);
    expect(calls('clipboard_cleanup_apply')).toHaveLength(1);
  });

  it('saves Copy Only as the row activation behavior', async () => {
    await openSettings();
    fireEvent.click(screen.getByRole('button', { name: 'When Activating an Item: Paste into Previous App' }));
    fireEvent.click(screen.getByRole('option', { name: 'Copy Only' }));
    await screen.findByRole('button', { name: 'When Activating an Item: Copy Only' });
    expect(calls('clipboard_behavior_set')[0][1]).toEqual({ activation: 'copy' });
  });
});
