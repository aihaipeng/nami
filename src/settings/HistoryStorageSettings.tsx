import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  Check,
  CircleAlert,
  FileText,
  Folder,
  FolderOpen,
  FolderSync,
  Image,
  RotateCcw,
} from 'lucide-react';
import type { MessageKey, Translate } from '../shared/i18n';
import { SETTINGS_AUTO_SAVE_DELAY_MS } from './autoSave';
import { BackupSettings } from './BackupSettings';
import { SettingsDialog } from './SettingsDialog';
import { SettingHintTip } from './SettingHintTip';
import { errorCode } from './ShortcutCapture';

type Policy = {
  limit: number;
  maxAgeDays?: number;
  maxStorageMb?: number;
  autoCleanupIntervalHours: number;
};
type CategoryUsage = { bytes: number; count: number };
type StorageBreakdown = { text: CategoryUsage; image: CategoryUsage; files: CategoryUsage };
type Storage = Policy & { bytes: number; breakdown?: StorageBreakdown };
type StorageUpdatePayload = number | { bytes: number; breakdown?: StorageBreakdown };
type Preview = { count: number; token: string; categories?: [number, number, number] };
type StorageLocationInfo = {
  dataDir: string;
  defaultDataDir: string;
  isCustomDataDir: boolean;
  logDir: string;
};
type CleanupApplyResult = {
  removed: number;
  reclaimedBytes?: number;
  mediaCleanupFailed: boolean;
};

const categories = ['clipboardCleanupOrdinary', 'clipboardCleanupFavorites', 'clipboardCleanupPinned'] as const;
const breakdownKinds = [
  { key: 'text', label: 'text', icon: FileText },
  { key: 'image', label: 'clipboardImages', icon: Image },
  { key: 'files', label: 'clipboardFiles', icon: Folder },
] as const;
const TOAST_DURATION_MS = 3000;
const MAX_STORAGE_MB = 100_000;

function storageSize(bytes: number, locale: string) {
  let value = bytes;
  for (const unit of ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'] as const) {
    if (value < 1000 || unit === 'terabyte') return new Intl.NumberFormat(locale, { style: 'unit', unit, maximumFractionDigits: 1 }).format(value);
    value /= 1000;
  }
  return `${bytes} B`;
}

function migrationErrorKey(error: unknown): MessageKey {
  const code = errorCode(error);
  if (code === 'storage_dir_same') return 'storageDirSame';
  if (code === 'storage_dir_nested') return 'storageDirNested';
  if (code === 'storage_dir_not_empty') return 'storageDirNotEmpty';
  if (code === 'storage_dir_unwritable') return 'storageDirUnwritable';
  return 'storageDirMigrateFailed';
}

