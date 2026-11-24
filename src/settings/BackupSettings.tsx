import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Translate } from '../shared/i18n';
import { SettingsDialog } from './SettingsDialog';

interface BackupSummary {
  exportedAt: string;
  translationCount: number;
  clipboardCount: number;
  imageCount: number;
  fileSize: number;
}

interface BackupImportPreview {
  token: string;
  fileName: string;
  summary: BackupSummary;
}

interface BackupImportReport {
  mode: 'merge' | 'replace';
  importedTranslations: number;
  skippedTranslations: number;
  importedClipboard: number;
  skippedClipboard: number;
  restoredImages: number;
}

function errorKey(error: unknown, fallback: 'backupExportFailed' | 'backupImportFailed') {
  const code = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
  if (code === 'backup_invalid') return 'backupInvalid';
  if (code === 'backup_unsupported_version') return 'backupUnsupportedVersion';
  if (code === 'backup_changed' || code === 'backup_token_expired') return 'backupChanged';
  return fallback;
}

export function BackupSettings({ t, available }: { t: Translate; available: boolean }) {
  const [busy, setBusy] = useState<'export' | 'pick' | 'import' | null>(null);
  const [preview, setPreview] = useState<BackupImportPreview | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  function formatCounts(translations: number, clipboard: number, images: number) {
    return `${translations} ${t('backupTranslations')} · ${clipboard} ${t('backupClipboard')} · ${images} ${t('backupImages')}`;
  }

  async function handleExport() {
    if (!available || busy) return;
    setBusy('export');
    setError(null);
    setStatus(null);
    try {
      const summary = await invoke<BackupSummary | null>('backup_export');
      if (summary) {
        setStatus(`${t('backupExported')} · ${formatCounts(summary.translationCount, summary.clipboardCount, summary.imageCount)}`);
      }
    } catch (cause) {
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
    try {
      const picked = await invoke<BackupImportPreview | null>('backup_import_pick');
      if (picked) setPreview(picked);
    } catch (cause) {
      setError(t(errorKey(cause, 'backupImportFailed')));
    } finally {
      setBusy(null);
    }
  }

  async function handleImportApply(mode: 'merge' | 'replace') {
    if (!preview || busy) return;
    const token = preview.token;
    setBusy('import');
    setError(null);
    try {
      const report = await invoke<BackupImportReport>('backup_import_apply', { token, mode });
      setPreview(null);
      setStatus(`${t('backupImported')} · ${formatCounts(report.importedTranslations, report.importedClipboard, report.restoredImages)}`);
    } catch (cause) {
      setPreview(null);
      setError(t(errorKey(cause, 'backupImportFailed')));
    } finally {
      setBusy(null);
    }
  }

  return <section aria-labelledby="settings-backup">
    <h2 id="settings-backup">{t('backupAndMigration')}</h2>
    {available ? <div className="settings-group">
      <div className="settings-field settings-backup-actions">
        <button type="button" className="settings-action" disabled={busy !== null} onClick={() => void handleExport()}>
          {busy === 'export' ? t('backupExporting') : t('backupExport')}
        </button>
        <button type="button" className="settings-action" disabled={busy !== null} onClick={() => void handleImportPick()}>
          {busy === 'import' ? t('backupImporting') : t('backupImport')}
        </button>
      </div>
      {status && <p className="settings-field-note settings-backup-status" role="status">{status}</p>}
      {error && <p className="settings-field-note" role="alert">{error}</p>}
    </div> : <p className="settings-callout">{t('platformUnavailable')}</p>}
    {preview && <SettingsDialog
      title={t('backupImportHeading')}
      description={preview.fileName}
      onClose={() => { if (busy !== 'import') setPreview(null); }}
      className="settings-backup-dialog"
    >
      <p className="settings-backup-summary">
        {formatCounts(preview.summary.translationCount, preview.summary.clipboardCount, preview.summary.imageCount)}
      </p>
      <div className="settings-dialog-actions">
        <button type="button" className="settings-action" disabled={busy === 'import'} onClick={() => setPreview(null)}>{t('clipboardCancel')}</button>
        <button type="button" className="settings-action clipboard-confirm-action" disabled={busy === 'import'} onClick={() => void handleImportApply('replace')}>{t('backupImportReplace')}</button>
        <button type="button" className="settings-action" data-initial-focus disabled={busy === 'import'} onClick={() => void handleImportApply('merge')}>{t('backupImportMerge')}</button>
      </div>
    </SettingsDialog>}
  </section>;
}
