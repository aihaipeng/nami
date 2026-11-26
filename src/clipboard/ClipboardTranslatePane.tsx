import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowLeftRight, CircleCheck, Globe } from 'lucide-react';
import type { ViewProps } from '../App';
import { ActivityBeam } from '../shared/ActivityBeam';
import { HistoryList } from '../shared/HistoryList';
import { RecordFeedback } from '../shared/RecordFeedback';
import { restorePanelFocus } from '../shared/FloatingPanel';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { translationShortcutEnabled, type TranslationOutput, type TranslationPreferences } from '../shared/contracts';
import { inputWithinLimits } from '../shared/input';
import { languageName, providerLanguages, supportsLanguage, supportsTranslation, type TranslationEngine as Engine } from '../shared/languages';
import { editsText, matchesBinding, type ShortcutAction } from '../shared/shortcuts';
import { HistoryIcon } from '../translation/HistoryIcon';
import { appleLanguagePackMissing, availableTranslationServices, translationRetryServices, translationServices } from '../shared/translationServices';

type Tab = 'source' | 'primary' | 'secondary';
type TranslationRequest = { text: string; source: string; targets: string[] };
type PartialTranslation = { id: string; translations: string[]; sourceLanguage: string; errors: (string | null)[] };
type ClipboardTranslationContext = { recordId: string; text: string; error: string | null };

const TRANSLATION_STRATEGY_VERSION = 1;

