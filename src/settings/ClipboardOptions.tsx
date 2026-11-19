import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AppWindow, Info, RefreshCw, X } from 'lucide-react';
import { FloatingPanel } from '../shared/FloatingPanel';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import type { MessageKey, Translate } from '../shared/i18n';
import { SettingsDialog } from './SettingsDialog';
import { errorCode } from './ShortcutCapture';

type HistoryPolicy = { days: number | null; limit: number | null };
type ExcludedApp = { bundleId: string; name: string; iconDataUrl?: string };
type Activation = 'paste' | 'copy';
type Options = { excludedApps: ExcludedApp[]; historyPolicy: HistoryPolicy; activation: Activation };
type Storage = { bytes: number };
type Cleanup = { policy: HistoryPolicy | null; count: number; token: string; categories?: [number, number, number] };
const cleanupCategoryLabels = ['clipboardCleanupOrdinary', 'clipboardCleanupFavorites', 'clipboardCleanupPinned'] as const;
type CleanupResult = { mediaCleanupFailed: boolean };

function storageSize(bytes: number, locale: string) {
  let value = bytes;
  for (const unit of ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'] as const) {
    if (value < 1000 || unit === 'terabyte') return new Intl.NumberFormat(locale, { style: 'unit', unit, maximumFractionDigits: 1 }).format(value);
    value /= 1000;
  }
}

function Setting({ label, children }: { label: string; children: ReactNode }) {
  return <div className="settings-field clipboard-option-row">
    <div className="settings-field-content">{label}</div>
    {children}
  </div>;
}

export function useClipboardOptions(available: boolean) {
  const [options, setOptions] = useState<Options>();
  const [loadError, setLoadError] = useState(false);
  const [revision, reload] = useState(0);
  useEffect(() => {
    if (!available) return;
    let current = true;
    void invoke<Options>('clipboard_options_get').then((value) => { if (current) setOptions(value); }, () => { if (current) setLoadError(true); });
    return () => { current = false; };
  }, [available, revision]);
  return { options, setOptions, loadError, retry: () => { setLoadError(false); reload((value) => value + 1); } };
}

type OptionsState = ReturnType<typeof useClipboardOptions>;

export function ClipboardExcludedApps({ t, state, available }: { t: Translate; state: OptionsState; available: boolean }) {
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const [error, setError] = useState(false);
  const add = useRef<HTMLButtonElement>(null);
  const apps = state.options?.excludedApps;

  async function change(bundleId?: string, trigger?: HTMLButtonElement) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError(false);
    try {
      const next = bundleId
        ? await invoke<ExcludedApp[]>('clipboard_excluded_app_remove', { bundleId })
        : await invoke<ExcludedApp[] | null>('clipboard_excluded_app_pick');
      if (next) {
        if (trigger && trigger === document.activeElement) add.current?.focus({ preventScroll: true });
        state.setOptions((previous) => previous && { ...previous, excludedApps: next });
      }
    } catch { setError(true); }
    finally {
      pending.current = false;
      setBusy(false);
    }
  }

  return <section aria-labelledby="clipboard-excluded-heading">
    <div className="settings-section-heading clipboard-section-heading">
      <div className="clipboard-section-title">
        <h2 id="clipboard-excluded-heading">{t('clipboardExcludedAppsHeading')}</h2>
        <ExclusionTip label={t('clipboardAboutExcludedApps')} text={t('clipboardExcludedAppsDescription')} />
      </div>
      {available ? <button type="button" ref={add} className="settings-action" disabled={!apps} aria-disabled={busy || undefined}
        onClick={() => void change()}>{t('clipboardAddApp')}</button>
        : <span className="settings-badge">{t('platformUnavailable')}</span>}
    </div>
    {available && !!apps?.length && <ul className="settings-group clipboard-excluded-list" role="list" aria-labelledby="clipboard-excluded-heading">
      {apps.map(({ bundleId, name, iconDataUrl }) => {
        const duplicate = apps.some((app) => app.bundleId !== bundleId && app.name === name);
        const identity = duplicate ? `${name} (${bundleId})` : name;
        const remove = t('clipboardRemoveApp').replace('{app}', identity);
        return <li key={bundleId}>
          <div className="clipboard-excluded-name">
            <ExcludedAppIcon url={iconDataUrl} />
            <strong title={bundleId}>{name}</strong>
            {duplicate && <ExclusionTip label={t('clipboardAboutExcludedApp').replace('{app}', identity)} text={bundleId} />}
          </div>
          <button type="button" aria-disabled={busy || undefined} className="settings-action clipboard-icon-button" title={remove}
            aria-label={remove} onClick={(event) => void change(bundleId, event.currentTarget)}><X aria-hidden="true" /></button>
        </li>;
      })}
    </ul>}
    {error && <p className="clipboard-setting-note" role="alert">{t('clipboardManageAppsFailed')}</p>}
  </section>;
}

function ExcludedAppIcon({ url }: { url?: string }) {
  const [failed, setFailed] = useState<string>();
  return url && url !== failed
    ? <img className="clipboard-excluded-icon" src={url} alt="" onError={() => setFailed(url)} />
    : <AppWindow className="clipboard-excluded-icon clipboard-icon-fallback" aria-hidden="true" />;
}

