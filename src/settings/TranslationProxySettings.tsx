import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import type { TranslationPreferences } from '../shared/contracts';

export function TranslationProxySettings({ t, preferences, native, command }: Pick<ViewProps, 't' | 'native' | 'command'> & { preferences: TranslationPreferences }) {
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<'proxyCredentials' | 'proxyUrlInvalid' | 'proxySaveFailed' | null>(null);
  const [saving, setSaving] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const value = draft ?? preferences.proxyUrl;
  const url = value.trim();
  const changed = url !== preferences.proxyUrl;
  const credentials = proxyContainsCredentials(url);
  const feedback = credentials ? 'proxyCredentials' : error;

  async function save(enabled?: boolean) {
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

  return <section aria-labelledby="translation-connection-heading">
    <h2 id="translation-connection-heading">{t('settingsConnection')}</h2>
    <form className="settings-group" onSubmit={(event) => {
      event.preventDefault();
      input.current?.focus({ preventScroll: true });
      void save();
    }}>
      <div className="settings-field settings-proxy-row">
        <label className="settings-field-content" htmlFor="translation-proxy-url">{t('httpProxy')}</label>
        <div className="settings-proxy-controls">
          <input ref={input} id="translation-proxy-url" className="settings-text-input" value={value} placeholder="http://proxy.example.com:8080"
            readOnly={saving} spellCheck={false} autoComplete="off" aria-invalid={feedback === 'proxyCredentials' || feedback === 'proxyUrlInvalid'}
            aria-describedby={feedback ? 'translation-proxy-error' : undefined}
            onChange={(event) => { setDraft(event.target.value); setError(null); }}
            onKeyDown={(event) => { if (event.key === 'Escape' && !saving) { setDraft(null); setError(null); } }} />
          {changed && <button type="submit" className="settings-action" aria-disabled={saving || undefined}>{t('saveProxy')}</button>}
          <input className="settings-switch" type="checkbox" role="switch" aria-label={t('useHttpProxy')} aria-disabled={saving || undefined}
            aria-describedby={feedback ? 'translation-proxy-error' : undefined} checked={preferences.proxyEnabled}
            onChange={(event) => void save(event.target.checked)} />
        </div>
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
