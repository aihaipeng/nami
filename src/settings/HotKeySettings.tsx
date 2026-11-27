import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import { clipboardPanelShortcuts, clipboardShortcutAvailable, clipboardShortcutLabels, type ClipboardRecordAction } from '../shared/clipboardShortcuts';
import { translationShortcutEnabled } from '../shared/contracts';
import type { MessageKey } from '../shared/i18n';
import { shortcutTitles, type KeyBinding, type ShortcutAction } from '../shared/shortcuts';
import { SettingsDialog } from './SettingsDialog';
import { ShortcutCapture, ShortcutRow, errorCode, shortcutMessage } from './ShortcutCapture';

type ClipboardAction = 'global' | ClipboardRecordAction;
type ShortcutRestoreGroup = 'global' | 'window' | 'records' | 'translation';
type ClipboardErrorGroup = 'global' | 'records';
type Recorder = { domain: 'clipboard'; action: ClipboardAction } | { domain: 'translation'; action: ShortcutAction };

const recordActionOrder: readonly ClipboardRecordAction[] = [
  'preview', 'pastePlain', 'favorite', 'pin', 'note',
  'translate', 'ocr', 'openTarget', 'delete',
];

const translationHints: Partial<Record<ShortcutAction, MessageKey>> = {
  openTranslation: 'openTranslationHint',
  selection: 'autofillInputHint',
};

const recordHints: Partial<Record<ClipboardRecordAction, MessageKey>> = {
  pastePlain: 'pastePlainHint',
  translate: 'translateRecordHint',
  ocr: 'ocrRecordHint',
};

