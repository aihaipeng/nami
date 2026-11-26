import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FileText, Folder, Image } from 'lucide-react';
import type { ViewProps } from '../App';
import { resolveLanguage } from '../shared/i18n';
import { defaultClipboardSearch } from '../shared/clipboardSearch';
import { ClipboardSearchSettings } from './ClipboardSearchSettings';
import { ClipboardIgnoreRules, ClipboardReuseSettings, useClipboardOptions } from './ClipboardOptions';
import { HistoryStorageSettings } from './HistoryStorageSettings';
import { SettingHintTip } from './SettingHintTip';

const rows = [
  { kind: 'text', label: 'text', icon: FileText },
  { kind: 'image', label: 'clipboardImages', note: 'clipboardImageDescription', icon: Image },
  { kind: 'files', label: 'clipboardFiles', note: 'clipboardFilesDescription', icon: Folder },
] as const;

export function ClipboardSettings({ t, snapshot, native }: Pick<ViewProps, 't' | 'snapshot' | 'native'>) {
  const [savingKey, setSavingKey] = useState<'capture' | typeof rows[number]['kind'] | null>(null);
  const pendingCapture = useRef(false);
  const [saveError, setSaveError] = useState<'capture' | null>(null);
  const canManage = native && !snapshot.storageError;
  const options = useClipboardOptions(canManage);
  const canCapture = canManage && snapshot.capabilities.clipboardWrite;
  const enabled = snapshot.clipboardCaptureEnabled;
  const types = snapshot.clipboardRecordingTypes;
  const captureStatus = !canCapture ? (snapshot.storageError ? null : 'platformUnavailable')
    : !enabled ? 'clipboardRecordingPaused' : Object.values(types).some(Boolean) ? null : 'clipboardNoTypes';
  async function saveRecording(next: boolean) {
    if (pendingCapture.current) return;
    pendingCapture.current = true;
    setSavingKey('capture');
    setSaveError(null);
    try { await invoke('clipboard_capture_set', { enabled: next }); }
    catch { setSaveError('capture'); }
    finally { pendingCapture.current = false; setSavingKey(null); }
  }
  async function saveCapture(kind: typeof rows[number]['kind'], enabled: boolean) {
    if (pendingCapture.current) return;
    pendingCapture.current = true;
    setSavingKey(kind);
    setSaveError(null);
    try { await invoke('clipboard_recording_type_set', { kind, enabled }); }
    catch { setSaveError('capture'); }
    finally { pendingCapture.current = false; setSavingKey(null); }
  }

  return <div className="clipboard-settings">
    {options.loadError && <div className="clipboard-load-error" role="alert"><span>{t('clipboardSettingsLoadFailed')}</span>
      <button type="button" className="settings-action" onClick={options.retry}>{t('retry')}</button></div>}
    <section aria-labelledby="clipboard-capture-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <h2 id="clipboard-capture-heading">{t('capture')}</h2>
      </div>
      <div className="settings-group">
        <div className="settings-field">
          <span id="clipboard-capture-enabled-label" className="settings-field-content">{t('clipboardRecordingEnabled')}</span>
          <input className="settings-checkbox" id="clipboard-capture-enabled" type="checkbox" aria-labelledby="clipboard-capture-enabled-label" checked={enabled} disabled={!canCapture} aria-disabled={savingKey === 'capture' || undefined} onChange={(event) => { if (!pendingCapture.current) void saveRecording(event.target.checked); }} />
        </div>
        <fieldset className="clipboard-recording-types" aria-label={t('clipboardRecordingTypes')} disabled={!canCapture}>
          {rows.map(({ kind, label, icon: Icon, ...row }) => <div key={kind} className="settings-field">
            <Icon className="clipboard-setting-icon" aria-hidden="true" />
            <div className="settings-shortcut-name">
              <span id={`clipboard-type-${kind}-label`} className="settings-field-content">{t(label)}</span>
              {'note' in row && <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t(label))} text={t(row.note)} />}
            </div>
            <input className="settings-checkbox" id={`clipboard-type-${kind}`} type="checkbox" aria-labelledby={`clipboard-type-${kind}-label`} aria-description={'note' in row ? t(row.note) : undefined} checked={types[kind]} aria-disabled={savingKey === kind || undefined} onChange={(event) => { if (!pendingCapture.current) void saveCapture(kind, event.target.checked); }} />
          </div>)}
        </fieldset>
        {captureStatus && <p className="clipboard-setting-note" role="status">{t(captureStatus)}</p>}
        {saveError === 'capture' && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
      </div>
    </section>
    <ClipboardIgnoreRules t={t} state={options} available={canCapture} />
    <HistoryStorageSettings t={t} available={canManage} locale={resolveLanguage(snapshot.locale)} />
    <ClipboardSearchSettings t={t} search={snapshot.clipboardControls.search ?? defaultClipboardSearch} available={canManage} />
    <ClipboardReuseSettings t={t} state={options} available={canManage} platformAvailable={canCapture}
      locale={resolveLanguage(snapshot.locale)} />
  </div>;
}
