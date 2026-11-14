import { useEffect, useState } from 'react';
import { Check, ClipboardList, Info, Languages, Settings2, ShieldCheck } from 'lucide-react';
import type { ViewProps } from '../App';
import type { Locale } from '../shared/contracts';
import './settings.css';
import { SearchableDropdown } from '../shared/SearchableDropdown';

const pages = [
  { value: 'general', icon: Settings2, description: 'generalDescription' },
  { value: 'translation', icon: Languages, description: 'translationDescription' },
  { value: 'clipboard', icon: ClipboardList, description: 'clipboardDescription' },
  { value: 'privacy', icon: ShieldCheck, description: 'privacyDescription' },
] as const;

export function Settings({ t, snapshot, command, native }: ViewProps) {
  const [page, setPage] = useState<typeof pages[number]['value']>('general');
  const [languageOpen, setLanguageOpen] = useState(false);
  const [engineOpen, setEngineOpen] = useState(false);
  useEffect(() => { setLanguageOpen(false); }, [page, snapshot.locale]);
  const current = pages.find((item) => item.value === page)!;
  return <div className="settings-shell">
    <aside className="settings-sidebar">
      <div className="settings-brand" data-tauri-drag-region><span>Nami<small>{t('settings')}</small></span></div>
      <nav aria-label={t('settings')}>
        {pages.map(({ value, icon: Icon }) => <button type="button" key={value} aria-current={page === value ? 'page' : undefined} onClick={() => setPage(value)}><Icon aria-hidden="true" />{t(value)}</button>)}
      </nav>
      <div className="settings-stage"><span>Nami · 0.1.0 · P0</span></div>
    </aside>
    <section className="settings-content" aria-label={t(page)}>
      <header className="settings-page-heading"><h1>{t(page)}</h1><p className="settings-intro">{t(current.description)}</p></header>
      {page === 'general' && <>
        <h2>{t('personalization')}</h2>
        <div className="settings-group">
          <div className="settings-field" data-slot="field"><div className="settings-field-content" data-slot="field-content"><label data-slot="field-label" htmlFor="interface-language">{t('interfaceLanguage')}</label><p data-slot="field-description">{t('languageDescription')}</p></div>
            <SearchableDropdown id="interface-language" value={snapshot.locale} onChange={(locale) => void command('locale_set', { locale: locale as Locale })}
              label={t('interfaceLanguage')} items={[{ code: 'system', name: t('followSystem') }, { code: 'zh-CN', name: 'Chinese (Simplified)' }, { code: 'en', name: 'English' }]}
              open={languageOpen} onOpenChange={setLanguageOpen} t={t} side="bottom" searchable={false} capsule />
          </div>
          <hr className="settings-separator" />
          <div className="settings-field" data-slot="field"><div className="settings-field-content" data-slot="field-content"><span data-slot="field-label">{t('appearance')}</span><p data-slot="field-description">{t('systemAppearance')}</p></div><span className="settings-badge">{t('followSystem')}</span></div>
        </div>
      </>}
      {page === 'translation' && <>
        <h2>{t('service')}</h2>
        <div className="settings-group">
          <div className="settings-field" data-slot="field"><div className="settings-field-content" data-slot="field-content"><span data-slot="field-label">{t('service')}</span><p data-slot="field-description">Text is sent to the selected service when you translate.</p></div><SearchableDropdown value={snapshot.translationEngine} onChange={(engine) => void command('translation_engine_set', { engine })} label={t('service')} items={[{ code: 'edge', name: 'Edge' }, { code: 'google', name: 'Google' }]} open={engineOpen} onOpenChange={setEngineOpen} t={t} side="bottom" searchable={false} capsule /></div>
          <hr className="settings-separator" />
          <div className="settings-field" data-slot="field"><div className="settings-field-content" data-slot="field-content"><span data-slot="field-label">{t('shortcut')}</span><p data-slot="field-description">{t('globalShortcut')}</p></div>{snapshot.capabilities.selection ? <span className="keycaps"><kbd>{snapshot.capabilities.primaryModifier === 'meta' ? '⌘' : 'Ctrl'}</kbd><kbd>T</kbd></span> : <span className="settings-badge">Not available on this platform yet</span>}</div>
        </div>
      </>}
      {page === 'clipboard' && <>
        <h2>{t('capture')}</h2>
        <div className="settings-group">
          <div className="settings-field" data-slot="field" data-disabled><div className="settings-field-content" data-slot="field-content"><label data-slot="field-label" htmlFor="clipboard-capture">{t('capture')}</label><p data-slot="field-description">{t('captureDisabled')}</p></div><input className="settings-switch" id="clipboard-capture" type="checkbox" role="switch" checked={false} disabled /></div>
          <hr className="settings-separator" />
          <div className="settings-field" data-slot="field"><div className="settings-field-content" data-slot="field-content"><span data-slot="field-label">{t('shortcut')}</span><p data-slot="field-description">{t('globalShortcut')}</p></div><span className="keycaps"><kbd>{snapshot.capabilities.platform === 'macos' ? '⌃' : 'Ctrl'}</kbd><kbd>{snapshot.capabilities.platform === 'macos' ? '⌥' : 'Alt'}</kbd><kbd>V</kbd></span></div>
        </div>
        <div className="settings-note settings-callout" role="note"><Info aria-hidden="true" /><div><h5>{t('sampleMode')}</h5><p>{t('sampleNotice')}</p></div></div>
      </>}
      {page === 'privacy' && <>
        <h2>{t('systemPermission')}</h2>
        {snapshot.capabilities.accessibilityPermission ? <div className="settings-group">
          <div className="settings-field" data-slot="field"><div className="settings-field-content" data-slot="field-content"><span data-slot="field-label">{t('accessibility')}</span><p data-slot="field-description">{snapshot.accessibility ? t('permissionDescription') : t('required')}</p></div><span className={snapshot.accessibility ? 'settings-badge' : 'settings-badge outline'}>{snapshot.accessibility ? <><Check aria-hidden="true" />{t('granted')}</> : t('notGranted')}</span></div>
          <hr className="settings-separator" />
          <div className="permission-actions"><button type="button" className="settings-action" disabled={!native} onClick={() => void command('permission_request')}>{t('requestPermission')}</button><button type="button" className="settings-action" disabled={!native} onClick={() => void command('app_snapshot')}>{t('refreshPermission')}</button></div>
        </div> : <p className="settings-intro">Selected text capture and automatic paste are not available on this platform yet. You can enter text to translate.</p>}
        <div className="settings-note settings-callout" role="note"><ShieldCheck aria-hidden="true" /><div><h5>{t('localPrivacy')}</h5><p>{t('privacyDetail')}</p></div></div>
        <div className="version-row"><span>{t('version')}</span><span>0.1.0 · P0</span></div>
      </>}
      {snapshot.shortcutErrors.length > 0 && <div className="settings-callout settings-error" role="alert">{t('shortcutFailed')}</div>}
      <p className="settings-footnote"><Info aria-hidden="true" />{t('p0Settings')}</p>
    </section>
  </div>;
}