function ExclusionTip({ label, text }: { label: string; text: string }) {
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  const [open, setOpen] = useState(false);
  return <>
    <button ref={trigger} type="button" className="settings-action clipboard-tip-trigger" aria-label={label} aria-describedby={open ? id : undefined} aria-expanded={open}
      onClick={(event) => { event.currentTarget.focus({ preventScroll: true }); setOpen(!open); }} onBlur={() => setOpen(false)}
      onKeyDown={(event) => {
        if (open && event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setOpen(false); }
      }}><Info aria-hidden="true" /></button>
    {open && <FloatingPanel anchor={trigger} onClose={() => setOpen(false)} id={id} role="tooltip" width={220} side="bottom" className="settings-popover clipboard-tip-panel" dismissOnSettingsScroll>
      {text}
    </FloatingPanel>}
  </>;
}

export function ClipboardReuseSettings({ t, state, storageAvailable, platformAvailable, locale }: {
  t: Translate; state: OptionsState; storageAvailable: boolean; platformAvailable: boolean; locale: string;
}) {
  const [storage, setStorage] = useState<Storage>();
  const [storageError, setStorageError] = useState(false);
  const [refresh, refreshStorage] = useState(0);
  const [open, setOpen] = useState<'days' | 'limit' | 'behavior' | null>(null);
  const [cleanup, setCleanup] = useState<Cleanup>();
  const [selectedCategories, setSelectedCategories] = useState(0);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [behaviorBusy, setBehaviorBusy] = useState(false);
  const [historyFeedback, setHistoryFeedback] = useState<{ message: MessageKey; error?: boolean }>();
  const [behaviorError, setBehaviorError] = useState(false);
  const historyPending = useRef(false);
  const behaviorPending = useRef(false);
  const mounted = useRef(false);
  const policy = state.options?.historyPolicy;
  const activation = state.options?.activation;
  const number = new Intl.NumberFormat(locale);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setOpen(null); }, [locale]);
  useEffect(() => {
    if (!storageAvailable) return;
    let current = true;
    void invoke<Storage>('clipboard_storage_info').then((value) => {
      if (current) { setStorage(value); setStorageError(false); }
    }, () => { if (current) { setStorage(undefined); setStorageError(true); } });
    return () => { current = false; };
  }, [storageAvailable, refresh]);

  async function apply({ policy, token }: Cleanup, categories?: number) {
    const result = await invoke<CleanupResult>('clipboard_cleanup_apply', { policy, token, ...(categories === undefined ? {} : { categories }) });
    if (policy) state.setOptions((previous) => previous && { ...previous, historyPolicy: policy });
    if (result.mediaCleanupFailed) setHistoryFeedback({ message: 'clipboardCleanupMediaFailed', error: true });
    else if (policy) setHistoryFeedback({ message: 'clipboardRetentionUpdated' });
    refreshStorage((value) => value + 1);
  }

  async function historyAction(action: () => Promise<void>) {
    if (historyPending.current) return;
    historyPending.current = true;
    setHistoryBusy(true);
    setHistoryFeedback(undefined);
    try { await action(); }
    catch (error) {
      const stale = errorCode(error) === 'clipboard_cleanup_changed';
      setHistoryFeedback({ message: stale ? 'clipboardCleanupChanged' : 'clipboardCleanupFailed', error: true });
      if (stale) refreshStorage((value) => value + 1);
    } finally { historyPending.current = false; setHistoryBusy(false); }
  }

  function preview(next: HistoryPolicy | null) {
    if (cleanup) return;
    void historyAction(async () => {
      const result = await invoke<Omit<Cleanup, 'policy'>>('clipboard_cleanup_preview', { policy: next });
      if (!mounted.current) return;
      const candidate = { policy: next, ...result };
      if (!next || candidate.count) {
        setSelectedCategories(0); setCleanup(candidate);
      } else await apply(candidate);
    });
  }

  async function changeBehavior(next: Activation) {
    if (!activation || next === activation || behaviorPending.current) return;
    behaviorPending.current = true;
    setBehaviorBusy(true);
    setBehaviorError(false);
    try {
      await invoke('clipboard_behavior_set', { activation: next });
      state.setOptions((previous) => previous && { ...previous, activation: next });
    } catch { setBehaviorError(true); }
    finally { behaviorPending.current = false; setBehaviorBusy(false); }
  }

  const days = [{ code: '', name: t('clipboardKeepForever') }, ...[7, 30, 90].map((days) => ({ code: String(days), name: t('clipboardKeepDays').replace('{days}', number.format(days)) }))];
  const limits = [{ code: '', name: t('clipboardHistoryUnlimited') }, ...[200, 500, 1000].map((limit) => ({ code: String(limit), name: number.format(limit) }))];
  const activations = [{ code: 'copy', name: t('clipboardActivationCopy') }, { code: 'paste', name: t('clipboardActivationPaste') }];

  function storagePicker(field: keyof HistoryPolicy, label: string, items: { code: string; name: string }[]) {
    if (!policy) return <span className="settings-badge">—</span>;
    return <div className="clipboard-picker" data-pending={historyBusy || undefined}>
      <SearchableDropdown value={String(policy[field] ?? '')} label={label} items={items} t={t} searchable={false} settings side="bottom"
        description={t(field === 'days' ? 'clipboardKeepHistoryHelp' : 'clipboardHistoryLimitHelp')}
        open={open === field} onOpenChange={(next) => { if (!next || !historyPending.current) setOpen(next ? field : null); }}
        onChange={(value) => { const next = value ? Number(value) : null; if (policy[field] !== next) preview({ ...policy, [field]: next }); }} />
    </div>;
  }

  return <>
    <section aria-labelledby="clipboard-storage-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <h2 id="clipboard-storage-heading">{t('clipboardHistoryStorage')}</h2>
        <button type="button" className="settings-action clipboard-clear-records" aria-haspopup="dialog"
          disabled={!storageAvailable} aria-disabled={historyBusy || undefined} onClick={(event) => { event.currentTarget.focus({ preventScroll: true }); preview(null); }}>{t('clipboardClearRecords')}</button>
      </div>
      <div className="settings-group">
        <Setting label={t('clipboardKeepHistory')}>
          {storagePicker('days', t('clipboardKeepHistory'), days)}
        </Setting>
        <Setting label={t('clipboardHistoryLimit')}>
          {storagePicker('limit', t('clipboardHistoryLimit'), limits)}
        </Setting>
        <Setting label={t('clipboardStorageUsed')}>
          <div className="clipboard-storage-value">
            <span aria-live="polite" title={t('clipboardStorageUsedHelp')}>{storage ? storageSize(storage.bytes, locale) : storageError ? t('clipboardStorageUnavailable') : '—'}</span>
            {storageAvailable && <button type="button" className="settings-action clipboard-icon-button" title={t('clipboardRefreshStorage')}
              aria-label={t('clipboardRefreshStorage')} aria-description={t('clipboardStorageUsedHelp')} onClick={() => refreshStorage((value) => value + 1)}><RefreshCw aria-hidden="true" /></button>}
          </div>
        </Setting>
        {historyFeedback && <p className="clipboard-setting-note" role={historyFeedback.error ? 'alert' : 'status'}>
          {t(historyFeedback.message)}</p>}
      </div>
    </section>
    <section aria-labelledby="clipboard-behavior-heading">
      <h2 id="clipboard-behavior-heading">{t('clipboardBehavior')}</h2>
      <div className="settings-group">
        <Setting label={t('clipboardActivation')}>
          {platformAvailable && activation ? <div className="clipboard-picker" data-pending={behaviorBusy || undefined}>
            <SearchableDropdown value={activation} label={t('clipboardActivation')} items={activations} t={t} settings searchable={false} description={t('clipboardActivationHelp')} side="top" menuWidth={218}
              open={open === 'behavior'} onOpenChange={(next) => { if (!next || !behaviorPending.current) setOpen(next ? 'behavior' : null); }}
              onChange={(value) => void changeBehavior(value as Activation)} />
          </div> : <span className="settings-badge">{platformAvailable ? '—' : t('platformUnavailable')}</span>}
        </Setting>
        {behaviorError && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
      </div>
    </section>
    {cleanup && <SettingsDialog title={t(cleanup.policy ? 'clipboardHistoryPolicyHeading' : 'clipboardCleanupClear')}
      description={cleanup.policy ? t(cleanup.count === 1 ? 'clipboardCleanupMessageOne' : 'clipboardCleanupMessage').replace('{count}', number.format(cleanup.count)) : undefined}
      onClose={() => { if (!historyPending.current) setCleanup(undefined); }}>
      {cleanup.categories && <fieldset className="clipboard-cleanup-categories" aria-label={t('clipboardCleanupCategories')} disabled={historyBusy}>
        {cleanup.categories.map((count, index) => <label className="clipboard-cleanup-category" key={cleanupCategoryLabels[index]}>
          <input type="checkbox" checked={Boolean(selectedCategories & (1 << index))} disabled={!count}
            onChange={() => setSelectedCategories((current) => current ^ (1 << index))} />
          <span>{t(cleanupCategoryLabels[index])}</span><span className="clipboard-cleanup-count">{number.format(count)}</span>
        </label>)}
      </fieldset>}
      <div className="settings-dialog-actions">
        <button type="button" data-initial-focus className="settings-action" aria-disabled={historyBusy || undefined}
          onClick={() => { if (!historyPending.current) setCleanup(undefined); }}>{t('clipboardCancel')}</button>
        <button type="button" className="settings-action clipboard-confirm-action" disabled={!cleanup.policy && !selectedCategories} aria-disabled={historyBusy || undefined}
          onClick={() => void historyAction(async () => {
            if (!cleanup.policy && !selectedCategories) return;
            try { await apply(cleanup, cleanup.policy ? undefined : selectedCategories); } finally { setCleanup(undefined); }
          })}>{t(cleanup.policy ? 'clipboardCleanupApply' : 'clipboardCleanupClear')}</button>
      </div>
    </SettingsDialog>}
  </>;
}
