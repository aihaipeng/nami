import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { Image } from 'lucide-react';
import type { ViewProps } from '../App';
import type { ClipboardResult } from '../shared/contracts';
import type { MessageKey } from '../shared/i18n';
import { ActivityBeam } from '../shared/ActivityBeam';
import { HistoryIcon } from '../translation/HistoryIcon';
import { HistoryWordmark } from '../shared/HistoryWordmark';
import { historyPreview } from '../translation/history';
import { cn } from '../lib/utils';
import fallbackIcon from '../assets/history/reference-nami.png';
import { useClipboard, type ClipboardRecord } from './useClipboard';
import { HistoryList } from '../shared/HistoryList';
import { HistoryFooter } from '../shared/HistoryFooter';
import { ClipboardNotePreview } from './ClipboardNotePreview';
import { ClipboardEditor } from './ClipboardEditor';
import { ClipboardMenu } from './ClipboardMenu';
import './clipboard.css';

function ImagePreview({ recordId, native, label }: { recordId: string; native: boolean; label: string }) {
  const element = useRef<HTMLSpanElement>(null);
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let requested = false;
    let objectUrl: string | undefined;
    async function load() {
      if (requested) return;
      requested = true;
      try {
        const image = await invoke<{ bytes: number[]; mime: string }>('clipboard_image', { recordId });
        if (disposed) return;
        objectUrl = URL.createObjectURL(new Blob([new Uint8Array(image.bytes)], { type: image.mime }));
        setUrl(objectUrl);
      } catch { /* Keep the image indicator when its saved preview is unavailable. */ }
    }
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) { observer?.disconnect(); void load(); }
    });
    if (observer && element.current) observer.observe(element.current);
    else void load();
    return () => { disposed = true; observer?.disconnect(); if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [recordId, native]);
  return <span ref={element} className="clipboard-image-preview">{url ? <img src={url} alt={label} /> : <Image aria-label={label} />}</span>;
}

