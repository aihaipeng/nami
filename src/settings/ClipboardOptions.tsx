import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AppWindow, Plus, X } from 'lucide-react';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { DEFAULT_HOVER_PREVIEW_DELAY_MS, MAX_HOVER_PREVIEW_DELAY_MS } from '../shared/clipboardShortcuts';
import type { MessageKey, Translate } from '../shared/i18n';
import { SETTINGS_AUTO_SAVE_DELAY_MS } from './autoSave';
import { SettingHintTip } from './SettingHintTip';
import { errorCode } from './ShortcutCapture';

type ExcludedApp = { bundleId: string; name: string; iconDataUrl?: string };
type Activation = 'paste' | 'copy';
type Options = {
  excludedApps: ExcludedApp[];
  ignoredPasteboardTypes?: string[];
  ignoredRegexps?: string[];
  ignoreSecrets?: boolean;
  activation: Activation;
  pasteAsPlainText: boolean;
  hoverPreviewEnabled?: boolean;
  hoverPreviewDelayMs?: number;
};
const UNIVERSAL_CLIPBOARD_TYPE = 'com.apple.is-remote-clipboard';

function Setting({ label, hint, t, children }: { label: string; hint?: string; t?: Translate; children: ReactNode }) {
  return <div className="settings-field clipboard-option-row">
    {hint && t ? <div className="settings-shortcut-name">
      <span className="settings-field-content">{label}</span>
      <SettingHintTip label={t('shortcutHintLabel').replace('{title}', label)} text={hint} />
    </div> : <div className="settings-field-content">{label}</div>}
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
  const [savingPreset, setSavingPreset] = useState<'universal' | 'secrets' | null>(null);
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
    setSavingPreset('universal');
    setPresetError(null);
    setTypeError(null);
    try {
      const next = await invoke<string[]>(enabled ? 'clipboard_ignored_type_add' : 'clipboard_ignored_type_remove', { pasteboardType: UNIVERSAL_CLIPBOARD_TYPE });
      state.setOptions((previous) => previous && { ...previous, ignoredPasteboardTypes: next });
    } catch (err) { setPresetError(ruleErrorMessage(err, 'type')); }
    finally { pending.current = false; setSavingPreset(null); }
  }

  async function toggleIgnoreSecrets(enabled: boolean) {
    if (pending.current || !state.options) return;
    pending.current = true;
    setSavingPreset('secrets');
    setPresetError(null);
    try {
      const next = await invoke<boolean>('clipboard_ignore_secrets_set', { enabled });
      state.setOptions((previous) => previous && { ...previous, ignoreSecrets: next });
    } catch { setPresetError('clipboardSettingsSaveFailed'); }
    finally { pending.current = false; setSavingPreset(null); }
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
          <div className="settings-shortcut-name">
            <span id="clipboard-excluded-heading" className="settings-field-content">{t('clipboardExcludedAppsHeading')}</span>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardExcludedAppsHeading'))} text={t('clipboardExcludedAppsDescription')} />
          </div>
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
        tipLabel={t('shortcutHintLabel').replace('{title}', t('clipboardIgnoredTypesHeading'))}
        addLabel={t('clipboardAddType')}
        submitLabel={t('clipboardAddRule')}
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
        tipLabel={t('shortcutHintLabel').replace('{title}', t('clipboardIgnoredRegexHeading'))}
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
        <div className="settings-field">
          <div className="settings-shortcut-name">
            <span id="clipboard-ignore-universal-label" className="settings-field-content">{t('clipboardIgnoreUniversal')}</span>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardIgnoreUniversal'))} text={t('clipboardIgnoreUniversalNote')} />
          </div>
          <input className="settings-checkbox" id="clipboard-ignore-universal" type="checkbox"
            aria-labelledby="clipboard-ignore-universal-label" aria-description={t('clipboardIgnoreUniversalNote')}
            checked={ignoreUniversal} disabled={!state.options} aria-disabled={savingPreset === 'universal' || undefined}
            onChange={(event) => { if (!pending.current) void toggleUniversalClipboard(event.target.checked); }} />
        </div>
        <div className="settings-field">
          <div className="settings-shortcut-name">
            <span id="clipboard-ignore-secrets-label" className="settings-field-content">{t('clipboardIgnoreSecrets')}</span>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardIgnoreSecrets'))} text={t('clipboardIgnoreSecretsNote')} />
          </div>
          <input className="settings-checkbox" id="clipboard-ignore-secrets" type="checkbox"
            aria-labelledby="clipboard-ignore-secrets-label" aria-description={t('clipboardIgnoreSecretsNote')}
            checked={ignoreSecrets} disabled={!state.options} aria-disabled={savingPreset === 'secrets' || undefined}
            onChange={(event) => { if (!pending.current) void toggleIgnoreSecrets(event.target.checked); }} />
        </div>
        {presetError && <p className="clipboard-setting-note" role="alert">{t(presetError)}</p>}
      </div>
    </div>}
  </section>;
}

