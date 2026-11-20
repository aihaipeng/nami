import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AppWindow, Info, X } from 'lucide-react';
import { FloatingPanel } from '../shared/FloatingPanel';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import type { Translate } from '../shared/i18n';

type ExcludedApp = { bundleId: string; name: string; iconDataUrl?: string };
type Activation = 'paste' | 'copy';
type Options = { excludedApps: ExcludedApp[]; activation: Activation; autoImageIndex: boolean };

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

export function ClipboardReuseSettings({ t, state, platformAvailable, locale }: {
  t: Translate; state: OptionsState; platformAvailable: boolean; locale: string;
}) {
  const [open, setOpen] = useState(false);
  const [behaviorBusy, setBehaviorBusy] = useState(false);
  const [behaviorError, setBehaviorError] = useState(false);
  const behaviorPending = useRef(false);
  const activation = state.options?.activation;
  useEffect(() => { setOpen(false); }, [locale]);

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

  const activations = [{ code: 'copy', name: t('clipboardActivationCopy') }, { code: 'paste', name: t('clipboardActivationPaste') }];

  return <section aria-labelledby="clipboard-behavior-heading">
      <h2 id="clipboard-behavior-heading">{t('clipboardBehavior')}</h2>
      <div className="settings-group">
        <Setting label={t('clipboardActivation')}>
          {platformAvailable && activation ? <div className="clipboard-picker" data-pending={behaviorBusy || undefined}>
            <SearchableDropdown value={activation} label={t('clipboardActivation')} items={activations} t={t} settings searchable={false} description={t('clipboardActivationHelp')} side="top" menuWidth={218}
              open={open} onOpenChange={(next) => { if (!next || !behaviorPending.current) setOpen(next); }}
              onChange={(value) => void changeBehavior(value as Activation)} />
          </div> : <span className="settings-badge">{platformAvailable ? '—' : t('platformUnavailable')}</span>}
        </Setting>
        {behaviorError && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
      </div>
    </section>;
}
