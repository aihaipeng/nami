import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MessageCircle, X } from 'lucide-react';
import type { ViewProps } from '../App';
import { languageName, providerLanguages, supportsLanguage, supportsTranslation } from '../shared/languages';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { bindingFromEvent, keycapLabels, normalizeBinding, shortcutActions, type KeyBinding, type ShortcutAction, type TranslationShortcuts } from '../shared/shortcuts';
import type { Translate } from '../shared/i18n';

const titles = {
  selection: 'translateSelected',
  translate: 'translation',
  copyPrimary: 'copyPrimary',
  copySecondary: 'copySecondary',
  pin: 'pinWindow',
  actions: 'actions',
} as const;

const proxyPlaceholder = 'http://proxy.example.com:8080';

function shortcutMessage(code: string, t: Translate) {
  if (code === 'shortcut_reserved') return t('shortcutReserved');
  if (code === 'shortcut_modifier') return t('shortcutModifier');
  if (code === 'shortcut_clipboard') return t('shortcutClipboard');
  if (code.startsWith('shortcut_conflict:')) {
    const action = code.slice('shortcut_conflict:'.length) as ShortcutAction;
    return shortcutActions.includes(action) ? `${t('shortcutConflict')} ${t(titles[action])}.` : t('shortcutKey');
  }
  return t('shortcutKey');
}

function errorCode(error: unknown) {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return 'shortcut_key';
}

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

  return <div className="finder-settings">
    <header className="finder-intro">
      <div className="finder-emblem" aria-hidden="true"><MessageCircle /></div>
      <h1>{t('translationPage')}</h1>
      <p>{t('translationIntro')}</p>
    </header>
    <div className="finder-card">
      <SettingsChoice label={t('settingsService')} value={preferences.engine} open={open === 'engine'} onOpenChange={(value) => setOpen(value ? 'engine' : null)} searchable={false} menuWidth={240}
        items={[{ code: 'edge', name: 'Microsoft Edge Translate' }, { code: 'google', name: 'Google Translate' }]}
        onChange={(engine) => void command('translation_preferences_set', { engine })} t={t} />
      <SettingsChoice label={t('settingsSource')} value={preferences.sourceLanguage} open={open === 'source'} onOpenChange={(value) => setOpen(value ? 'source' : null)}
        items={[{ code: 'auto', name: 'Auto-Detect' }, ...sourceItems]} valueLabel={selectedLabel(preferences.sourceLanguage, 'source')} onChange={(sourceLanguage) => void command('translation_preferences_set', { sourceLanguage })} t={t} />
      <SettingsChoice label={t('settingsPrimary')} value={preferences.primaryLanguage} open={open === 'primary'} onOpenChange={(value) => setOpen(value ? 'primary' : null)}
        items={targetItems} valueLabel={selectedLabel(preferences.primaryLanguage, 'target')} onChange={(primaryLanguage) => void command('translation_preferences_set', { primaryLanguage })} t={t} />
      <SettingsChoice label={t('settingsSecondary')} value={preferences.secondaryLanguage} open={open === 'secondary'} onOpenChange={(value) => setOpen(value ? 'secondary' : null)}
        items={[{ code: 'none', name: 'None' }, ...targetItems]} valueLabel={selectedLabel(preferences.secondaryLanguage, 'target')} onChange={(secondaryLanguage) => void command('translation_preferences_set', { secondaryLanguage })} t={t} />
      <div className="finder-row finder-proxy-row">
        <span>{t('httpProxy')}</span>
        <div className="finder-proxy-slot">
          <label className="finder-proxy">
            <span aria-hidden="true">{proxyUrl || proxyPlaceholder}</span>
            <input aria-label={t('httpProxy')} placeholder={proxyPlaceholder} value={proxyUrl} spellCheck={false}
              onChange={(event) => { const next = event.target.value; setProxyUrl(next); if (proxyContainsCredentials(next)) { setProxyError(t('proxyCredentials')); return; } setProxyError(null); void command('translation_preferences_set', { proxyUrl: next }); }} />
          </label>
        </div>
        <input className="finder-switch" type="checkbox" role="switch" aria-label={t('useHttpProxy')} checked={preferences.proxyEnabled} onChange={(event) => void command('translation_preferences_set', { proxyEnabled: event.target.checked })} />
      </div>
      <div className="finder-row">
        <span>{t('translateSelected')}</span>
        <input className="finder-switch" type="checkbox" role="switch" aria-label={t('translateSelected')} checked={preferences.selectionEnabled} disabled={!snapshot.capabilities.selection}
          onChange={(event) => void command('translation_preferences_set', { selectionEnabled: event.target.checked })} />
      </div>
    </div>
    {!languagesAvailable && <p className="settings-callout" role="status">{t('languageNotSupported')}</p>}
    <div className="finder-shortcut-heading"><h2>{t('keyboardShortcut')}</h2>
      <button type="button" className="finder-restore" onClick={() => { setShortcutError(null); void invoke('translation_shortcuts_restore').catch((error) => setShortcutError(shortcutMessage(errorCode(error), t))); }}>{t('restoreDefaults')}</button>
    </div>
    <div className="finder-card">
      {shortcutActions.map((action) => <ShortcutRow key={action} action={action} title={t(titles[action])} binding={preferences.shortcuts[action]} macos={macos}
        disabled={action === 'selection' && !snapshot.capabilities.selection} unavailable={t('platformUnavailable')} recording={recording === action}
        onRecord={() => { setShortcutError(null); setRecording(action); }} onCancel={() => setRecording(null)} onCommit={(binding) => void saveShortcut(action, binding)} onClear={() => void saveShortcut(action, null)} t={t} />)}
    </div>
    {proxyError && <div className="settings-callout settings-error" role="alert">{proxyError}</div>}
    {shortcutError && <div className="finder-alert-backdrop"><div className="finder-alert" role="alertdialog" aria-labelledby="shortcut-alert-title">
      <h3 id="shortcut-alert-title">{t('shortcutUnavailable')}</h3>
      <p role="alert">{shortcutError}</p>
      <button type="button" onClick={() => setShortcutError(null)}>{t('alertOk')}</button>
    </div></div>}
    {native && recording && <ShortcutCapture onCancel={() => setRecording(null)} onCommit={(binding) => void saveShortcut(recording, binding)} onClear={() => void saveShortcut(recording, null)} onCycle={cycleRecording} />}
  </div>;
}