function IgnoreRuleBlock({
  headingId, heading, note, tipLabel, addLabel, submitLabel, placeholder, items, removeLabel, editLabel,
  open, onOpenChange, draft, onDraftChange, error, ready, busy, onSubmit, onEdit, onRemove,
}: {
  headingId: string; heading: string; note: string; tipLabel: string; addLabel: string; submitLabel: string; placeholder: string;
  items: string[]; removeLabel: (value: string) => string; editLabel?: (value: string) => string; open: boolean; onOpenChange: (open: boolean) => void;
  draft: string; onDraftChange: (value: string) => void; error: string | null; ready: boolean; busy: boolean;
  onSubmit: () => Promise<void>; onEdit?: (oldValue: string, nextValue: string) => Promise<boolean>; onRemove: (value: string, trigger?: HTMLButtonElement, fallback?: HTMLButtonElement | null) => Promise<void>;
}) {
  const addButton = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const editInputRef = useRef<HTMLInputElement>(null);
  const editAutoSaveTimer = useRef<number | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState('');

  function clearEditAutoSave() {
    if (editAutoSaveTimer.current !== null) {
      window.clearTimeout(editAutoSaveTimer.current);
      editAutoSaveTimer.current = null;
    }
  }

  useEffect(() => () => clearEditAutoSave(), []);
  useEffect(() => {
    if (open) inputRef.current?.focus({ preventScroll: true });
  }, [open]);
  useEffect(() => {
    clearEditAutoSave();
    if (editing !== null) {
      editInputRef.current?.focus({ preventScroll: true });
      editInputRef.current?.select();
    }
  }, [editing]);
  useEffect(() => {
    if (editing !== null && !items.includes(editing)) { clearEditAutoSave(); setEditing(null); setEditDraft(''); }
  }, [items, editing]);

  function close(restoreFocus = false) {
    onOpenChange(false);
    if (restoreFocus) addButton.current?.focus({ preventScroll: true });
  }

  async function saveEdit() {
    clearEditAutoSave();
    if (editing === null || !onEdit) return;
    if (editDraft.trim() === editing) { setEditing(null); setEditDraft(''); return; }
    const saved = await onEdit(editing, editDraft);
    if (saved) { setEditing(null); setEditDraft(''); }
  }

  const latestSaveEditRef = useRef(saveEdit);
  latestSaveEditRef.current = saveEdit;

  function handleEditChange(nextValue: string) {
    setEditDraft(nextValue);
    clearEditAutoSave();
    if (editing === null || !onEdit) return;
    const trimmed = nextValue.trim();
    if (!trimmed || trimmed === editing) return;
    editAutoSaveTimer.current = window.setTimeout(() => {
      editAutoSaveTimer.current = null;
      void latestSaveEditRef.current();
    }, SETTINGS_AUTO_SAVE_DELAY_MS);
  }

  return <div className="clipboard-ignore-block">
    <div className="settings-field clipboard-ignore-row">
      <div className="settings-shortcut-name">
        <span id={headingId} className="settings-field-content">{heading}</span>
        <SettingHintTip label={tipLabel} text={note} />
      </div>
      <button type="button" ref={addButton} className="settings-action clipboard-icon-button clipboard-ignore-add"
        title={addLabel} aria-label={addLabel} aria-description={note} aria-expanded={open}
        disabled={!ready} aria-disabled={busy || undefined}
        onClick={() => { clearEditAutoSave(); setEditing(null); onOpenChange(!open); }}><Plus aria-hidden="true" /></button>
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
                onChange={(event) => handleEditChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') {
                    event.preventDefault();
                    event.stopPropagation();
                    clearEditAutoSave();
                    setEditing(null);
                    setEditDraft('');
                  }
                }} />
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
  return <SettingHintTip label={label} text={text} className="clipboard-tip-trigger" width={220} />;
}

