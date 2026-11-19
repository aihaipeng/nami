import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import type { ClipboardResult } from '../shared/contracts';
import type { MessageKey } from '../shared/i18n';
import { ActivityBeam } from '../shared/ActivityBeam';
import { RecordToolbar } from '../shared/RecordToolbar';
import { useRecordBrowser } from '../shared/useRecordBrowser';
import { historyPreview, historyScrollTarget } from '../translation/history';
import { cn } from '../lib/utils';
import { RecordHeading } from '../shared/RecordHeading';
import { RecordActions } from '../shared/RecordActions';
import { RecordFeedback } from '../shared/RecordFeedback';
import { useClipboard, type ClipboardKind, type ClipboardRecord } from './useClipboard';
import { useClipboardImageText } from './useClipboardImageText';
import { HistoryList } from '../shared/HistoryList';
import { HistoryFooter } from '../shared/HistoryFooter';
import { ClipboardMedia } from './ClipboardMedia';
import { ClipboardNotePreview } from './ClipboardNotePreview';
import { ClipboardEditor } from './ClipboardEditor';
import { ClipboardMenu } from './ClipboardMenu';
import { clipboardUseError, clipboardUseFeedback } from './feedback';
import './clipboard.css';

export function Clipboard({ t, snapshot, command, native, active = true }: ViewProps & { active?: boolean }) {
  const [session, setSession] = useState(0);
  const [kindFilter, setKindFilter] = useState<ClipboardKind | null>(null);
  const [processing, setProcessing] = useState<string | null>(null);
  const [processingError, setProcessingError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<MessageKey | null>(null);
  const operation = useRef(0);
  const processingRevision = useRef(0);
  const reveal = useRef<string | null>(null);
  const revealContext = useRef({ browsing: true });
  const mounted = useRef(true);
  const rowRefs = useRef(new Map<string, HTMLButtonElement>());
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const browser = useRecordBrowser(snapshot.window.recordsSession, active, list, () => clipboard.displayReady, () => clipboard.page.records, kindFilter);
  const { query, setQuery, favoritesOnly, setFavoritesOnly, remember, restoreScroll } = browser;
  const clipboard = useClipboard(native, query, favoritesOnly, () => (list.current?.scrollTop ?? 0) > 0, kindFilter);
  const records = clipboard.page.records;
  const imageText = useClipboardImageText(records, active && native && snapshot.capabilities.imageOcr);
  const menuRecord = menu ? records.find((record) => record.id === menu.id) : null;
  const canTranslateRecord = (record: ClipboardRecord) => record.kind === 'text' ? !!record.text.trim() : snapshot.capabilities.imageOcr && imageText.get(record.id) === true;
  const canOcrRecord = (record: ClipboardRecord) => record.kind !== 'text' && canTranslateRecord(record) && snapshot.capabilities.clipboardWrite;
  const selection = snapshot.window.translationVisible ? snapshot.window.clipboardDetailId : selected;
  const visibleIds = new Set(records.map(({ id }) => id));
  const ordered = [...records.filter((record) => record.isPinned), ...records.filter((record) => !record.isPinned)];
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let remove: (() => void) | undefined;
    void listen<{ label: string; visible: boolean }>('window://visibility', ({ payload }) => {
      if (disposed || payload.label !== 'clipboard' || snapshot.window.recordsSession !== undefined) return;
      ++operation.current;
      ++processingRevision.current;
      setEditing(null); setMenu(null); setRevealed(null); setSelected(null); setFeedback(null);
      setQuery(''); setFavoritesOnly(false); setKindFilter(null); setProcessing(null); setProcessingError(null); setSession((value) => value + 1);
      list.current?.scrollTo(0, 0); pinnedList.current?.scrollTo(0, 0);
      if (!payload.visible) (document.activeElement as HTMLElement | null)?.blur();
    }).then((unlisten) => { if (disposed) unlisten(); else remove = unlisten; }).catch(() => { if (!disposed) setFeedback('nativeError'); });
    return () => { disposed = true; remove?.(); };
  }, [native, snapshot.window.recordsSession]);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const subscription = listen<ClipboardResult>('clipboard_use_failed', ({ payload }) => {
      if (disposed) return;
      setFeedback(payload.code === 'clipboard_file_missing' ? 'clipboardFileUnavailable' : payload.code === 'clipboard_image_missing' ? 'clipboardImageUnavailable' : payload.copyStatus === 'superseded' ? 'clipboardChanged' : payload.copyStatus === 'current' ? null : 'copyFailed');
    });
    return () => { disposed = true; void subscription.then((remove) => remove()); };
  }, [native]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++operation.current; }; }, []);
  useEffect(() => { setMenu(null); setRevealed(null); }, [query, favoritesOnly, kindFilter]);
  useEffect(() => {
    const dismiss = () => setMenu(null);
    window.addEventListener('scroll', dismiss, true);
    return () => window.removeEventListener('scroll', dismiss, true);
  }, []);
  useEffect(() => {
    ++operation.current;
    ++processingRevision.current;
    setKindFilter(null); setProcessing(null); setProcessingError(null);
    setEditing(null); setMenu(null); setRevealed(null); setSelected(null); setFeedback(null); setSession((value) => value + 1);
  }, [snapshot.window.recordsSession]);
  useEffect(() => { if (!active) { ++operation.current; ++processingRevision.current; setEditing(null); setMenu(null); setRevealed(null); setFeedback(null); setProcessing(null); setProcessingError(null); setSession((value) => value + 1); } }, [active]);
  useEffect(() => {
    if (active && snapshot.window.recordsFocus !== undefined) search.current?.focus({ preventScroll: true });
  }, [active, snapshot.window.recordsFocus]);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<string>('clipboard_note_requested', ({ payload }) => {
      if (!disposed) { setMenu(null); setEditing(payload); }
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native]);

  revealContext.current = { browsing: snapshot.window.recordsBrowsing !== false };
  function previewRecord(id: string, explicit = false) {
    if (!explicit && snapshot.window.recordsBrowsing === false) return;
    if (!snapshot.window.translationVisible || !explicit && snapshot.window.clipboardDetailId === id || editing) return;
    void command('clipboard_details_select', { recordId: id, ...(explicit ? { browsing: true } : {}) });
  }
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<string>('clipboard_text_created', ({ payload }) => {
      if (disposed || !payload || revealContext.current.browsing) return;
      reveal.current = payload;
      setSelected(payload);
      setKindFilter(null); setFavoritesOnly(false); setQuery('');
      list.current?.scrollTo(0, 0);
      clipboard.resetFilters({ query: '', favoritesOnly: false, kind: null });
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native, clipboard.resetFilters, setFavoritesOnly, setQuery]);
  useEffect(() => {
    const id = reveal.current;
    if (snapshot.window.recordsBrowsing) { reveal.current = null; return; }
    if (!active || !id || query || favoritesOnly || kindFilter || clipboard.loading || !clipboard.displayReady) return;
    if (!records.some((record) => record.id === id)) { reveal.current = null; return; }
    reveal.current = null;
    setSelected(id);
    const element = rowRefs.current.get(id)?.closest<HTMLElement>('.history-row');
    const scroller = element?.closest<HTMLElement>('.history-list');
    if (!element || !scroller) return;
    const bounds = scroller.getBoundingClientRect();
    const top = historyScrollTarget(scroller.scrollTop, { top: bounds.top, bottom: bounds.bottom }, element.getBoundingClientRect());
    if (top !== null) scroller.scrollTo({ top, behavior: 'instant' });
  }, [active, records, query, favoritesOnly, kindFilter, clipboard.loading, clipboard.displayReady, snapshot.window.translationVisible, snapshot.window.clipboardDetailId, snapshot.window.recordsBrowsing, command]);
  useEffect(() => {
    if (!active || !snapshot.window.translationVisible || clipboard.loading || !clipboard.displayReady || reveal.current) return;
    if (snapshot.window.clipboardDetailId && records.some(({ id }) => id === snapshot.window.clipboardDetailId)) return;
    const first = records.find((record) => !record.isPinned) ?? records[0];
    if ((first?.id ?? null) !== (snapshot.window.clipboardDetailId ?? null)) void command('clipboard_details_select', { recordId: first?.id ?? null });
  }, [active, snapshot.window.translationVisible, snapshot.window.clipboardDetailId, records, clipboard.loading]);
  async function toggleDetails() {
    if (!snapshot.window.translationVisible) {
      const first = records.find((record) => !record.isPinned) ?? records[0];
      await command('clipboard_details_select', { recordId: first?.id ?? null, browsing: true });
    }
    await command('records_details_toggle');
  }

  async function useRecord(recordId: string, action: 'copy' | 'paste' | 'activate') {
    if (!native || (action === 'paste' ? !snapshot.capabilities.autoPaste : !snapshot.capabilities.clipboardWrite)) return false;
    const current = ++operation.current;
    setSelected(recordId); setFeedback(null);
    try {
      const result = await invoke<ClipboardResult>('clipboard_record_use', { recordId, action });
      if (!mounted.current) return false;
      // Closing the panel bumps the operation generation before this returns. A row click writes the clipboard too.
      if (result.copyStatus === 'current') clipboard.noteCopied(recordId);
      if (operation.current !== current) return false;
      setFeedback(clipboardUseFeedback(result));
      return result.copyStatus === 'current' && result.status === 'copied';
    } catch (error) {
      if (mounted.current && operation.current === current) setFeedback(clipboardUseError(error, action === 'activate' ? 'paste' : action));
    }
    return false;
  }

  async function processRecord(recordId: string, action: 'translate' | 'ocr') {
    const current = ++operation.current;
    const revision = ++processingRevision.current;
    setProcessing(recordId); setProcessingError(null); setFeedback(null);
    try {
      await invoke('clipboard_record_process', { recordId, action });
    } catch (error) {
      if (mounted.current && operation.current === current && String(error) !== 'cancelled') {
        setProcessingError(action === 'ocr' ? 'Unable to extract and copy text. Try again.' : 'Unable to open this text for translation. Try again.');
      }
    } finally {
      if (mounted.current && processingRevision.current === revision) setProcessing(null);
    }
  }

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (!active || event.defaultPrevented || event.isComposing || event.keyCode === 229 || (editing && event.key === 'Escape')) return;
      if (event.key === 'Escape' || ((snapshot.capabilities.primaryModifier === 'meta' ? event.metaKey : event.ctrlKey) && event.key.toLowerCase() === 'w')) {
        event.preventDefault(); ++operation.current;
        void command('window_hide', { region: 'clipboard' });
      }
    };
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [snapshot.capabilities.primaryModifier, command, editing, active]);
  function focusRow(id: string) { setSelected(id); rowRefs.current.get(id)?.focus(); }
  function loadNearEnd(element: HTMLDivElement) {
    if (element.clientHeight > 0 && element.scrollHeight - element.scrollTop - element.clientHeight < 180) void clipboard.loadMore();
  }
  function row(record: ClipboardRecord) {
    const canDelete = !record.isFavorite && !record.isPinned;
    const canTranslate = canTranslateRecord(record);
    const canOcr = canOcrRecord(record);
    const kind = record.kind === 'image' ? t('clipboardImage') : record.kind === 'files' ? t('clipboardFiles') : t('text');
    const index = ordered.findIndex(({ id }) => id === record.id);
    return <article data-note-revealed={record.title ? revealed === record.id : undefined} onMouseEnter={() => { if (record.title) setRevealed(record.id); }} onMouseMove={() => previewRecord(record.id)} onMouseLeave={() => setRevealed((current) => current === record.id ? null : current)} onContextMenu={(event) => { event.preventDefault(); setMenu({ id: record.id, x: event.clientX, y: event.clientY }); }} key={record.id} hidden={!visibleIds.has(record.id)} data-record-id={record.id} className={cn('history-row', selection === record.id && 'selected', menu?.id === record.id && 'context-target', record.isPinned && 'pinned')} style={{ '--history-actions-width': `${(canDelete ? 124 : 99) + 25 * (Number(canTranslate) + Number(canOcr))}px` } as CSSProperties}>
      <button type="button" ref={(element) => { if (element) rowRefs.current.set(record.id, element); else rowRefs.current.delete(record.id); }} className="history-row-select" aria-label={`${t('clipboard')}: ${record.text || kind}`} aria-pressed={selection === record.id} onFocus={() => { setSelected(record.id); previewRecord(record.id); }} onClick={() => { previewRecord(record.id, true); void useRecord(record.id, 'activate'); }} onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'ArrowDown' && ordered[index + 1]) { event.preventDefault(); focusRow(ordered[index + 1].id); previewRecord(ordered[index + 1].id, true); }
        else if (event.key === 'ArrowUp') { event.preventDefault(); if (ordered[index - 1]) { focusRow(ordered[index - 1].id); previewRecord(ordered[index - 1].id, true); } else search.current?.focus(); }
      }}>
        <RecordHeading date={record.date} label={kind} sourceApp={record.sourceApp} />
        {record.title ? <ClipboardNotePreview record={record} revealed={revealed === record.id} native={native} media={record.kind !== 'text' ? <ClipboardMedia record={record} native={native} /> : undefined} /> : record.kind !== 'text' ? <ClipboardMedia record={record} native={native} /> : <span className="history-preview">{historyPreview(record.text)}</span>}
      </button>
      <RecordActions recordId={record.id} t={t} native={native} favorite={record.isFavorite} pinned={record.isPinned} canCopy={snapshot.capabilities.clipboardWrite} copyLabel={t('copy')} deleteLabel={t('deleteClipboard')}
        onNote={() => setEditing(record.id)} busy={processing !== null} onTranslate={canTranslate ? () => { void processRecord(record.id, 'translate'); } : undefined} onOcr={canOcr ? () => { void processRecord(record.id, 'ocr'); } : undefined}
        onCopy={() => useRecord(record.id, 'copy')} onUpdate={(action) => { void clipboard.update(record.id, action); }} />
    </article>;
  }
  return <section inert={editing !== null} className="history-panel clipboard-panel" aria-label={t('clipboard')}>
    <ActivityBeam label={t('working')} active={active && clipboard.showActivity} />
    <div className="drag-strip" data-tauri-drag-region />
    <RecordToolbar t={t} snapshot={snapshot} command={command} view="clipboard" query={query} onQuery={setQuery} search={search} onDetailsToggle={() => { void toggleDetails(); }} onArrowDown={() => { if (ordered[0]) { focusRow(ordered[0].id); previewRecord(ordered[0].id, true); } }} />
    {clipboard.error && <p role="alert" className="history-error">{t('nativeError')} <button onClick={() => void clipboard.refresh()}>{t('retry')}</button></p>}
    <HistoryList onScrollOwnerChange={restoreScroll} active={active} onScroll={() => { remember(); clipboard.refreshDeferred(); }} list={list} pinnedList={pinnedList} pinnedCount={records.filter((record) => record.isPinned).length} pinned={clipboard.retainedRecords.filter((record) => record.isPinned).map(row)} onNearEnd={loadNearEnd}>
      {clipboard.retainedRecords.filter((record) => !record.isPinned).map(row)}
      {!clipboard.loading && !clipboard.error && !records.length && <div className="history-empty clipboard-empty" role="status"><p>{query ? t('clipboardNoMatches') : favoritesOnly ? t('clipboardFavoritesEmpty') : kindFilter ? t(kindFilter === 'text' ? 'clipboardTextEmpty' : kindFilter === 'image' ? 'clipboardImagesEmpty' : 'clipboardFilesEmpty') : snapshot.clipboardCaptureEnabled ? t('clipboardEmpty') : t('clipboardCaptureOff')}</p>{kindFilter && <button onClick={() => { remember(); setKindFilter(null); }}>{t('clearClipboardFilter')}</button>}{!query && !favoritesOnly && !kindFilter && !snapshot.clipboardCaptureEnabled && <button onClick={() => void command('window_show', { target: 'settings' })}>{t('openSettings')}</button>}</div>}
    </HistoryList>
    <HistoryFooter key={session} favoritesOnly={favoritesOnly} onFavorites={() => { remember(); setKindFilter(null); setFavoritesOnly(kindFilter !== null || !favoritesOnly); }} kind={kindFilter} onKindChange={(next) => { remember(); setFavoritesOnly(false); setKindFilter(next); }} total={clipboard.page.totalRecords} clipboard primaryModifier={snapshot.capabilities.primaryModifier} />
    {menu && menuRecord && <ClipboardMenu key={menuRecord.id} record={menuRecord} x={menu.x} y={menu.y} canPaste={native && snapshot.capabilities.autoPaste} canCopy={native && snapshot.capabilities.clipboardWrite} canTranslate={canTranslateRecord(menuRecord)} canOcr={canOcrRecord(menuRecord)} busy={processing !== null} onClose={() => setMenu(null)} onReturnFocus={() => focusRow(menuRecord.id)} onAction={(action) => {
      setMenu(null);
      if (action === 'paste' || action === 'copy') void useRecord(menuRecord.id, action);
      else if (action === 'translate' || action === 'ocr') void processRecord(menuRecord.id, action);
      else if (action === 'note') setEditing(menuRecord.id);
      else void clipboard.update(menuRecord.id, action);
    }} />}
    {editing && <ClipboardEditor recordId={editing} onClose={() => setEditing(null)} onSaved={(detail) => clipboard.applyEdit(editing, detail)} onReturnFocus={() => focusRow(editing)} />}
    <RecordFeedback message={active ? processingError ?? (feedback ? t(feedback) : null) : null} />
  </section>;
}
