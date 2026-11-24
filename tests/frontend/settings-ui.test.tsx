// @vitest-environment jsdom
import { fireEvent, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { bridge, calls, emit, mount } from './harness';

describe('Settings', () => {
  it('uses the same leading title and group structure across all four pages', async () => {
    await mount('settings');
    for (const page of ['General', 'Translation', 'Clipboard', 'Privacy & data']) {
      fireEvent.click(screen.getByRole('button', { name: page }));
      const region = screen.getByRole('region', { name: page });
      expect(within(region).getByRole('heading', { level: 1 })).toBe(region.querySelector('.settings-page-heading h1'));
      expect(region.querySelector('.settings-group')).not.toBeNull();
      expect(region.querySelector('.settings-page-heading p')).toBeNull();
    }
  });

  it.each([
    ['Open Translation', 'translation_shortcut_set', 'record'],
    ['Preview', 'clipboard_record_shortcut_set', 'choose'],
    ['Open Clipboard History', 'clipboard_shortcut_set', 'record'],
  ])('explains why Command-W cannot replace the %s shortcut (%s, %s)', async (label, command, mode) => {
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'HotKey' }));
    const recorder = screen.getByRole('button', { name: `${label} shortcut` });
    const previous = recorder.textContent;
    const invoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((name, args) => name === command
      ? Promise.reject('shortcut_reserved_close_window') : invoke(name, args));

    fireEvent.click(recorder);
    if (mode === 'choose') {
      fireEvent.click(screen.getByRole('button', { name: 'Choose Keys' }));
      const manual = within(screen.getByRole('dialog', { name: 'Choose Keys' }));
      fireEvent.change(manual.getByRole('combobox', { name: 'Key' }), { target: { value: 'KeyW' } });
      fireEvent.click(manual.getByRole('checkbox', { name: '⌘ Command' }));
      fireEvent.click(manual.getByRole('button', { name: 'Save' }));
    } else {
      fireEvent.keyDown(recorder, { key: 'w', code: 'KeyW', metaKey: true });
    }

    expect(await screen.findByText('⌘W is reserved for closing the current window.')).toBeTruthy();
    expect(calls(command)).toHaveLength(1);
    expect(calls(command)[0][1]).toMatchObject({ binding: { code: 'KeyW', modifiers: ['meta'] } });
    expect(recorder.textContent).toBe(previous);
  });

  it('dismisses a settings picker when its anchor scrolls and returns focus without closing on menu scroll', async () => {
    await mount('settings');
    const button = screen.getByRole('button', { name: /^Interface language:/i });
    fireEvent.click(button);
    const options = screen.getByRole('listbox', { name: 'Interface language' });
    fireEvent.scroll(options);
    expect(button.getAttribute('aria-expanded')).toBe('true');
    fireEvent.scroll(screen.getByRole('region', { name: 'General' }));
    expect(screen.queryByRole('listbox', { name: 'Interface language' })).toBeNull();
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(button);
  });

  it('restores persisted locale from snapshot, sends locale_set, and guards on storageError', async () => {
    bridge.snapshot.locale = 'zh-CN';
    const view = await mount('settings');
    const picker = screen.getByRole('button', { name: /^界面语言:/i });
    expect(picker.textContent).toContain('Chinese (Simplified)');
    fireEvent.click(picker);
    fireEvent.click(screen.getByRole('option', { name: 'English' }));
    expect(calls('locale_set').at(-1)?.[1]).toEqual({ locale: 'en' });
    await emit('locale_changed', 'en');
    expect(screen.getByRole('button', { name: /^Interface language:/i }).textContent).toContain('English');
    view.unmount();

    bridge.snapshot.storageError = 'storage_unavailable';
    await mount('settings');
    expect(screen.queryByRole('button', { name: /^Interface language:/i })).toBeNull();
  });

  it('persists window position strategy via window_position_mode_set and syncs events', async () => {
    bridge.snapshot.windowPositionMode = 'mouse';
    const view = await mount('settings');
    const picker = screen.getByRole('button', { name: /^Window Position:/i });
    expect(picker.textContent).toContain('Follow Mouse');
    fireEvent.click(picker);
    fireEvent.click(screen.getByRole('option', { name: 'Follow Text Cursor' }));
    expect(calls('window_position_mode_set').at(-1)?.[1]).toEqual({ mode: 'cursor' });
    await emit('window_position_mode_changed', 'center');
    expect(screen.getByRole('button', { name: /^Window Position:/i }).textContent).toContain('Screen Center');
    view.unmount();

    bridge.snapshot.storageError = 'storage_unavailable';
    await mount('settings');
    expect(screen.queryByRole('button', { name: /^Window Position:/i })).toBeNull();
  });

  it('exports backups, previews and applies merge/replace imports, and handles errors and unsupported hosts', async () => {
    const baseInvoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'backup_export') {
        return { exportedAt: '2026-11-24T10:00:00Z', translationCount: 3, clipboardCount: 12, imageCount: 2, fileSize: 4096 };
      }
      if (name === 'backup_import_pick') {
        return {
          token: 'tok-backup-1',
          fileName: 'Nami_Backup_2026-11-24.namibak',
          summary: { exportedAt: '2026-11-24T10:00:00Z', translationCount: 3, clipboardCount: 12, imageCount: 2, fileSize: 4096 },
        };
      }
      if (name === 'backup_import_apply') {
        const { mode } = args as { token: string; mode: 'merge' | 'replace' };
        return { mode, importedTranslations: 3, skippedTranslations: 0, importedClipboard: 10, skippedClipboard: 2, restoredImages: 2 };
      }
      return baseInvoke(name, args);
    });

    const view = await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Privacy & data' }));
    const backupSection = screen.getByRole('region', { name: 'Backup & Migration' });

    fireEvent.click(within(backupSection).getByRole('button', { name: 'Export Backup' }));
    const exportDialog = await screen.findByRole('alertdialog', { name: 'Export Backup' });
    fireEvent.click(within(exportDialog).getByRole('button', { name: 'Export' }));
    expect(await within(backupSection).findByRole('status')).toBeTruthy();
    expect(within(backupSection).getByRole('status').textContent).toContain('Backup exported');
    expect(within(backupSection).getByRole('status').textContent).toContain('12 Clipboard');

    fireEvent.click(within(backupSection).getByRole('button', { name: 'Import Backup' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Import Backup' });
    expect(dialog.textContent).toContain('Nami_Backup_2026-11-24.namibak');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Merge' }));
    expect(calls('backup_import_apply').at(-1)?.[1]).toEqual({ token: 'tok-backup-1', mode: 'merge' });
    expect(await within(backupSection).findByRole('status')).toBeTruthy();
    expect(within(backupSection).getByRole('status').textContent).toContain('Import complete');

    // Invalid backup error surfaces clearly.
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) =>
      name === 'backup_import_pick' ? Promise.reject('backup_invalid') : baseInvoke(name, args),
    );
    fireEvent.click(within(backupSection).getByRole('button', { name: 'Import Backup' }));
    expect((await within(backupSection).findByRole('alert')).textContent).toBe('Backup file is invalid or corrupted.');
    view.unmount();

    // Unsupported platform disables backup actions cleanly.
    bridge.snapshot.capabilities.localBackup = false;
    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Privacy & data' }));
    expect(within(screen.getByRole('region', { name: 'Backup & Migration' })).queryByRole('button', { name: 'Export Backup' })).toBeNull();
  });

  it('supports password-encrypted backup export and encrypted backup unlock/import with retry', async () => {
    const baseInvoke = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
      if (name === 'backup_export') {
        return { exportedAt: '2026-11-24T10:00:00Z', translationCount: 2, clipboardCount: 8, imageCount: 1, fileSize: 8192 };
      }
      if (name === 'backup_import_pick') {
        return {
          token: 'tok-enc-1',
          fileName: 'Secret_Backup.namibak',
          encrypted: true,
          summary: null,
        };
      }
      if (name === 'backup_import_unlock') {
        const { password } = args as { token: string; password: string };
        if (password !== 'secret-123') throw 'backup_wrong_password';
        return { exportedAt: '2026-11-24T10:00:00Z', translationCount: 2, clipboardCount: 8, imageCount: 1, fileSize: 8192 };
      }
      if (name === 'backup_import_apply') {
        const { mode } = args as { token: string; mode: 'merge' | 'replace'; password?: string };
        return { mode, importedTranslations: 2, skippedTranslations: 0, importedClipboard: 8, skippedClipboard: 0, restoredImages: 1 };
      }
      return baseInvoke(name, args);
    });

    await mount('settings');
    fireEvent.click(screen.getByRole('button', { name: 'Privacy & data' }));
    const backupSection = screen.getByRole('region', { name: 'Backup & Migration' });

    // Export with password validates confirmation mismatch before submitting.
    fireEvent.click(within(backupSection).getByRole('button', { name: 'Export Backup' }));
    const exportDialog = await screen.findByRole('alertdialog', { name: 'Export Backup' });
    fireEvent.change(within(exportDialog).getByLabelText('Backup Password'), { target: { value: 'secret-123' } });
    fireEvent.change(within(exportDialog).getByLabelText('Confirm Password'), { target: { value: 'mismatch' } });
    fireEvent.click(within(exportDialog).getByRole('button', { name: 'Export' }));
    expect((await within(exportDialog).findByRole('alert')).textContent).toBe('Passwords do not match.');
    expect(calls('backup_export')).toHaveLength(0);

    fireEvent.change(within(exportDialog).getByLabelText('Confirm Password'), { target: { value: 'secret-123' } });
    fireEvent.click(within(exportDialog).getByRole('button', { name: 'Export' }));
    expect(calls('backup_export').at(-1)?.[1]).toEqual({ password: 'secret-123' });
    expect((await within(backupSection).findByRole('status')).textContent).toContain('Encrypted');

    // Import encrypted backup prompts for password, handles wrong password retry, then applies.
    fireEvent.click(within(backupSection).getByRole('button', { name: 'Import Backup' }));
    const unlockDialog = await screen.findByRole('alertdialog', { name: 'Unlock Encrypted Backup' });
    fireEvent.change(within(unlockDialog).getByLabelText('Backup Password'), { target: { value: 'wrong' } });
    fireEvent.click(within(unlockDialog).getByRole('button', { name: 'Unlock' }));
    expect((await within(unlockDialog).findByRole('alert')).textContent).toBe('Incorrect password or corrupted backup file.');

    fireEvent.change(within(unlockDialog).getByLabelText('Backup Password'), { target: { value: 'secret-123' } });
    fireEvent.click(within(unlockDialog).getByRole('button', { name: 'Unlock' }));
    const confirmDialog = await screen.findByRole('alertdialog', { name: 'Import Backup' });
    expect(confirmDialog.textContent).toContain('Secret_Backup.namibak');
    expect(confirmDialog.textContent).toContain('Encrypted');
    fireEvent.click(within(confirmDialog).getByRole('button', { name: 'Replace' }));
    expect(calls('backup_import_apply').at(-1)?.[1]).toEqual({ token: 'tok-enc-1', mode: 'replace', password: 'secret-123' });
    expect((await within(backupSection).findByRole('status')).textContent).toContain('Import complete');
  });
});
