import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AppWindow, Info, Plus, X } from 'lucide-react';
import { FloatingPanel } from '../shared/FloatingPanel';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import type { MessageKey, Translate } from '../shared/i18n';
import { errorCode } from './ShortcutCapture';

type ExcludedApp = { bundleId: string; name: string; iconDataUrl?: string };
type Activation = 'paste' | 'copy';
type Options = { excludedApps: ExcludedApp[]; ignoredPasteboardTypes?: string[]; ignoredRegexps?: string[]; ignoreSecrets?: boolean; activation: Activation; pasteAsPlainText: boolean };
const UNIVERSAL_CLIPBOARD_TYPE = 'com.apple.is-remote-clipboard';

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
  const ignoreUniversal = ignoredTypes.includes(UNIVERSAL_CLIPBOARD_TYPE);
  const ignoreSecrets = Boolean(state.options?.ignoreSecrets);
  const [typeOpen, setTypeOpen] = useState(false);
  const [typeDraft, setTypeDraft] = useState('');
  const [typeError, setTypeError] = useState<MessageKey | null>(null);
  const [regexOpen, setRegexOpen] = useState(false);
  const [regexDraft, setRegexDraft] = useState('');
  const [regexError, setRegexError] = useState<MessageKey | null>(null);
  const [presetError, setPresetError] = useState<MessageKey | null>(null);

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

  async function removeType(pasteboardType: string, trigger?: HTMLButtonElement, fallback?: HTMLButtonElement | null) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setTypeError(null);
    try {
      const next = await invoke<string[]>('clipboard_ignored_type_remove', { pasteboardType });
      if (trigger && trigger === document.activeElement) fallback?.focus({ preventScroll: true });
      state.setOptions((previous) => previous && { ...previous, ignoredPasteboardTypes: next });
    } catch { setTypeError('clipboardSettingsSaveFailed'); }
    finally { pending.current = false; setBusy(false); }
  }

  async function updateType(oldPasteboardType: string, nextValue: string) {
    if (pending.current || !state.options) return false;
    const value = nextValue.trim();
    if (!value) { setTypeError('clipboardInvalidType'); return false; }
    pending.current = true;
    setBusy(true);
    setTypeError(null);
    try {
      const next = await invoke<string[]>('clipboard_ignored_type_update', { oldPasteboardType, newPasteboardType: value });
      state.setOptions((previous) => previous && { ...previous, ignoredPasteboardTypes: next });
      return true;
    } catch (err) {
      setTypeError(ruleErrorMessage(err, 'type'));
      return false;
    } finally { pending.current = false; setBusy(false); }
  }

  async function toggleUniversalClipboard(enabled: boolean) {
    if (pending.current || !state.options) return;
    pending.current = true;
    setBusy(true);
    setPresetError(null);
    setTypeError(null);
    try {
      const next = await invoke<string[]>(enabled ? 'clipboard_ignored_type_add' : 'clipboard_ignored_type_remove', { pasteboardType: UNIVERSAL_CLIPBOARD_TYPE });
      state.setOptions((previous) => previous && { ...previous, ignoredPasteboardTypes: next });
    } catch (err) { setPresetError(ruleErrorMessage(err, 'type')); }
    finally { pending.current = false; setBusy(false); }
  }

  async function toggleIgnoreSecrets(enabled: boolean) {
    if (pending.current || !state.options) return;
    pending.current = true;
    setBusy(true);
    setPresetError(null);
    try {
      const next = await invoke<boolean>('clipboard_ignore_secrets_set', { enabled });
      state.setOptions((previous) => previous && { ...previous, ignoreSecrets: next });
    } catch { setPresetError('clipboardSettingsSaveFailed'); }
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

  async function removeRegex(pattern: string, trigger?: HTMLButtonElement, fallback?: HTMLButtonElement | null) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setRegexError(null);
    try {
      const next = await invoke<string[]>('clipboard_ignored_regex_remove', { pattern });
      if (trigger && trigger === document.activeElement) fallback?.focus({ preventScroll: true });
      state.setOptions((previous) => previous && { ...previous, ignoredRegexps: next });
    } catch { setRegexError('clipboardSettingsSaveFailed'); }
    finally { pending.current = false; setBusy(false); }
  }

  return <section aria-labelledby="clipboard-ignore-rules-heading">
    <div className="settings-section-heading clipboard-section-heading">
      <h2 id="clipboard-ignore-rules-heading">{t('clipboardIgnoreRulesHeading')}</h2>
      {!available && <span className="settings-badge">{t('platformUnavailable')}</span>}
    </div>
    {available && <div className="settings-group clipboard-ignore-group">
      <div className="clipboard-ignore-block">
        <div className="settings-field clipboard-ignore-row">
          <span id="clipboard-excluded-heading" className="settings-field-content" title={t('clipboardExcludedAppsDescription')}>{t('clipboardExcludedAppsHeading')}</span>
          <button type="button" ref={add} className="settings-action clipboard-icon-button clipboard-ignore-add" disabled={!apps} aria-disabled={busy || undefined}
            title={t('clipboardAddApp')} aria-label={t('clipboardAddApp')} aria-description={t('clipboardExcludedAppsDescription')}
            onClick={() => void change()}><Plus aria-hidden="true" /></button>
        </div>
        {!!apps?.length && <ul className="clipboard-ignore-chips clipboard-excluded-list" role="list" aria-labelledby="clipboard-excluded-heading">
          {apps.map(({ bundleId, name, iconDataUrl }) => {
            const duplicate = apps.some((app) => app.bundleId !== bundleId && app.name === name);
            const identity = duplicate ? `${name} (${bundleId})` : name;
            const remove = t('clipboardRemoveApp').replace('{app}', identity);
            return <li className="clipboard-ignore-chip" key={bundleId}>
              <ExcludedAppIcon url={iconDataUrl} />
              <span className="clipboard-excluded-label" title={bundleId}>{name}</span>
              {duplicate && <ClipboardSettingTip label={t('clipboardAboutExcludedApp').replace('{app}', identity)} text={bundleId} />}
              <button type="button" aria-disabled={busy || undefined} className="settings-action clipboard-icon-button clipboard-chip-remove" title={remove}
                aria-label={remove} onClick={(event) => void change(bundleId, event.currentTarget)}><X aria-hidden="true" /></button>
            </li>;
          })}
        </ul>}
        {error && <p className="clipboard-setting-note" role="alert">{t('clipboardManageAppsFailed')}</p>}
      </div>

      <IgnoreRuleBlock
        headingId="clipboard-ignored-types-heading"
        heading={t('clipboardIgnoredTypesHeading')}
        note={t('clipboardIgnoredTypesNote')}
        addLabel={t('clipboardAddType')}
        submitLabel={t('clipboardAddRule')}
        saveLabel={t('clipboardSaveRule')}
        placeholder={t('clipboardIgnoredTypesPlaceholder')}
        items={ignoredTypes}
        removeLabel={(value) => t('clipboardRemoveType').replace('{type}', value)}
        editLabel={(value) => t('clipboardEditType').replace('{type}', value)}
        open={typeOpen}
        onOpenChange={(next) => { setTypeOpen(next); if (!next) { setTypeDraft(''); setTypeError(null); } }}
        draft={typeDraft}
        onDraftChange={(value) => { setTypeDraft(value); setTypeError(null); }}
        error={typeError ? t(typeError) : null}
        ready={Boolean(state.options)}
        busy={busy}
        onSubmit={submitType}
        onEdit={updateType}
        onRemove={removeType}
      />

      <IgnoreRuleBlock
        headingId="clipboard-ignored-regex-heading"
        heading={t('clipboardIgnoredRegexHeading')}
        note={t('clipboardIgnoredRegexNote')}
        addLabel={t('clipboardAddRegex')}
        submitLabel={t('clipboardAddRule')}
        placeholder={t('clipboardIgnoredRegexPlaceholder')}
        items={ignoredRegexps}
        removeLabel={(value) => t('clipboardRemoveRegex').replace('{pattern}', value)}
        open={regexOpen}
        onOpenChange={(next) => { setRegexOpen(next); if (!next) { setRegexDraft(''); setRegexError(null); } }}
        draft={regexDraft}
        onDraftChange={(value) => { setRegexDraft(value); setRegexError(null); }}
        error={regexError ? t(regexError) : null}
        ready={Boolean(state.options)}
        busy={busy}
        onSubmit={submitRegex}
        onRemove={removeRegex}
      />

      <div className="clipboard-ignore-block">
        <label className="settings-field" title={t('clipboardIgnoreUniversalNote')}>
          <span className="settings-field-content"><span>{t('clipboardIgnoreUniversal')}</span></span>
          <input className="settings-checkbox" id="clipboard-ignore-universal" type="checkbox"
            aria-label={t('clipboardIgnoreUniversal')} aria-description={t('clipboardIgnoreUniversalNote')}
            checked={ignoreUniversal} disabled={!state.options} aria-disabled={busy || undefined}
            onChange={(event) => { if (!busy) void toggleUniversalClipboard(event.target.checked); }} />
        </label>
        <label className="settings-field" title={t('clipboardIgnoreSecretsNote')}>
          <span className="settings-field-content"><span>{t('clipboardIgnoreSecrets')}</span></span>
          <input className="settings-checkbox" id="clipboard-ignore-secrets" type="checkbox"
            aria-label={t('clipboardIgnoreSecrets')} aria-description={t('clipboardIgnoreSecretsNote')}
            checked={ignoreSecrets} disabled={!state.options} aria-disabled={busy || undefined}
            onChange={(event) => { if (!busy) void toggleIgnoreSecrets(event.target.checked); }} />
        </label>
        {presetError && <p className="clipboard-setting-note" role="alert">{t(presetError)}</p>}
      </div>
    </div>}
  </section>;
}

