import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import { clipboardPanelShortcuts, clipboardShortcutAvailable, clipboardOnlyActions, sharedRecordActions, clipboardShortcutLabels, defaultClipboardShortcuts, sameClipboardBinding, type ClipboardRecordAction } from '../shared/clipboardShortcuts';
import { translationShortcutEnabled } from '../shared/contracts';
import { defaultClipboardBinding, sameBinding, shortcutTitles, type KeyBinding, type ShortcutAction } from '../shared/shortcuts';
import { SettingsDialog } from './SettingsDialog';
import { ManualShortcut, ShortcutCapture, ShortcutRow, errorCode, shortcutMessage } from './ShortcutCapture';

type ClipboardAction = 'global' | ClipboardRecordAction;
type Recorder = ({ domain: 'clipboard'; action: ClipboardAction } | { domain: 'translation'; action: ShortcutAction }) & { manual?: boolean };

export function HotKeySettings({ t, snapshot, native }: Pick<ViewProps, 't' | 'snapshot' | 'native'>) {
  const [recording, setRecording] = useState<Recorder | null>(null);
  const [clipboardError, setClipboardError] = useState<{ group: 'general' | 'clipboard'; message: string }>();
  const [translationError, setTranslationError] = useState<string | null>(null);
  const [restoring, setRestoring] = useState(false);
  const [savingClipboard, setSavingClipboard] = useState(false);
  const pendingClipboard = useRef(false);
  const preferences = snapshot.translationPreferences;
  const canManageClipboard = native && !snapshot.storageError;
  const canRecordClipboard = canManageClipboard && snapshot.capabilities.selection;
  const canUseSelection = snapshot.capabilities.selection;
  const macos = snapshot.capabilities.platform === 'macos';
  const clipboardShortcuts = clipboardPanelShortcuts(snapshot);
  const clipboardDefaults = defaultClipboardShortcuts(snapshot.capabilities.primaryModifier);
  const clipboardIsDefault = snapshot.clipboardShortcutEnabled && snapshot.clipboardBinding && sameBinding(snapshot.clipboardBinding, defaultClipboardBinding)
    && clipboardOnlyActions.every((action) => {
      const shortcut = snapshot.clipboardControls.shortcuts[action];
      return shortcut.enabled && shortcut.binding && sameClipboardBinding(shortcut.binding, clipboardDefaults[action].binding);
    });
  useEffect(() => { setRecording(null); setClipboardError(undefined); setTranslationError(null); }, [snapshot.locale]);

  async function saveClipboard(action: ClipboardAction, change: { binding: KeyBinding | null } | { enabled: boolean } | { restore: true }) {
    if (pendingClipboard.current) return;
    pendingClipboard.current = true;
    setRecording(null);
    setSavingClipboard(true);
    setClipboardError(undefined);
    try {
      await (action === 'global' ? invoke('clipboard_shortcut_set', change) : invoke('clipboard_record_shortcut_set', { action, ...change }));
    } catch (error) {
      setClipboardError({ group: action !== 'global' && sharedRecordActions.some((shared) => shared === action) ? 'general' : 'clipboard', message: shortcutMessage(errorCode(error), t) });
    } finally {
      pendingClipboard.current = false;
      setSavingClipboard(false);
    }
  }

  async function saveTranslation(action: ShortcutAction, change: { binding: KeyBinding | null } | { enabled: boolean }) {
    setRecording(null);
    setTranslationError(null);
    try { await invoke('translation_shortcut_set', { action, ...change }); }
    catch (error) { setTranslationError(shortcutMessage(errorCode(error), t)); }
  }

  async function restore(general: boolean) {
    if (restoring || savingClipboard) return;
    setRecording(null);
    setTranslationError(null);
    setClipboardError(undefined);
    setRestoring(true);
    try { await invoke('translation_shortcuts_restore', { general }); }
    catch (error) { setTranslationError(shortcutMessage(errorCode(error), t)); }
    finally { setRestoring(false); }
  }

  function toggleRecorder(next: Recorder) {
    setClipboardError(undefined);
    setTranslationError(null);
    setRecording((current) => current?.domain === next.domain && current.action === next.action ? null : next);
  }

  function openManual() { setRecording((current) => current && { ...current, manual: true }); }

  function translationRow(action: ShortcutAction) {
    const title = t(shortcutTitles[action]);
    return <ShortcutRow key={action} title={title} label={`${title} ${t('clipboardShortcutLabel')}`}
      hint={action === 'selection' ? t('autofillInputHint') : action === 'openTranslation' ? t('openTranslationHint') : undefined} binding={preferences.shortcuts[action]} macos={macos}
      unavailable={(action === 'selection' || action === 'openTranslation') && !canUseSelection ? t('platformUnavailable') : undefined}
      recording={recording?.domain === 'translation' && recording.action === action} enabled={translationShortcutEnabled(preferences, action)}
      onEnabled={(enabled) => void saveTranslation(action, { enabled })} onToggle={() => toggleRecorder({ domain: 'translation', action })}
      onManual={openManual} onClear={() => void saveTranslation(action, { binding: null })} t={t} />;
  }

  function clipboardRow(action: ClipboardAction, title: string, binding: KeyBinding | null, enabled: boolean, unavailable?: string, hint?: string) {
    return <ShortcutRow key={action} title={title} label={`${title} ${t('clipboardShortcutLabel')}`} hint={hint} unavailable={unavailable}
      binding={binding} macos={macos} recording={recording?.domain === 'clipboard' && recording.action === action} saving={savingClipboard}
      enabled={enabled} onEnabled={(enabled) => void saveClipboard(action, { enabled })} onToggle={() => toggleRecorder({ domain: 'clipboard', action })}
      onManual={openManual} onClear={() => void saveClipboard(action, { binding: null })} t={t} />;
  }

  function localRow(action: ClipboardRecordAction) {
    const shortcut = clipboardShortcuts.find((shortcut) => shortcut.action === action)!;
    return clipboardRow(action, t(clipboardShortcutLabels[action]), shortcut.binding, shortcut.enabled,
      canManageClipboard && clipboardShortcutAvailable(snapshot, native, action) ? undefined : t('platformUnavailable'),
      shortcut.globalConflict ? t('clipboardShortcutGlobalConflict') : undefined);
  }

  const canCapture = recording?.domain === 'translation' ? native
    : recording?.action === 'global' ? canRecordClipboard : canManageClipboard;

  function commitRecording(binding: KeyBinding | null) {
    if (recording?.domain === 'translation') void saveTranslation(recording.action, { binding });
    else if (recording?.domain === 'clipboard') void saveClipboard(recording.action, { binding });
  }

  return <div className="settings-sections hotkey-settings">
    <section aria-labelledby="general-shortcut-heading">
      <div className="settings-section-heading">
        <h2 id="general-shortcut-heading">{t('general')}</h2>
        <button type="button" className="settings-action" disabled={!canManageClipboard || restoring || savingClipboard} onClick={() => void restore(true)}>{t('restoreDefaultHotKeys')}</button>
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="general-shortcut-heading">
        {translationRow('focusSearch')}
        {(['navigateUp', 'navigateDown', 'switchView', 'previousCategory', 'nextCategory', 'focusRecords', 'preview', 'copyRecord', 'favorite', 'pin', 'note', 'delete'] as const).map(localRow)}
        {translationRow('pin')}
        {translationRow('showShortcuts')}
      </div>
      {clipboardError?.group === 'general' && <p className="settings-field-note" role="alert">{clipboardError.message}</p>}
    </section>
    <section aria-labelledby="clipboard-shortcut-heading">
      <div className="settings-section-heading">
        <h2 id="clipboard-shortcut-heading">{t('clipboard')}</h2>
        {canRecordClipboard && <button type="button" className="settings-action" disabled={Boolean(clipboardIsDefault) || savingClipboard || restoring}
          onClick={() => void saveClipboard('global', { restore: true })}>{t('restoreDefaultHotKeys')}</button>}
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="clipboard-shortcut-heading">
        {clipboardRow('global', t('clipboardOpenHistory'), snapshot.clipboardBinding, snapshot.clipboardShortcutEnabled,
          canRecordClipboard ? undefined : t(snapshot.storageError ? 'clipboardSettingsSaveFailed' : 'platformUnavailable'))}
        {localRow('translate')}
        {localRow('ocr')}
      </div>
      {clipboardError?.group === 'clipboard' && <p className="settings-field-note" role="alert">{clipboardError.message}</p>}
    </section>
    <section aria-labelledby="translation-shortcuts-heading">
      <div className="settings-section-heading">
        <h2 id="translation-shortcuts-heading">{t('translationPage')}</h2>
        <button type="button" className="settings-action" disabled={!canManageClipboard || restoring || savingClipboard} onClick={() => void restore(false)}>{t('restoreDefaultHotKeys')}</button>
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="translation-shortcuts-heading">
        {(['selection', 'openTranslation', 'translate', 'toggleHistory', 'copyPrimary', 'copySecondary'] as const).map(translationRow)}
      </div>
    </section>
    {translationError && <SettingsDialog title={t('shortcutUnavailable')} description={translationError} onClose={() => setTranslationError(null)}>
      <div className="settings-dialog-actions"><button type="button" className="settings-action" data-initial-focus onClick={() => setTranslationError(null)}>{t('alertOk')}</button></div>
    </SettingsDialog>}
    {recording && canCapture && (recording.manual
      ? <ManualShortcut macos={macos} t={t} onCancel={() => setRecording(null)} onCommit={commitRecording} />
      : <ShortcutCapture onCancel={() => setRecording(null)} onCommit={commitRecording} />)}
  </div>;
}
