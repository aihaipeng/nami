import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ClipboardList, FileText, Folder, Image, Keyboard } from 'lucide-react';
import type { ViewProps } from '../App';
import { resolveLanguage, type MessageKey } from '../shared/i18n';
import { defaultClipboardBinding, keycapLabels, sameBinding, type KeyBinding } from '../shared/shortcuts';
import { ShortcutCapture, errorCode, shortcutMessage } from './ShortcutCapture';
import { ClipboardExcludedApps, ClipboardReuseSettings, useClipboardOptions } from './ClipboardOptions';

const rows = [
  { kind: 'text', label: 'text', icon: FileText },
  { kind: 'image', label: 'clipboardImages', note: 'clipboardImageDescription', icon: Image },
  { kind: 'files', label: 'clipboardFileTypes', note: 'clipboardFilesDescription', icon: Folder },
] as const;

export function ClipboardSettings({ t, snapshot, native }: Pick<ViewProps, 't' | 'snapshot' | 'native'>) {
  const [recording, setRecording] = useState(false);
  const [savingShortcut, setSavingShortcut] = useState(false);
  const [savingCapture, setSavingCapture] = useState(false);
  const pendingShortcut = useRef(false);
  const [shortcutFeedback, setShortcutFeedback] = useState<{ text: string; error?: boolean }>();
  const [captureError, setCaptureError] = useState(false);
  const canManage = native && !snapshot.storageError;
  const options = useClipboardOptions(canManage);
  const canCapture = canManage && snapshot.capabilities.clipboardWrite;
  const canRecord = native && snapshot.capabilities.selection && !snapshot.storageError;
  const enabled = snapshot.clipboardCaptureEnabled;
  const types = snapshot.clipboardRecordingTypes;
  const captureStatus = !canCapture ? (snapshot.storageError ? null : 'platformUnavailable')
    : !enabled ? 'clipboardRecordingPaused' : Object.values(types).some(Boolean) ? null : 'clipboardNoTypes';
  const binding = snapshot.clipboardBinding;
  const keys = binding ? keycapLabels(binding, snapshot.capabilities.platform === 'macos') : [];
  const isDefault = binding && sameBinding(binding, defaultClipboardBinding);
  const recorder = useRef<HTMLButtonElement>(null);
  useEffect(() => { setRecording(false); setShortcutFeedback(undefined); }, [snapshot.locale]);

  async function saveCapture(name: string, args: Record<string, unknown>) {
    setSavingCapture(true);
    setCaptureError(false);
    try { await invoke(name, args); }
    catch { setCaptureError(true); }
    finally { setSavingCapture(false); }
  }

  async function saveShortcut(next: KeyBinding | null, message: MessageKey) {
    if (pendingShortcut.current) return;
    pendingShortcut.current = true;
    setRecording(false);
    setSavingShortcut(true);
    try {
      await invoke('clipboard_shortcut_set', { binding: next });
      setShortcutFeedback({ text: t(message) });
    } catch (error) {
      setShortcutFeedback({ text: shortcutMessage(errorCode(error), t), error: true });
    } finally {
      pendingShortcut.current = false;
      setSavingShortcut(false);
    }
  }

  function toggleRecorder() {
    setShortcutFeedback(undefined);
    setRecording(!recording);
  }

  return <div className="clipboard-settings">
    {options.loadError && <div className="clipboard-load-error" role="alert"><span>{t('clipboardSettingsLoadFailed')}</span>
      <button type="button" className="settings-action" onClick={options.retry}>{t('retry')}</button></div>}
    <section aria-labelledby="clipboard-capture-heading">
      <h2 id="clipboard-capture-heading"><ClipboardList aria-hidden="true" />{t('clipboardCaptureHeading')}</h2>
      <div className="settings-group">
        <label className="settings-field" title={t('captureDescription')}>
          <span className="settings-field-content"><span>{t('capture')}</span></span>
          <input className="settings-switch" id="clipboard-capture" type="checkbox" role="switch" aria-label={t('capture')} aria-description={t('captureDescription')} checked={enabled} disabled={!canCapture} aria-disabled={savingCapture || undefined} onChange={(event) => { if (!savingCapture) void saveCapture('clipboard_capture_set', { enabled: event.target.checked }); }} />
        </label>
        <fieldset className="clipboard-recording-types" aria-label={t('clipboardRecordingTypes')} disabled={!canCapture || !enabled}>
          {rows.map(({ kind, label, icon: Icon, ...row }) => <label key={kind} className="settings-field" title={'note' in row ? t(row.note) : undefined}>
            <Icon className="clipboard-setting-icon" aria-hidden="true" />
            <span className="settings-field-content"><span>{t(label)}</span></span>
            <input className="settings-switch" id={`clipboard-type-${kind}`} role="switch" type="checkbox" aria-label={t(label)} aria-description={'note' in row ? t(row.note) : undefined} checked={types[kind]} aria-disabled={savingCapture || undefined} onChange={(event) => { if (!savingCapture) void saveCapture('clipboard_recording_type_set', { kind, enabled: event.target.checked }); }} />
          </label>)}
        </fieldset>
        {captureStatus && <p className="clipboard-setting-note" role="status">{t(captureStatus)}</p>}
        {captureError && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
        <ClipboardExcludedApps t={t} state={options} available={canCapture} onPick={() => setRecording(false)} />
      </div>
    </section>
    <ClipboardReuseSettings t={t} state={options} platformAvailable={canCapture} storageAvailable={canManage}
      locale={resolveLanguage(snapshot.locale)} onDialogOpen={() => setRecording(false)} />
    <section aria-labelledby="clipboard-shortcut-heading">
      <h2 id="clipboard-shortcut-heading"><Keyboard aria-hidden="true" />{t('clipboardKeyboardShortcuts')}</h2>
      <div className="settings-group">
        <div className="settings-field" data-recording={recording || undefined}>
          <div className="settings-field-content"><span>{t('clipboardOpenHistory')}</span></div>
          {canRecord ? <div className="settings-recorder-wrap">
            <button type="button" ref={recorder} className="settings-recorder" aria-pressed={recording} aria-describedby="clipboard-shortcut-feedback" aria-label={`${t('clipboardOpenHistory')} ${t('clipboardShortcutLabel')}${keys.length ? `: ${keys.join(' ')}` : ''}`} aria-disabled={savingShortcut || undefined} onClick={() => { if (!savingShortcut) toggleRecorder(); }}>
              {recording ? t('clipboardPressShortcut') : keys.length ? <span className="keycaps">{keys.map((key) => <kbd key={key}>{key}</kbd>)}</span> : t('clipboardAddHotkey')}
            </button>
            <button type="button" className="settings-action" disabled={Boolean(isDefault) || savingShortcut} onClick={() => { recorder.current?.focus(); void saveShortcut(defaultClipboardBinding, 'clipboardShortcutRestored'); }}>{t('restoreDefaults')}</button>
          </div> : <span className="settings-badge">{t(snapshot.storageError ? 'clipboardSettingsSaveFailed' : 'platformUnavailable')}</span>}
        </div>
        <p className="clipboard-setting-note" id="clipboard-shortcut-feedback" role={shortcutFeedback?.error ? 'alert' : 'status'} hidden={!recording && !shortcutFeedback}>
          {recording ? t('clipboardShortcutInstructions') : shortcutFeedback?.text}
        </p>
      </div>
    </section>
    {recording && canRecord && <ShortcutCapture onCancel={() => setRecording(false)} onCommit={(next) => void saveShortcut(next, 'clipboardShortcutSaved')} onClear={() => void saveShortcut(null, 'clipboardShortcutCleared')} />}
  </div>;
}