function IgnoreRuleBlock({
  headingId, heading, note, addLabel, submitLabel, saveLabel, placeholder, items, removeLabel, editLabel,
  open, onOpenChange, draft, onDraftChange, error, ready, busy, onSubmit, onEdit, onRemove,
}: {
  headingId: string; heading: string; note: string; addLabel: string; submitLabel: string; saveLabel?: string; placeholder: string;
  items: string[]; removeLabel: (value: string) => string; editLabel?: (value: string) => string; open: boolean; onOpenChange: (open: boolean) => void;
  draft: string; onDraftChange: (value: string) => void; error: string | null; ready: boolean; busy: boolean;
  onSubmit: () => Promise<void>; onEdit?: (oldValue: string, nextValue: string) => Promise<boolean>; onRemove: (value: string, trigger?: HTMLButtonElement, fallback?: HTMLButtonElement | null) => Promise<void>;
}) {
  const addButton = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const editInputRef = useRef<HTMLInputElement>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState('');
  useEffect(() => {
    if (open) inputRef.current?.focus({ preventScroll: true });
  }, [open]);
  useEffect(() => {
    if (editing !== null) {
      editInputRef.current?.focus({ preventScroll: true });
      editInputRef.current?.select();
    }
  }, [editing]);
  useEffect(() => {
    if (editing !== null && !items.includes(editing)) { setEditing(null); setEditDraft(''); }
  }, [items, editing]);

  function close(restoreFocus = false) {
    onOpenChange(false);
    if (restoreFocus) addButton.current?.focus({ preventScroll: true });
  }

  async function saveEdit() {
    if (editing === null || !onEdit) return;
    if (editDraft.trim() === editing) { setEditing(null); setEditDraft(''); return; }
    const saved = await onEdit(editing, editDraft);
    if (saved) { setEditing(null); setEditDraft(''); }
  }

  return <div className="clipboard-ignore-block">
    <div className="settings-field clipboard-ignore-row">
      <span id={headingId} className="settings-field-content" title={note}>{heading}</span>
      <button type="button" ref={addButton} className="settings-action clipboard-icon-button clipboard-ignore-add"
        title={addLabel} aria-label={addLabel} aria-description={note} aria-expanded={open}
        disabled={!ready} aria-disabled={busy || undefined}
        onClick={() => { setEditing(null); onOpenChange(!open); }}><Plus aria-hidden="true" /></button>
    </div>
    {!!items.length && <ul className="clipboard-ignore-chips clipboard-rule-list" role="list" aria-labelledby={headingId}>
      {items.map((value) => {
        const remove = removeLabel(value);
        if (onEdit && editing === value) {
          return <li className="clipboard-ignore-chip clipboard-ignore-chip-editing" key={value}>
            <form className="clipboard-rule-inline-editor" onSubmit={(event) => { event.preventDefault(); void saveEdit(); }}>
              <input ref={editInputRef} type="text" className="settings-text-input clipboard-rule-input" value={editDraft}
                disabled={!ready} readOnly={busy} spellCheck={false} autoComplete="off"
                aria-label={editLabel ? editLabel(value) : heading} aria-invalid={error ? true : undefined}
                onChange={(event) => setEditDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') {
                    event.preventDefault();
                    event.stopPropagation();
                    setEditing(null);
                    setEditDraft('');
                  }
                }} />
              <button type="submit" className="settings-action clipboard-composer-submit" disabled={!ready || !editDraft.trim()} aria-disabled={busy || undefined}>{saveLabel ?? submitLabel}</button>
            </form>
          </li>;
        }
        return <li className="clipboard-ignore-chip" key={value}>
          {onEdit ? <button type="button" className="clipboard-rule-edit-trigger" title={editLabel ? editLabel(value) : value}
            aria-label={editLabel ? editLabel(value) : value} disabled={!ready} aria-disabled={busy || undefined}
            onClick={() => { if (busy) return; onOpenChange(false); setEditing(value); setEditDraft(value); }}>
            <code className="clipboard-rule-value">{value}</code>
          </button> : <code className="clipboard-rule-value" title={value}>{value}</code>}
          <button type="button" aria-disabled={busy || undefined} className="settings-action clipboard-icon-button clipboard-chip-remove" title={remove}
            aria-label={remove} onClick={(event) => void onRemove(value, event.currentTarget, addButton.current)}><X aria-hidden="true" /></button>
        </li>;
      })}
    </ul>}
    {open && <form className="clipboard-rule-composer"
      onSubmit={(event) => { event.preventDefault(); void onSubmit(); }}
      onBlur={(event) => {
        if (busy || draft.trim() || error) return;
        const next = event.relatedTarget as Node | null;
        if (!event.currentTarget.contains(next) && next !== addButton.current) close();
      }}>
      <input ref={inputRef} type="text" className="settings-text-input clipboard-rule-input" value={draft}
        disabled={!ready} readOnly={busy} spellCheck={false} autoComplete="off"
        aria-label={heading} aria-description={note} aria-invalid={error ? true : undefined}
        placeholder={placeholder} onChange={(event) => onDraftChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            close(true);
          }
        }} />
      <button type="submit" className="settings-action clipboard-composer-submit" disabled={!ready || !draft.trim()} aria-disabled={busy || undefined}>{submitLabel}</button>
    </form>}
    {error && <p className="clipboard-setting-note" role="alert">{error}</p>}
  </div>;
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
