import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import { languages } from '../shared/languages';
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
  const languageItems = languages.map(([code, name]) => ({ code, name }));
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

  return <>
    <div className="settings-group settings-rows">
      <SettingsChoice label={t('service')} value={preferences.engine} open={open === 'engine'} onOpenChange={(value) => setOpen(value ? 'engine' : null)} searchable={false}
        items={[{ code: 'edge', name: 'Microsoft Edge Translate' }, { code: 'google', name: 'Google Translate' }]}
        onChange={(engine) => void command('translation_preferences_set', { engine })} t={t} />
      <SettingsChoice label={t('sourceLanguage')} value={preferences.sourceLanguage} open={open === 'source'} onOpenChange={(value) => setOpen(value ? 'source' : null)}
        items={[{ code: 'auto', name: 'Auto-Detect' }, ...languageItems]} onChange={(sourceLanguage) => void command('translation_preferences_set', { sourceLanguage })} t={t} />
      <SettingsChoice label={t('primaryLanguage')} value={preferences.primaryLanguage} open={open === 'primary'} onOpenChange={(value) => setOpen(value ? 'primary' : null)}
        items={languageItems} onChange={(primaryLanguage) => void command('translation_preferences_set', { primaryLanguage })} t={t} />
      <SettingsChoice label={t('secondaryLanguage')} value={preferences.secondaryLanguage} open={open === 'secondary'} onOpenChange={(value) => setOpen(value ? 'secondary' : null)}
        items={[{ code: 'none', name: 'None' }, ...languageItems]} onChange={(secondaryLanguage) => void command('translation_preferences_set', { secondaryLanguage })} t={t} />
      <div className="settings-row">
        <span>{t('httpProxy')}</span>
        <label className="settings-proxy"><input aria-label={t('httpProxy')} placeholder="http://proxy.example.com:8080" value={proxyUrl} spellCheck={false}
          onChange={(event) => { const proxyUrl = event.target.value; setProxyUrl(proxyUrl); if (proxyContainsCredentials(proxyUrl)) { setProxyError(t('proxyCredentials')); return; } setProxyError(null); void command('translation_preferences_set', { proxyUrl }); }} /></label>
        <input className="settings-switch" type="checkbox" role="switch" aria-label={t('useHttpProxy')} checked={preferences.proxyEnabled} onChange={(event) => void command('translation_preferences_set', { proxyEnabled: event.target.checked })} />
      </div>
    </div>
    <div className="settings-shortcut-heading"><h2>{t('keyboardShortcut')}</h2>
      <button type="button" className="settings-action" onClick={() => { setShortcutError(null); void invoke('translation_shortcuts_restore').catch((error) => setShortcutError(shortcutMessage(errorCode(error), t))); }}>{t('restoreDefaults')}</button>
    </div>
    <div className="settings-group settings-rows">
      {shortcutActions.map((action) => <ShortcutRow key={action} action={action} title={t(titles[action])} binding={preferences.shortcuts[action]} macos={macos}
        disabled={action === 'selection' && !snapshot.capabilities.selection} unavailable={t('platformUnavailable')} recording={recording === action}
        onRecord={() => { setShortcutError(null); setRecording(action); }} onCancel={() => setRecording(null)} onCommit={(binding) => void saveShortcut(action, binding)} onClear={() => void saveShortcut(action, null)} t={t} />)}
    </div>
    {(proxyError || shortcutError) && <div className="settings-callout settings-error" role="alert">{proxyError ?? shortcutError}</div>}
    {native && recording && <ShortcutCapture onCancel={() => setRecording(null)} onCommit={(binding) => void saveShortcut(recording, binding)} onClear={() => void saveShortcut(recording, null)} />}
  </>;
}

function SettingsChoice({ label, value, items, open, onOpenChange, onChange, t, searchable = true }: {
  label: string; value: string; items: { code: string; name: string }[]; open: boolean; onOpenChange: (open: boolean) => void; onChange: (value: string) => void; t: Translate; searchable?: boolean;
}) {
  return <div className="settings-row"><span>{label}</span><SearchableDropdown label={label} value={value} items={items} open={open} onOpenChange={onOpenChange} onChange={onChange} t={t} side="bottom" searchable={searchable} capsule /></div>;
}

function ShortcutRow({ action, title, binding, macos, disabled, unavailable, recording, onRecord, onCancel, onCommit, onClear, t }: {
  action: ShortcutAction; title: string; binding: TranslationShortcuts[ShortcutAction]; macos: boolean; disabled: boolean; unavailable: string; recording: boolean;
  onRecord: () => void; onCancel: () => void; onCommit: (binding: KeyBinding) => void; onClear: () => void; t: Translate;
}) {
  return <div className="settings-row"><span>{title}</span>
    {disabled ? <span className="settings-badge">{unavailable}</span> : <span className="settings-recorder-wrap">
      {binding && <button type="button" className="settings-clear" aria-label={t('clearShortcut')} onClick={onClear}>×</button>}
      <button type="button" className="settings-recorder" data-recording={recording || undefined} aria-label={`${title} shortcut`} aria-pressed={recording} disabled={disabled}
        onClick={() => recording ? onCancel() : onRecord()}>
        {recording ? <em>{t('addShortcut')}</em> : binding ? <span className="keycaps">{keycapLabels(binding, macos).map((key) => <kbd key={`${action}-${key}`}>{key}</kbd>)}</span> : <em>{t('addShortcut')}</em>}
      </button>
    </span>}
  </div>;
}

function proxyContainsCredentials(value: string) {
  const body = value.trim().split('://')[1];
  return Boolean(body?.split(/[/?#]/)[0]?.includes('@'));
}

function ShortcutCapture({ onCancel, onCommit, onClear }: { onCancel: () => void; onCommit: (binding: KeyBinding) => void; onClear: () => void }) {
  const cancel = useRef(onCancel);
  const commit = useRef(onCommit);
  const clear = useRef(onClear);
  cancel.current = onCancel;
  commit.current = onCommit;
  clear.current = onClear;
  useEffect(() => {
    void invoke('translation_shortcut_capture', { active: true }).catch(() => {});
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.key === 'Escape' || event.key === 'Tab') { cancel.current(); return; }
      if (event.key === 'Backspace' || event.key === 'Delete') { clear.current(); return; }
      const binding = bindingFromEvent(event);
      if (binding) commit.current(binding);
    };
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      void invoke('translation_shortcut_capture', { active: false }).catch(() => {});
    };
  }, []);
  return null;
}
