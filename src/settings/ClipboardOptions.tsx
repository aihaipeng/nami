import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AppWindow, Info, X } from 'lucide-react';
import { FloatingPanel } from '../shared/FloatingPanel';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import type { MessageKey, Translate } from '../shared/i18n';
import { errorCode } from './ShortcutCapture';

type ExcludedApp = { bundleId: string; name: string; iconDataUrl?: string };
type Activation = 'paste' | 'copy';
type Options = { excludedApps: ExcludedApp[]; ignoredPasteboardTypes?: string[]; ignoredRegexps?: string[]; activation: Activation; pasteAsPlainText: boolean };

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

function ruleErrorMessage(error: unknown, kind: 'type' | 'regex'): MessageKey {
  const code = errorCode(error);
  if (code === 'clipboard_exclusions_full') return 'clipboardExclusionsFull';
  if (code === 'clipboard_invalid_pasteboard_type') return 'clipboardInvalidType';
  if (code === 'clipboard_invalid_regex') return 'clipboardInvalidRegex';
  return kind === 'type' ? 'clipboardSettingsSaveFailed' : 'clipboardSettingsSaveFailed';
}

export function ClipboardIgnoreRules({ t, state, available }: { t: Translate; state: OptionsState; available: boolean }) {
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const [error, setError] = useState(false);
  const add = useRef<HTMLButtonElement>(null);
  const apps = state.options?.excludedApps;
  const ignoredTypes = state.options?.ignoredPasteboardTypes ?? [];
  const ignoredRegexps = state.options?.ignoredRegexps ?? [];
  const [typeDraft, setTypeDraft] = useState('');
  const [typeError, setTypeError] = useState<MessageKey | null>(null);
  const [regexDraft, setRegexDraft] = useState('');
  const [regexError, setRegexError] = useState<MessageKey | null>(null);

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

  async function submitType() {
    if (pending.current || !state.options) return;
    const value = typeDraft.trim();
    if (!value) { setTypeError('clipboardInvalidType'); return; }
    pending.current = true;
    setBusy(true);
    setTypeError(null);
    try {
      const next = await invoke<string[]>('clipboard_ignored_type_add', { pasteboardType: value });
      state.setOptions((previous) => previous && { ...previous, ignoredPasteboardTypes: next });
      setTypeDraft('');
    } catch (err) { setTypeError(ruleErrorMessage(err, 'type')); }
    finally { pending.current = false; setBusy(false); }
  }

  async function removeType(pasteboardType: string) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setTypeError(null);
    try {
      const next = await invoke<string[]>('clipboard_ignored_type_remove', { pasteboardType });
      state.setOptions((previous) => previous && { ...previous, ignoredPasteboardTypes: next });
    } catch { setTypeError('clipboardSettingsSaveFailed'); }
    finally { pending.current = false; setBusy(false); }
  }

  async function submitRegex() {
    if (pending.current || !state.options) return;
    if (!regexDraft.trim()) { setRegexError('clipboardInvalidRegex'); return; }
    pending.current = true;
    setBusy(true);
    setRegexError(null);
    try {
      const next = await invoke<string[]>('clipboard_ignored_regex_add', { pattern: regexDraft });
      state.setOptions((previous) => previous && { ...previous, ignoredRegexps: next });
      setRegexDraft('');
    } catch (err) { setRegexError(ruleErrorMessage(err, 'regex')); }
    finally { pending.current = false; setBusy(false); }
  }

  async function removeRegex(pattern: string) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setRegexError(null);
    try {
      const next = await invoke<string[]>('clipboard_ignored_regex_remove', { pattern });
      state.setOptions((previous) => previous && { ...previous, ignoredRegexps: next });
    } catch { setRegexError('clipboardSettingsSaveFailed'); }
    finally { pending.current = false; setBusy(false); }
  }

  return <>
    <section aria-labelledby="clipboard-excluded-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <div className="clipboard-section-title">
          <h2 id="clipboard-excluded-heading">{t('clipboardExcludedAppsHeading')}</h2>
          <ClipboardSettingTip label={t('clipboardAboutExcludedApps')} text={t('clipboardExcludedAppsDescription')} />
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
              {duplicate && <ClipboardSettingTip label={t('clipboardAboutExcludedApp').replace('{app}', identity)} text={bundleId} />}
            </div>
            <button type="button" aria-disabled={busy || undefined} className="settings-action clipboard-icon-button" title={remove}
              aria-label={remove} onClick={(event) => void change(bundleId, event.currentTarget)}><X aria-hidden="true" /></button>
          </li>;
        })}
      </ul>}
      {error && <p className="clipboard-setting-note" role="alert">{t('clipboardManageAppsFailed')}</p>}
    </section>

    <section aria-labelledby="clipboard-ignored-types-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <div className="clipboard-section-title">
          <h2 id="clipboard-ignored-types-heading">{t('clipboardIgnoredTypesHeading')}</h2>
          <ClipboardSettingTip label={t('clipboardIgnoredTypesHeading')} text={t('clipboardIgnoredTypesNote')} />
        </div>
        {!available && <span className="settings-badge">{t('platformUnavailable')}</span>}
      </div>
      {available && <div className="settings-group">
        <form className="settings-field clipboard-rule-form" onSubmit={(event) => { event.preventDefault(); void submitType(); }}>
          <input type="text" className="settings-text-input clipboard-rule-input" value={typeDraft} disabled={!state.options} readOnly={busy}
            aria-label={t('clipboardIgnoredTypesHeading')} aria-invalid={typeError ? true : undefined}
            placeholder={t('clipboardIgnoredTypesPlaceholder')} onChange={(event) => { setTypeDraft(event.target.value); setTypeError(null); }} />
          <button type="submit" className="settings-action" disabled={!state.options || !typeDraft.trim()} aria-disabled={busy || undefined}>{t('clipboardAddRule')}</button>
        </form>
        {!!ignoredTypes.length && <ul className="clipboard-rule-list" role="list" aria-labelledby="clipboard-ignored-types-heading">
          {ignoredTypes.map((kind) => {
            const remove = t('clipboardRemoveType').replace('{type}', kind);
            return <li key={kind}>
              <code className="clipboard-rule-value">{kind}</code>
              <button type="button" aria-disabled={busy || undefined} className="settings-action clipboard-icon-button" title={remove}
                aria-label={remove} onClick={() => void removeType(kind)}><X aria-hidden="true" /></button>
            </li>;
          })}
        </ul>}
        {typeError && <p className="clipboard-setting-note" role="alert">{t(typeError)}</p>}
      </div>}
    </section>

    <section aria-labelledby="clipboard-ignored-regex-heading">
      <div className="settings-section-heading clipboard-section-heading">
        <div className="clipboard-section-title">
          <h2 id="clipboard-ignored-regex-heading">{t('clipboardIgnoredRegexHeading')}</h2>
          <ClipboardSettingTip label={t('clipboardIgnoredRegexHeading')} text={t('clipboardIgnoredRegexNote')} />
        </div>
        {!available && <span className="settings-badge">{t('platformUnavailable')}</span>}
      </div>
      {available && <div className="settings-group">
        <form className="settings-field clipboard-rule-form" onSubmit={(event) => { event.preventDefault(); void submitRegex(); }}>
          <input type="text" className="settings-text-input clipboard-rule-input" value={regexDraft} disabled={!state.options} readOnly={busy}
            aria-label={t('clipboardIgnoredRegexHeading')} aria-invalid={regexError ? true : undefined}
            placeholder={t('clipboardIgnoredRegexPlaceholder')} onChange={(event) => { setRegexDraft(event.target.value); setRegexError(null); }} />
          <button type="submit" className="settings-action" disabled={!state.options || !regexDraft.trim()} aria-disabled={busy || undefined}>{t('clipboardAddRule')}</button>
        </form>
        {!!ignoredRegexps.length && <ul className="clipboard-rule-list" role="list" aria-labelledby="clipboard-ignored-regex-heading">
          {ignoredRegexps.map((pattern) => {
            const remove = t('clipboardRemoveRegex').replace('{pattern}', pattern);
            return <li key={pattern}>
              <code className="clipboard-rule-value">{pattern}</code>
              <button type="button" aria-disabled={busy || undefined} className="settings-action clipboard-icon-button" title={remove}
                aria-label={remove} onClick={() => void removeRegex(pattern)}><X aria-hidden="true" /></button>
            </li>;
          })}
        </ul>}
        {regexError && <p className="clipboard-setting-note" role="alert">{t(regexError)}</p>}
      </div>}
    </section>
  </>;
}

