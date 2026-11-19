import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'lucide-react';
import type { ViewProps } from '../App';
import { languageName, providerLanguages, supportsLanguage, supportsTranslation } from '../shared/languages';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { keycapLabels, normalizeBinding, shortcutActions, type KeyBinding, type ShortcutAction, type TranslationShortcuts } from '../shared/shortcuts';
import type { Translate } from '../shared/i18n';
import { SettingsDialog } from './SettingsDialog';
import { ShortcutCapture, errorCode, shortcutMessage, shortcutTitles } from './ShortcutCapture';

const proxyPlaceholder = 'http://proxy.example.com:8080';

export function TranslationSettings({ t, snapshot, command, native }: Omit<ViewProps, 'activity'>) {
  const preferences = snapshot.translationPreferences;
  const [open, setOpen] = useState<'engine' | 'source' | 'primary' | 'secondary' | null>(null);
  const [proxyUrl, setProxyUrl] = useState(preferences.proxyUrl);
  const [recording, setRecording] = useState<ShortcutAction | null>(null);
  const [shortcutError, setShortcutError] = useState<string | null>(null);
  const [proxyError, setProxyError] = useState<string | null>(null);
  const macos = snapshot.capabilities.platform === 'macos';
  const sourceItems = providerLanguages(preferences.engine, 'source');
  const targetItems = providerLanguages(preferences.engine, 'target');
  const languagesAvailable = supportsTranslation(preferences.engine, preferences.sourceLanguage, [preferences.primaryLanguage, ...(preferences.secondaryLanguage === 'none' ? [] : [preferences.secondaryLanguage])]);
  const selectedLabel = (value: string, side: 'source' | 'target') => value === 'none' || supportsLanguage(preferences.engine, side, value) ? undefined : `${languageName(value)} (${t('languageUnavailable')})`;
  const recordable = shortcutActions.filter((action) => action !== 'selection' || snapshot.capabilities.selection);
  useEffect(() => { setProxyUrl(preferences.proxyUrl); }, [preferences.proxyUrl]);
  useEffect(() => { setOpen(null); setRecording(null); }, [snapshot.locale]);

  async function saveShortcut(action: ShortcutAction, binding: KeyBinding | null) {
    const next = binding ? normalizeBinding(binding) : null;
    if (binding && !next) {
      setShortcutError(shortcutMessage(binding.modifiers.some((modifier) => modifier === 'meta' || modifier === 'control' || modifier === 'alt') ? 'shortcut_key' : 'shortcut_modifier', t));
      setRecording(null);
      return;
    }
    try {
      await invoke('translation_shortcut_set', { action, binding: next });
      setShortcutError(null);
    } catch (error) {
      setShortcutError(shortcutMessage(errorCode(error), t));
    }
    setRecording(null);
  }

  function cycleRecording(backward: boolean) {
    if (!recording) return;
    const index = recordable.indexOf(recording);
    const next = recordable[(index + (backward ? recordable.length - 1 : 1)) % recordable.length];
    setRecording(next ?? null);
  }

  return <div className="settings-sections">
    <section aria-labelledby="translation-languages-heading">
      <h2 id="translation-languages-heading">{t('settingsLanguages')}</h2>
      <div className="settings-group">
        <SettingsChoice label={t('settingsService')} value={preferences.engine} open={open === 'engine'} onOpenChange={(value) => setOpen(value ? 'engine' : null)} searchable={false} menuWidth={240}
          items={[{ code: 'edge', name: 'Microsoft Edge Translate' }, { code: 'google', name: 'Google Translate' }]}
          onChange={(engine) => void command('translation_preferences_set', { engine })} t={t} />
        <SettingsChoice label={t('settingsSource')} value={preferences.sourceLanguage} open={open === 'source'} onOpenChange={(value) => setOpen(value ? 'source' : null)}
          items={[{ code: 'auto', name: 'Auto-Detect' }, ...sourceItems]} valueLabel={selectedLabel(preferences.sourceLanguage, 'source')} onChange={(sourceLanguage) => void command('translation_preferences_set', { sourceLanguage })} t={t} />
        <SettingsChoice label={t('settingsPrimary')} value={preferences.primaryLanguage} open={open === 'primary'} onOpenChange={(value) => setOpen(value ? 'primary' : null)}
          items={targetItems} valueLabel={selectedLabel(preferences.primaryLanguage, 'target')} onChange={(primaryLanguage) => void command('translation_preferences_set', { primaryLanguage })} t={t} />
        <SettingsChoice label={t('settingsSecondary')} value={preferences.secondaryLanguage} open={open === 'secondary'} onOpenChange={(value) => setOpen(value ? 'secondary' : null)}
          items={[{ code: 'none', name: 'None' }, ...targetItems]} valueLabel={selectedLabel(preferences.secondaryLanguage, 'target')} onChange={(secondaryLanguage) => void command('translation_preferences_set', { secondaryLanguage })} t={t} />
        {!languagesAvailable && <p className="settings-field-note" role="status">{t('languageNotSupported')}</p>}
      </div>
    </section>
    <section aria-labelledby="translation-connection-heading">
      <h2 id="translation-connection-heading">{t('settingsConnection')}</h2>
      <div className="settings-group">
        <label className="settings-field">
          <span className="settings-field-content">{t('useHttpProxy')}</span>
          <input className="settings-switch" type="checkbox" role="switch" aria-label={t('useHttpProxy')} checked={preferences.proxyEnabled}
            onChange={(event) => void command('translation_preferences_set', { proxyEnabled: event.target.checked })} />
        </label>
        <label className="settings-field">
          <span className="settings-field-content">{t('httpProxy')}</span>
          <input className="settings-text-input" aria-label={t('httpProxy')} placeholder={proxyPlaceholder} value={proxyUrl} spellCheck={false}
            aria-invalid={Boolean(proxyError)} aria-describedby={proxyError ? 'translation-proxy-error' : undefined}
            onChange={(event) => { const next = event.target.value; setProxyUrl(next); if (proxyContainsCredentials(next)) { setProxyError(t('proxyCredentials')); return; } setProxyError(null); void command('translation_preferences_set', { proxyUrl: next }); }} />
        </label>
        {proxyError && <p id="translation-proxy-error" className="settings-field-note settings-error" role="alert">{proxyError}</p>}
      </div>
    </section>
    <section aria-labelledby="translation-shortcuts-heading">
      <div className="settings-section-heading">
        <h2 id="translation-shortcuts-heading">{t('keyboardShortcut')}</h2>
        <button type="button" className="settings-action" onClick={(event) => { event.currentTarget.focus({ preventScroll: true }); setShortcutError(null); void invoke('translation_shortcuts_restore').catch((error) => setShortcutError(shortcutMessage(errorCode(error), t))); }}>{t('restoreDefaults')}</button>
      </div>
      <div className="settings-group">
        <label className="settings-field">
          <span className="settings-field-content">{t('translateSelected')}</span>
          <input className="settings-switch" type="checkbox" role="switch" aria-label={t('translateSelected')} checked={preferences.selectionEnabled} disabled={!snapshot.capabilities.selection}
            onChange={(event) => void command('translation_preferences_set', { selectionEnabled: event.target.checked })} />
        </label>
        {shortcutActions.map((action) => <ShortcutRow key={action} action={action} title={action === 'selection' ? t('selectionShortcut') : t(shortcutTitles[action])} ariaLabel={`${t(shortcutTitles[action])} shortcut`} binding={preferences.shortcuts[action]} macos={macos}
          disabled={action === 'selection' && !snapshot.capabilities.selection} unavailable={t('platformUnavailable')} recording={recording === action}
          onRecord={() => { setShortcutError(null); setRecording(action); }} onCancel={() => setRecording(null)} onClear={() => void saveShortcut(action, null)} t={t} />)}
      </div>
    </section>
    {shortcutError && <SettingsDialog title={t('shortcutUnavailable')} description={shortcutError} onClose={() => setShortcutError(null)}>
      <div className="settings-dialog-actions"><button type="button" className="settings-action" data-initial-focus onClick={() => setShortcutError(null)}>{t('alertOk')}</button></div>
    </SettingsDialog>}
    {native && recording && <ShortcutCapture onCancel={() => setRecording(null)} onCommit={(binding) => void saveShortcut(recording, binding)} onClear={() => void saveShortcut(recording, null)} onCycle={cycleRecording} />}
  </div>;
}

