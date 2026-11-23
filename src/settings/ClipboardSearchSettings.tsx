import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ClipboardSearchMode, ClipboardSearchOptions } from '../shared/clipboardSearch';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import type { Translate } from '../shared/i18n';

const modes = { exact: 'clipboardSearchExact', fuzzy: 'clipboardSearchFuzzy', regex: 'clipboardSearchRegex', mixed: 'clipboardSearchMixed' } as const;

export function ClipboardSearchSettings({ t, search, available }: { t: Translate; search: ClipboardSearchOptions; available: boolean }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const pending = useRef(false);

  async function save(mode: ClipboardSearchMode) {
    if (!available || pending.current) return;
    pending.current = true;
    setBusy(true); setError(false);
    try { await invoke('clipboard_search_set', { search: { mode, body: true, note: true, imageText: true } }); }
    catch { setError(true); }
    finally { pending.current = false; setBusy(false); }
  }

  return <section aria-labelledby="clipboard-search-heading">
    <h2 id="clipboard-search-heading">{t('clipboardSearchSettings')}</h2>
    <div className="settings-group">
      <div className="settings-field clipboard-option-row">
        <div className="settings-field-content">{t('clipboardSearchRule')}</div>
        {available ? <div className="clipboard-picker" data-pending={busy || undefined}>
          <SearchableDropdown value={search.mode} label={t('clipboardSearchRule')} items={Object.entries(modes).map(([code, label]) => ({ code, name: t(label) }))}
            t={t} settings searchable={false} open={open} onOpenChange={(next) => { if (!next || !pending.current) setOpen(next); }}
            description={t('clipboardSearchRuleHelp')} onChange={(mode) => { if (mode !== search.mode) void save(mode as ClipboardSearchMode); }} />
        </div> : <span className="settings-badge">{t('platformUnavailable')}</span>}
      </div>
      {error && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
    </div>
  </section>;
}
