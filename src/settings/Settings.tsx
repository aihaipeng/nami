import { useEffect, useState } from 'react';
import { Check, ClipboardList, Languages, Settings2, ShieldCheck } from 'lucide-react';
import type { ViewProps } from '../App';
import type { Locale } from '../shared/contracts';
import './settings.css';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { ClipboardSettings } from './ClipboardSettings';
import { TranslationSettings } from './TranslationSettings';

const pages = [
  { value: 'general', icon: Settings2 },
  { value: 'translation', icon: Languages },
  { value: 'clipboard', icon: ClipboardList },
  { value: 'privacy', icon: ShieldCheck },
] as const;

export function Settings({ t, snapshot, command, native }: ViewProps) {
  const [page, setPage] = useState<typeof pages[number]['value']>('general');
  const [languageOpen, setLanguageOpen] = useState(false);
  useEffect(() => { setLanguageOpen(false); }, [page, snapshot.locale]);
  return <div className="settings-shell">
    <aside className="settings-sidebar">
      <div className="settings-brand" data-tauri-drag-region>{t('settings')}</div>
      <nav aria-label={t('settings')}>
        {pages.map(({ value, icon: Icon }) => {
          const label = t(value === 'translation' ? 'translationPage' : value);
          return <button type="button" key={value} aria-label={label} title={label} aria-current={page === value ? 'page' : undefined} onClick={() => setPage(value)}><Icon aria-hidden="true" /><span>{label}</span></button>;
        })}
      </nav>
    </aside>
    <section key={page} className="settings-content" aria-labelledby="settings-page-title">
      <div className="settings-page">
        <header className="settings-page-heading"><h1 id="settings-page-title">{t(page === 'translation' ? 'translationPage' : page)}</h1></header>
        {page === 'general' && <section aria-labelledby="settings-personalization">
          <h2 id="settings-personalization">{t('personalization')}</h2>
          <div className="settings-group">
            <div className="settings-field">
              <label className="settings-field-content" htmlFor="interface-language">{t('interfaceLanguage')}</label>
              <SearchableDropdown id="interface-language" value={snapshot.locale} onChange={(locale) => void command('locale_set', { locale: locale as Locale })}
                label={t('interfaceLanguage')} items={[{ code: 'system', name: t('followSystem') }, { code: 'zh-CN', name: 'Chinese (Simplified)' }, { code: 'en', name: 'English' }]}
                open={languageOpen} onOpenChange={setLanguageOpen} t={t} side="bottom" searchable={false} settings />
            </div>
            <div className="settings-field"><span className="settings-field-content">{t('appearance')}</span><span className="settings-badge">{t('followSystem')}</span></div>
          </div>
        </section>}
        {page === 'translation' && <TranslationSettings t={t} snapshot={snapshot} command={command} native={native} />}
        {page === 'clipboard' && <ClipboardSettings t={t} snapshot={snapshot} native={native} />}
        {page === 'privacy' && <div className="settings-sections">
          <section aria-labelledby="settings-permission">
            <h2 id="settings-permission">{t('systemPermission')}</h2>
            {snapshot.capabilities.accessibilityPermission ? <div className="settings-group">
              <div className="settings-field"><div className="settings-field-content"><span>{t('accessibility')}</span>{!snapshot.accessibility && <p>{t('required')}</p>}</div><span className="settings-badge">{snapshot.accessibility ? <><Check aria-hidden="true" />{t('granted')}</> : t('notGranted')}</span></div>
              <div className="settings-field settings-permission-actions"><button type="button" className="settings-action" disabled={!native} onClick={() => void command('permission_request')}>{t('requestPermission')}</button><button type="button" className="settings-action" disabled={!native} onClick={() => void command('app_snapshot')}>{t('refreshPermission')}</button></div>
            </div> : <p className="settings-callout">{t('settingsPermissionUnavailable')}</p>}
          </section>
          <section aria-labelledby="settings-data">
            <h2 id="settings-data">{t('settingsDataHandling')}</h2>
            <div className="settings-group">
              {(['settingsDataTranslation', 'settingsDataSelection', 'settingsDataHistory'] as const).map((key) => <div className="settings-field" key={key}><div className="settings-field-content"><span>{t(key)}</span><p>{t(`${key}Detail`)}</p></div></div>)}
            </div>
          </section>
          <div className="version-row"><span>{t('version')}</span><span>0.1.0 · P0</span></div>
        </div>}
        {(page === 'translation' || page === 'clipboard') && snapshot.shortcutErrors.length > 0 && <p className="settings-callout settings-error" role="alert">{t('shortcutFailed')}</p>}
      </div>
    </section>
  </div>;
}
