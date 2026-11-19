import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RefreshCw } from 'lucide-react';
import type { MessageKey, Translate } from '../shared/i18n';
import { SettingsDialog } from './SettingsDialog';
import { errorCode } from './ShortcutCapture';

type Policy = { limit: number };
type Storage = { bytes: number; limit: number };
type Preview = { count: number; token: string; categories?: [number, number, number] };
const categories = ['clipboardCleanupOrdinary', 'clipboardCleanupFavorites', 'clipboardCleanupPinned'] as const;

function storageSize(bytes: number, locale: string) {
  let value = bytes;
  for (const unit of ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'] as const) {
    if (value < 1000 || unit === 'terabyte') return new Intl.NumberFormat(locale, { style: 'unit', unit, maximumFractionDigits: 1 }).format(value);
    value /= 1000;
  }
}

export function HistoryStorageSettings({ t, available, view, locale }: { t: Translate; available: boolean; view: 'clipboard' | 'translation'; locale: string }) {
  const id = useId();
  const [storage, setStorage] = useState<Storage>();
  const [storageError, setStorageError] = useState(false);
  const [refresh, refreshStorage] = useState(0);
  const [draft, setDraft] = useState<string | null>(null);
  const [cleanup, setCleanup] = useState<(Preview & { policy: Policy | null })>();
  const [mask, setMask] = useState(0);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<{ message: MessageKey; error?: boolean }>();
  const pending = useRef(false);
  const mounted = useRef(false);
  const limit = Number(draft);
  const canSave = draft !== null && /^\d+$/.test(draft) && limit >= 1 && limit <= 1_000_000 && limit !== storage?.limit;
  const number = new Intl.NumberFormat(locale);
  const storageHelp = t(view === 'clipboard' ? 'clipboardStorageUsedHelp' : 'translationStorageUsedHelp');
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const reload = () => refreshStorage((value) => value + 1);
  useEffect(() => {
    if (!available) return;
    let current = true;
    void invoke<Storage>('records_storage_info', { view }).then((value) => {
      if (current) { setStorage(value); setStorageError(false); }
    }, () => { if (current) setStorageError(true); });
    return () => { current = false; };
  }, [available, view, refresh]);

  async function perform(action: () => Promise<void>) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setFeedback(undefined);
    try { await action(); }
    catch (error) {
      const stale = errorCode(error) === 'clipboard_cleanup_changed';
      setFeedback({ message: stale ? 'clipboardCleanupChanged' : 'clipboardCleanupFailed', error: true });
      if (stale) reload();
    } finally { pending.current = false; setBusy(false); }
  }

  async function apply({ policy, token }: Preview & { policy: Policy | null }, selected?: number) {
    const result = await invoke<{ mediaCleanupFailed: boolean }>('records_cleanup_apply', { view, policy, token, ...(selected === undefined ? {} : { categories: selected }) });
    if (policy) {
      setStorage((previous) => previous && { ...previous, limit: policy.limit });
      setDraft(null);
    }
    if (result.mediaCleanupFailed) setFeedback({ message: 'clipboardCleanupMediaFailed', error: true });
    reload();
  }

  function preview(policy: Policy | null) {
    if (!available || cleanup) return;
    void perform(async () => {
      const result = await invoke<Preview>('records_cleanup_preview', { view, policy });
      if (!mounted.current) return;
      const candidate = { ...result, policy };
      if (!policy || candidate.count) { setMask(0); setCleanup(candidate); }
      else await apply(candidate);
    });
  }

  function close() {
    if (pending.current) return;
    if (cleanup?.policy) setDraft(null);
    setCleanup(undefined);
  }

  function confirm() {
    if (!cleanup || (!cleanup.policy && !mask)) return;
    void perform(async () => {
      try { await apply(cleanup, cleanup.policy ? undefined : mask); }
      finally { setCleanup(undefined); }
    });
  }

  return <section aria-labelledby={`${view}-storage-heading`}>
    <div className="settings-section-heading clipboard-section-heading">
      <h2 id={`${view}-storage-heading`}>{t('clipboardHistoryStorage')}</h2>
      <button type="button" className="settings-action clipboard-clear-records" aria-haspopup="dialog" disabled={!available} aria-disabled={busy || undefined}
        onClick={(event) => { event.currentTarget.focus({ preventScroll: true }); preview(null); }}>{t('clipboardClearRecords')}</button>
    </div>
    <div className="settings-group">
      <form className="settings-field" onSubmit={(event) => { event.preventDefault(); if (canSave) preview({ limit }); }}>
        <label className="settings-field-content" htmlFor={`${id}-limit`}>{t('clipboardHistoryLimit')}</label>
        {available && storage ? <div className="settings-history-limit">
          <input id={`${id}-limit`} type="text" inputMode="numeric" pattern="[0-9]+" maxLength={7} className="settings-text-input"
            value={draft ?? String(storage.limit)} readOnly={busy} title={t('clipboardHistoryLimitHelp')} aria-description={t('clipboardHistoryLimitHelp')}
            onChange={(event) => setDraft(event.target.value)} onKeyDown={(event) => { if (event.key === 'Escape' && !busy) setDraft(null); }} />
          {canSave && <button type="submit" className="settings-action" aria-disabled={busy || undefined}>{t('shortcutManualSave')}</button>}
        </div> : <span className="settings-badge">—</span>}
      </form>
      <div className="settings-field">
        <span className="settings-field-content">{t('clipboardStorageUsed')}</span>
        <div className="clipboard-storage-value">
          <span aria-live="polite" title={storageHelp}>{storageError ? t('clipboardStorageUnavailable') : storage ? storageSize(storage.bytes, locale) : '—'}</span>
          {available && <button type="button" className="settings-action clipboard-icon-button" title={t('clipboardRefreshStorage')}
            aria-label={t('clipboardRefreshStorage')} aria-description={storageHelp} onClick={reload}><RefreshCw aria-hidden="true" /></button>}
        </div>
      </div>
      {feedback && <p className="settings-field-note" role={feedback.error ? 'alert' : 'status'}>{t(feedback.message)}</p>}
    </div>
    {cleanup && <SettingsDialog className="settings-cleanup-dialog" title={t(cleanup.policy ? 'clipboardHistoryPolicyHeading' : 'clipboardCleanupClear')}
      description={cleanup.policy ? t(cleanup.count === 1 ? 'clipboardCleanupMessageOne' : 'clipboardCleanupMessage').replace('{count}', number.format(cleanup.count)) : undefined} onClose={close} onKeyDown={(event) => {
        if (event.defaultPrevented || event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'Escape' || event.key === 'Enter') {
          event.preventDefault();
          if (!event.repeat) { if (event.key === 'Escape') close(); else confirm(); }
        }
      }}>
      <form onSubmit={(event) => { event.preventDefault(); confirm(); }}>
        {cleanup.categories && <fieldset className="clipboard-cleanup-categories" aria-label={t('clipboardCleanupCategories')} disabled={busy}>
          {cleanup.categories.map((count, index) => <label className="clipboard-cleanup-category" key={categories[index]}>
            <input type="checkbox" className="settings-checkbox" checked={Boolean(mask & (1 << index))} disabled={!count}
              onChange={() => setMask((current) => current ^ (1 << index))} />
            <span>{t(categories[index])}</span><span className="clipboard-cleanup-count">{number.format(count)}</span>
          </label>)}
        </fieldset>}
        <div className="settings-dialog-actions">
          <button type="button" data-initial-focus className="settings-action" aria-keyshortcuts="Escape" aria-disabled={busy || undefined} onClick={close}>
            {t('clipboardCancel')}<span className="keycaps" aria-hidden="true"><kbd>Esc</kbd></span>
          </button>
          <button type="submit" className="settings-action clipboard-confirm-action" aria-keyshortcuts="Enter" disabled={!cleanup.policy && !mask} aria-disabled={busy || undefined}>
            {t(cleanup.policy ? 'clipboardCleanupApply' : 'clipboardCleanupClear')}<span className="keycaps" aria-hidden="true"><kbd>↵</kbd></span>
          </button>
        </div>
      </form>
    </SettingsDialog>}
  </section>;
}