function ExcludedAppIcon({ url }: { url?: string }) {
  const [failed, setFailed] = useState<string>();
  return url && url !== failed
    ? <img className="clipboard-excluded-icon" src={url} alt="" onError={() => setFailed(url)} />
    : <AppWindow className="clipboard-excluded-icon clipboard-icon-fallback" aria-hidden="true" />;
}

export function ClipboardSettingTip({ label, text }: { label: string; text: string }) {
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
  const [open, setOpen] = useState<'activation' | 'pasteFormat' | null>(null);
  const [behaviorBusy, setBehaviorBusy] = useState(false);
  const [behaviorError, setBehaviorError] = useState(false);
  const behaviorPending = useRef(false);
  const options = state.options;
  useEffect(() => { setOpen(null); }, [locale]);

  async function changeBehavior(next: { activation: Activation } | { pasteAsPlainText: boolean }) {
    if (!options || behaviorPending.current || ('activation' in next ? next.activation === options.activation : next.pasteAsPlainText === options.pasteAsPlainText)) return;
    behaviorPending.current = true;
    setBehaviorBusy(true);
    setBehaviorError(false);
    try {
      await invoke('clipboard_behavior_set', next);
      state.setOptions((previous) => previous && { ...previous, ...next });
    } catch { setBehaviorError(true); }
    finally { behaviorPending.current = false; setBehaviorBusy(false); }
  }

  const activations = [{ code: 'copy', name: t('clipboardActivationCopy') }, { code: 'paste', name: t('clipboardActivationPaste') }];
  const pasteFormats = [{ code: 'original', name: t('clipboardPasteOriginal') }, { code: 'plain', name: t('clipboardPastePlain') }];
  const unavailable = <span className="settings-badge">{platformAvailable ? '—' : t('platformUnavailable')}</span>;

  return <section aria-labelledby="clipboard-behavior-heading">
      <h2 id="clipboard-behavior-heading">{t('clipboardBehavior')}</h2>
      <div className="settings-group">
        <Setting label={t('clipboardActivation')}>
          {platformAvailable && options ? <div className="clipboard-picker" data-pending={behaviorBusy || undefined}>
            <SearchableDropdown value={options.activation} label={t('clipboardActivation')} items={activations} t={t} settings searchable={false} description={t('clipboardActivationHelp')} side="top" menuWidth={218}
              open={open === 'activation'} onOpenChange={(next) => { if (!next || !behaviorPending.current) setOpen(next ? 'activation' : null); }}
              onChange={(value) => void changeBehavior({ activation: value as Activation })} />
          </div> : unavailable}
        </Setting>
        <Setting label={t('clipboardPasteFormat')}>
          {platformAvailable && options ? <div className="clipboard-picker" data-pending={behaviorBusy || undefined}>
            <SearchableDropdown value={options.pasteAsPlainText ? 'plain' : 'original'} label={t('clipboardPasteFormat')} items={pasteFormats} t={t} settings searchable={false} description={t('clipboardPasteFormatHelp')} side="top" menuWidth={218}
              open={open === 'pasteFormat'} onOpenChange={(next) => { if (!next || !behaviorPending.current) setOpen(next ? 'pasteFormat' : null); }}
              onChange={(value) => void changeBehavior({ pasteAsPlainText: value === 'plain' })} />
          </div> : unavailable}
        </Setting>
        {behaviorError && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
      </div>
    </section>;
}