function SettingsChoice({ label, value, items, open, onOpenChange, onChange, t, searchable = true, menuWidth = 200, valueLabel }: {
  label: string; value: string; items: { code: string; name: string }[]; open: boolean; onOpenChange: (open: boolean) => void; onChange: (value: string) => void; t: Translate; searchable?: boolean; menuWidth?: number; valueLabel?: string;
}) {
  return <div className="finder-row"><span>{label}</span><SearchableDropdown label={label} value={value} items={items} open={open} onOpenChange={onOpenChange} onChange={onChange} t={t} side="bottom" searchable={searchable} settings menuWidth={menuWidth} valueLabel={valueLabel} /></div>;
}

function ShortcutRow({ action, title, binding, macos, disabled, unavailable, recording, onRecord, onCancel, onCommit, onClear, t }: {
  action: ShortcutAction; title: string; binding: TranslationShortcuts[ShortcutAction]; macos: boolean; disabled: boolean; unavailable: string; recording: boolean;
  onRecord: () => void; onCancel: () => void; onCommit: (binding: KeyBinding) => void; onClear: () => void; t: Translate;
}) {
  return <div className="finder-row" data-recording={recording || undefined}><span>{title}</span>
    {disabled ? <span className="settings-badge">{unavailable}</span> : <span className="finder-recorder" data-empty={binding ? undefined : true} data-recording={recording || undefined}>
      {binding && <button type="button" className="finder-clear" aria-label={t('clearShortcut')} onClick={onClear}><X aria-hidden="true" /></button>}
      <button type="button" className="finder-recorder-hit" aria-label={`${title} shortcut`} aria-pressed={recording}
        onClick={() => recording ? onCancel() : onRecord()}>
        {binding ? <span className="keycaps">{keycapLabels(binding, macos).map((key) => <kbd key={`${action}-${key}`}>{key}</kbd>)}</span> : <em>{t('addHotkey')}</em>}
      </button>
    </span>}
  </div>;
}

function proxyContainsCredentials(value: string) {
  const body = value.trim().split('://')[1];
  return Boolean(body?.split(/[/?#]/)[0]?.includes('@'));
}

function ShortcutCapture({ onCancel, onCommit, onClear, onCycle }: { onCancel: () => void; onCommit: (binding: KeyBinding) => void; onClear: () => void; onCycle: (backward: boolean) => void }) {
  const cancel = useRef(onCancel);
  const commit = useRef(onCommit);
  const clear = useRef(onClear);
  const cycle = useRef(onCycle);
  cancel.current = onCancel;
  commit.current = onCommit;
  clear.current = onClear;
  cycle.current = onCycle;
  useEffect(() => {
    void invoke('translation_shortcut_capture', { active: true }).catch(() => {});
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.key === 'Escape') { cancel.current(); return; }
      if (event.key === 'Tab') { cycle.current(event.shiftKey); return; }
      if (event.key === 'Backspace' || event.key === 'Delete') { clear.current(); return; }
      const binding = bindingFromEvent(event);
      if (binding) commit.current(binding);
    };
    const onPointer = (event: MouseEvent) => {
      const row = document.querySelector('[data-recording]');
      if (row && event.target instanceof Node && row.contains(event.target)) return;
      cancel.current();
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('mousedown', onPointer, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('mousedown', onPointer, true);
      void invoke('translation_shortcut_capture', { active: false }).catch(() => {});
    };
  }, []);
  return null;
}
