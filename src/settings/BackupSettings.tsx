import { useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Translate } from '../shared/i18n';
import { SettingsDialog } from './SettingsDialog';

interface BackupSummary {
  exportedAt: string;
  clipboardCount: number;
  imageCount: number;
  fileSize: number;
}

interface BackupImportPreview {
  token: string;
  fileName: string;
  encrypted?: boolean;
  summary: BackupSummary | null;
}

interface BackupImportReport {
  mode: 'merge' | 'replace';
  importedClipboard: number;
  skippedClipboard: number;
  restoredImages: number;
}

function errorKey(error: unknown, fallback: 'backupExportFailed' | 'backupImportFailed') {
  const code = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
  if (code === 'backup_wrong_password') return 'backupWrongPassword';
  if (code === 'backup_password_required') return 'backupPasswordRequired';
  if (code === 'backup_invalid') return 'backupInvalid';
  if (code === 'backup_unsupported_version') return 'backupUnsupportedVersion';
  if (code === 'backup_changed' || code === 'backup_token_expired') return 'backupChanged';
  return fallback;
}

export function BackupSettings({ t, available }: { t: Translate; available: boolean }) {
  const id = useId();
  const [busy, setBusy] = useState<'export' | 'pick' | 'unlock' | 'import' | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportPassword, setExportPassword] = useState('');
  const [exportConfirm, setExportConfirm] = useState('');
  const [exportError, setExportError] = useState<string | null>(null);
  const [preview, setPreview] = useState<BackupImportPreview | null>(null);
  const [importPassword, setImportPassword] = useState('');
  const [unlockError, setUnlockError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  function formatCounts(clipboard: number, images: number) {
    return `${clipboard} ${t('backupClipboard')} · ${images} ${t('backupImages')}`;
  }

  function closeExportDialog() {
    if (busy === 'export') return;
    setExportOpen(false);
    setExportPassword('');
    setExportConfirm('');
    setExportError(null);
  }

  function closeImportDialog() {
    if (busy === 'unlock' || busy === 'import') return;
    setPreview(null);
    setImportPassword('');
    setUnlockError(null);
  }

  async function handleExportSubmit() {
    if (!available || busy) return;
    if (exportPassword && exportPassword !== exportConfirm) {
      setExportError(t('backupPasswordMismatch'));
      return;
    }
    const password = exportPassword || undefined;
    setBusy('export');
    setExportError(null);
    setError(null);
    setStatus(null);
    try {
      const summary = await invoke<BackupSummary | null>(
        'backup_export',
        password ? { password } : undefined,
      );
      setExportOpen(false);
      setExportPassword('');
      setExportConfirm('');
      if (summary) {
        const badge = password ? ` (${t('backupEncryptedBadge')})` : '';
        setStatus(`${t('backupExported')}${badge} · ${formatCounts(summary.clipboardCount, summary.imageCount)}`);
      }
    } catch (cause) {
      setExportOpen(false);
      setExportPassword('');
      setExportConfirm('');
      setError(t(errorKey(cause, 'backupExportFailed')));
    } finally {
      setBusy(null);
    }
  }

  async function handleImportPick() {
    if (!available || busy) return;
    setBusy('pick');
    setError(null);
    setStatus(null);
    setUnlockError(null);
    setImportPassword('');
    try {
      const picked = await invoke<BackupImportPreview | null>('backup_import_pick');
      if (picked) setPreview(picked);
    } catch (cause) {
      setError(t(errorKey(cause, 'backupImportFailed')));
    } finally {
      setBusy(null);
    }
  }

  async function handleImportUnlock() {
    if (!preview || busy) return;
    if (!importPassword) {
      setUnlockError(t('backupPasswordRequired'));
      return;
    }
    const token = preview.token;
    setBusy('unlock');
    setUnlockError(null);
    try {
      const summary = await invoke<BackupSummary>('backup_import_unlock', { token, password: importPassword });
      setPreview((current) => current && current.token === token ? { ...current, summary } : current);
    } catch (cause) {
      const code = typeof cause === 'string' ? cause : cause instanceof Error ? cause.message : '';
      if (code === 'backup_changed' || code === 'backup_token_expired') {
        setPreview(null);
        setImportPassword('');
        setError(t(errorKey(cause, 'backupImportFailed')));
      } else {
        setUnlockError(t(errorKey(cause, 'backupImportFailed')));
      }
    } finally {
      setBusy(null);
    }
  }

  async function handleImportApply(mode: 'merge' | 'replace') {
    if (!preview || !preview.summary || busy) return;
    const token = preview.token;
    const password = preview.encrypted ? importPassword : undefined;
    setBusy('import');
    setError(null);
    try {
      const report = await invoke<BackupImportReport>('backup_import_apply', {
        token,
        mode,
        ...(password ? { password } : {}),
      });
      setPreview(null);
      setImportPassword('');
      setUnlockError(null);
      setStatus(`${t('backupImported')} · ${formatCounts(report.importedClipboard, report.restoredImages)}`);
    } catch (cause) {
      setPreview(null);
      setImportPassword('');
      setUnlockError(null);
      setError(t(errorKey(cause, 'backupImportFailed')));
    } finally {
      setBusy(null);
    }
  }

  return <section aria-labelledby="settings-backup">
    <h2 id="settings-backup">{t('backupAndMigration')}</h2>
    {available ? <div className="settings-group">
      <div className="settings-field settings-backup-actions">
        <button type="button" className="settings-action" disabled={busy !== null} onClick={() => {
          setExportPassword('');
          setExportConfirm('');
          setExportError(null);
          setExportOpen(true);
        }}>
          {busy === 'export' ? t('backupExporting') : t('backupExport')}
        </button>
        <button type="button" className="settings-action" disabled={busy !== null} onClick={() => void handleImportPick()}>
          {busy === 'import' ? t('backupImporting') : t('backupImport')}
        </button>
      </div>
      {status && <p className="settings-field-note settings-backup-status" role="status">{status}</p>}
      {error && <p className="settings-field-note" role="alert">{error}</p>}
    </div> : <p className="settings-callout">{t('platformUnavailable')}</p>}

    {exportOpen && <SettingsDialog
      title={t('backupExportHeading')}
      onClose={closeExportDialog}
      className="settings-backup-dialog"
    >
      <form className="settings-backup-form" onSubmit={(event) => { event.preventDefault(); void handleExportSubmit(); }}>
        <label className="settings-backup-field" htmlFor={`${id}-export-password`}>
          <span>{t('backupPasswordLabel')}</span>
          <input
            id={`${id}-export-password`}
            type="password"
            data-initial-focus
            autoComplete="new-password"
            className="settings-text-input settings-backup-input"
            placeholder={t('backupPasswordOptionalPlaceholder')}
            value={exportPassword}
            disabled={busy === 'export'}
            onChange={(event) => { setExportPassword(event.target.value); setExportError(null); }}
          />
        </label>
        {exportPassword.length > 0 && <label className="settings-backup-field" htmlFor={`${id}-export-confirm`}>
          <span>{t('backupPasswordConfirmLabel')}</span>
          <input
            id={`${id}-export-confirm`}
            type="password"
            autoComplete="new-password"
            className="settings-text-input settings-backup-input"
            placeholder={t('backupPasswordConfirmPlaceholder')}
            value={exportConfirm}
            disabled={busy === 'export'}
            onChange={(event) => { setExportConfirm(event.target.value); setExportError(null); }}
          />
        </label>}
        {exportError && <p className="settings-field-note" role="alert">{exportError}</p>}
        <div className="settings-dialog-actions">
          <button type="button" className="settings-action" disabled={busy === 'export'} onClick={closeExportDialog}>{t('clipboardCancel')}</button>
          <button type="submit" className="settings-action" disabled={busy === 'export'}>
            {busy === 'export' ? t('backupExporting') : t('backupExportConfirm')}
          </button>
        </div>
      </form>
    </SettingsDialog>}

    {preview && !preview.summary && <SettingsDialog
      title={t('backupUnlockHeading')}
      description={preview.fileName}
      onClose={closeImportDialog}
      className="settings-backup-dialog"
    >
      <form className="settings-backup-form" onSubmit={(event) => { event.preventDefault(); void handleImportUnlock(); }}>
        <label className="settings-backup-field" htmlFor={`${id}-unlock-password`}>
          <span>{t('backupPasswordLabel')}</span>
          <input
            id={`${id}-unlock-password`}
            type="password"
            data-initial-focus
            autoComplete="current-password"
            className="settings-text-input settings-backup-input"
            placeholder={t('backupPasswordEnterPlaceholder')}
            value={importPassword}
            disabled={busy === 'unlock'}
            onChange={(event) => { setImportPassword(event.target.value); setUnlockError(null); }}
          />
        </label>
        {unlockError && <p className="settings-field-note" role="alert">{unlockError}</p>}
        <div className="settings-dialog-actions">
          <button type="button" className="settings-action" disabled={busy === 'unlock'} onClick={closeImportDialog}>{t('clipboardCancel')}</button>
          <button type="submit" className="settings-action" disabled={busy === 'unlock'}>
            {busy === 'unlock' ? t('backupUnlocking') : t('backupUnlock')}
          </button>
        </div>
      </form>
    </SettingsDialog>}

    {preview && preview.summary && <SettingsDialog
      title={t('backupImportHeading')}
      description={preview.encrypted ? `${preview.fileName} · ${t('backupEncryptedBadge')}` : preview.fileName}
      onClose={closeImportDialog}
      className="settings-backup-dialog"
    >
      <p className="settings-backup-summary">
        {formatCounts(preview.summary.clipboardCount, preview.summary.imageCount)}
      </p>
      <div className="settings-dialog-actions">
        <button type="button" className="settings-action" disabled={busy === 'import'} onClick={closeImportDialog}>{t('clipboardCancel')}</button>
        <button type="button" className="settings-action clipboard-confirm-action" disabled={busy === 'import'} onClick={() => void handleImportApply('replace')}>{t('backupImportReplace')}</button>
        <button type="button" className="settings-action" data-initial-focus disabled={busy === 'import'} onClick={() => void handleImportApply('merge')}>{t('backupImportMerge')}</button>
      </div>
    </SettingsDialog>}
  </section>;
}