export function HotKeySettings({ t, snapshot, native }: Pick<ViewProps, 't' | 'snapshot' | 'native'>) {
  const [recording, setRecording] = useState<Recorder | null>(null);
  const [clipboardError, setClipboardError] = useState<{ group: ClipboardErrorGroup; message: string }>();
  const [translationError, setTranslationError] = useState<string | null>(null);
  const [restoringGroup, setRestoringGroup] = useState<ShortcutRestoreGroup | null>(null);
  const [savingAction, setSavingAction] = useState<ClipboardAction | null>(null);
  const pendingClipboard = useRef(false);
  const preferences = snapshot.translationPreferences;
  const canManageClipboard = native && !snapshot.storageError;
  const canRecordClipboard = canManageClipboard && snapshot.capabilities.selection;
  const canUseSelection = snapshot.capabilities.selection;
  const macos = snapshot.capabilities.platform === 'macos';
  const clipboardShortcuts = clipboardPanelShortcuts(snapshot);
  useEffect(() => { setRecording(null); setClipboardError(undefined); setTranslationError(null); }, [snapshot.locale]);

  async function saveClipboard(action: ClipboardAction, change: { binding: KeyBinding | null } | { enabled: boolean }) {
    if (pendingClipboard.current) return;
    pendingClipboard.current = true;
    setRecording(null);
    setSavingAction(action);
    setClipboardError(undefined);
    try {
      await (action === 'global' ? invoke('clipboard_shortcut_set', change) : invoke('clipboard_record_shortcut_set', { action, ...change }));
    } catch (error) {
      const group: ClipboardErrorGroup = action === 'global' ? 'global' : 'records';
      setClipboardError({ group, message: shortcutMessage(errorCode(error), t) });
    } finally {
      pendingClipboard.current = false;
      setSavingAction(null);
    }
  }

  async function saveTranslation(action: ShortcutAction, change: { binding: KeyBinding | null } | { enabled: boolean }) {
    setRecording(null);
    setTranslationError(null);
    try { await invoke('translation_shortcut_set', { action, ...change }); }
    catch (error) { setTranslationError(shortcutMessage(errorCode(error), t)); }
  }

  async function restore(group: ShortcutRestoreGroup) {
    if (restoringGroup || pendingClipboard.current) return;
    setRecording(null);
    setTranslationError(null);
    setClipboardError(undefined);
    setRestoringGroup(group);
    try { await invoke('translation_shortcuts_restore', { group }); }
    catch (error) { setTranslationError(shortcutMessage(errorCode(error), t)); }
    finally { setRestoringGroup(null); }
  }

  function toggleRecorder(next: Recorder) {
    setClipboardError(undefined);
    setTranslationError(null);
    setRecording((current) => current?.domain === next.domain && current.action === next.action ? null : next);
  }

  function translationRow(action: ShortcutAction) {
    const title = t(shortcutTitles[action]);
    const hintKey = translationHints[action];
    return <ShortcutRow key={action} title={title} label={`${title} ${t('clipboardShortcutLabel')}`}
      hint={hintKey ? t(hintKey) : undefined} binding={preferences.shortcuts[action]} macos={macos}
      unavailable={(action === 'selection' || action === 'openTranslation') && !canUseSelection ? t('platformUnavailable') : undefined}
      recording={recording?.domain === 'translation' && recording.action === action} enabled={translationShortcutEnabled(preferences, action)}
      onEnabled={(enabled) => void saveTranslation(action, { enabled })} onToggle={() => toggleRecorder({ domain: 'translation', action })}
      onClear={() => void saveTranslation(action, { binding: null })} t={t} />;
  }

  function clipboardRow(action: ClipboardAction, title: string, binding: KeyBinding | null, enabled: boolean, unavailable?: string, hint?: string, hintConflict?: boolean) {
    return <ShortcutRow key={action} title={title} label={`${title} ${t('clipboardShortcutLabel')}`} hint={hint} hintConflict={hintConflict} unavailable={unavailable}
      binding={binding} macos={macos} recording={recording?.domain === 'clipboard' && recording.action === action} saving={savingAction === action}
      enabled={enabled} onEnabled={(enabled) => void saveClipboard(action, { enabled })} onToggle={() => toggleRecorder({ domain: 'clipboard', action })}
      onClear={() => void saveClipboard(action, { binding: null })} t={t} />;
  }

  function localRow(action: ClipboardRecordAction) {
    const shortcut = clipboardShortcuts.find((shortcut) => shortcut.action === action)!;
    const hintKey = recordHints[action];
    const hint = shortcut.globalConflict ? t('clipboardShortcutGlobalConflict') : hintKey ? t(hintKey) : undefined;
    return clipboardRow(action, t(clipboardShortcutLabels[action]), shortcut.binding, shortcut.enabled,
      canManageClipboard && clipboardShortcutAvailable(snapshot, native, action) ? undefined : t('platformUnavailable'),
      hint, shortcut.globalConflict);
  }

  const canCapture = recording?.domain === 'translation' ? native
    : recording?.action === 'global' ? canRecordClipboard : canManageClipboard;

  function commitRecording(binding: KeyBinding | null) {
    if (recording?.domain === 'translation') void saveTranslation(recording.action, { binding });
    else if (recording?.domain === 'clipboard') void saveClipboard(recording.action, { binding });
  }

  return <div className="settings-sections hotkey-settings">
    <section aria-labelledby="global-shortcuts-heading">
      <div className="settings-section-heading">
        <h2 id="global-shortcuts-heading">{t('hotkeyGroupGlobal')}</h2>
        {canRecordClipboard && <button type="button" className="settings-action" disabled={restoringGroup === 'global'} onClick={() => void restore('global')}>{t('restoreDefaultHotKeys')}</button>}
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="global-shortcuts-heading">
        {clipboardRow('global', t('clipboardOpenHistory'), snapshot.clipboardBinding, snapshot.clipboardShortcutEnabled,
          canRecordClipboard ? undefined : t(snapshot.storageError ? 'clipboardSettingsSaveFailed' : 'platformUnavailable'))}
        {translationRow('openTranslation')}
        {translationRow('selection')}
      </div>
      {clipboardError?.group === 'global' && <p className="settings-field-note" role="alert">{clipboardError.message}</p>}
    </section>
    <section aria-labelledby="window-shortcuts-heading">
      <div className="settings-section-heading">
        <h2 id="window-shortcuts-heading">{t('hotkeyGroupWindow')}</h2>
        <button type="button" className="settings-action" disabled={!canManageClipboard || restoringGroup === 'window'} onClick={() => void restore('window')}>{t('restoreDefaultHotKeys')}</button>
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="window-shortcuts-heading">
        {translationRow('focusSearch')}
        {translationRow('pin')}
        {translationRow('showShortcuts')}
        {translationRow('openSettings')}
      </div>
    </section>
    <section aria-labelledby="record-shortcuts-heading">
      <div className="settings-section-heading">
        <h2 id="record-shortcuts-heading">{t('hotkeyGroupRecords')}</h2>
        <button type="button" className="settings-action" disabled={!canManageClipboard || restoringGroup === 'records'} onClick={() => void restore('records')}>{t('restoreDefaultHotKeys')}</button>
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="record-shortcuts-heading">
        {recordActionOrder.map(localRow)}
      </div>
      {clipboardError?.group === 'records' && <p className="settings-field-note" role="alert">{clipboardError.message}</p>}
    </section>
    <section aria-labelledby="translation-shortcuts-heading">
      <div className="settings-section-heading">
        <h2 id="translation-shortcuts-heading">{t('translationPage')}</h2>
        <button type="button" className="settings-action" disabled={!canManageClipboard || restoringGroup === 'translation'} onClick={() => void restore('translation')}>{t('restoreDefaultHotKeys')}</button>
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="translation-shortcuts-heading">
        {(['translate', 'copyPrimary', 'copySecondary'] as const).map(translationRow)}
      </div>
    </section>
    {translationError && <SettingsDialog title={t('shortcutUnavailable')} description={translationError} onClose={() => setTranslationError(null)}>
      <div className="settings-dialog-actions"><button type="button" className="settings-action" data-initial-focus onClick={() => setTranslationError(null)}>{t('alertOk')}</button></div>
    </SettingsDialog>}
    {recording && canCapture && <ShortcutCapture onCancel={() => setRecording(null)} onCommit={commitRecording} />}
  </div>;
}
