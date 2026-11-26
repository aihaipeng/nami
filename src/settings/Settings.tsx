import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Check, ClipboardList, HardDrive, Keyboard, Languages, Settings2, ShieldCheck } from 'lucide-react';
import type { ViewProps } from '../App';
import type { RecordsOpenFocus } from '../shared/clipboardShortcuts';
import type { Locale, WindowPositionMode } from '../shared/contracts';
import { resolveLanguage } from '../shared/i18n';
import './settings.css';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { HotKeySettings } from './HotKeySettings';
import { ClipboardSettings } from './ClipboardSettings';
import { HistoryStorageSettings } from './HistoryStorageSettings';
import { SettingHintTip } from './SettingHintTip';
import { TranslationSettings } from './TranslationSettings';

const pages = [
  { value: 'general', icon: Settings2 },
  { value: 'translationPage', icon: Languages },
  { value: 'clipboard', icon: ClipboardList },
  { value: 'storage', icon: HardDrive },
  { value: 'hotKey', icon: Keyboard },
  { value: 'privacy', icon: ShieldCheck },
] as const;

type SettingsPage = typeof pages[number]['value'];

function resolveSettingsPage(candidate?: string): SettingsPage {
  return pages.some(({ value }) => value === candidate) ? (candidate as SettingsPage) : 'general';
}