export function ClipboardTranslatePane({ t, snapshot, command, native }: ViewProps) {
  const state = snapshot.window;
  const active = state.clipboardVisible && state.clipboardTranslationVisible;
  const capabilities = snapshot.capabilities;
  const [draft, setDraft] = useState('');
  const [activeTab, setActiveTab] = useState<Tab>('primary');
  const [source, setSource] = useState(snapshot.translationPreferences.sourceLanguage);
  const [detectedSource, setDetectedSource] = useState<string | null>(null);
  const [primary, setPrimary] = useState(snapshot.translationPreferences.primaryLanguage);
  const [secondary, setSecondary] = useState(snapshot.translationPreferences.secondaryLanguage);
  const [cachedResults, setCachedResults] = useState<string[]>([]);
  const [partial, setPartial] = useState<PartialTranslation | null>(null);
  const [partialEngine, setPartialEngine] = useState<Engine | null>(null);
  const [failedRequest, setFailedRequest] = useState<{ request: TranslationRequest; engine: Engine; reuseTargetIndex?: number } | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [translating, setTranslating] = useState(false);
  const [notice, setNotice] = useState<'inputLimit' | null>(null);
  const [serviceError, setServiceError] = useState<string | null>(null);
  const [popup, setPopup] = useState<'language' | 'engine' | null>(null);
  const [sessionEngine, setSessionEngine] = useState<Engine | null>(null);
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);

  const requestRevision = useRef(0);
  const completedOutput = useRef<TranslationOutput | null>(null);
  const partialTicket = useRef<{ id: string; request: TranslationRequest; pending: boolean } | null>(null);
  const cancellation = useRef<Promise<unknown>>(Promise.resolve());
  const copyOperation = useRef(0);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const copying = useRef(false);
  const resultList = useRef<HTMLDivElement>(null);
  const resultPinned = useRef<HTMLDivElement>(null);
  const sourceRef = useRef(source);
  const primaryRef = useRef(primary);
  const secondaryRef = useRef(secondary);
  const draftRef = useRef(draft);
  const sessionEngineRef = useRef<Engine | null>(sessionEngine);
  const previousGlobalEngine = useRef<Engine>(snapshot.translationEngine);
  const preferencesRef = useRef(snapshot.translationPreferences);
  const lastBroadcast = useRef(snapshot.translationPreferences);
  const receiveContext = useRef<(context: ClipboardTranslationContext) => void>(() => {});
  const applyPreferences = useRef<(preferences: TranslationPreferences) => void>(() => {});
  sourceRef.current = source;
  primaryRef.current = primary;
  secondaryRef.current = secondary;
  draftRef.current = draft;
  sessionEngineRef.current = sessionEngine;
  preferencesRef.current = snapshot.translationPreferences;

  const currentTab: Tab = secondary === 'none' && activeTab === 'secondary' ? 'primary' : activeTab;
  const effectiveEngine: Engine = sessionEngine ?? snapshot.translationEngine;
  const sourceSupported = supportsLanguage(effectiveEngine, 'source', source);
  const primarySupported = supportsLanguage(effectiveEngine, 'target', primary);
  const secondarySupported = secondary === 'none' || supportsLanguage(effectiveEngine, 'target', secondary);
  const activeTabSupported = currentTab === 'source'
    ? sourceSupported
    : currentTab === 'secondary'
      ? sourceSupported && secondarySupported
      : sourceSupported && primarySupported;
  const shortcutBinding = (action: ShortcutAction) => {
    const preferences = preferencesRef.current;
    return translationShortcutEnabled(preferences, action) ? preferences.shortcuts[action] : null;
  };
  const sourceLabel = `${languageName(source === 'auto' && detectedSource ? detectedSource : source)}${supportsLanguage(effectiveEngine, 'source', source) ? '' : ` (${t('languageUnavailable')})`}`;
  const primaryLabel = `${languageName(primary)}${supportsLanguage(effectiveEngine, 'target', primary) ? '' : ` (${t('languageUnavailable')})`}`;
  const secondaryLabel = `${languageName(secondary)}${secondary === 'none' || supportsLanguage(effectiveEngine, 'target', secondary) ? '' : ` (${t('languageUnavailable')})`}`;

  function selectTab(next: Tab) {
    setPopup(null);
    setActiveTab(next);
  }

  function clearCopyState() {
    ++copyOperation.current;
    copying.current = false;
    clearTimeout(copyTimer.current);
    setCopied(false);
    setCopyFailed(false);
  }

  function discardRequestView() {
    ++requestRevision.current;
    partialTicket.current = null;
    setPartial(null);
    setRetrying(false);
    setTranslating(false);
    setFailedRequest(null);
    setPartialEngine(null);
  }

  function cancelRequest() {
    discardRequestView();
    if (native) cancellation.current = cancellation.current.then(() => invoke('translation_cancel', { scope: 'clipboard' })).catch(() => {});
  }

  function resetPane() {
    completedOutput.current = null;
    cancelRequest();
    clearCopyState();
    sessionEngineRef.current = null;
    setSessionEngine(null);
    setDraft('');
    setActiveTab('primary');
    setCachedResults([]);
    setDetectedSource(null);
    setNotice(null);
    setServiceError(null);
    setPopup(null);
  }

  useEffect(() => () => {
    ++copyOperation.current;
    clearTimeout(copyTimer.current);
    ++requestRevision.current;
    if (native) void invoke('translation_cancel', { scope: 'clipboard' }).catch(() => {});
  }, [native]);

  useEffect(() => {
    clearCopyState();
  }, [currentTab, draft, cachedResults, partial?.id, state.recordsSession]);

  useEffect(() => {
    resultList.current?.scrollTo(0, 0);
  }, [currentTab, state.clipboardTranslationRecordId, state.recordsSession]);

  useEffect(() => {
    if (!active) resetPane();
  }, [active, state.recordsSession]);

  useEffect(() => {
    if (previousGlobalEngine.current === snapshot.translationEngine) return;
    previousGlobalEngine.current = snapshot.translationEngine;
    completedOutput.current = null;
    sessionEngineRef.current = null;
    setSessionEngine(null);
    setServiceError(null);
    setDetectedSource(null);
    const text = draftRef.current;
    if (active && text.trim()) {
      const targets = [primaryRef.current, ...(secondaryRef.current === 'none' ? [] : [secondaryRef.current])];
      void translateText(text, sourceRef.current, targets, undefined, undefined, snapshot.translationEngine);
    } else {
      cancelRequest();
      setCachedResults([]);
    }
  }, [snapshot.translationEngine]);

  useEffect(() => { setPopup(null); }, [snapshot.locale, active, currentTab]);
  useEffect(() => {
    const close = () => setPopup(null);
    window.addEventListener('blur', close);
    window.addEventListener('resize', close);
    window.addEventListener('nami:window-moved', close);
    return () => {
      window.removeEventListener('blur', close);
      window.removeEventListener('resize', close);
      window.removeEventListener('nami:window-moved', close);
    };
  }, []);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<TranslationPreferences>('translation_preferences_changed', ({ payload }) => {
      if (!disposed) applyPreferences.current(payload);
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native]);

  useEffect(() => {
    if (!native || !active) return;
    let disposed = false;
    const take = async () => {
      const context = await invoke<ClipboardTranslationContext | null>('clipboard_translation_take');
      if (!disposed && context) receiveContext.current(context);
    };
    const remove = listen('clipboard_translation_ready', () => {
      void take().catch(() => { if (!disposed) setServiceError('Unable to read clipboard text.'); });
    });
    void remove.then(take).catch(() => { if (!disposed) setServiceError('Unable to read clipboard text.'); });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native, active]);

  async function translateText(text: string, sourceLanguage: string, targetLanguages: string[], reuse?: TranslationOutput, refreshTarget?: number, engineOverride?: Engine) {
    cancelRequest();
    const targetEngine = engineOverride ?? sessionEngineRef.current ?? snapshot.translationEngine;
    const preservesDetection = refreshTarget !== undefined && reuse?.text === text && reuse.translation.requestedSource === sourceLanguage;
    const retained = targetLanguages.map((target, index) => index !== refreshTarget && reuse?.text === text
      && reuse.translation.requestedSource === sourceLanguage
      && (reuse.translation.targetEngines?.[index] ?? reuse.translation.engine) === targetEngine
      && reuse.translation.strategyVersion === TRANSLATION_STRATEGY_VERSION
      && reuse.targetLanguages[index] === target ? reuse.translations[index] ?? '' : '');
    setPopup(null);
    setCachedResults(retained);
    setServiceError(null);
    setNotice(null);
    if (!preservesDetection) setDetectedSource(null);
    const supportedIndices = supportsLanguage(targetEngine, 'source', sourceLanguage)
      ? targetLanguages.map((target, index) => supportsLanguage(targetEngine, 'target', target) ? index : -1).filter((index) => index >= 0)
      : [];
    if (snapshot.storageError || supportedIndices.length === 0) return;
    if (!inputWithinLimits(text)) { setNotice('inputLimit'); return; }
    if (!native) { setServiceError('Open Nami to translate.'); return; }
    const revision = requestRevision.current;
    await cancellation.current;
    if (requestRevision.current !== revision) return;
    const requestTargets = supportedIndices.map((index) => targetLanguages[index]);
    const fullTargets = supportedIndices.length === targetLanguages.length;
    await runTranslation(
      { text, source: sourceLanguage, targets: requestTargets },
      revision,
      {
        ...(fullTargets && refreshTarget !== undefined && reuse?.text === text ? { reuseTargetIndex: refreshTarget } : {}),
        ...(targetEngine !== snapshot.translationEngine ? { retryEngine: targetEngine } : {}),
      },
      fullTargets ? undefined : { totalTargets: targetLanguages.length, targetLanguages, slotIndices: supportedIndices },
    );
  }

  async function runTranslation(
    request: TranslationRequest,
    revision: number,
    options: { retryId?: string; retryEngine?: Engine; reuseTargetIndex?: number },
    mapping?: { totalTargets: number; targetLanguages: string[]; slotIndices: number[] },
  ) {
    const engine = options.retryEngine ?? (options.retryId ? partialEngine : null) ?? sessionEngineRef.current ?? snapshot.translationEngine;
    setTranslating(true);
    try {
      const result = await invoke<TranslationOutput | { partial: PartialTranslation }>('translation_run', { ...request, scope: 'clipboard', ...options });
      if (requestRevision.current !== revision) return;
      const output = 'partial' in result ? result.partial : result;
      const expand = <T,>(values: T[], fallback: T) => {
        if (!mapping) return values;
        const expanded = Array.from({ length: mapping.totalTargets }, () => fallback);
        mapping.slotIndices.forEach((slot, index) => {
          if (index < values.length) expanded[slot] = values[index];
        });
        return expanded;
      };
      const translations = expand(output.translations, '');
      if ('partial' in result) {
        completedOutput.current = null;
        partialTicket.current = { id: result.partial.id, request, pending: false };
        setPartial(mapping ? { ...result.partial, translations, errors: expand(result.partial.errors, null) } : result.partial);
        setPartialEngine(engine);
      } else {
        completedOutput.current = mapping
          ? { ...result, targetLanguages: mapping.targetLanguages, translations }
          : result;
        partialTicket.current = null;
        setPartial(null);
      }
      setCachedResults(translations);
      setDetectedSource(output.sourceLanguage === 'auto' ? null : output.sourceLanguage);
    } catch (error) {
      if (requestRevision.current === revision && String(error) !== 'cancelled') {
        setServiceError(String(error));
        if (!options.retryId) setFailedRequest({ request, engine, reuseTargetIndex: options.reuseTargetIndex });
      }
    } finally {
      if (requestRevision.current === revision) {
        if (options.retryId || options.retryEngine) {
          if (partialTicket.current) partialTicket.current.pending = false;
          setRetrying(false);
        }
        setTranslating(false);
      }
    }
  }

  receiveContext.current = (context) => {
    completedOutput.current = null;
    cancelRequest();
    clearCopyState();
    const preferred = preferencesRef.current;
    sessionEngineRef.current = null;
    setSessionEngine(null);
    setDraft(context.text);
    setActiveTab('primary');
    setSource(preferred.sourceLanguage);
    setDetectedSource(null);
    setPrimary(preferred.primaryLanguage);
    setSecondary(preferred.secondaryLanguage);
    setCachedResults([]);
    setNotice(null);
    sourceRef.current = preferred.sourceLanguage;
    primaryRef.current = preferred.primaryLanguage;
    secondaryRef.current = preferred.secondaryLanguage;
    setServiceError(context.error ? context.error : null);
    const targets = [preferred.primaryLanguage, ...(preferred.secondaryLanguage === 'none' ? [] : [preferred.secondaryLanguage])];
    if (context.text.trim()) void translateText(context.text, preferred.sourceLanguage, targets, undefined, undefined, snapshot.translationEngine);
  };

  applyPreferences.current = (preferences) => {
    const previous = lastBroadcast.current;
    lastBroadcast.current = preferences;
    const current = { source: sourceRef.current, primary: primaryRef.current, secondary: secondaryRef.current };
    const next = {
      source: preferences.sourceLanguage === previous.sourceLanguage ? current.source : preferences.sourceLanguage,
      primary: preferences.primaryLanguage === previous.primaryLanguage ? current.primary : preferences.primaryLanguage,
      secondary: preferences.secondaryLanguage === previous.secondaryLanguage ? current.secondary : preferences.secondaryLanguage,
    };
    if (next.source === current.source && next.primary === current.primary && next.secondary === current.secondary) return;
    sourceRef.current = next.source;
    primaryRef.current = next.primary;
    secondaryRef.current = next.secondary;
    setSource(next.source);
    setPrimary(next.primary);
    setSecondary(next.secondary);
    if (next.secondary === 'none') setActiveTab((tab) => tab === 'secondary' ? 'primary' : tab);
    const text = draftRef.current;
    if (!active || !text.trim()) return;
    const targets = [next.primary, ...(next.secondary === 'none' ? [] : [next.secondary])];
    const sourceChanged = next.source !== current.source;
    const primaryChanged = next.primary !== current.primary;
    const secondaryChanged = next.secondary !== current.secondary;
    if (sourceChanged || (primaryChanged && secondaryChanged)) void translateText(text, next.source, targets);
    else if (primaryChanged) void translateText(text, next.source, targets, completedOutput.current ?? undefined, 0);
    else void translateText(text, next.source, targets, completedOutput.current ?? undefined, 1);
  };

  function changeLanguage(side: 'source' | 'primary' | 'secondary', value: string) {
    const current = { source, primary, secondary };
    if (current[side] === value) return;
    const next = { ...current, [side]: value };
    sourceRef.current = next.source;
    primaryRef.current = next.primary;
    secondaryRef.current = next.secondary;
    ({ source: setSource, primary: setPrimary, secondary: setSecondary })[side](value);
    if (side === 'secondary' && value === 'none') setActiveTab((tab) => tab === 'secondary' ? 'primary' : tab);
    const persistSide = side === 'source' ? 'source' : 'target';
    if (value === 'auto' || value === 'none' || supportsLanguage(snapshot.translationEngine, persistSide, value)) {
      void command('translation_preferences_set', side === 'source' ? { sourceLanguage: value } : side === 'primary' ? { primaryLanguage: value } : { secondaryLanguage: value });
    }
    if (side === 'source') setDetectedSource(null);
    const targets = [next.primary, ...(next.secondary === 'none' ? [] : [next.secondary])];
    if (draft.trim()) {
      void translateText(draft, next.source, targets, side === 'source' ? undefined : completedOutput.current ?? undefined, side === 'primary' ? 0 : side === 'secondary' ? 1 : undefined);
    } else {
      cancelRequest();
      setCachedResults([]);
      setNotice(null);
      setServiceError(null);
    }
  }

  const currentRetryEngine = partialEngine ?? failedRequest?.engine;
  const retryRequest = partial ? partialTicket.current?.request : failedRequest?.request;
  const retryServices = currentRetryEngine && retryRequest
    ? translationRetryServices(capabilities.appleTranslation, currentRetryEngine, retryRequest.source,
      retryRequest.targets.filter((_, index) => !partial || !!partial.errors[index]))
    : [];

  function retryTranslation(engine?: Engine) {
    const ticket = partialTicket.current;
    if (retrying || snapshot.storageError) return;
    if (ticket && ticket.id === partial?.id && !ticket.pending) {
      ticket.pending = true;
      setRetrying(true);
      setServiceError(null);
      void runTranslation(ticket.request, ++requestRevision.current, { retryId: ticket.id, ...(engine ? { retryEngine: engine } : {}) });
    } else if (failedRequest && engine) {
      setRetrying(true);
      setServiceError(null);
      setFailedRequest(null);
      void runTranslation(failedRequest.request, ++requestRevision.current, {
        retryEngine: engine,
        ...(failedRequest.reuseTargetIndex !== undefined ? { reuseTargetIndex: failedRequest.reuseTargetIndex } : {}),
      });
    }
  }

  const languagePackAction = (error: string | null | undefined) => capabilities.appleTranslation && appleLanguagePackMissing(error) &&
    <button type="button" onClick={() => void command('translation_language_settings_open')}>{t('openTranslationLanguages')}</button>;
  const retryAfterDownload = !partial && appleLanguagePackMissing(serviceError) &&
    <button type="button" disabled={retrying || !!snapshot.storageError} onClick={() => retryTranslation('apple')}>{t('retry')}</button>;
  const retryElsewhere = retryServices.map(({ code, retry }) => <button key={code} type="button" disabled={retrying || !!snapshot.storageError} onClick={() => retryTranslation(code)}>
    {t(retry)}
  </button>);

  async function copyPart(part: Tab) {
    const text = part === 'source' ? draft : cachedResults[part === 'secondary' ? 1 : 0];
    if (!native || !capabilities.clipboardWrite || !text || copying.current) return;
    setPopup(null);
    setCopyFailed(false);
    copying.current = true;
    const operation = ++copyOperation.current;
    try {
      await invoke('translation_copy', {
        scope: 'clipboard',
        part,
        ...(part === 'source' ? { sourceText: draft } : {}),
      });
      if (operation === copyOperation.current) {
        setCopied(true);
        clearTimeout(copyTimer.current);
        copyTimer.current = setTimeout(() => setCopied(false), 1000);
      }
    } catch {
      if (operation === copyOperation.current) setCopyFailed(true);
    } finally {
      if (operation === copyOperation.current) copying.current = false;
    }
  }

  function keydown(event: ReactKeyboardEvent) {
    if (event.defaultPrevented || event.nativeEvent.isComposing || event.keyCode === 229) return;
    const closeWindow = matchesBinding({ code: 'KeyW', modifiers: [capabilities.primaryModifier] }, event);
    if ((event.key === 'Escape' || closeWindow) && popup) {
      event.preventDefault();
      event.stopPropagation();
      setPopup(null);
      restorePanelFocus(event.currentTarget.querySelector<HTMLButtonElement>('button[aria-haspopup=dialog][aria-expanded=true]'));
      return;
    }
    if (editsText(event.nativeEvent)) return;
    if (matchesBinding(shortcutBinding('copyPrimary'), event)) {
      event.preventDefault();
      event.stopPropagation();
      void copyPart('primary');
    } else if (matchesBinding(shortcutBinding('copySecondary'), event) && secondary !== 'none') {
      event.preventDefault();
      event.stopPropagation();
      void copyPart('secondary');
    } else if (event.key === 'Escape' || closeWindow) {
      event.preventDefault();
      event.stopPropagation();
      setPopup(null);
      void command('clipboard_translation_close', { returnFocus: true });
    }
  }

  function selectEngine(nextEngine: Engine) {
    setPopup(null);
    if (nextEngine === activeEngine && !serviceError && !partial) return;
    completedOutput.current = null;
    sessionEngineRef.current = nextEngine;
    setSessionEngine(nextEngine);
    const nextSource = supportsLanguage(nextEngine, 'source', source) ? source : 'auto';
    if (nextSource !== source) {
      sourceRef.current = nextSource;
      setSource(nextSource);
    }
    const targets = [primary, ...(secondary === 'none' ? [] : [secondary])];
    if (draft.trim()) {
      void translateText(draft, nextSource, targets, undefined, undefined, nextEngine);
    } else {
      cancelRequest();
      setCachedResults([]);
      setNotice(null);
      setServiceError(null);
    }
  }

  const activePickerLabel = t(currentTab === 'source' ? 'sourceLanguage' : currentTab === 'primary' ? 'primaryLanguage' : 'secondaryLanguage');
  const activePickerValue = currentTab === 'source' ? source : currentTab === 'primary' ? primary : secondary;
  const activePickerValueLabel = currentTab === 'source' ? sourceLabel : currentTab === 'primary' ? primaryLabel : secondaryLabel;
  const activePickerItems = [
    ...(currentTab === 'source' ? [{ code: 'auto', name: 'Auto-Detect' }] : currentTab === 'secondary' ? [{ code: 'none', name: 'None' }] : []),
    ...providerLanguages(effectiveEngine, currentTab === 'source' ? 'source' : 'target'),
  ];
  const activePickerDescription = currentTab === 'source' && source === 'auto' && detectedSource
    ? `${t('detectedLanguage')}: ${languageName(detectedSource)}`
    : undefined;

  const activeIndex = currentTab === 'secondary' ? 1 : 0;
  const activeCopyText = currentTab === 'source' ? draft : cachedResults[activeIndex] ?? '';
  const activeResult = currentTab === 'source' ? '' : (notice ? '' : cachedResults[activeIndex] ?? '');
  const activePartialError = currentTab === 'source' ? null : partial?.errors[activeIndex] ?? null;
  const taskStatus = serviceError ?? (notice ? t(notice) : !activeTabSupported ? t('languageNotSupported') : null);
  const copyLabel = currentTab === 'source' ? t('copySource') : currentTab === 'secondary' ? t('copySecondary') : t('copyPrimary');
  const sourceTabTitle = activePickerDescription && currentTab === 'source'
    ? `${t('sourceLanguage')}: ${sourceLabel} (${activePickerDescription})`
    : `${t('sourceLanguage')}: ${sourceLabel}`;
  const activeEngine: Engine = (currentTab !== 'source'
    ? (completedOutput.current?.translation.targetEngines?.[activeIndex] ?? completedOutput.current?.translation.engine ?? partialEngine)
    : (completedOutput.current?.translation.engine ?? partialEngine)) ?? effectiveEngine;
  const serviceMeta = translationServices[activeEngine];
  const engineItems = availableTranslationServices(capabilities.appleTranslation);
  const displayedText = currentTab === 'source' ? draft : activeResult;
  const characterCount = Array.from(displayedText).length;
  const activeMetaRoleLabel = t(currentTab === 'source' ? 'translateTabSource' : currentTab === 'secondary' ? 'translateTabSecondary' : 'translateTabPrimary');
  const activeMetaLanguageLabel = currentTab === 'source'
    ? (source === 'auto'
      ? (detectedSource ? `Auto-Detect · ${languageName(detectedSource)}` : 'Auto-Detect')
      : sourceLabel)
    : currentTab === 'secondary'
      ? secondaryLabel
      : primaryLabel;

  return <section className="history-panel clipboard-translate-pane" aria-label={t('translatePane')} aria-busy={translating} tabIndex={-1} onKeyDown={keydown}>
    <ActivityBeam label={t('working')} active={active && translating} />
    <header className="clipboard-detail-toolbar" data-tauri-drag-region>
      <div className="clipboard-detail-mode clipboard-translate-tabs" role="group" aria-label={t('translateViewTabs')} data-tauri-drag-region="false">
        <button type="button" data-tab="source" aria-pressed={currentTab === 'source'} title={sourceTabTitle} onClick={() => selectTab('source')}>{sourceLabel}</button>
        <button type="button" data-tab="primary" aria-pressed={currentTab === 'primary'} title={`${t('primaryLanguage')}: ${primaryLabel}`} onClick={() => selectTab('primary')}>{primaryLabel}</button>
        {secondary !== 'none' && <button type="button" data-tab="secondary" aria-pressed={currentTab === 'secondary'} title={`${t('secondaryLanguage')}: ${secondaryLabel}`} onClick={() => selectTab('secondary')}>{secondaryLabel}</button>}
      </div>
      <div className="history-row-actions" role="group" aria-label={t('actions')} data-tauri-drag-region="false">
        <span className="history-action" data-action="language">
          <SearchableDropdown
            value={activePickerValue}
            onChange={(value) => changeLanguage(currentTab, value)}
            label={activePickerLabel}
            items={activePickerItems}
            t={t}
            valueLabel={activePickerValueLabel}
            description={activePickerDescription}
            triggerTitle={`${t('switchLanguage')} · ${activePickerLabel}`}
            iconTrigger={<Globe aria-hidden="true" />}
            side="bottom"
            open={popup === 'language'}
            onOpenChange={(open) => setPopup(open ? 'language' : null)}
          />
        </span>
        <span className="history-action" data-action="engine">
          <SearchableDropdown
            value={activeEngine}
            onChange={(value) => selectEngine(value as Engine)}
            label={t('settingsService')}
            items={engineItems}
            t={t}
            searchable={false}
            menuWidth={232}
            triggerTitle={t('switchTranslationEngine')}
            iconTrigger={<ArrowLeftRight aria-hidden="true" />}
            side="bottom"
            open={popup === 'engine'}
            onOpenChange={(open) => setPopup(open ? 'engine' : null)}
          />
        </span>
        <span className="history-action" data-action="copy">
          <button type="button" className={`icon-button${copied ? ' record-copied' : ''}`} title={copyLabel} aria-label={copyLabel}
            disabled={!native || !capabilities.clipboardWrite || !activeCopyText} onClick={() => { void copyPart(currentTab); }}>
            {copied ? <CircleCheck className="copy-success" aria-hidden="true" /> : <HistoryIcon name="doc.on.doc" />}
          </button>
        </span>
      </div>
    </header>
    <div className="clipboard-translate-body" data-tauri-drag-region="false">
      <HistoryList scrollOnly active={active} list={resultList} pinnedList={resultPinned} pinned={[]} onNearEnd={() => {}}>
        {currentTab === 'source'
          ? <div className="clipboard-translate-result" role="region" aria-label={t('sourceLabel')}>
            {draft && <div className="clipboard-detail-text">{draft}</div>}
          </div>
          : <div className="clipboard-translate-result" role="region" aria-label={currentTab === 'secondary' ? t('secondaryLanguage') : t('primaryLanguage')}>
            {activeResult && <div className="clipboard-detail-text">{activeResult}</div>}
            {activePartialError && <div className="result-error">
              <span role="alert">{activePartialError}</span>
              {languagePackAction(activePartialError)}
              <button type="button" disabled={retrying || !!snapshot.storageError} onClick={() => retryTranslation()}>{t(retrying ? 'translationRetrying' : 'retry')}</button>
              {retryElsewhere}
            </div>}
            {taskStatus && !activeResult && !activePartialError && <div className="translation-status">
              <span role={serviceError ? 'alert' : 'status'}>{taskStatus}</span>
              {languagePackAction(serviceError)}{retryAfterDownload}{serviceError && retryElsewhere}
            </div>}
            {taskStatus && activeResult && <div className="translation-status with-results">
              <span role={serviceError ? 'alert' : 'status'}>{taskStatus}</span>
              {languagePackAction(serviceError)}{retryAfterDownload}{serviceError && !partial && retryElsewhere}
            </div>}
            {!activeResult && !activePartialError && !taskStatus && !translating && <div className="clipboard-translate-empty">{t('resultEmpty')}</div>}
          </div>}
      </HistoryList>
    </div>
    <footer className="clipboard-detail-metadata">
      <div><span>{t('translateMetaCharacters')}</span><span>{characterCount.toLocaleString('en')}</span></div>
      <div><span>{activeMetaRoleLabel}</span><span title={activeMetaLanguageLabel}>{activeMetaLanguageLabel}</span></div>
      <div>
        <span>{t('translateMetaService')}</span>
        <span className="history-row-heading">
          {serviceMeta.icon && <img className="history-origin" src={serviceMeta.icon} alt="" aria-hidden="true" />}
          <span className="history-languages" title={serviceMeta.name}>{serviceMeta.name}</span>
        </span>
      </div>
    </footer>
    <RecordFeedback message={copyFailed ? t('copyFailed') : null} />
  </section>;
}
