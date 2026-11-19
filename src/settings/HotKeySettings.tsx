import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import { clipboardPanelShortcuts, clipboardShortcutAvailable, clipboardShortcutLabels, defaultClipboardShortcuts, sameClipboardBinding, type ClipboardRecordAction } from '../shared/clipboardShortcuts';
import { translationShortcutEnabled } from '../shared/contracts';
import { defaultClipboardBinding, generalShortcutActions, normalizeBinding, sameBinding, shortcutActions, shortcutTitles, translationWindowActions, type KeyBinding, type ShortcutAction } from '../shared/shortcuts';
import { SettingsDialog } from './SettingsDialog';
import { ShortcutCapture, ShortcutRow, errorCode, shortcutMessage } from './ShortcutCapture';

type ClipboardAction = 'global' | ClipboardRecordAction;
type Recorder = { domain: 'clipboard'; action: ClipboardAction } | { domain: 'translation'; action: ShortcutAction };

export function HotKeySettings({ t, snapshot, native }: Pick<ViewProps, 't' | 'snapshot' | 'native'>) {
  const [recording, setRecording] = useState<Recorder | null>(null);
  const [clipboardError, setClipboardError] = useState<{ group: 'general' | 'clipboard'; message: string }>();
  const [translationError, setTranslationError] = useState<string | null>(null);
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
    && clipboardShortcuts.every(({ action, enabled, binding }) => enabled && binding && sameClipboardBinding(binding, clipboardDefaults[action].binding));
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
      setClipboardError({ group: action === 'global' && !('restore' in change) ? 'general' : 'clipboard', message: shortcutMessage(errorCode(error), t) });
    } finally {
      pendingClipboard.current = false;
      setSavingClipboard(false);
    }
  }

  async function saveTranslation(action: ShortcutAction, change: { binding: KeyBinding | null } | { enabled: boolean }) {
    setRecording(null);
    setTranslationError(null);
    if ('binding' in change && change.binding) {
      const binding = normalizeBinding(change.binding);
      if (!binding) {
        setTranslationError(shortcutMessage(change.binding.modifiers.some((modifier) => modifier === 'meta' || modifier === 'control' || modifier === 'alt') ? 'shortcut_key' : 'shortcut_modifier', t));
        return;
      }
      change = { binding };
    }
    try { await invoke('translation_shortcut_set', { action, ...change }); }
    catch (error) { setTranslationError(shortcutMessage(errorCode(error), t)); }
  }

  function toggleRecorder(next: Recorder) {
    setClipboardError(undefined);
    setTranslationError(null);
    setRecording((current) => current?.domain === next.domain && current.action === next.action ? null : next);
  }

  function cycleTranslation(backward: boolean) {
    if (recording?.domain !== 'translation') return;
    const actions = shortcutActions.filter((action) => canUseSelection || action !== 'selection');
    const index = actions.indexOf(recording.action);
    setRecording({ domain: 'translation', action: actions[(index + (backward ? actions.length - 1 : 1)) % actions.length] });
  }

  function translationRow(action: ShortcutAction) {
    const title = t(shortcutTitles[action]);
    return <ShortcutRow key={action} title={title} label={action === 'selection' ? t('selectionShortcut') : `${title} ${t('clipboardShortcutLabel')}`}
      hint={action === 'selection' ? t('openTranslationHint') : undefined} binding={preferences.shortcuts[action]} macos={macos}
      unavailable={action === 'selection' && !canUseSelection ? t('platformUnavailable') : undefined}
      recording={recording?.domain === 'translation' && recording.action === action} enabled={translationShortcutEnabled(preferences, action)}
      onEnabled={(enabled) => void saveTranslation(action, { enabled })} onToggle={() => toggleRecorder({ domain: 'translation', action })}
      onClear={() => void saveTranslation(action, { binding: null })} t={t} />;
  }

  function clipboardRow(action: ClipboardAction, title: string, binding: KeyBinding | null, enabled: boolean, unavailable?: string, hint?: string) {
    return <ShortcutRow key={action} title={title} label={`${title} ${t('clipboardShortcutLabel')}`} hint={hint} unavailable={unavailable}
      binding={binding} macos={macos} recording={recording?.domain === 'clipboard' && recording.action === action} saving={savingClipboard}
      enabled={enabled} onEnabled={(enabled) => void saveClipboard(action, { enabled })} onToggle={() => toggleRecorder({ domain: 'clipboard', action })}
      onClear={() => void saveClipboard(action, { binding: null })} t={t} />;
  }

  const canCapture = recording?.domain === 'translation' ? native
    : recording?.action === 'global' ? canRecordClipboard : canManageClipboard;

  function commitRecording(binding: KeyBinding | null) {
    if (recording?.domain === 'translation') void saveTranslation(recording.action, { binding });
    else if (recording?.domain === 'clipboard') void saveClipboard(recording.action, { binding });
  }

  return <div className="settings-sections hotkey-settings">
    <section aria-labelledby="general-shortcut-heading">
      <h2 id="general-shortcut-heading">{t('general')}</h2>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="general-shortcut-heading">
        {clipboardRow('global', t('clipboardOpenHistory'), snapshot.clipboardBinding, snapshot.clipboardShortcutEnabled,
          canRecordClipboard ? undefined : t(snapshot.storageError ? 'clipboardSettingsSaveFailed' : 'platformUnavailable'))}
        {generalShortcutActions.map(translationRow)}
      </div>
      {clipboardError?.group === 'general' && <p className="settings-field-note" role="alert">{clipboardError.message}</p>}
    </section>
    <section aria-labelledby="clipboard-shortcut-heading">
      <div className="settings-section-heading">
        <h2 id="clipboard-shortcut-heading">{t('clipboard')}</h2>
        {canRecordClipboard && <button type="button" className="settings-action" disabled={Boolean(clipboardIsDefault) || savingClipboard}
          title={t('restoreClipboardHotKeysHint')} onClick={() => void saveClipboard('global', { restore: true })}>{t('restoreClipboardHotKeys')}</button>}
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="clipboard-shortcut-heading">
        {clipboardShortcuts.map(({ action, binding, enabled, globalConflict }) => clipboardRow(action, t(clipboardShortcutLabels[action]), binding, enabled,
          canManageClipboard && clipboardShortcutAvailable(snapshot, native, action) ? undefined : t('platformUnavailable'),
          globalConflict ? t('clipboardShortcutGlobalConflict') : undefined))}
      </div>
      {clipboardError?.group === 'clipboard' && <p className="settings-field-note" role="alert">{clipboardError.message}</p>}
    </section>
    <section aria-labelledby="translation-shortcuts-heading">
      <div className="settings-section-heading">
        <h2 id="translation-shortcuts-heading">{t('translationPage')}</h2>
        <button type="button" className="settings-action" title={t('restoreTranslationHotKeysHint')} onClick={() => {
          setRecording(null);
          setTranslationError(null);
          void invoke('translation_shortcuts_restore').catch((error) => setTranslationError(shortcutMessage(errorCode(error), t)));
        }}>{t('restoreTranslationHotKeys')}</button>
      </div>
      <div className="settings-group hotkey-group" role="group" aria-labelledby="translation-shortcuts-heading">
        {translationWindowActions.map(translationRow)}
      </div>
    </section>
    {translationError && <SettingsDialog title={t('shortcutUnavailable')} description={translationError} onClose={() => setTranslationError(null)}>
      <div className="settings-dialog-actions"><button type="button" className="settings-action" data-initial-focus onClick={() => setTranslationError(null)}>{t('alertOk')}</button></div>
    </SettingsDialog>}
    {recording && canCapture && <ShortcutCapture onCancel={() => setRecording(null)} onCommit={commitRecording}
      onClear={() => commitRecording(null)} onCycle={recording.domain === 'translation' ? cycleTranslation : undefined} />}
  </div>;
}
