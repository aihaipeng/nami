import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { flushSync } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import type { ViewProps } from '../App';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { languageName, providerLanguages, supportsLanguage, supportsTranslation, type TranslationEngine as Engine } from '../shared/languages';
import { appleLanguagePackMissing, translationRetryServices } from '../shared/translationServices';
import { inputWithinLimits } from '../shared/input';
import { displaySource, dragSourceSelection, editSource, sourceDragDelta } from '../shared/source';
import { editsText, matchesBinding, type ShortcutAction } from '../shared/shortcuts';
import { translationShortcutEnabled, type TranslationOutput, type TranslationPreferences } from '../shared/contracts';
import { cn } from '../lib/utils';
import { ClipboardDetail } from '../clipboard/ClipboardDetail';
import { ClipboardTranslatePane } from '../clipboard/ClipboardTranslatePane';
import { RecordsWorkspace } from '../shared/RecordsWorkspace';
import { useRecordsPreviewShortcut } from '../shared/recordPreviewShortcut';
import { RecordFeedback } from '../shared/RecordFeedback';
import { HistoryDivider } from './HistoryDivider';
import { useNativeChrome } from './useNativeChrome';

type Popup = 'source' | 'primary' | 'secondary';
type TranslationRequest = { text: string; source: string; targets: string[] };
type PartialTranslation = { id: string; translations: string[]; sourceLanguage: string; errors: (string | null)[] };

const TRANSLATION_STRATEGY_VERSION = 1;
let sourceMeasure: CanvasRenderingContext2D | null | undefined;

function sourcePrefixWidth(field: HTMLTextAreaElement, prefix: string) {
  sourceMeasure ??= document.createElement('canvas').getContext('2d');
  if (!sourceMeasure) return prefix.length;
  const style = getComputedStyle(field);
  sourceMeasure.font = style.font || `${style.fontSize} ${style.fontFamily}`;
  return sourceMeasure.measureText(prefix).width;
}

function ClipboardPaneDivider({ t, width, command }: Pick<ViewProps, 't' | 'command'> & { width: number }) {
  const drag = useRef<{ x: number; width: number } | null>(null);
  const frame = useRef(0);
  useEffect(() => () => cancelAnimationFrame(frame.current), []);
  useEffect(() => { if (drag.current) drag.current.width = width; }, [width]);

  function dragWidth(x: number) {
    const current = drag.current!;
    current.width = Math.min(920, Math.max(320, current.width + x - current.x));
    current.x = x;
    return current.width;
  }

  return <div className="clipboard-pane-divider" role="separator" aria-label={t('resizeClipboardDetails')} aria-orientation="vertical" aria-valuemin={320} aria-valuemax={920} aria-valuenow={width} tabIndex={0}
    onKeyDown={(event) => {
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') {
        event.preventDefault();
        void command('clipboard_detail_width_set', { width: event.key === 'Home' ? 320 : event.key === 'End' ? 920 : width + (event.key === 'ArrowRight' ? 24 : -24) });
      }
    }}
    onPointerDown={(event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      drag.current = { x: event.screenX, width };
      event.currentTarget.setPointerCapture(event.pointerId);
    }}
    onPointerMove={(event) => {
      if (!drag.current) return;
      const next = dragWidth(event.screenX);
      cancelAnimationFrame(frame.current);
      frame.current = requestAnimationFrame(() => { void command('clipboard_detail_width_set', { width: next }); });
    }}
    onPointerUp={(event) => {
      if (!drag.current) return;
      const next = dragWidth(event.screenX);
      drag.current = null;
      cancelAnimationFrame(frame.current);
      event.currentTarget.releasePointerCapture(event.pointerId);
      void command('clipboard_detail_width_set', { width: next });
    }}
    onLostPointerCapture={() => { drag.current = null; cancelAnimationFrame(frame.current); }} />;
}

