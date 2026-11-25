import { useEffect, useState } from 'react';
import type { ViewProps } from '../App';
import { languageName, providerLanguages, supportsLanguage, supportsTranslation } from '../shared/languages';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { type Translate } from '../shared/i18n';
import { TranslationProxySettings } from './TranslationProxySettings';

export function TranslationSettings({ t, snapshot, command, native }: Omit<ViewProps, 'activity'>) {
  const preferences = snapshot.translationPreferences;
  const [open, setOpen] = useState<'engine' | 'source' | 'primary' | 'secondary' | null>(null);
  const sourceItems = providerLanguages(preferences.engine, 'source');
  const targetItems = providerLanguages(preferences.engine, 'target');
  const languagesAvailable = supportsTranslation(preferences.engine, preferences.sourceLanguage, [preferences.primaryLanguage, ...(preferences.secondaryLanguage === 'none' ? [] : [preferences.secondaryLanguage])]);
  const selectedLabel = (value: string, side: 'source' | 'target') => value === 'none' || supportsLanguage(preferences.engine, side, value) ? undefined : `${languageName(value)} (${t('languageUnavailable')})`;
  useEffect(() => { setOpen(null); }, [snapshot.locale]);

  return <div className="settings-sections">
    <section aria-labelledby="translation-languages-heading">
      <div className="settings-section-heading">
        <h2 id="translation-languages-heading">{t('settingsLanguages')}</h2>
      </div>
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
    <TranslationProxySettings t={t} preferences={preferences} native={native} command={command} />
  </div>;
}

function SettingsChoice({ label, value, items, open, onOpenChange, onChange, t, searchable = true, menuWidth = 200, valueLabel }: {
  label: string; value: string; items: { code: string; name: string }[]; open: boolean; onOpenChange: (open: boolean) => void; onChange: (value: string) => void; t: Translate; searchable?: boolean; menuWidth?: number; valueLabel?: string;
}) {
  return <div className="settings-field"><span className="settings-field-content">{label}</span><SearchableDropdown label={label} value={value} items={items} open={open} onOpenChange={onOpenChange} onChange={onChange} t={t} side="bottom" searchable={searchable} settings menuWidth={menuWidth} valueLabel={valueLabel} /></div>;
}