export function Settings({ t, snapshot, command, native }: ViewProps) {
  const [page, setPage] = useState<SettingsPage>(() => resolveSettingsPage(snapshot.initialSettingsPage));
  const [openDropdown, setOpenDropdown] = useState<'language' | 'windowPosition' | 'recordsFocus' | 'clipboardPreviewLines' | null>(null);
  const [savingLaunch, setSavingLaunch] = useState(false);
  const [launchError, setLaunchError] = useState(false);
  const [previewSaveFailed, setPreviewSaveFailed] = useState(false);
  const savingPreview = useRef(false);
  const canManageRecords = native && !snapshot.storageError;
  useEffect(() => { setOpenDropdown(null); setLaunchError(false); setPreviewSaveFailed(false); }, [page, snapshot.locale]);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let remove: (() => void) | undefined;
    void listen<string>('settings_page_requested', ({ payload }) => {
      if (!disposed) setPage(resolveSettingsPage(payload));
    }).then((unlisten) => { if (disposed) unlisten(); else remove = unlisten; }).catch(() => {});
    return () => { disposed = true; remove?.(); };
  }, [native]);

  async function saveLaunchAtLogin(enabled: boolean) {
    if (!native || !snapshot.capabilities.launchAtLogin || savingLaunch) return;
    setSavingLaunch(true);
    setLaunchError(false);
    try { await invoke('launch_at_login_set', { enabled }); }
    catch { setLaunchError(true); }
    finally { setSavingLaunch(false); }
  }

  async function savePreviewLines(code: string) {
    const current = snapshot.clipboardControls.previewLines?.clipboard ?? 3;
    if (!canManageRecords || savingPreview.current || Number(code) === current) return;
    savingPreview.current = true;
    setPreviewSaveFailed(false);
    try { await invoke('records_preview_lines_set', { lines: Number(code) }); }
    catch { setPreviewSaveFailed(true); }
    finally { savingPreview.current = false; }
  }

  return <div className="settings-shell">
    <aside className="settings-sidebar">
      <div className="settings-brand" data-tauri-drag-region>{t('settings')}</div>
      <nav aria-label={t('settings')}>
        {pages.map(({ value, icon: Icon }) => {
          const label = t(value === 'storage' ? 'storagePage' : value);
          return <button type="button" key={value} aria-label={label} title={label} aria-current={page === value ? 'page' : undefined} onClick={() => setPage(value)}><Icon aria-hidden="true" /><span>{label}</span></button>;
        })}
      </nav>
    </aside>
    <section key={page} className="settings-content" aria-labelledby="settings-page-title">
      <div className="settings-page">
        <header className="settings-page-heading"><h1 id="settings-page-title">{t(page === 'storage' ? 'storagePage' : page)}</h1></header>
        {page === 'general' && <section aria-labelledby="settings-personalization">
          <h2 id="settings-personalization">{t('personalization')}</h2>
          <div className="settings-group">
            <div className="settings-field">
              <label className="settings-field-content" htmlFor="interface-language">{t('interfaceLanguage')}</label>
              {native && snapshot.storageError ? <span className="settings-badge">{t('platformUnavailable')}</span> :
                <SearchableDropdown id="interface-language" value={snapshot.locale} onChange={(locale) => void command('locale_set', { locale: locale as Locale })}
                  label={t('interfaceLanguage')} items={[{ code: 'system', name: t('followSystem') }, { code: 'zh-CN', name: 'Chinese (Simplified)' }, { code: 'en', name: 'English' }]}
                  open={openDropdown === 'language'} onOpenChange={(open) => setOpenDropdown(open ? 'language' : null)} t={t} side="bottom" searchable={false} settings />}
            </div>
            <div className="settings-field">
              <span id="launch-at-login-label" className="settings-field-content">{t('launchAtLogin')}</span>
              {native && snapshot.capabilities.launchAtLogin
                ? <input id="launch-at-login" type="checkbox" className="settings-checkbox" aria-labelledby="launch-at-login-label" checked={snapshot.launchAtLogin} aria-disabled={savingLaunch || undefined} onChange={(event) => void saveLaunchAtLogin(event.target.checked)} />
                : <span className="settings-badge">{t('platformUnavailable')}</span>}
            </div>
            {launchError && <p className="settings-field-note" role="alert">{t('launchAtLoginFailed')}</p>}
            <div className="settings-field">
              <label className="settings-field-content" htmlFor="window-position-mode">{t('windowPosition')}</label>
              {native && snapshot.storageError ? <span className="settings-badge">{t('platformUnavailable')}</span> :
                <SearchableDropdown id="window-position-mode" value={snapshot.windowPositionMode ?? 'lastPosition'}
                  onChange={(mode) => void command('window_position_mode_set', { mode: mode as WindowPositionMode })}
                  label={t('windowPosition')} items={[
                    { code: 'lastPosition', name: t('windowPositionLast') },
                    { code: 'center', name: t('windowPositionCenter') },
                    { code: 'mouse', name: t('windowPositionMouse') },
                    { code: 'cursor', name: t('windowPositionCursor') },
                  ]}
                  open={openDropdown === 'windowPosition'} onOpenChange={(open) => setOpenDropdown(open ? 'windowPosition' : null)} t={t} side="bottom" searchable={false} settings />}
            </div>
            <div className="settings-field">
              <label className="settings-field-content" htmlFor="records-open-focus">{t('recordsOpenFocus')}</label>
              {native && snapshot.storageError ? <span className="settings-badge">{t('platformUnavailable')}</span> :
                <SearchableDropdown id="records-open-focus" value={snapshot.clipboardControls.recordsOpenFocus ?? 'list'}
                  onChange={(focus) => void command('records_open_focus_set', { focus: focus as RecordsOpenFocus })}
                  label={t('recordsOpenFocus')} items={[{ code: 'search', name: t('recordsFocusSearch') }, { code: 'list', name: t('recordsFocusList') }]}
                  open={openDropdown === 'recordsFocus'} onOpenChange={(open) => setOpenDropdown(open ? 'recordsFocus' : null)} t={t} side="bottom" searchable={false} settings />}
            </div>
            <div className="settings-field">
              <label className="settings-field-content" htmlFor="clipboard-preview-lines">{t('clipboardPreviewLines')}</label>
              {canManageRecords ? <SearchableDropdown id="clipboard-preview-lines" label={t('clipboardPreviewLines')} value={String(snapshot.clipboardControls.previewLines?.clipboard ?? 3)}
                items={['1', '2', '3'].map((code) => ({ code, name: code }))} open={openDropdown === 'clipboardPreviewLines'} onOpenChange={(open) => setOpenDropdown(open ? 'clipboardPreviewLines' : null)}
                onChange={(code) => void savePreviewLines(code)} t={t} searchable={false} side="bottom" settings menuWidth={90} /> : <span className="settings-badge">—</span>}
            </div>
            {previewSaveFailed && <p className="settings-field-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
            <div className="settings-field"><span className="settings-field-content">{t('appearance')}</span><span className="settings-badge">{t('followSystem')}</span></div>
          </div>
        </section>}
        {page === 'translationPage' && <TranslationSettings t={t} snapshot={snapshot} command={command} native={native} />}
        {page === 'clipboard' && <ClipboardSettings t={t} snapshot={snapshot} native={native} />}
        {page === 'storage' && <HistoryStorageSettings t={t} available={canManageRecords} locationsAvailable={native && !snapshot.storageError && snapshot.capabilities.storageLocations} backupAvailable={native && !snapshot.storageError && snapshot.capabilities.localBackup} locale={resolveLanguage(snapshot.locale)} />}
        {page === 'hotKey' && <HotKeySettings t={t} snapshot={snapshot} native={native} />}
        {page === 'privacy' && <div className="settings-sections">
          <section aria-labelledby="settings-permission">
            <h2 id="settings-permission">{t('systemPermission')}</h2>
            {snapshot.capabilities.accessibilityPermission ? <div className="settings-group">
              <div className="settings-field">
                <div className="settings-shortcut-name">
                  <span className="settings-field-content">{t('accessibility')}</span>
                  <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('accessibility'))} text={t('required')} />
                </div>
                <span className="settings-badge">{snapshot.accessibility ? <><Check aria-hidden="true" />{t('granted')}</> : t('notGranted')}</span>
              </div>
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
        {page === 'hotKey' && snapshot.shortcutErrors.length > 0 && <p className="settings-callout settings-error" role="alert">{t('shortcutFailed')}</p>}
      </div>
    </section>
  </div>;
}