export function Translation({ t, snapshot, command, native, activity }: ViewProps) {
  const [draft, setDraft] = useState('');
  const [source, setSource] = useState(snapshot.translationPreferences.sourceLanguage);
  const [detectedSource, setDetectedSource] = useState<string | null>(null);
  const [primary, setPrimary] = useState(snapshot.translationPreferences.primaryLanguage);
  const [secondary, setSecondary] = useState(snapshot.translationPreferences.secondaryLanguage);
  const [notice, setNotice] = useState<'inputLimit' | null>(null);
  const [serviceError, setServiceError] = useState<string | null>(null);
  const [copyFailed, setCopyFailed] = useState(false);
  const copyOperation = useRef(0);
  const copying = useRef(new Map<number, number>());
  const requestRevision = useRef(0);
  const completedOutput = useRef<TranslationOutput | null>(null);
  const partialTicket = useRef<{ id: string; request: TranslationRequest; pending: boolean } | null>(null);
  const [partial, setPartial] = useState<PartialTranslation | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [failedRequest, setFailedRequest] = useState<{ request: TranslationRequest; engine: Engine; reuseTargetIndex?: number } | null>(null);
  const [partialEngine, setPartialEngine] = useState<Engine | null>(null);
  const cancellation = useRef<Promise<unknown>>(Promise.resolve());
  const receiveSelection = useRef<(context: { text: string; error: string | null }) => void>(() => {});
  const [popup, setPopup] = useState<Popup | null>(null);
  const [cachedResults, setCachedResults] = useState<string[]>([]);
  const [sourceWidth, setSourceWidth] = useState(140);
  const sourceBar = useRef<HTMLElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const composing = useRef(false);
  const normalizedCaret = useRef<number | null>(null);
  const sourceDrag = useRef({ active: false, taken: false, anchor: 0, pointer: 0, frame: 0 });
  const sourceRef = useRef(source);
  const primaryRef = useRef(primary);
  const secondaryRef = useRef(secondary);
  const draftRef = useRef(draft);
  const preferencesRef = useRef(snapshot.translationPreferences);
  const lastBroadcast = useRef(snapshot.translationPreferences);
  const applyPreferences = useRef<(preferences: TranslationPreferences) => void>(() => {});
  sourceRef.current = source;
  primaryRef.current = primary;
  secondaryRef.current = secondary;
  draftRef.current = draft;
  preferencesRef.current = snapshot.translationPreferences;

  const state = snapshot.window;
  const standaloneVisible = state.standaloneTranslationVisible;
  const clipboardRightVisible = state.clipboardVisible && (state.clipboardDetailVisible || state.clipboardTranslationVisible);
  const rightVisible = standaloneVisible || clipboardRightVisible;
  useRecordsPreviewShortcut({
    active: clipboardRightVisible,
    blocked: state.recordsEditorOpen,
    includedContainer: snapshot.capabilities.nativeHistory ? undefined : '.clipboard-right-panes',
    excludedContainers: '.translation-panel, .preview-bar',
    snapshot,
    native,
    onPreview: (_, target) => {
      if (state.clipboardDetailVisible) {
        const closingFocusedPane = !state.clipboardTranslationVisible
          || target instanceof Element && !!target.closest('.clipboard-detail-slot, .clipboard-pane-divider');
        void command('records_details_toggle', closingFocusedPane ? { returnFocus: true } : undefined);
      } else {
        void command('records_details_toggle', { recordId: state.clipboardTranslationRecordId ?? null });
      }
    },
  });
  const committedClipboardWidth = useRef(state.clipboardWidth);
  const clipboardWidthChanged = committedClipboardWidth.current !== state.clipboardWidth;
  useLayoutEffect(() => { committedClipboardWidth.current = state.clipboardWidth; }, [state.clipboardWidth]);

  useEffect(() => {
    ++copyOperation.current; copying.current.clear(); setCopyFailed(false);
    return () => { ++copyOperation.current; };
  }, [draft, cachedResults, partial?.id, standaloneVisible]);

  const capabilities = snapshot.capabilities;
  const languagesAvailable = supportsTranslation(snapshot.translationEngine, source, [primary, ...(secondary === 'none' ? [] : [secondary])]);
  const shortcutBinding = (action: ShortcutAction) => {
    const preferences = preferencesRef.current;
    return translationShortcutEnabled(preferences, action) ? preferences.shortcuts[action] : null;
  };
  const primaryPressed = (event: KeyboardEvent) => capabilities.primaryModifier === 'meta' ? event.metaKey : event.ctrlKey;
  const sourceLabel = `${languageName(source === 'auto' && detectedSource ? detectedSource : source)}${supportsLanguage(snapshot.translationEngine, 'source', source) ? '' : ` (${t('languageUnavailable')})`}`;
  const nativeChrome = useNativeChrome(native && capabilities.nativeChrome && standaloneVisible, { sourceLabel, sourceWidth }, () => {
    setPopup((current) => current === 'source' ? null : 'source');
  });

  useEffect(() => {
    if (!native || !capabilities.selection) return;
    let disposed = false;
    let latestPresentation = 0;
    const take = async () => {
      const context = await invoke<{ text: string; error: string | null; presentationId?: number } | null>('selection_take');
      if (disposed || !context) return;
      if (context.presentationId !== undefined) {
        if (context.presentationId <= latestPresentation) return;
        latestPresentation = context.presentationId;
      }
      flushSync(() => receiveSelection.current(context));
      if (context.presentationId !== undefined) await invoke('selection_present', { presentationId: context.presentationId });
    };
    const remove = listen('selection_ready', () => { void take().catch(() => setServiceError('Unable to read selected text.')); });
    void remove.then(take).catch(() => { if (!disposed) setServiceError('Unable to read selected text.'); });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()); };
  }, [native, capabilities.selection]);

  useEffect(() => {
    completedOutput.current = null;
    cancelRequest(); setCachedResults([]); setServiceError(null); setDetectedSource(null);
  }, [snapshot.translationEngine]);

  useEffect(() => { void command('translation_columns_set', { secondaryEnabled: secondary !== 'none' }); }, [secondary]);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<TranslationPreferences>('translation_preferences_changed', ({ payload }) => {
      if (!disposed) applyPreferences.current(payload);
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()); };
  }, [native]);

  useEffect(() => {
    if (standaloneVisible) input.current?.focus({ preventScroll: true });
  }, [standaloneVisible]);

  useLayoutEffect(() => {
    if (normalizedCaret.current === null) return;
    input.current?.setSelectionRange(normalizedCaret.current, normalizedCaret.current);
    normalizedCaret.current = null;
  }, [draft]);

  useEffect(() => { setPopup(null); }, [snapshot.locale, state.clipboardWidth, standaloneVisible]);
  useEffect(() => () => { sourceDrag.current.active = false; cancelAnimationFrame(sourceDrag.current.frame); }, []);
  useEffect(() => {
    const close = () => setPopup(null);
    window.addEventListener('blur', close);
    window.addEventListener('resize', close);
    window.addEventListener('nami:window-moved', close);
    return () => { window.removeEventListener('blur', close); window.removeEventListener('resize', close); window.removeEventListener('nami:window-moved', close); };
  }, []);

  useLayoutEffect(() => {
    const bar = sourceBar.current;
    const picker = bar?.querySelector<HTMLElement>('.language-picker');
    if (!capabilities.nativeHistory || !standaloneVisible || !bar || !picker) return;
    const measure = () => {
      bar.style.paddingRight = `${picker.offsetWidth + 24}px`;
      if (picker.offsetWidth > 0) setSourceWidth(Math.min(360, Math.max(32, picker.offsetWidth)));
    };
    const observer = new ResizeObserver(measure);
    observer.observe(picker);
    void document.fonts.ready.then(measure);
    measure();
    return () => observer.disconnect();
  }, [source, detectedSource, snapshot.locale, standaloneVisible, capabilities.nativeHistory]);

  function stepSourceDrag() {
    const field = input.current;
    const drag = sourceDrag.current;
    if (!drag.active || !field) return;
    const rect = field.getBoundingClientRect();
    if (!drag.taken && sourceDragDelta(drag.pointer, rect.left, rect.right) !== 0) {
      drag.taken = true;
      drag.anchor = field.selectionDirection === 'backward' ? field.selectionEnd : field.selectionStart;
    }
    if (drag.taken) {
      dragSourceSelection(field, {
        pointerX: drag.pointer, left: rect.left, right: rect.right,
        paddingLeft: Number.parseFloat(getComputedStyle(field).paddingLeft) || 0,
        anchor: drag.anchor, width: (prefix) => sourcePrefixWidth(field, prefix),
      });
    }
    drag.frame = requestAnimationFrame(stepSourceDrag);
  }

  function beginSourceDrag(event: PointerEvent<HTMLTextAreaElement>) {
    if (event.button !== 0 || event.detail > 1 || composing.current) return;
    const drag = sourceDrag.current;
    drag.active = true; drag.taken = false; drag.pointer = event.clientX;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    cancelAnimationFrame(drag.frame);
    drag.frame = requestAnimationFrame(stepSourceDrag);
  }

  function moveSourceDrag(event: PointerEvent<HTMLTextAreaElement>) {
    const drag = sourceDrag.current;
    if (!drag.active) return;
    drag.pointer = event.clientX;
    if (drag.taken) event.preventDefault();
  }

  function stopSourceDrag(field?: HTMLTextAreaElement, pointerId?: number) {
    const drag = sourceDrag.current;
    if (!drag.active) return;
    drag.active = false;
    cancelAnimationFrame(drag.frame);
    if (pointerId !== undefined) field?.releasePointerCapture?.(pointerId);
  }

  function discardRequestView() {
    ++requestRevision.current;
    partialTicket.current = null; setPartial(null); setRetrying(false); setFailedRequest(null); setPartialEngine(null);
  }

  function cancelRequest() {
    discardRequestView();
    if (native) cancellation.current = cancellation.current.then(() => invoke('translation_cancel', { scope: 'standalone' })).catch(() => {});
  }

  async function translateText(text: string, sourceLanguage: string, targetLanguages: string[], reuse?: TranslationOutput, refreshTarget?: number) {
    cancelRequest();
    const retained = targetLanguages.map((target, index) => index !== refreshTarget && reuse?.text === text
      && reuse.translation.requestedSource === sourceLanguage
      && (reuse.translation.targetEngines?.[index] ?? reuse.translation.engine) === snapshot.translationEngine
      && reuse.translation.strategyVersion === TRANSLATION_STRATEGY_VERSION
      && reuse.targetLanguages[index] === target ? reuse.translations[index] ?? '' : '');
    setPopup(null); setCachedResults(retained); setServiceError(null); setNotice(null); setDetectedSource(null);
    if (snapshot.storageError || !supportsTranslation(snapshot.translationEngine, sourceLanguage, targetLanguages)) return;
    if (!inputWithinLimits(text)) { setNotice('inputLimit'); return; }
    if (!native) { setServiceError('Open Nami to translate.'); return; }
    const revision = requestRevision.current;
    await cancellation.current;
    if (requestRevision.current !== revision) return;
    await runTranslation(
      { text, source: sourceLanguage, targets: targetLanguages },
      revision,
      { ...(refreshTarget !== undefined && reuse?.text === text ? { reuseTargetIndex: refreshTarget } : {}) },
    );
  }

  async function runTranslation(request: TranslationRequest, revision: number, options: { retryId?: string; retryEngine?: Engine; reuseTargetIndex?: number }) {
    const engine = options.retryEngine ?? (options.retryId ? partialEngine : null) ?? snapshot.translationEngine;
    activity(1);
    try {
      const result = await invoke<TranslationOutput | { partial: PartialTranslation }>('translation_run', { ...request, scope: 'standalone', ...options });
      if (requestRevision.current !== revision) return;
      const output = 'partial' in result ? result.partial : result;
      if ('partial' in result) {
        completedOutput.current = null;
        partialTicket.current = { id: result.partial.id, request, pending: false };
        setPartial(result.partial); setPartialEngine(engine);
      } else {
        completedOutput.current = result;
        partialTicket.current = null;
        setPartial(null);
      }
      setCachedResults(output.translations); setDetectedSource(output.sourceLanguage === 'auto' ? null : output.sourceLanguage);
    } catch (error) {
      if (requestRevision.current === revision && String(error) !== 'cancelled') {
        setServiceError(String(error));
        if (!options.retryId) setFailedRequest({ request, engine, reuseTargetIndex: options.reuseTargetIndex });
      }
    } finally {
      if ((options.retryId || options.retryEngine) && requestRevision.current === revision) {
        if (partialTicket.current) partialTicket.current.pending = false;
        setRetrying(false);
      }
      activity(-1);
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
      setRetrying(true); setServiceError(null);
      void runTranslation(ticket.request, ++requestRevision.current, { retryId: ticket.id, ...(engine ? { retryEngine: engine } : {}) });
    } else if (failedRequest && engine) {
      setRetrying(true); setServiceError(null); setFailedRequest(null);
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

  function submit() {
    if (!draft.trim() || composing.current || snapshot.storageError || !languagesAvailable) return;
    void translateText(draft, source, [primary, ...(secondary === 'none' ? [] : [secondary])]);
    input.current?.focus({ preventScroll: true });
  }

  receiveSelection.current = (context) => {
    completedOutput.current = null;
    cancelRequest(); normalizedCaret.current = null;
    const preferred = preferencesRef.current;
    setDraft(context.text); setSource(preferred.sourceLanguage); setDetectedSource(null); setPrimary(preferred.primaryLanguage); setSecondary(preferred.secondaryLanguage); setCachedResults([]); setNotice(null);
    sourceRef.current = preferred.sourceLanguage; primaryRef.current = preferred.primaryLanguage; secondaryRef.current = preferred.secondaryLanguage;
    setServiceError(context.error === 'permission_required' ? 'Allow Accessibility in Settings → Privacy & Data to translate selected text.' : context.error === 'selection_copy_no_change' ? t('selectionCopyDisabled') : context.error ? 'Selected text is unavailable in this application.' : null);
    const targets = [preferred.primaryLanguage, ...(preferred.secondaryLanguage === 'none' ? [] : [preferred.secondaryLanguage])];
    if (context.text.trim()) void translateText(context.text, preferred.sourceLanguage, targets);
    input.current?.focus({ preventScroll: true });
  };

  function changeLanguage(side: 'source' | 'primary' | 'secondary', value: string) {
    const current = { source, primary, secondary };
    if (current[side] === value) return;
    const next = { ...current, [side]: value };
    sourceRef.current = next.source; primaryRef.current = next.primary; secondaryRef.current = next.secondary;
    ({ source: setSource, primary: setPrimary, secondary: setSecondary })[side](value);
    void command('translation_preferences_set', side === 'source' ? { sourceLanguage: value } : side === 'primary' ? { primaryLanguage: value } : { secondaryLanguage: value });
    setDetectedSource(null);
    const targets = [next.primary, ...(next.secondary === 'none' ? [] : [next.secondary])];
    if (draft.trim()) {
      void translateText(draft, next.source, targets, side === 'source' ? undefined : completedOutput.current ?? undefined, side === 'primary' ? 0 : side === 'secondary' ? 1 : undefined);
    } else {
      cancelRequest(); setCachedResults([]); setNotice(null); setServiceError(null);
    }
  }

  function copyTranslation(requestedIndex: number) {
    const index = partial ? requestedIndex : requestedIndex === 1 && cachedResults[1] ? 1 : 0;
    if (!native || !capabilities.clipboardWrite || !cachedResults[index] || copying.current.has(index)) return;
    setPopup(null); setCopyFailed(false);
    const operation = ++copyOperation.current;
    copying.current.set(index, operation);
    void invoke('translation_copy', { scope: 'standalone', part: index ? 'secondary' : 'primary' })
      .catch(() => { if (operation === copyOperation.current) setCopyFailed(true); })
      .finally(() => { if (copying.current.get(index) === operation) copying.current.delete(index); });
  }

  function focusInput() {
    setPopup(null);
    input.current?.focus({ preventScroll: true });
  }

  function keydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.nativeEvent.isComposing || composing.current || event.keyCode === 229) return;
    if (editsText(event.nativeEvent)) return;
    if (event.key === 'Escape' && popup) {
      event.preventDefault(); setPopup(null); input.current?.focus({ preventScroll: true });
    } else if (matchesBinding(shortcutBinding('focusSearch'), event)) {
      event.preventDefault(); if (!event.repeat) focusInput();
    } else if (matchesBinding(shortcutBinding('translate'), event)) {
      event.preventDefault();
      if (!event.repeat) submit();
    } else if (matchesBinding(shortcutBinding('copyPrimary'), event)) {
      event.preventDefault(); copyTranslation(0);
    } else if (matchesBinding(shortcutBinding('copySecondary'), event)) {
      event.preventDefault(); copyTranslation(1);
    } else if (matchesBinding(shortcutBinding('pin'), event)) {
      event.preventDefault(); if (!event.nativeEvent.repeat) togglePin();
    } else if (event.key === 'Escape' || matchesBinding({ code: 'KeyW', modifiers: [capabilities.primaryModifier] }, event)) {
      event.preventDefault(); setPopup(null); void command('window_hide', { region: 'translation' });
    } else if (event.target === input.current && event.key === 'Enter' && !primaryPressed(event)) {
      event.preventDefault();
    }
  }

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
    sourceRef.current = next.source; primaryRef.current = next.primary; secondaryRef.current = next.secondary;
    setSource(next.source); setPrimary(next.primary); setSecondary(next.secondary);
    const text = draftRef.current;
    if (!text.trim()) return;
    const targets = [next.primary, ...(next.secondary === 'none' ? [] : [next.secondary])];
    const sourceChanged = next.source !== current.source;
    const primaryChanged = next.primary !== current.primary;
    const secondaryChanged = next.secondary !== current.secondary;
    if (sourceChanged || (primaryChanged && secondaryChanged)) void translateText(text, next.source, targets);
    else if (primaryChanged) void translateText(text, next.source, targets, completedOutput.current ?? undefined, 0);
    else void translateText(text, next.source, targets, completedOutput.current ?? undefined, 1);
  };

  function togglePin() {
    setPopup(null);
    void command('window_pin', { region: 'translation', pinned: !state.translationPinned });
    input.current?.focus({ preventScroll: true });
  }

  function picker(id: Popup, value: string, onChange: (value: string) => void, extra?: 'auto' | 'none') {
    return <SearchableDropdown value={value} onChange={onChange}
      label={t(id === 'source' ? 'sourceLanguage' : id === 'primary' ? 'primaryLanguage' : 'secondaryLanguage')}
      items={[...(extra ? [{ code: extra, name: extra === 'auto' ? 'Auto-Detect' : 'None' }] : []), ...providerLanguages(snapshot.translationEngine, id === 'source' ? 'source' : 'target')]} t={t}
      valueLabel={id === 'source' ? sourceLabel : value === 'none' || supportsLanguage(snapshot.translationEngine, 'target', value) ? undefined : `${languageName(value)} (${t('languageUnavailable')})`}
      description={id === 'source' && source === 'auto' && detectedSource ? `${t('detectedLanguage')}: ${languageName(detectedSource)}` : undefined} side={id === 'source' ? 'bottom' : 'top'} open={popup === id}
      hiddenTrigger={id === 'source' && nativeChrome}
      onOpenChange={(open) => setPopup(open ? id : null)} />;
  }

  const taskStatus = serviceError ?? (notice ? t(notice) : !languagesAvailable ? t('languageNotSupported') : null);
  const hasResults = !notice && cachedResults.some(Boolean);
  const shellStyle = {
    '--history-width': `${state.clipboardWidth}px`,
    '--translation-width': `${state.translationWidth}px`,
    '--history-extent': `${state.clipboardExtent}px`,
    '--translation-extent': `${state.rightExtent}px`,
    '--clipboard-detail-width': `${state.clipboardDetailWidth}px`,
    '--clipboard-translation-width': `${state.clipboardTranslationWidth}px`,
  } as CSSProperties;

  return <div className={cn('translation-shell', state.clipboardVisible && 'with-history', !rightVisible && 'history-only')} style={shellStyle}>
    {!capabilities.nativeHistory && <div className="history-clip" style={clipboardWidthChanged ? { transition: 'none' } : undefined} hidden={!state.clipboardVisible && state.clipboardExtent === 0} aria-hidden={!state.clipboardVisible} inert={!state.clipboardVisible}>
      <RecordsWorkspace t={t} snapshot={snapshot} command={command} native={native} activity={activity} />
    </div>}
    <div className="translation-clip" hidden={!capabilities.nativeHistory && !rightVisible && state.rightExtent === 0} aria-hidden={!rightVisible} inert={!rightVisible}>
      <div hidden={!clipboardRightVisible} className={cn('clipboard-right-panes', state.clipboardDetailVisible && state.clipboardTranslationVisible && 'split')}>
        <div hidden={!state.clipboardDetailVisible} className="clipboard-detail-slot">
          <ClipboardDetail t={t} snapshot={snapshot} command={command} native={native} activity={activity} />
        </div>
        {state.clipboardDetailVisible && state.clipboardTranslationVisible && <ClipboardPaneDivider t={t} width={state.clipboardDetailWidth} command={command} />}
        {state.clipboardTranslationVisible && <div className="clipboard-translate-slot">
          <ClipboardTranslatePane t={t} snapshot={snapshot} command={command} native={native} activity={activity} />
        </div>}
      </div>
      <section hidden={!standaloneVisible} className={cn('translation-panel', nativeChrome && 'native-chrome')} data-native-chrome={nativeChrome} data-tauri-drag-region="deep" aria-label={t('translation')} tabIndex={-1}
        onKeyDown={keydown}
        onKeyDownCapture={(event) => {
          if (primaryPressed(event)) { keydown(event); if (event.defaultPrevented) event.stopPropagation(); }
        }}>
        <div className="drag-strip" data-tauri-drag-region />
        <header ref={sourceBar} className="source-bar" data-detected-language={detectedSource ?? undefined}>
          <textarea ref={input} className="source-input" aria-label={t('sourceLabel')} rows={1} wrap="off" spellCheck={false} value={displaySource(draft)}
            onChange={(event) => {
              completedOutput.current = null;
              cancelRequest(); setServiceError(null); setDetectedSource(null);
              const next = event.target.value;
              // Like FinderX, leave ordinary typing and IME selection to the editor.
              normalizedCaret.current = displaySource(next) === next ? null : displaySource(next.slice(0, event.target.selectionStart)).length;
              setDraft(editSource(draft, next)); setNotice(null); setCachedResults([]);
            }}
            onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }}
            onPointerDown={beginSourceDrag} onPointerMove={moveSourceDrag} onPointerUp={(event) => stopSourceDrag(event.currentTarget, event.pointerId)} onPointerCancel={(event) => stopSourceDrag(event.currentTarget, event.pointerId)} onBlur={() => stopSourceDrag()} />
          {picker('source', source, (value) => changeLanguage('source', value), 'auto')}
        </header>
        <div className={cn('result-columns', secondary === 'none' && 'single-column', taskStatus && !hasResults && 'task-status-only')}>
          {[primary, ...(secondary === 'none' ? [] : [secondary])].map((_, index) => <section className="result-column" tabIndex={-1} key={index} data-tauri-drag-region="deep" aria-label={index ? t('secondaryLanguage') : t('primaryLanguage')}>
            {!notice && cachedResults[index] && <span data-tauri-drag-region="false">{cachedResults[index]}</span>}
            {partial?.errors[index] && <div className="result-error" data-tauri-drag-region="false">
              <span role="alert">{partial.errors[index]}</span>
              {languagePackAction(partial.errors[index])}
              <button type="button" disabled={retrying || !!snapshot.storageError} onClick={() => retryTranslation()}>{t(retrying ? 'translationRetrying' : 'retry')}</button>
              {retryElsewhere}
            </div>}
          </section>)}
          {taskStatus && !hasResults && <div className="translation-status" data-tauri-drag-region="false"><span role={serviceError ? 'alert' : 'status'}>{taskStatus}</span>{languagePackAction(serviceError)}{retryAfterDownload}{serviceError && retryElsewhere}</div>}
          <RecordFeedback message={copyFailed ? t('copyFailed') : null} />
        </div>
        {taskStatus && hasResults && <div className="translation-status with-results" data-tauri-drag-region="false"><span role={serviceError ? 'alert' : 'status'}>{taskStatus}</span>{languagePackAction(serviceError)}{retryAfterDownload}{serviceError && !partial && retryElsewhere}</div>}
        <footer className={cn('translation-footer', secondary === 'none' && 'single-column')}>
          <div className="language-slot">{picker('primary', primary, (value) => changeLanguage('primary', value))}</div>
          <div className={cn('language-slot', secondary === 'none' && 'secondary-off')}>
            {picker('secondary', secondary, (value) => changeLanguage('secondary', value), 'none')}
          </div>
        </footer>
      </section>
    </div>
    {!capabilities.nativeHistory && state.clipboardVisible && clipboardRightVisible && <HistoryDivider t={t} width={state.clipboardWidth} command={command}
      onStart={() => setPopup(null)} />}
  </div>;
}