export function Clipboard({ t, snapshot, command, native, activity }: ViewProps) {
  const [session, setSession] = useState(0);
  const [revealed, setRevealed] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [query, setQuery] = useState('');
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<MessageKey | null>(null);
  const clipboard = useClipboard(native, query, favoritesOnly);
  const records = clipboard.page.records;
  const menuRecord = menu ? records.find((record) => record.id === menu.id) : null;
  const visibleIds = new Set(records.map(({ id }) => id));
  const operation = useRef(0);
  const mounted = useRef(true);
  const rowRefs = useRef(new Map<string, HTMLButtonElement>());
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinnedList = useRef<HTMLDivElement>(null);
  const ordered = [...records.filter((record) => record.isPinned), ...records.filter((record) => !record.isPinned)];
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let remove: (() => void) | undefined;
    void listen<{ label: string; visible: boolean }>('window://visibility', ({ payload }) => {
      if (disposed || payload.label !== 'clipboard') return;
      ++operation.current;
      setEditing(null); setMenu(null); setRevealed(null); setSelected(null); setFeedback(null);
      setQuery(''); setFavoritesOnly(false); setSession((value) => value + 1);
      list.current?.scrollTo(0, 0); pinnedList.current?.scrollTo(0, 0);
      if (!payload.visible) (document.activeElement as HTMLElement | null)?.blur();
    }).then((unlisten) => { if (disposed) unlisten(); else remove = unlisten; }).catch(() => { if (!disposed) setFeedback('nativeError'); });
    return () => { disposed = true; remove?.(); };
  }, [native]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++operation.current; }; }, []);
  useEffect(() => { setMenu(null); setRevealed(null); }, [query, favoritesOnly]);
  useEffect(() => {
    const dismiss = () => setMenu(null);
    window.addEventListener('scroll', dismiss, true);
    return () => window.removeEventListener('scroll', dismiss, true);
  }, []);
  useEffect(() => { list.current?.scrollTo(0, 0); pinnedList.current?.scrollTo(0, 0); }, [query, favoritesOnly]);

  async function useRecord(recordId: string, action: 'copy' | 'paste') {
    if (!native || (action === 'copy' ? !snapshot.capabilities.clipboardWrite : !snapshot.capabilities.autoPaste)) return;
    const current = ++operation.current;
    activity(1); setSelected(recordId); setFeedback(null);
    try {
      const result = await invoke<ClipboardResult>('clipboard_record_use', { recordId, action });
      if (!mounted.current || operation.current !== current) return;
      if (result.copyStatus === 'superseded') setFeedback('clipboardChanged');
      else if (result.status === 'dispatched' || result.status === 'copied') setFeedback(null);
      else setFeedback(result.code === 'clipboard_file_missing' ? 'clipboardFileUnavailable' : result.code === 'clipboard_image_missing' ? 'clipboardImageUnavailable' : result.copyStatus === 'current' ? null : 'copyFailed');
    } catch (error) {
      const code = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
      if (mounted.current && operation.current === current) setFeedback(code === 'clipboard_file_missing' ? 'clipboardFileUnavailable' : code === 'clipboard_image_missing' ? 'clipboardImageUnavailable' : 'nativeError');
    }
    finally { activity(-1); }
  }

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || (editing && event.key === 'Escape')) return;
      if (event.key === 'Escape' || ((snapshot.capabilities.primaryModifier === 'meta' ? event.metaKey : event.ctrlKey) && event.key.toLowerCase() === 'w')) {
        event.preventDefault(); ++operation.current;
        void command('window_hide', { region: 'clipboard' });
      }
    };
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [snapshot.capabilities.primaryModifier, command, editing]);
  function focusRow(id: string) { setSelected(id); rowRefs.current.get(id)?.focus(); }
  function loadNearEnd(element: HTMLDivElement) {
    if (element.clientHeight > 0 && element.scrollHeight - element.scrollTop - element.clientHeight < 180) void clipboard.loadMore();
  }
  function row(record: ClipboardRecord) {
    const canDelete = !record.isFavorite && !record.isPinned;
    const date = new Date(record.date);
    const two = (value: number) => String(value).padStart(2, '0');
    const time = `${date.toDateString() === new Date().toDateString() ? '' : `${two(date.getMonth() + 1)}-${two(date.getDate())} `}${two(date.getHours())}:${two(date.getMinutes())}`;
    const kind = record.kind === 'image' ? t('clipboardImage') : record.kind === 'files' ? t('clipboardFiles') : t('text');
    const index = ordered.findIndex(({ id }) => id === record.id);
    return <article data-note-revealed={record.title ? revealed === record.id : undefined} onMouseEnter={() => { if (record.title) setRevealed(record.id); }} onMouseLeave={() => setRevealed((current) => current === record.id ? null : current)} onContextMenu={(event) => { event.preventDefault(); setSelected(record.id); setMenu({ id: record.id, x: event.clientX, y: event.clientY }); }} key={record.id} hidden={!visibleIds.has(record.id)} data-record-id={record.id} className={cn('history-row', selected === record.id && 'selected', record.isPinned && 'pinned')} style={{ '--history-actions-width': `${canDelete ? 99 : 74}px` } as CSSProperties}>
      <button type="button" ref={(element) => { if (element) rowRefs.current.set(record.id, element); else rowRefs.current.delete(record.id); }} className="history-row-select" disabled={!native || !snapshot.capabilities.autoPaste} aria-label={`${t('clipboard')}: ${record.text || kind}`} aria-pressed={selected === record.id} onFocus={() => setSelected(record.id)} onClick={() => void useRecord(record.id, 'paste')} onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'ArrowDown' && ordered[index + 1]) { event.preventDefault(); focusRow(ordered[index + 1].id); }
        else if (event.key === 'ArrowUp') { event.preventDefault(); if (ordered[index - 1]) focusRow(ordered[index - 1].id); else search.current?.focus(); }
      }}>
        <span className="history-row-heading">{record.sourceApp?.iconDataUrl
          ? <img className="history-origin" src={record.sourceApp.iconDataUrl} alt={record.sourceApp.name ?? 'Nami'} title={record.sourceApp.name ?? 'Nami'} />
          : <span className="history-origin history-symbol" role="img" aria-label={record.sourceApp?.name ?? 'Nami'} title={record.sourceApp?.name ?? 'Nami'} style={{ '--symbol-url': `url("${fallbackIcon}")`, borderRadius: 0, background: '#fff' } as CSSProperties} />}<span className="history-languages">{kind}</span><time dateTime={record.date}>{time}</time></span>
        {record.title ? <ClipboardNotePreview record={record} revealed={revealed === record.id} native={native} image={record.kind === 'image' ? <ImagePreview recordId={record.id} native={native} label={kind} /> : undefined} /> : record.kind === 'image' ? <ImagePreview recordId={record.id} native={native} label={kind} /> : <span className="history-preview">{historyPreview(record.text)}</span>}
      </button>
      <div className="history-row-actions" role="group" aria-label={t('actions')}>
        <button className="icon-button" title={t('copy')} aria-label={t('copy')} disabled={!native || !snapshot.capabilities.clipboardWrite} onClick={() => void useRecord(record.id, 'copy')}><HistoryIcon name="doc.on.doc" /></button>
        <button className="icon-button" title={record.isFavorite ? t('unfavorite') : t('favorite')} aria-label={record.isFavorite ? t('unfavorite') : t('favorite')} aria-pressed={record.isFavorite} disabled={!native} onClick={() => void clipboard.update(record.id, 'favorite')}><HistoryIcon name={record.isFavorite ? 'star.fill' : 'star'} /></button>
        <button className="icon-button" title={record.isPinned ? t('unpinRecord') : t('pinRecord')} aria-label={record.isPinned ? t('unpinRecord') : t('pinRecord')} aria-pressed={record.isPinned} disabled={!native} onClick={() => void clipboard.update(record.id, 'pin')}><HistoryIcon name={record.isPinned ? 'pin.fill' : 'pin'} /></button>
        {canDelete && <button className="icon-button history-delete" title={t('deleteClipboard')} aria-label={t('deleteClipboard')} disabled={!native} onClick={() => void clipboard.update(record.id, 'delete')}><HistoryIcon name="trash" /></button>}
      </div>
    </article>;
  }
  return <section inert={editing !== null} className="history-panel clipboard-panel" aria-label={t('clipboard')}>
    {clipboard.showActivity && <ActivityBeam label={t('working')} />}
    <div className="drag-strip" data-tauri-drag-region />
    <header className="history-toolbar" onMouseDown={(event) => { if ((event.target as HTMLElement).closest('button')) event.preventDefault(); }}>
      <HistoryWordmark />
      <label className="history-search"><HistoryIcon name="magnifyingglass" /><input ref={search} type="search" aria-label={t('searchClipboard')} placeholder={t('searchClipboard')} value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if (event.key === 'ArrowDown' && !event.nativeEvent.isComposing && ordered[0]) { event.preventDefault(); focusRow(ordered[0].id); } }} />
        {query && <button type="button" aria-label={t('clearClipboardSearch')} onClick={() => { setQuery(''); search.current?.focus(); }}><HistoryIcon name="xmark.circle.fill" /></button>}
      </label>
      <button className="icon-button" title={t('allClipboard')} aria-label={t('allClipboard')} aria-pressed={!favoritesOnly} onClick={() => setFavoritesOnly(false)}><HistoryIcon name="list.bullet" /></button>
      <button className="icon-button" title={t('favoriteClipboard')} aria-label={t('favoriteClipboard')} aria-pressed={favoritesOnly} onClick={() => setFavoritesOnly(true)}><HistoryIcon name={favoritesOnly ? 'star.fill' : 'star'} /></button>
      <button className="icon-button history-pin" title={snapshot.window.clipboardPinned ? t('unpin') : t('pin')} aria-label={snapshot.window.clipboardPinned ? t('unpin') : t('pin')} aria-pressed={snapshot.window.clipboardPinned} onClick={() => void command('window_pin', { region: 'clipboard', pinned: !snapshot.window.clipboardPinned })}><HistoryIcon name={snapshot.window.clipboardPinned ? 'pin.fill' : 'pin'} /></button>
    </header>
    {clipboard.error && <p role="alert" className="history-error">{t('nativeError')} <button onClick={() => void clipboard.refresh()}>{t('retry')}</button></p>}
    <HistoryList list={list} pinnedList={pinnedList} pinnedCount={records.filter((record) => record.isPinned).length} pinned={clipboard.retainedRecords.filter((record) => record.isPinned).map(row)} onNearEnd={loadNearEnd}>
      {clipboard.retainedRecords.filter((record) => !record.isPinned).map(row)}
      {!clipboard.loading && !clipboard.error && !records.length && <div className="history-empty clipboard-empty" role="status"><p>{query ? t('clipboardNoMatches') : favoritesOnly ? t('clipboardFavoritesEmpty') : snapshot.clipboardCaptureEnabled ? t('clipboardEmpty') : t('clipboardCaptureOff')}</p>{!query && !favoritesOnly && !snapshot.clipboardCaptureEnabled && <button onClick={() => void command('window_show', { target: 'settings' })}>{t('openSettings')}</button>}</div>}
    </HistoryList>
    <HistoryFooter key={session} total={clipboard.page.totalRecords} clipboard primaryModifier={snapshot.capabilities.primaryModifier} />
    {menu && menuRecord && <ClipboardMenu record={menuRecord} x={menu.x} y={menu.y} canPaste={native && snapshot.capabilities.autoPaste} onClose={() => setMenu(null)} onReturnFocus={() => focusRow(menuRecord.id)} onAction={(action) => {
      setMenu(null);
      if (action === 'paste') void useRecord(menuRecord.id, 'paste');
      else if (action === 'note') setEditing(menuRecord.id);
      else void clipboard.update(menuRecord.id, action);
    }} />}
    {editing && <ClipboardEditor recordId={editing} onClose={() => setEditing(null)} onReturnFocus={() => focusRow(editing)} />}
    {feedback && <div className="clipboard-feedback" role="status">{t(feedback)}</div>}
  </section>;
}
