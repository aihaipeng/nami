import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import type { TranslationPreferences } from '../shared/contracts';
import { SETTINGS_AUTO_SAVE_DELAY_MS } from './autoSave';

export function TranslationProxySettings({ t, preferences, native, command }: Pick<ViewProps, 't' | 'native' | 'command'> & { preferences: TranslationPreferences }) {
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<'proxyCredentials' | 'proxyUrlInvalid' | 'proxySaveFailed' | null>(null);
  const [saving, setSaving] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const autoSaveTimer = useRef<number | null>(null);
  const value = draft ?? preferences.proxyUrl;
  const url = value.trim();
  const changed = url !== preferences.proxyUrl;
  const credentials = proxyContainsCredentials(url);
  const feedback = credentials ? 'proxyCredentials' : error;

  function clearAutoSave() {
    if (autoSaveTimer.current !== null) {
      window.clearTimeout(autoSaveTimer.current);
      autoSaveTimer.current = null;
    }
  }

  useEffect(() => () => clearAutoSave(), []);

  async function save(enabled?: boolean) {
    clearAutoSave();
    if (saving || (enabled === undefined && !changed) || (credentials && enabled !== false)) return;
    const patch = enabled === false ? { proxyEnabled: false }
      : enabled === true ? { proxyEnabled: true, proxyUrl: url } : { proxyUrl: url };
    setError(null);
    setSaving(true);
    try {
      if (native) await invoke('translation_preferences_set', patch);
      else await command('translation_preferences_set', patch);
      if (enabled !== false) setDraft(null);
    } catch (error) {
      setError(error === 'proxy_credentials' ? 'proxyCredentials' : error === 'proxy_url' ? 'proxyUrlInvalid' : 'proxySaveFailed');
    } finally {
      setSaving(false);
    }
  }

  const latestSaveRef = useRef(save);
  latestSaveRef.current = save;

  function handleChange(nextRaw: string) {
    setDraft(nextRaw);
    setError(null);
    clearAutoSave();
    const nextUrl = nextRaw.trim();
    if (nextUrl === preferences.proxyUrl || proxyContainsCredentials(nextUrl)) return;
    autoSaveTimer.current = window.setTimeout(() => {
      autoSaveTimer.current = null;
      void latestSaveRef.current();
    }, SETTINGS_AUTO_SAVE_DELAY_MS);
  }

  return <section aria-labelledby="translation-connection-heading">
    <div className="settings-section-heading">
      <h2 id="translation-connection-heading">{t('settingsConnection')}</h2>
    </div>
    <form className="settings-group" onSubmit={(event) => {
      event.preventDefault();
      input.current?.focus({ preventScroll: true });
      void save();
    }}>
      <div className="settings-field settings-proxy-row">
        <span id="translation-proxy-label" className="settings-field-content">{t('useHttpProxy')}</span>
        <input ref={input} id="translation-proxy-url" className="settings-text-input" aria-label={t('httpProxy')} value={value} placeholder="http://proxy.example.com:8080"
          readOnly={saving} spellCheck={false} autoComplete="off" aria-invalid={feedback === 'proxyCredentials' || feedback === 'proxyUrlInvalid'}
          aria-describedby={feedback ? 'translation-proxy-error' : undefined}
          onChange={(event) => handleChange(event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Escape' && !saving) { clearAutoSave(); setDraft(null); setError(null); } }} />
        <input id="translation-proxy-enabled" className="settings-checkbox" type="checkbox" aria-labelledby="translation-proxy-label" aria-disabled={saving || undefined}
          aria-describedby={feedback ? 'translation-proxy-error' : undefined} checked={preferences.proxyEnabled}
          onChange={(event) => void save(event.target.checked)} />
      </div>
      {feedback && <p id="translation-proxy-error" className="settings-field-note settings-error" role="alert">{t(feedback)}</p>}
    </form>
  </section>;
}

function proxyContainsCredentials(value: string) {
  if (value.split('://', 2)[1]?.split(/[/?#]/)[0]?.includes('@')) return true;
  try {
    const url = new URL(value);
    return Boolean(url.username || url.password);
  } catch {
    return false;
  }
}
