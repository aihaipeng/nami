import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FileText, Folder, Image } from 'lucide-react';
import type { ViewProps } from '../App';
import { resolveLanguage } from '../shared/i18n';
import { ClipboardExcludedApps, ClipboardReuseSettings, ClipboardSettingTip, useClipboardOptions } from './ClipboardOptions';

import { HistoryStorageSettings } from './HistoryStorageSettings';

const rows = [
  { kind: 'text', label: 'text', icon: FileText },
  { kind: 'image', label: 'clipboardImages', note: 'clipboardImageDescription', icon: Image },
  { kind: 'files', label: 'clipboardFiles', note: 'clipboardFilesDescription', icon: Folder },
] as const;

export function ClipboardSettings({ t, snapshot, native }: Pick<ViewProps, 't' | 'snapshot' | 'native'>) {
  const [savingCapture, setSavingCapture] = useState(false);
  const [saveError, setSaveError] = useState<'capture' | 'index' | null>(null);
  const canManage = native && !snapshot.storageError;
  const options = useClipboardOptions(canManage);
  const canCapture = canManage && snapshot.capabilities.clipboardWrite;
  const enabled = snapshot.clipboardCaptureEnabled;
  const types = snapshot.clipboardRecordingTypes;
  const captureStatus = !canCapture ? (snapshot.storageError ? null : 'platformUnavailable')
    : !enabled ? 'clipboardRecordingPaused' : Object.values(types).some(Boolean) ? null : 'clipboardNoTypes';
  async function saveCapture(kind: typeof rows[number]['kind'], enabled: boolean) {
    setSavingCapture(true);
    setSaveError(null);
    try { await invoke('clipboard_recording_type_set', { kind, enabled }); }
    catch { setSaveError('capture'); }
    finally { setSavingCapture(false); }
  }

  async function saveImageIndex(enabled: boolean) {
    setSavingCapture(true);
    setSaveError(null);
    try {
      await invoke('clipboard_auto_image_index_set', { enabled });
      options.setOptions((current) => current && { ...current, autoImageIndex: enabled });
    } catch { setSaveError('index'); }
    finally { setSavingCapture(false); }
  }

  return <div className="clipboard-settings">
    {options.loadError && <div className="clipboard-load-error" role="alert"><span>{t('clipboardSettingsLoadFailed')}</span>
      <button type="button" className="settings-action" onClick={options.retry}>{t('retry')}</button></div>}
    <section aria-labelledby="clipboard-capture-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <h2 id="clipboard-capture-heading">{t('capture')}</h2>
      </div>
      <div className="settings-group">
        <fieldset className="clipboard-recording-types" aria-label={t('clipboardRecordingTypes')} disabled={!canCapture}>
          {rows.map(({ kind, label, icon: Icon, ...row }) => <label key={kind} className="settings-field" title={'note' in row ? t(row.note) : undefined}>
            <Icon className="clipboard-setting-icon" aria-hidden="true" />
            <span className="settings-field-content"><span>{t(label)}</span></span>
            <input className="settings-checkbox" id={`clipboard-type-${kind}`} type="checkbox" aria-label={t(label)} aria-description={'note' in row ? t(row.note) : undefined} checked={types[kind]} aria-disabled={savingCapture || undefined} onChange={(event) => { if (!savingCapture) void saveCapture(kind, event.target.checked); }} />
          </label>)}
        </fieldset>
        {captureStatus && <p className="clipboard-setting-note" role="status">{t(captureStatus)}</p>}
        {saveError === 'capture' && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
      </div>
    </section>
    {snapshot.capabilities.imageOcr && <section aria-labelledby="clipboard-image-index-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <div className="clipboard-section-title">
          <h2 id="clipboard-image-index-heading">{t('clipboardAutoImageIndex')}</h2>
          <ClipboardSettingTip label={t('clipboardAboutImageIndex')} text={t('clipboardAutoImageIndexDescription')} />
        </div>
        <input className="settings-checkbox" type="checkbox" aria-labelledby="clipboard-image-index-heading"
          aria-description={t('clipboardAutoImageIndexDescription')}
          checked={options.options?.autoImageIndex ?? false} disabled={!canCapture || !options.options || savingCapture}
          onChange={(event) => void saveImageIndex(event.target.checked)} />
      </div>
      {saveError === 'index' && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
    </section>}
    <ClipboardExcludedApps t={t} state={options} available={canCapture} />
    <HistoryStorageSettings t={t} available={canManage} view="clipboard" previewLines={snapshot.clipboardControls.previewLines?.clipboard ?? 3} locale={resolveLanguage(snapshot.locale)} />
    <ClipboardReuseSettings t={t} state={options} platformAvailable={canCapture}
      locale={resolveLanguage(snapshot.locale)} />
  </div>;
}
