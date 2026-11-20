import { clipboardOpenError, clipboardOpenTarget } from './openTarget';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ViewProps } from '../App';
import { RecordActions } from '../shared/RecordActions';
import { RecordHeading } from '../shared/RecordHeading';
import { ActivityBeam } from '../shared/ActivityBeam';
import { RecordFeedback } from '../shared/RecordFeedback';
import { HistoryList } from '../shared/HistoryList';
import { FileList, ImagePreview, type FileEntry } from './ClipboardMedia';
import type { ClipboardRecord } from './useClipboard';
import { clipboardUseError, clipboardUseFeedback } from './feedback';
import type { ClipboardResult } from '../shared/contracts';
import { useClipboardImageText } from './useClipboardImageText';
import './clipboard.css';

export interface ClipboardDetailData extends ClipboardRecord {
  width: number | null;
  height: number | null;
  size: number | null;
  characterCount: number | null;
  files: FileEntry[];
}

function formatSize(bytes: number) {
  return bytes < 1024 * 1024 ? `${(bytes / 1024).toLocaleString('en', { maximumFractionDigits: 1 })} KB` : `${(bytes / (1024 * 1024)).toLocaleString('en', { maximumFractionDigits: 1 })} MB`;
}

export function ClipboardDetail({ snapshot, native, t, command }: ViewProps) {
  const id = snapshot.window.clipboardDetailId;
  const active = snapshot.window.recordView === 'clipboard' && snapshot.window.translationVisible;
  const [detail, setDetail] = useState<ClipboardDetailData | null>(null);
  const [failed, setFailed] = useState<'unavailable' | 'retry' | null>(null);
  const [loading, setLoading] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const operation = useRef(0);
  const feedbackOperation = useRef(0);
  const processing = useRef(false);
  const list = useRef<HTMLDivElement>(null);
  const pinned = useRef<HTMLDivElement>(null);
  const matched = detail && detail.id === id ? [detail] : [];
  const imageText = useClipboardImageText(matched, active && native && snapshot.capabilities.imageOcr && matched.length > 0);
  useEffect(() => {
    ++operation.current; processing.current = false; setBusy(false); setFeedback(null); setFailed(null);
    list.current?.scrollTo(0, 0);
    if (!active || !native || !id) { setDetail(null); setLoading(false); return; }
    let disposed = false;
    let request = 0;
    const load = async () => {
      if (disposed) return;
      const current = ++request;
      setLoading(true); setFailed(null);
      try {
        const value = await invoke<ClipboardDetailData>('clipboard_record_detail', { recordId: id });
        if (!disposed && current === request) { setDetail(value); setFailed(null); }
      } catch (error) {
        if (!disposed && current === request) {
          setDetail(null);
          setFailed((error instanceof Error ? error.message : error) === 'not_found' ? 'unavailable' : 'retry');
        }
      } finally { if (!disposed && current === request) setLoading(false); }
    };
    const remove = listen('clipboard_changed', () => { void load(); });
    setLoading(true);
    void remove.then(load).catch(() => { if (!disposed) { setDetail(null); setFailed('retry'); setLoading(false); } });
    return () => { disposed = true; ++operation.current; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [id, active, native, snapshot.window.recordsSession, attempt]);
  const shown = active && detail && !failed ? detail : null;
  const current = shown?.id === id ? shown : null;
  const canTranslate = shown?.kind === 'text' ? !!shown.text.trim() : !!shown && snapshot.capabilities.imageOcr && imageText.get(shown.id) === true;
  const canOcr = canTranslate && shown?.kind !== 'text' && snapshot.capabilities.clipboardWrite;
  async function revealFile(index: number) {
    if (!current || loading) return;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try { await clipboardOpenTarget(current.id, true, index); }
    catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t(clipboardOpenError(error)));
    }
  }
  async function action(type: 'copy' | 'translate' | 'ocr' | 'favorite' | 'pin' | 'delete') {
    const process = type === 'translate' || type === 'ocr';
    if (!current || loading || processing.current && (process || type === 'delete')) return false;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    if (process) { processing.current = true; setBusy(true); }
    setFeedback(null);
    try {
      if (type === 'copy') {
        const result = await invoke<ClipboardResult>('clipboard_record_use', { recordId: current.id, action: 'copy' });
        if (operation.current !== revision) return false;
        const message = clipboardUseFeedback(result);
        if (feedbackOperation.current === feedbackRevision) setFeedback(message ? t(message) : null);
        return result.copyStatus === 'current' && result.status === 'copied';
      }
      if (process) await invoke('clipboard_record_process', { recordId: current.id, action: type });
      else await invoke('clipboard_record_update', { recordId: current.id, action: type });
    } catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision && String(error) !== 'cancelled') setFeedback(type === 'copy' ? t(clipboardUseError(error)) : process ? type === 'ocr' ? 'Unable to extract and copy text. Try again.' : 'Unable to open this text for translation. Try again.' : t('nativeError'));
    } finally {
      if (process && operation.current === revision) { processing.current = false; setBusy(false); }
    }
    return false;
  }
  const singleImage = current?.kind === 'image' || current?.kind === 'files' && current.files.length === 1 && current.files[0].exists && current.files[0].isImage;
  useEffect(() => {
    if (!active || snapshot.window.recordsEditorOpen) return;
    const dismiss = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
      if (event.key === 'Escape' || ((snapshot.capabilities.primaryModifier === 'meta' ? event.metaKey : event.ctrlKey) && event.key.toLowerCase() === 'w')) {
        event.preventDefault(); void command('window_hide', { region: 'translation' });
      }
    };
    window.addEventListener('keydown', dismiss);
    return () => window.removeEventListener('keydown', dismiss);
  }, [active, snapshot.window.recordsEditorOpen, snapshot.capabilities.primaryModifier, command]);
  return <section inert={snapshot.window.recordsEditorOpen} className="history-panel clipboard-detail" aria-label="Clipboard details" aria-busy={loading}>
    <ActivityBeam label={t('working')} active={active && loading} />
    <header className="clipboard-detail-toolbar" data-tauri-drag-region>
      {shown && <RecordActions allowMarkedDelete recordId={shown.id} t={t} native={native && current !== null && !loading} favorite={shown.isFavorite} pinned={shown.isPinned} deleteLabel={t('deleteClipboard')} canCopy={snapshot.capabilities.clipboardWrite} onCopy={() => action('copy')}
        onTranslate={canTranslate ? () => { void action('translate'); } : undefined} onOcr={canOcr ? () => { void action('ocr'); } : undefined}
        onNote={() => { if (current) void command('clipboard_note_open', { recordId: current.id }); }} onUpdate={(type) => { void action(type); }} busy={busy} retainDelete />}
    </header>
    <HistoryList scrollOnly active={active} list={list} pinnedList={pinned} pinned={[]} onNearEnd={() => {}}>
      {current ? <div className="clipboard-detail-content" key={current.id}>
        {current.kind === 'text' ? <div className="clipboard-detail-text" data-tauri-drag-region="false">{current.text}</div> : singleImage ? <ImagePreview key={current.id} recordId={current.id} native={native} /> : <FileList entries={current.files} onReveal={snapshot.capabilities.clipboardOpenTarget && current.files.length > 1 ? (index) => { void revealFile(index); } : undefined} revealLabel={t('clipboardRevealFile')} />}
      </div> : <div className="history-empty" role={failed ? 'alert' : 'status'}>{failed ? t(failed === 'unavailable' ? 'clipboardDetailUnavailable' : 'clipboardDetailLoadError') : !id ? 'No record selected' : ''}{failed === 'retry' && <button onClick={() => setAttempt((value) => value + 1)}>{t('retry')}</button>}</div>}
    </HistoryList>
    {current && <footer className="clipboard-detail-metadata">
      <div><span>Application</span><RecordHeading date={current.date} label={current.sourceApp?.name ?? 'Nami'} sourceApp={current.sourceApp} /></div>
      {current.width != null && current.height != null && <div><span>Dimensions</span><span>{current.width} × {current.height}</span></div>}
      {current.size != null && <div><span>Size</span><span>{formatSize(current.size)}</span></div>}
      {current.kind === 'text' && <div><span>Characters</span><span>{current.characterCount?.toLocaleString('en')}</span></div>}
    </footer>}
    <RecordFeedback message={feedback} />
  </section>;
}