export function HistoryStorageSettings({
  t,
  available,
  locationsAvailable,
  backupAvailable,
  locale,
}: {
  t: Translate;
  available: boolean;
  locationsAvailable: boolean;
  backupAvailable: boolean;
  locale: string;
}) {
  const id = useId();
  const [storage, setStorage] = useState<Storage>();
  const [storageError, setStorageError] = useState(false);
  const [location, setLocation] = useState<StorageLocationInfo>();
  const [locationBusy, setLocationBusy] = useState<'openData' | 'changeData' | 'resetData' | 'openLog' | null>(null);
  const [locationFeedback, setLocationFeedback] = useState<{ text: string; error?: boolean }>();
  const [toast, setToast] = useState<{ id: number; text: string; error?: boolean }>();
  const [storageVersion, setStorageVersion] = useState(0);
  const [limitDraft, setLimitDraft] = useState<string | null>(null);
  const [ageDraft, setAgeDraft] = useState<string | null>(null);
  const [storageLimitDraft, setStorageLimitDraft] = useState<string | null>(null);
  const [intervalDraft, setIntervalDraft] = useState<string | null>(null);
  const [draftRevision, setDraftRevision] = useState(0);
  const [cleanup, setCleanup] = useState<(Preview & { policy: Policy | null })>();
  const [mask, setMask] = useState(0);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<{ message: MessageKey; error?: boolean }>();
  const pending = useRef(false);
  const mounted = useRef(false);
  const toastSequence = useRef(0);
  const policyAutoSaveTimer = useRef<number | null>(null);

  const currentLimit = storage?.limit ?? 500;
  const currentAge = storage?.maxAgeDays ?? 0;
  const currentStorageMb = storage?.maxStorageMb ?? 0;
  const currentInterval = storage?.autoCleanupIntervalHours ?? 1;

  const parsedLimit = Number(limitDraft);
  const limitValid = limitDraft !== null && /^\d+$/.test(limitDraft) && parsedLimit >= 0 && parsedLimit <= 1_000_000;
  const limitInvalid = limitDraft !== null && !limitValid;
  const canSaveLimit = limitValid && parsedLimit !== currentLimit;

  const parsedAge = Number(ageDraft);
  const ageValid = ageDraft !== null && /^\d+$/.test(ageDraft) && parsedAge >= 0 && parsedAge <= 3650;
  const ageInvalid = ageDraft !== null && !ageValid;
  const canSaveAge = ageValid && parsedAge !== currentAge;

  const parsedStorageMb = Number(storageLimitDraft);
  const storageMbValid = storageLimitDraft !== null && /^\d+$/.test(storageLimitDraft) && parsedStorageMb >= 0 && parsedStorageMb <= MAX_STORAGE_MB;
  const storageMbInvalid = storageLimitDraft !== null && !storageMbValid;
  const canSaveStorageMb = storageMbValid && parsedStorageMb !== currentStorageMb;

  const parsedInterval = Number(intervalDraft);
  const intervalValid = intervalDraft !== null && /^\d+$/.test(intervalDraft) && parsedInterval >= 0 && parsedInterval <= 720;
  const intervalInvalid = intervalDraft !== null && !intervalValid;
  const canSaveInterval = intervalValid && parsedInterval !== currentInterval;

  const hasInvalidDraft = limitInvalid || ageInvalid || storageMbInvalid || intervalInvalid;
  const hasValidPolicyChange = canSaveLimit || canSaveAge || canSaveStorageMb || canSaveInterval;

  function clearPolicyAutoSave() {
    if (policyAutoSaveTimer.current !== null) {
      window.clearTimeout(policyAutoSaveTimer.current);
      policyAutoSaveTimer.current = null;
    }
  }

  const number = new Intl.NumberFormat(locale);
  const storageHelp = t('clipboardStorageUsedHelp');
  const breakdown = storage?.breakdown;
  const breakdownTotal = breakdown ? breakdown.text.bytes + breakdown.image.bytes + breakdown.files.bytes : 0;

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => { setToast(undefined); }, TOAST_DURATION_MS);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const reload = () => setStorageVersion((value) => value + 1);

  function showToast(text: string, error = false) {
    toastSequence.current += 1;
    setToast({ id: toastSequence.current, text, error });
  }

  useEffect(() => {
    if (!available) return;
    let current = true;
    void invoke<Storage>('records_storage_info').then(
      (value) => {
        if (current && value) {
          setStorage({
            ...value,
            autoCleanupIntervalHours: value.autoCleanupIntervalHours ?? 1,
          });
          setStorageError(false);
        }
      },
      () => { if (current) setStorageError(true); },
    );
    return () => { current = false; };
  }, [available, storageVersion]);

  useEffect(() => {
    if (!available) return;
    let current = true;
    void invoke<StorageLocationInfo>('storage_location_info').then(
      (value) => { if (current) setLocation(value); },
      () => {},
    );
    return () => { current = false; };
  }, [available]);

  useEffect(() => {
    if (!available) return;
    let disposed = false;
    let remove: (() => void) | undefined;
    void listen<StorageUpdatePayload>('records_storage_changed', ({ payload }) => {
      if (disposed) return;
      if (typeof payload === 'number') {
        setStorage((previous) => previous && { ...previous, bytes: payload });
      } else {
        setStorage((previous) => previous && {
          ...previous,
          bytes: payload.bytes,
          ...(payload.breakdown ? { breakdown: payload.breakdown } : {}),
        });
      }
    }).then((unlisten) => { if (disposed) unlisten(); else remove = unlisten; }).catch(() => {});
    return () => { disposed = true; remove?.(); };
  }, [available]);

  function buildPolicy(overrides: Partial<{ limit: number; maxAgeDays: number; maxStorageMb: number; autoCleanupIntervalHours: number }>): Policy {
    const nextLimit = overrides.limit ?? currentLimit;
    const nextAge = overrides.maxAgeDays ?? currentAge;
    const nextStorageMb = overrides.maxStorageMb ?? (storage?.maxStorageMb ?? 0);
    const nextInterval = overrides.autoCleanupIntervalHours ?? currentInterval;
    return {
      limit: nextLimit,
      ...(nextAge > 0 ? { maxAgeDays: nextAge } : {}),
      ...(nextStorageMb > 0 ? { maxStorageMb: nextStorageMb } : {}),
      autoCleanupIntervalHours: nextInterval,
    };
  }

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
    const result = await invoke<CleanupApplyResult>('records_cleanup_apply', { policy, token, ...(selected === undefined ? {} : { categories: selected }) });
    if (policy) {
      setStorage((previous) => previous && {
        bytes: previous.bytes,
        ...(previous.breakdown ? { breakdown: previous.breakdown } : {}),
        ...policy,
      });
      setLimitDraft(null);
      setAgeDraft(null);
      setStorageLimitDraft(null);
      setIntervalDraft(null);
    } else if (mounted.current && result.removed > 0) {
      const reclaimed = storageSize(result.reclaimedBytes ?? 0, locale);
      showToast(
        (result.removed === 1 ? t('storageManualCleanedOne') : t('storageManualCleaned'))
          .replace('{count}', number.format(result.removed))
          .replace('{size}', reclaimed),
      );
    }
    if (result.mediaCleanupFailed) setFeedback({ message: 'clipboardCleanupMediaFailed', error: true });
    reload();
  }

  function buildMergedPolicy(): Policy | null {
    if (!available || !storage || cleanup || busy || hasInvalidDraft || !hasValidPolicyChange) return null;
    return buildPolicy({
      ...(canSaveLimit ? { limit: parsedLimit } : {}),
      ...(canSaveAge ? { maxAgeDays: parsedAge } : {}),
      ...(canSaveStorageMb ? { maxStorageMb: parsedStorageMb } : {}),
      ...(canSaveInterval ? { autoCleanupIntervalHours: parsedInterval } : {}),
    });
  }

  function preview(policy: Policy | null) {
    clearPolicyAutoSave();
    if (!available || cleanup) return;
    void perform(async () => {
      const result = await invoke<Preview>('records_cleanup_preview', { policy });
      if (!mounted.current) return;
      const candidate = { ...result, policy };
      if (!policy || candidate.count) { setMask(0); setCleanup(candidate); }
      else await apply(candidate);
    });
  }

  const latestAutoSaveRef = useRef(() => {
    const nextPolicy = buildMergedPolicy();
    if (nextPolicy) preview(nextPolicy);
  });
  latestAutoSaveRef.current = () => {
    const nextPolicy = buildMergedPolicy();
    if (nextPolicy) preview(nextPolicy);
  };

  useEffect(() => {
    if (draftRevision === 0) return;
    clearPolicyAutoSave();
    if (!available || !storage || cleanup || busy || hasInvalidDraft || !hasValidPolicyChange) return;
    policyAutoSaveTimer.current = window.setTimeout(() => {
      policyAutoSaveTimer.current = null;
      latestAutoSaveRef.current();
    }, SETTINGS_AUTO_SAVE_DELAY_MS);
    return clearPolicyAutoSave;
    // Intentionally schedule only when user edits a draft so failed saves never loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftRevision]);

  useEffect(() => clearPolicyAutoSave, []);

  function close() {
    if (pending.current) return;
    clearPolicyAutoSave();
    if (cleanup?.policy) {
      setLimitDraft(null);
      setAgeDraft(null);
      setStorageLimitDraft(null);
      setIntervalDraft(null);
    }
    setCleanup(undefined);
  }

  function confirm() {
    if (!cleanup || (!cleanup.policy && !mask)) return;
    void perform(async () => {
      try { await apply(cleanup, cleanup.policy ? undefined : mask); }
      finally { setCleanup(undefined); }
    });
  }

  async function handleOpenDataDir() {
    if (!locationsAvailable || locationBusy) return;
    setLocationBusy('openData');
    setLocationFeedback(undefined);
    try {
      await invoke('storage_open_data_dir');
    } catch {
      if (mounted.current) setLocationFeedback({ text: t('storageOpenDirFailed'), error: true });
    } finally {
      if (mounted.current) setLocationBusy(null);
    }
  }

  async function handleChangeDataDir() {
    if (!locationsAvailable || locationBusy) return;
    setLocationBusy('changeData');
    setLocationFeedback(undefined);
    try {
      const updated = await invoke<StorageLocationInfo | null>('storage_data_dir_pick');
      if (mounted.current && updated) {
        setLocation(updated);
        setLocationFeedback({ text: t('storageDirMigrated') });
        reload();
      }
    } catch (error) {
      if (mounted.current) setLocationFeedback({ text: t(migrationErrorKey(error)), error: true });
    } finally {
      if (mounted.current) setLocationBusy(null);
    }
  }

  async function handleResetDataDir() {
    if (!locationsAvailable || !location?.isCustomDataDir || locationBusy) return;
    setLocationBusy('resetData');
    setLocationFeedback(undefined);
    try {
      const updated = await invoke<StorageLocationInfo>('storage_data_dir_reset');
      if (mounted.current) {
        setLocation(updated);
        setLocationFeedback({ text: t('storageDirResetDone') });
        reload();
      }
    } catch (error) {
      if (mounted.current) setLocationFeedback({ text: t(migrationErrorKey(error)), error: true });
    } finally {
      if (mounted.current) setLocationBusy(null);
    }
  }

  async function handleOpenLogDir() {
    if (!locationsAvailable || locationBusy) return;
    setLocationBusy('openLog');
    setLocationFeedback(undefined);
    try {
      await invoke('storage_open_log_dir');
    } catch {
      if (mounted.current) setLocationFeedback({ text: t('storageOpenDirFailed'), error: true });
    } finally {
      if (mounted.current) setLocationBusy(null);
    }
  }

  function formatCount(count: number) {
    return count === 1 ? t('storageItemCountOne') : t('storageItemCount').replace('{count}', number.format(count));
  }

  return <div className="settings-sections">
    {toast && <div className="storage-toast" role={toast.error ? 'alert' : 'status'} aria-live={toast.error ? 'assertive' : 'polite'}>
      <span className="storage-toast-icon" data-kind={toast.error ? 'error' : 'success'} aria-hidden="true">
        {toast.error ? <CircleAlert strokeWidth={2.4} /> : <Check strokeWidth={3} />}
      </span>
      <span>{toast.text}</span>
    </div>}

    <section aria-labelledby="clipboard-storage-usage-heading">
      <h2 id="clipboard-storage-usage-heading">{t('storageUsageHeading')}</h2>
      <div className="settings-group">
        <div className="storage-usage-overview">
          <div className="storage-usage-header">
            <div className="settings-shortcut-name">
              <span className="settings-field-content">{t('clipboardStorageUsed')}</span>
              <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardStorageUsed'))} text={storageHelp} />
            </div>
            <span className="storage-usage-total" aria-live="polite">{storageError ? t('clipboardStorageUnavailable') : storage ? storageSize(storage.bytes, locale) : '—'}</span>
          </div>
          <div className="storage-usage-bar" aria-hidden="true">
            {breakdownTotal > 0 && breakdown && breakdownKinds.map(({ key }) => {
              const bytes = breakdown[key].bytes;
              if (!bytes) return null;
              return <span key={key} className="storage-usage-segment" data-kind={key} style={{ width: `${(bytes / breakdownTotal) * 100}%` }} />;
            })}
          </div>
        </div>
        {breakdownKinds.map(({ key, label, icon: Icon }) => {
          const usage = breakdown?.[key];
          return <div className="settings-field storage-usage-row" key={key} data-storage-kind={key}>
            <span className="storage-usage-dot" data-kind={key} aria-hidden="true" />
            <Icon className="clipboard-setting-icon" aria-hidden="true" />
            <span className="settings-field-content">{t(label)}</span>
            <div className="storage-usage-meta">
              <span className="storage-usage-count">{usage ? formatCount(usage.count) : '—'}</span>
              <span className="storage-usage-size">{usage ? storageSize(usage.bytes, locale) : '—'}</span>
            </div>
          </div>;
        })}
      </div>
    </section>

    <section aria-labelledby="storage-location-heading">
      <h2 id="storage-location-heading">{t('storageLocationHeading')}</h2>
      <div className="settings-group">
        <div className="settings-field storage-location-item">
          <div className="settings-shortcut-name">
            <span className="settings-field-content">{t('storageDataDir')}</span>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('storageDataDir'))} text={t('storageDataDirHelp')} />
          </div>
          <div className="storage-location-detail">
            <p className="storage-location-path" title={location?.dataDir ?? undefined}>{location?.dataDir ?? '—'}</p>
            <div className="storage-location-actions">
              <button type="button" className="settings-action clipboard-icon-button" title={t('storageOpenFolder')}
                aria-label={t('storageOpenFolder')} disabled={!locationsAvailable || locationBusy !== null}
                onClick={() => void handleOpenDataDir()}><FolderOpen aria-hidden="true" /></button>
              <button type="button" className="settings-action clipboard-icon-button" title={t('storageChangeFolder')}
                aria-label={t('storageChangeFolder')} disabled={!locationsAvailable || locationBusy !== null}
                onClick={() => void handleChangeDataDir()}><FolderSync aria-hidden="true" /></button>
              <button type="button" className="settings-action clipboard-icon-button" title={t('storageResetFolder')}
                aria-label={t('storageResetFolder')} disabled={!locationsAvailable || !location?.isCustomDataDir || locationBusy !== null}
                onClick={() => void handleResetDataDir()}><RotateCcw aria-hidden="true" /></button>
            </div>
          </div>
        </div>
        <div className="settings-field storage-location-item">
          <div className="settings-shortcut-name">
            <span className="settings-field-content">{t('storageLogDir')}</span>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('storageLogDir'))} text={t('storageLogDirDescription')} />
          </div>
          <div className="storage-location-detail">
            <p className="storage-location-path" title={location?.logDir ?? undefined}>{location?.logDir ?? '—'}</p>
            <div className="storage-location-actions">
              <button type="button" className="settings-action clipboard-icon-button" title={t('storageOpenLogFolder')}
                aria-label={t('storageOpenLogFolder')} disabled={!locationsAvailable || locationBusy !== null}
                onClick={() => void handleOpenLogDir()}><FolderOpen aria-hidden="true" /></button>
            </div>
          </div>
        </div>
        {locationFeedback && <p className={`settings-field-note${locationFeedback.error ? '' : ' storage-location-status'}`}
          role={locationFeedback.error ? 'alert' : 'status'}>{locationFeedback.text}</p>}
      </div>
    </section>

    <section aria-labelledby="clipboard-storage-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <h2 id="clipboard-storage-heading">{t('clipboardHistoryStorage')}</h2>
        <button type="button" className="settings-action clipboard-clear-records" aria-haspopup="dialog" disabled={!available} aria-disabled={busy || undefined}
          onClick={(event) => { event.currentTarget.focus({ preventScroll: true }); preview(null); }}>{t('clipboardManualCleanup')}</button>
      </div>
      <div className="settings-group">
        <form className="settings-field" onSubmit={(event) => {
          event.preventDefault();
          clearPolicyAutoSave();
          if (canSaveAge) preview(buildPolicy({ maxAgeDays: parsedAge }));
        }}>
          <div className="settings-shortcut-name">
            <label className="settings-field-content" htmlFor={`${id}-retention`}>{t('clipboardHistoryAge')}</label>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardHistoryAge'))} text={t('clipboardHistoryAgeHelp')} />
          </div>
          {available && storage ? <div className="settings-history-limit">
            <input id={`${id}-retention`} type="text" inputMode="numeric" pattern="[0-9]+" maxLength={4} className="settings-text-input"
              value={ageDraft ?? String(currentAge)} readOnly={busy} aria-invalid={ageInvalid || undefined} aria-description={t('clipboardHistoryAgeHelp')}
              onChange={(event) => { setAgeDraft(event.target.value); setDraftRevision((v) => v + 1); }}
              onBlur={() => { if (ageDraft !== null && (!ageValid || parsedAge === currentAge)) { setAgeDraft(null); setDraftRevision((v) => v + 1); } }}
              onKeyDown={(event) => { if (event.key === 'Escape' && !busy) { clearPolicyAutoSave(); setAgeDraft(null); } }} />
            <span className="settings-input-unit">{t('storageUnitDays')}</span>
          </div> : <span className="settings-badge">—</span>}
        </form>
        <form className="settings-field" onSubmit={(event) => {
          event.preventDefault();
          clearPolicyAutoSave();
          if (canSaveLimit) preview(buildPolicy({ limit: parsedLimit }));
        }}>
          <div className="settings-shortcut-name">
            <label className="settings-field-content" htmlFor={`${id}-limit`}>{t('clipboardHistoryLimit')}</label>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardHistoryLimit'))} text={t('clipboardHistoryLimitHelp')} />
          </div>
          {available && storage ? <div className="settings-history-limit">
            <input id={`${id}-limit`} type="text" inputMode="numeric" pattern="[0-9]+" maxLength={7} className="settings-text-input"
              value={limitDraft ?? String(currentLimit)} readOnly={busy} aria-invalid={limitInvalid || undefined} aria-description={t('clipboardHistoryLimitHelp')}
              onChange={(event) => { setLimitDraft(event.target.value); setDraftRevision((v) => v + 1); }}
              onBlur={() => { if (limitDraft !== null && (!limitValid || parsedLimit === currentLimit)) { setLimitDraft(null); setDraftRevision((v) => v + 1); } }}
              onKeyDown={(event) => { if (event.key === 'Escape' && !busy) { clearPolicyAutoSave(); setLimitDraft(null); } }} />
            <span className="settings-input-unit">{t('storageUnitItems')}</span>
          </div> : <span className="settings-badge">—</span>}
        </form>
        <form className="settings-field" onSubmit={(event) => {
          event.preventDefault();
          clearPolicyAutoSave();
          if (canSaveStorageMb) preview(buildPolicy({ maxStorageMb: parsedStorageMb }));
        }}>
          <div className="settings-shortcut-name">
            <label className="settings-field-content" htmlFor={`${id}-storage-limit`}>{t('clipboardStorageLimit')}</label>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardStorageLimit'))} text={t('clipboardStorageLimitHelp')} />
          </div>
          {available && storage ? <div className="settings-history-limit">
            <input id={`${id}-storage-limit`} type="text" inputMode="numeric" pattern="[0-9]+" maxLength={6} className="settings-text-input"
              value={storageLimitDraft ?? String(currentStorageMb)} readOnly={busy} aria-invalid={storageMbInvalid || undefined} aria-description={t('clipboardStorageLimitHelp')}
              onChange={(event) => { setStorageLimitDraft(event.target.value); setDraftRevision((v) => v + 1); }}
              onBlur={() => { if (storageLimitDraft !== null && (!storageMbValid || parsedStorageMb === currentStorageMb)) { setStorageLimitDraft(null); setDraftRevision((v) => v + 1); } }}
              onKeyDown={(event) => { if (event.key === 'Escape' && !busy) { clearPolicyAutoSave(); setStorageLimitDraft(null); } }} />
            <span className="settings-input-unit">{t('storageUnitMb')}</span>
          </div> : <span className="settings-badge">—</span>}
        </form>
        <form className="settings-field" onSubmit={(event) => {
          event.preventDefault();
          clearPolicyAutoSave();
          if (canSaveInterval) preview(buildPolicy({ autoCleanupIntervalHours: parsedInterval }));
        }}>
          <div className="settings-shortcut-name">
            <label className="settings-field-content" htmlFor={`${id}-cleanup-interval`}>{t('storageAutoCleanupInterval')}</label>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('storageAutoCleanupInterval'))} text={t('storageAutoCleanupIntervalHelp')} />
          </div>
          {available && storage ? <div className="settings-history-limit">
            <input id={`${id}-cleanup-interval`} type="text" inputMode="numeric" pattern="[0-9]+" maxLength={3} className="settings-text-input"
              value={intervalDraft ?? String(currentInterval)} readOnly={busy} aria-invalid={intervalInvalid || undefined} aria-description={t('storageAutoCleanupIntervalHelp')}
              onChange={(event) => { setIntervalDraft(event.target.value); setDraftRevision((v) => v + 1); }}
              onBlur={() => { if (intervalDraft !== null && (!intervalValid || parsedInterval === currentInterval)) { setIntervalDraft(null); setDraftRevision((v) => v + 1); } }}
              onKeyDown={(event) => { if (event.key === 'Escape' && !busy) { clearPolicyAutoSave(); setIntervalDraft(null); } }} />
            <span className="settings-input-unit">{t('storageUnitHours')}</span>
          </div> : <span className="settings-badge">—</span>}
        </form>
        {feedback && <p className="settings-field-note" role={feedback.error ? 'alert' : 'status'}>{t(feedback.message)}</p>}
      </div>
    </section>

    <BackupSettings t={t} available={backupAvailable} />

    {cleanup && <SettingsDialog className="settings-cleanup-dialog" title={t(cleanup.policy ? 'clipboardHistoryPolicyHeading' : 'clipboardManualCleanup')}
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
  </div>;
}
