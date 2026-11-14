import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ViewProps } from '../App';
import { SearchableDropdown } from '../shared/SearchableDropdown';
import { languages } from '../shared/languages';
import { FloatingPanel } from '../shared/FloatingPanel';
import { inputWithinLimits } from '../shared/input';
import { displaySource, editSource } from '../shared/source';
import { cn } from '../lib/utils';
import { HistoryPanel } from './HistoryPanel';
import { HistoryIcon } from './HistoryIcon';
import { historySamples, type HistoryRecord, type HistorySelection } from './history';
import { HistoryDivider } from './HistoryDivider';
import { useNativeChrome } from './useNativeChrome';

type Popup = 'source' | 'primary' | 'secondary' | 'actions';

function Keycaps({ keys }: { keys: string[] }) {
  return <span className="keycaps" aria-hidden="true">{keys.map((key) => <kbd key={key}>{key}</kbd>)}</span>;
}

export function Translation({ t, snapshot, command, native, activity }: ViewProps) {
  const [draft, setDraft] = useState('');
  const [source, setSource] = useState('auto');
  const [primary, setPrimary] = useState('en');
  const [secondary, setSecondary] = useState('zh-CN');
  const [notice, setNotice] = useState<'inputLimit' | null>(null);
  const [serviceError, setServiceError] = useState<string | null>(null);
  const requestRevision = useRef(0);
  const completedRecord = useRef<HistoryRecord | null>(null);
  const cancellation = useRef<Promise<unknown>>(Promise.resolve());
  const fromSelection = useRef(false);
  const receiveSelection = useRef<(context: { text: string; error: string | null }) => void>(() => {});
  const [popup, setPopup] = useState<Popup | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [cachedResults, setCachedResults] = useState<string[]>([]);
  const [actionsFit, setActionsFit] = useState(true);
  const [sourceWidth, setSourceWidth] = useState(140);
  const footer = useRef<HTMLElement>(null);
  const sourceBar = useRef<HTMLElement>(null);
  const actionGroup = useRef<HTMLDivElement>(null);
  const actionButton = useRef<HTMLButtonElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const composing = useRef(false);
  const normalizedCaret = useRef<number | null>(null);
  const select = useRef<(record: HistoryRecord, retranslate: boolean) => void>(() => {});
  const state = snapshot.window;
  const committedHistoryWidth = useRef(state.historyWidth);
  const historyWidthChanged = committedHistoryWidth.current !== state.historyWidth;
  useLayoutEffect(() => { committedHistoryWidth.current = state.historyWidth; }, [state.historyWidth]);
  const capabilities = snapshot.capabilities;
  const primaryKey = capabilities.primaryModifier === 'meta' ? '⌘' : 'Ctrl';
  const primaryPressed = (event: KeyboardEvent) => capabilities.primaryModifier === 'meta' ? event.metaKey : event.ctrlKey;
  const nativeChrome = useNativeChrome(native && capabilities.nativeChrome, {
    sourceLabel: source === 'auto' ? 'Auto-Detect' : languages.find(([code]) => code === source)?.[1] ?? source,
    sourceWidth,
    translateLabel: t('translation'), actionsLabel: t('actions'),
    canTranslate: Boolean(draft.trim()) && !snapshot.storageError, actionsVisible: actionsFit,
  }, (action) => {
    if (action === 'translate') submit();
    else setPopup((current) => current === action ? null : action);
  });

  useEffect(() => {
    let disposed = false;
    void document.fonts.ready.then(() => {
      if (disposed) return;
      const context = document.createElement('canvas').getContext('2d');
      if (!context) return;
      context.font = '10px Inter';
      const longest = languages.map(([, name]) => name).reduce((a, b) => context.measureText(a).width > context.measureText(b).width ? a : b);
      const title = context.measureText(`${longest} - ${longest}, ${longest}`).width;
      context.font = '11px Inter';
      void command('history_width_set', { width: Math.ceil(title + context.measureText('88-88 88:88').width + 65), initialize: true });
    });
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<HistorySelection>('history_sample_selected', ({ payload }) => {
      if (disposed) return;
      if (payload.sampleId === null) { setSelectedId(null); return; }
      const record = payload.record ?? historySamples.find(({ id }) => id === payload.sampleId);
      if (record) select.current(record, payload.retranslate);
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()); };
  }, [native]);

  useEffect(() => {
    if (!native || !capabilities.selection) return;
    let disposed = false;
    const take = async () => {
      const context = await invoke<{ text: string; error: string | null } | null>('selection_take');
      if (!disposed && context) receiveSelection.current(context);
    };
    const remove = listen('selection_ready', () => { void take().catch(() => setServiceError('Unable to read selected text.')); });
    void remove.then(take).catch(() => { if (!disposed) setServiceError('Unable to read selected text.'); });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()); };
  }, [native, capabilities.selection]);

  useEffect(() => {
    completedRecord.current = null;
    cancelRequest(); setCachedResults([]); setServiceError(null);
  }, [snapshot.translationEngine]);

  useEffect(() => { void command('translation_columns_set', { secondaryEnabled: secondary !== 'none' }); }, [secondary]);

  useEffect(() => {
    if (state.translationVisible) input.current?.focus({ preventScroll: true });
  }, [state.translationVisible]);

  useLayoutEffect(() => {
    if (normalizedCaret.current === null) return;
    input.current?.setSelectionRange(normalizedCaret.current, normalizedCaret.current);
    normalizedCaret.current = null;
  }, [draft]);

  useEffect(() => { setPopup(null); }, [snapshot.locale, state.historyWidth]);
  useEffect(() => {
    const close = () => setPopup(null);
    window.addEventListener('blur', close);
    window.addEventListener('resize', close);
    window.addEventListener('nami:window-moved', close);
    return () => { window.removeEventListener('blur', close); window.removeEventListener('resize', close); window.removeEventListener('nami:window-moved', close); };
  }, []);

  useLayoutEffect(() => {
    const element = footer.current;
    if (!element) return;
    const measure = () => {
      const picker = sourceBar.current?.querySelector<HTMLElement>('.language-picker');
      if (capabilities.nativeHistory && picker && sourceBar.current) {
        sourceBar.current.style.paddingRight = `${picker.offsetWidth + 24}px`;
        if (picker.offsetWidth > 0) setSourceWidth(Math.min(200, Math.max(32, picker.offsetWidth)));
      }
      const last = [...element.querySelectorAll('.language-slot:not(.secondary-off) .language-picker')].at(-1)?.getBoundingClientRect();
      const actions = actionGroup.current?.getBoundingClientRect();
      setActionsFit(Boolean(last && actions && last.right + 12 <= actions.left));
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    element.querySelectorAll('button').forEach((button) => observer.observe(button));
    const sourcePicker = sourceBar.current?.querySelector('button.language-picker');
    if (sourcePicker) observer.observe(sourcePicker);
    void document.fonts.ready.then(measure);
    measure();
    return () => observer.disconnect();
  }, [source, secondary, primary, snapshot.locale, state.translationVisible, capabilities.nativeHistory]);

  function clearSelection() {
    if (native && selectedId !== null) void command('history_sample_select', { sampleId: null, retranslate: false });
    setSelectedId(null);
  }

  function cancelRequest() {
    ++requestRevision.current;
    if (native) cancellation.current = cancellation.current.then(() => invoke('translation_cancel')).catch(() => {});
  }

  async function translateText(text: string, sourceLanguage: string, targetLanguages: string[], captured: boolean, reuse?: HistoryRecord, refreshTarget?: number) {
    if (snapshot.storageError) return;
    const origin = reuse ?? completedRecord.current;
    cancelRequest();
    const retained = targetLanguages.map((target, index) => index !== refreshTarget && reuse?.text === text && reuse.sourceLanguage === sourceLanguage && reuse.targetLanguages[index] === target ? reuse.translations[index] ?? '' : '');
    setPopup(null); setCachedResults(retained); setServiceError(null); setNotice(null);
    if (!inputWithinLimits(text)) { setNotice('inputLimit'); return; }
    if (!native) { setServiceError('Open Nami to translate.'); return; }
    const revision = requestRevision.current;
    await cancellation.current;
    if (requestRevision.current !== revision) return;
    activity(1);
    try {
      const record = await invoke<HistoryRecord>('translation_run', { text, source: sourceLanguage, targets: targetLanguages, fromSelection: captured, ...(origin?.text === text ? { reuse: { recordId: origin.id, ...(refreshTarget !== undefined ? { targetIndex: refreshTarget } : {}) } } : {}) });
      if (requestRevision.current !== revision) return;
      completedRecord.current = record;
      setCachedResults(record.translations); setSelectedId(record.id);
    } catch (error) {
      if (requestRevision.current === revision && String(error) !== 'cancelled') setServiceError(String(error));
    } finally { activity(-1); }
  }

  function submit() {
    if (!draft.trim() || composing.current || snapshot.storageError) return;
    clearSelection();
    void translateText(draft, source, [primary, ...(secondary === 'none' ? [] : [secondary])], fromSelection.current);
    input.current?.focus({ preventScroll: true });
  }

  receiveSelection.current = (context) => {
    completedRecord.current = null;
    cancelRequest(); normalizedCaret.current = null;
    setDraft(context.text); setSource('auto'); setCachedResults([]); setNotice(null); clearSelection();
    fromSelection.current = true;
    setServiceError(context.error === 'permission_required' ? 'Allow Accessibility in Settings → Privacy & Data to translate selected text.' : context.error?.startsWith('clipboard_') ? 'Unable to safely capture the selection and restore the clipboard. Try again or paste text manually.' : context.error ? 'Selected text is unavailable in this application.' : null);
    if (context.text.trim()) void translateText(context.text, 'auto', [primary, ...(secondary === 'none' ? [] : [secondary])], true);
    input.current?.focus({ preventScroll: true });
  };

  function selectHistory(record: HistoryRecord, retranslate = false) {
    setPopup(null);
    if (!native && !retranslate && selectedId === record.id && state.translationVisible && !state.translationPinned) {
      void command('window_hide', { region: 'translation', keepHistory: true });
      return;
    }
    cancelRequest(); fromSelection.current = false; setServiceError(null);
    normalizedCaret.current = null;
    setSelectedId(record.id); setDraft(record.text);
    setSource(record.sourceLanguage); setPrimary(record.targetLanguages[0]);
    setSecondary(record.targetLanguages[1] ?? 'none');
    completedRecord.current = record;
    setCachedResults(retranslate ? [] : record.translations);
    setNotice(null);
    if (retranslate) void translateText(record.text, record.sourceLanguage, record.targetLanguages, false);
    if (!native) void command('window_show', { target: 'translation' });
    input.current?.focus({ preventScroll: true });
  }

  select.current = selectHistory;

  function changeLanguage(side: 'source' | 'primary' | 'secondary', value: string) {
    const current = { source, primary, secondary };
    if (current[side] === value) return;
    const next = { ...current, [side]: value };
    ({ source: setSource, primary: setPrimary, secondary: setSecondary })[side](value);
    clearSelection();
    const targets = [next.primary, ...(next.secondary === 'none' ? [] : [next.secondary])];
    if (draft.trim()) {
      void translateText(draft, next.source, targets, fromSelection.current, side === 'source' ? undefined : completedRecord.current ?? undefined, side === 'primary' ? 0 : side === 'secondary' ? 1 : undefined);
    } else {
      cancelRequest(); setCachedResults([]); setNotice(null); setServiceError(null);
    }
  }

  function copyTranslation(index: number) {
    if (!native || !capabilities.clipboardWrite || !selectedId || !cachedResults[index]) return;
    setPopup(null);
    void command('translation_sample_copy', { sampleId: selectedId, part: index ? 'secondary' : 'primary' });
  }

  function keydown(event: KeyboardEvent, region: 'translation' | 'history') {
    if (event.defaultPrevented || event.nativeEvent.isComposing || composing.current || event.keyCode === 229) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      if (popup) { setPopup(null); input.current?.focus({ preventScroll: true }); }
      else void command('window_hide', { region: 'translation' });
    } else if (primaryPressed(event) && event.key.toLowerCase() === 'w') {
      event.preventDefault(); setPopup(null); void command('window_hide', { region: 'translation' });
    } else if (region === 'translation' && primaryPressed(event) && event.key === 'Enter') {
      event.preventDefault(); submit();
    } else if (region === 'translation' && primaryPressed(event) && (event.key === '1' || event.key === '2')) {
      event.preventDefault(); copyTranslation(Number(event.key) - 1);
    } else if (region === 'translation' && primaryPressed(event) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      if (popup === 'actions') { setPopup(null); input.current?.focus({ preventScroll: true }); }
      else setPopup('actions');
    } else if (region === 'translation' && primaryPressed(event) && event.shiftKey && event.key.toLowerCase() === 'p') {
      event.preventDefault(); togglePin();
    }
  }

  function togglePin() {
    setPopup(null);
    void command('window_pin', { region: 'translation', pinned: !state.translationPinned });
    input.current?.focus({ preventScroll: true });
  }

  function picker(id: Exclude<Popup, 'actions'>, value: string, onChange: (value: string) => void, extra?: 'auto' | 'none') {
    return <SearchableDropdown value={value} onChange={onChange}
      label={t(id === 'source' ? 'sourceLanguage' : id === 'primary' ? 'primaryLanguage' : 'secondaryLanguage')}
      items={[...(extra ? [{ code: extra, name: extra === 'auto' ? 'Auto-Detect' : 'None' }] : []), ...languages.map(([code, name]) => ({ code, name }))]} t={t} side={id === 'source' ? 'bottom' : 'top'} open={popup === id}
      hiddenTrigger={id === 'source' && nativeChrome}
      onOpenChange={(open) => setPopup(open ? id : null)} />;
  }

  const shellStyle = {
    '--history-width': `${state.historyWidth}px`,
    '--translation-width': `${state.translationWidth}px`,
    '--history-extent': `${state.historyExtent}px`,
    '--translation-extent': `${state.translationExtent}px`,
  } as CSSProperties;

  return <div className={cn('translation-shell', state.historyVisible && 'with-history', !state.translationVisible && 'history-only')} style={shellStyle}>
    {!capabilities.nativeHistory && <div className="history-clip" style={historyWidthChanged ? { transition: 'none' } : undefined} hidden={!state.historyVisible && state.historyExtent === 0} aria-hidden={!state.historyVisible} inert={!state.historyVisible}>
    <HistoryPanel t={t} snapshot={snapshot} command={command} native={native} selectedId={selectedId} onSelect={native ? (record, retranslate = false) => { void command('history_sample_select', { sampleId: record.id, retranslate }); } : selectHistory} onKeyDown={(event) => keydown(event, 'history')} />
    </div>}
    <div className="translation-clip" hidden={!capabilities.nativeHistory && !state.translationVisible && state.translationExtent === 0} aria-hidden={!state.translationVisible} inert={!state.translationVisible}>
    <section className={cn('translation-panel', nativeChrome && 'native-chrome')} data-native-chrome={nativeChrome} data-tauri-drag-region="deep" aria-label={t('translation')}
      onKeyDown={(event) => keydown(event, 'translation')}
      onKeyDownCapture={(event) => {
        if (primaryPressed(event)) { keydown(event, 'translation'); if (event.defaultPrevented) event.stopPropagation(); }
      }}>
      <div className="drag-strip" data-tauri-drag-region />
      <header ref={sourceBar} className="source-bar">
        <button className="icon-button history-trigger" title={t('history')} aria-label={t('history')} aria-expanded={state.historyVisible}
          aria-controls={capabilities.nativeHistory ? undefined : 'translation-history'} aria-pressed={state.historyVisible} onMouseDown={(event) => event.preventDefault()}
          onClick={() => { setPopup(null); void command('history_toggle'); }}><HistoryIcon name="clock" /></button>
        <textarea ref={input} className="source-input" aria-label={t('sourceLabel')} rows={1} wrap="off" spellCheck={false} value={displaySource(draft)}
          onChange={(event) => {
            completedRecord.current = null;
            cancelRequest(); fromSelection.current = false; setServiceError(null);
            const next = event.target.value;
            // Like FinderX, leave ordinary typing and IME selection to the editor.
            normalizedCaret.current = displaySource(next) === next ? null : displaySource(next.slice(0, event.target.selectionStart)).length;
            setDraft(editSource(draft, next)); setNotice(null); clearSelection(); setCachedResults([]);
          }}
          onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }}
          onKeyDown={(event) => { if (event.key === 'Enter' && !event.nativeEvent.isComposing && !composing.current && !primaryPressed(event)) event.preventDefault(); }} />
        {picker('source', source, (value) => changeLanguage('source', value), 'auto')}
      </header>
      <div className={cn('result-columns', secondary === 'none' && 'single-column')}>
        {[primary, ...(secondary === 'none' ? [] : [secondary])].map((_, index) => <section className="result-column" key={index} data-tauri-drag-region="deep" aria-label={index ? t('secondaryLanguage') : t('primaryLanguage')}>
          {!notice && cachedResults[index] && <span data-tauri-drag-region="false">{cachedResults[index]}</span>}
          {serviceError && !cachedResults[index] && <p className="translation-status" data-tauri-drag-region="false" role="alert">{serviceError}</p>}
          {notice && <p className="translation-status" data-tauri-drag-region="false" role="status">{t(notice)}</p>}
        </section>)}
      </div>
      <footer ref={footer} className={cn('translation-footer', secondary === 'none' && 'single-column')}>
        <div className="language-slot">{picker('primary', primary, (value) => changeLanguage('primary', value))}</div>
        <div className={cn('language-slot', secondary === 'none' && 'secondary-off')}>
          {picker('secondary', secondary, (value) => changeLanguage('secondary', value), 'none')}
        </div>
        <div ref={actionGroup} className={cn('translation-actions', !actionsFit && 'actions-overflow')} aria-hidden={nativeChrome || undefined} inert={nativeChrome}>
          <button className="translation-action" disabled={!draft.trim() || Boolean(snapshot.storageError)} onMouseDown={(event) => event.preventDefault()} onClick={submit}>{t('translation')}<Keycaps keys={[primaryKey, '↵']} /></button>
          <button ref={actionButton} className="translation-action" aria-haspopup="menu" aria-expanded={popup === 'actions'} onClick={() => setPopup((value) => value === 'actions' ? null : 'actions')}>{t('actions')}<Keycaps keys={[primaryKey, 'K']} /></button>
        </div>
      </footer>
      {popup === 'actions' && <FloatingPanel anchor={actionButton} onClose={() => setPopup(null)} label={t('actions')} role="menu" width={310} side="top" className="action-menu">
        <div role="group" onKeyDown={(event) => {
          if (event.nativeEvent.isComposing || primaryPressed(event)) return;
          const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
          const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault(); buttons[(index + buttons.length + (event.key === 'ArrowDown' ? 1 : -1)) % buttons.length]?.focus();
          } else if (event.key === 'Home' || event.key === 'End') {
            event.preventDefault(); buttons[event.key === 'Home' ? 0 : buttons.length - 1]?.focus();
          }
        }}>
          <button role="menuitem" disabled={!native || !capabilities.clipboardWrite || !cachedResults[0]} onClick={() => copyTranslation(0)}>{t('copyPrimary')}<Keycaps keys={[primaryKey, '1']} /></button>
          {secondary !== 'none' && <button role="menuitem" disabled={!native || !capabilities.clipboardWrite || !cachedResults[1]} onClick={() => copyTranslation(1)}>{t('copySecondary')}<Keycaps keys={[primaryKey, '2']} /></button>}
          {secondary === 'none' && <button role="menuitem" onClick={() => { changeLanguage('secondary', 'zh-CN'); setPopup(null); input.current?.focus(); }}>Enable Second Language</button>}
          <button role="menuitemcheckbox" aria-checked={state.translationPinned} onClick={togglePin}>{state.translationPinned ? 'Unpin Window' : 'Pin Window'}<Keycaps keys={[primaryKey, '⇧', 'P']} /></button>
        </div>
      </FloatingPanel>}
    </section>
    </div>
    {!capabilities.nativeHistory && state.historyVisible && state.translationVisible && <HistoryDivider t={t} width={state.historyWidth} command={command}
      onStart={() => setPopup(null)} onEscape={() => input.current?.focus()} />}
  </div>;
}