function SettingsChoice({ label, value, items, open, onOpenChange, onChange, t, searchable = true, menuWidth = 200, valueLabel }: {
  label: string; value: string; items: { code: string; name: string }[]; open: boolean; onOpenChange: (open: boolean) => void; onChange: (value: string) => void; t: Translate; searchable?: boolean; menuWidth?: number; valueLabel?: string;
}) {
  return <div className="settings-field"><span className="settings-field-content">{label}</span><SearchableDropdown label={label} value={value} items={items} open={open} onOpenChange={onOpenChange} onChange={onChange} t={t} side="bottom" searchable={searchable} settings menuWidth={menuWidth} valueLabel={valueLabel} /></div>;
}

function ShortcutRow({ action, title, ariaLabel, binding, macos, disabled, unavailable, recording, onRecord, onCancel, onClear, t }: {
  action: ShortcutAction; title: string; ariaLabel: string; binding: TranslationShortcuts[ShortcutAction]; macos: boolean; disabled: boolean; unavailable: string; recording: boolean;
  onRecord: () => void; onCancel: () => void; onClear: () => void; t: Translate;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (recording) trigger.current?.focus({ preventScroll: true }); }, [recording]);
  return <div className="settings-field" data-recording={recording || undefined}><span className="settings-field-content">{title}</span>
    {disabled ? <span className="settings-badge">{unavailable}</span> : <span className="settings-recorder-wrap">
      {binding && <button type="button" className="settings-clear" aria-label={t('clearShortcut')} onClick={() => { trigger.current?.focus({ preventScroll: true }); onClear(); }}><X aria-hidden="true" /></button>}
      <button ref={trigger} type="button" className="settings-recorder" aria-label={ariaLabel} aria-pressed={recording}
        onClick={() => recording ? onCancel() : onRecord()}>
        {recording ? t('pressShortcut') : binding ? <span className="keycaps">{keycapLabels(binding, macos).map((key) => <kbd key={`${action}-${key}`}>{key}</kbd>)}</span> : t('addHotkey')}
      </button>
    </span>}
  </div>;
}

function proxyContainsCredentials(value: string) {
  const body = value.trim().split('://')[1];
  return Boolean(body?.split(/[/?#]/)[0]?.includes('@'));
}