export function ClipboardReuseSettings({ t, state, available, platformAvailable, locale }: {
  t: Translate; state: OptionsState; available?: boolean; platformAvailable: boolean; locale: string;
}) {
  const [open, setOpen] = useState<'activation' | 'pasteFormat' | null>(null);
  const [savingField, setSavingField] = useState<'activation' | 'pasteFormat' | 'autoPreview' | null>(null);
  const [behaviorError, setBehaviorError] = useState(false);
  const [delayDraft, setDelayDraft] = useState<string | null>(null);
  const behaviorPending = useRef(false);
  const delaySaving = useRef(false);
  const pendingDelay = useRef<number | null>(null);
  const delayAutoSaveTimer = useRef<number | null>(null);
  const id = useId();
  const options = state.options;
  const canConfigurePreview = available ?? platformAvailable;
  const hoverPreviewEnabled = Boolean(options?.hoverPreviewEnabled);
  const savedDelay = options?.hoverPreviewDelayMs ?? DEFAULT_HOVER_PREVIEW_DELAY_MS;
  const savedDelayRef = useRef(savedDelay);
  savedDelayRef.current = savedDelay;
  const trimmedDelay = delayDraft?.trim() ?? '';
  const parsedDelay = /^\d+$/.test(trimmedDelay) ? Number(trimmedDelay) : Number.NaN;
  const delayValid = Number.isInteger(parsedDelay) && parsedDelay >= 0 && parsedDelay <= MAX_HOVER_PREVIEW_DELAY_MS;
  const delayInvalid = delayDraft !== null && !delayValid;

  function clearDelayAutoSave() {
    if (delayAutoSaveTimer.current !== null) {
      window.clearTimeout(delayAutoSaveTimer.current);
      delayAutoSaveTimer.current = null;
    }
  }

  useEffect(() => () => clearDelayAutoSave(), []);
  useEffect(() => { setOpen(null); }, [locale]);

  async function flushDelay() {
    if (delaySaving.current) return;
    delaySaving.current = true;
    try {
      while (pendingDelay.current !== null) {
        const value = pendingDelay.current;
        pendingDelay.current = null;
        if (value === savedDelayRef.current) continue;
        await invoke('clipboard_behavior_set', { hoverPreviewDelayMs: value });
        savedDelayRef.current = value;
        state.setOptions((previous) => previous && { ...previous, hoverPreviewDelayMs: value });
      }
      if (pendingDelay.current === null && delayAutoSaveTimer.current === null) {
        setDelayDraft(null);
      }
    } catch {
      setBehaviorError(true);
    } finally {
      delaySaving.current = false;
      if (pendingDelay.current !== null) void flushDelay();
    }
  }

  function editDelay(raw: string) {
    setDelayDraft(raw);
    setBehaviorError(false);
    clearDelayAutoSave();
    const trimmed = raw.trim();
    if (!/^\d+$/.test(trimmed)) return;
    const value = Number(trimmed);
    if (!Number.isInteger(value) || value < 0 || value > MAX_HOVER_PREVIEW_DELAY_MS) return;
    if (value === savedDelayRef.current) return;
    delayAutoSaveTimer.current = window.setTimeout(() => {
      delayAutoSaveTimer.current = null;
      pendingDelay.current = value;
      void flushDelay();
    }, SETTINGS_AUTO_SAVE_DELAY_MS);
  }

  async function changeBehavior(
    field: 'activation' | 'pasteFormat' | 'autoPreview',
    next: { activation: Activation } | { pasteAsPlainText: boolean } | { hoverPreviewEnabled: boolean },
  ) {
    if (!options || behaviorPending.current) return;
    if ('activation' in next && next.activation === options.activation) return;
    if ('pasteAsPlainText' in next && next.pasteAsPlainText === options.pasteAsPlainText) return;
    if ('hoverPreviewEnabled' in next && next.hoverPreviewEnabled === Boolean(options.hoverPreviewEnabled)) return;
    behaviorPending.current = true;
    setSavingField(field);
    setBehaviorError(false);
    try {
      await invoke('clipboard_behavior_set', next);
      state.setOptions((previous) => previous && { ...previous, ...next });
      if ('hoverPreviewEnabled' in next && !next.hoverPreviewEnabled) {
        clearDelayAutoSave();
        setDelayDraft(null);
      }
    } catch { setBehaviorError(true); }
    finally { behaviorPending.current = false; setSavingField(null); }
  }

  const activations = [{ code: 'copy', name: t('clipboardActivationCopy') }, { code: 'paste', name: t('clipboardActivationPaste') }];
  const pasteFormats = [{ code: 'original', name: t('clipboardPasteOriginal') }, { code: 'plain', name: t('clipboardPastePlain') }];
  const unavailable = <span className="settings-badge">{platformAvailable ? '—' : t('platformUnavailable')}</span>;
  const pickerPending = savingField === 'activation' || savingField === 'pasteFormat';

  return <section aria-labelledby="clipboard-behavior-heading">
      <h2 id="clipboard-behavior-heading">{t('clipboardBehavior')}</h2>
      <div className="settings-group">
        <Setting label={t('clipboardActivation')} hint={t('clipboardActivationHelp')} t={t}>
          {platformAvailable && options ? <div className="clipboard-picker" data-pending={pickerPending || undefined}>
            <SearchableDropdown value={options.activation} label={t('clipboardActivation')} items={activations} t={t} settings searchable={false} description={t('clipboardActivationHelp')} side="top" menuWidth={218}
              open={open === 'activation'} onOpenChange={(next) => { if (!next || !behaviorPending.current) setOpen(next ? 'activation' : null); }}
              onChange={(value) => void changeBehavior('activation', { activation: value as Activation })} />
          </div> : unavailable}
        </Setting>
        <Setting label={t('clipboardPasteFormat')} hint={t('clipboardPasteFormatHelp')} t={t}>
          {platformAvailable && options ? <div className="clipboard-picker" data-pending={pickerPending || undefined}>
            <SearchableDropdown value={options.pasteAsPlainText ? 'plain' : 'original'} label={t('clipboardPasteFormat')} items={pasteFormats} t={t} settings searchable={false} description={t('clipboardPasteFormatHelp')} side="top" menuWidth={218}
              open={open === 'pasteFormat'} onOpenChange={(next) => { if (!next || !behaviorPending.current) setOpen(next ? 'pasteFormat' : null); }}
              onChange={(value) => void changeBehavior('pasteFormat', { pasteAsPlainText: value === 'plain' })} />
          </div> : unavailable}
        </Setting>
        <div className="settings-field">
          <span id={`${id}-auto-preview-label`} className="settings-field-content">{t('clipboardAutoPreview')}</span>
          {canConfigurePreview && options
            ? <input id={`${id}-auto-preview`} type="checkbox" className="settings-checkbox clipboard-behavior-checkbox" aria-labelledby={`${id}-auto-preview-label`}
              aria-description={t('clipboardAutoPreviewHelp')} checked={hoverPreviewEnabled} aria-disabled={savingField === 'autoPreview' || undefined}
              onChange={(event) => { if (!behaviorPending.current) void changeBehavior('autoPreview', { hoverPreviewEnabled: event.target.checked }); }} />
            : <span className="settings-badge">{canConfigurePreview ? '—' : t('platformUnavailable')}</span>}
        </div>
        <div className="settings-field settings-preview-delay" data-disabled={canConfigurePreview && options && !hoverPreviewEnabled || undefined}>
          <div className="settings-shortcut-name">
            <label className="settings-field-content" htmlFor={`${id}-preview-delay`}>{t('clipboardPreviewDelay')}</label>
            <SettingHintTip label={t('shortcutHintLabel').replace('{title}', t('clipboardPreviewDelay'))} text={t('clipboardPreviewDelayHelp')} />
          </div>
          {canConfigurePreview && options ? <div className="settings-history-limit">
            <input id={`${id}-preview-delay`} type="text" inputMode="numeric" pattern="[0-9]+" maxLength={5} className="settings-text-input"
              value={delayDraft ?? String(savedDelay)} disabled={!hoverPreviewEnabled}
              aria-invalid={delayInvalid || undefined} aria-description={t('clipboardPreviewDelayHelp')}
              onChange={(event) => editDelay(event.target.value)}
              onBlur={() => {
                if (delayDraft !== null && (!delayValid || parsedDelay === savedDelayRef.current)) {
                  clearDelayAutoSave();
                  setDelayDraft(null);
                }
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  clearDelayAutoSave();
                  setDelayDraft(null);
                  setBehaviorError(false);
                } else if (event.key === 'Enter' && delayValid && parsedDelay !== savedDelayRef.current) {
                  event.preventDefault();
                  clearDelayAutoSave();
                  pendingDelay.current = parsedDelay;
                  void flushDelay();
                }
              }} />
            <span className="settings-input-unit">ms</span>
          </div> : <span className="settings-badge">{canConfigurePreview ? '—' : t('platformUnavailable')}</span>}
        </div>
        {behaviorError && <p className="clipboard-setting-note" role="alert">{t('clipboardSettingsSaveFailed')}</p>}
      </div>
    </section>;
}
