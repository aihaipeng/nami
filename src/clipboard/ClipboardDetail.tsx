import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ViewProps } from '../App';
import type { ClipboardResult } from '../shared/contracts';
import { ActivityBeam } from '../shared/ActivityBeam';
import { RecordActions } from '../shared/RecordActions';
import { RecordHeading } from '../shared/RecordHeading';
import { RecordFeedback } from '../shared/RecordFeedback';
import { HistoryList } from '../shared/HistoryList';
import { FileList, ImagePreview, type FileEntry } from './ClipboardMedia';
import type { ClipboardRecord } from './useClipboard';
import { useClipboardImageText } from './useClipboardImageText';
import './clipboard.css';

export interface ClipboardDetailData extends ClipboardRecord {
  width: number | null;
  height: number | null;
  fileSize: number | null;
  characterCount: number | null;
  files: FileEntry[];
}

function fileSize(bytes: number) {
  return bytes < 1024 * 1024 ? `${(bytes / 1024).toLocaleString('en', { maximumFractionDigits: 1 })} KB` : `${(bytes / (1024 * 1024)).toLocaleString('en', { maximumFractionDigits: 1 })} MB`;
}

export function ClipboardDetail({ snapshot, native, t, command }: ViewProps) {
  const id = snapshot.window.clipboardDetailId;
  const active = snapshot.window.recordView === 'clipboard' && snapshot.window.translationVisible;
  const [detail, setDetail] = useState<ClipboardDetailData | null>(null);
  const [failed, setFailed] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const operation = useRef(0);
  const list = useRef<HTMLDivElement>(null);
  const pinned = useRef<HTMLDivElement>(null);
  const matched = detail && detail.id === id ? [detail] : [];
  const imageText = useClipboardImageText(matched, active && native && snapshot.capabilities.imageOcr && matched.length > 0);
  useEffect(() => {
    ++operation.current; setBusy(false); setFeedback(null); setFailed(false);
    list.current?.scrollTo(0, 0);
    if (!active || !native || !id) { setDetail(null); return; }
    let disposed = false;
    let request = 0;
    const load = async () => {
      const current = ++request;
      try {
        const value = await invoke<ClipboardDetailData>('clipboard_record_detail', { recordId: id });
        if (!disposed && current === request) { setDetail(value); setFailed(false); }
      } catch { if (!disposed && current === request) { setDetail(null); setFailed(true); } }
    };
    const remove = listen('clipboard_changed', () => { void load(); });
    void remove.then(load).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; ++operation.current; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [id, active, native, snapshot.window.recordsSession]);
  const shown = active && detail && !failed ? detail : null;
  const current = shown?.id === id ? shown : null;
  const canTranslate = shown?.kind === 'text' ? !!shown.text.trim() : !!shown && snapshot.capabilities.imageOcr && imageText.get(shown.id) === true;
  const canOcr = canTranslate && shown?.kind !== 'text' && snapshot.capabilities.clipboardWrite;
  async function action(type: 'copy' | 'translate' | 'ocr' | 'favorite' | 'pin' | 'delete') {
    if (!current || busy) return;
    const revision = ++operation.current;
    setBusy(true); setFeedback(null);
    try {
      if (type === 'copy') {
        const result = await invoke<ClipboardResult>('clipboard_record_use', { recordId: current.id, action: 'copy' });
        if (result.copyStatus === 'superseded' || result.status !== 'copied' && result.status !== 'dispatched') throw new Error('copy_failed');
      } else if (type === 'translate' || type === 'ocr') await invoke('clipboard_record_process', { recordId: current.id, action: type });
      else await invoke('clipboard_record_update', { recordId: current.id, action: type });
    } catch (error) {
      if (operation.current === revision && String(error) !== 'cancelled') setFeedback('Unable to complete this action. Try again.');
    } finally { if (operation.current === revision) setBusy(false); }
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
  return <section inert={snapshot.window.recordsEditorOpen} className="history-panel clipboard-detail" aria-label="Clipboard details">
    <header className="clipboard-detail-toolbar" data-tauri-drag-region>
      {shown && <RecordActions t={t} native={native && !busy} favorite={shown.isFavorite} pinned={shown.isPinned} canCopy={snapshot.capabilities.clipboardWrite} copyLabel={t('copy')} deleteLabel={t('deleteClipboard')}
        onCopy={() => { void action('copy'); }} onTranslate={canTranslate ? () => { void action('translate'); } : undefined} onOcr={canOcr ? () => { void action('ocr'); } : undefined}
        onNote={() => { if (current) void command('clipboard_note_open', { recordId: current.id }); }} onUpdate={(type) => { void action(type); }} busy={busy} retainDelete />}
    </header>
    {active && busy && <ActivityBeam label="Working" />}
    <HistoryList scrollOnly key={id ?? 'empty'} active={active} list={list} pinnedList={pinned} pinned={[]} onNearEnd={() => {}}>
      {current ? <div className="clipboard-detail-content" key={current.id}>
        {current.kind === 'text' ? <div className="clipboard-detail-text" data-tauri-drag-region="false">{current.text}</div> : singleImage ? <ImagePreview key={current.id} recordId={current.id} native={native} /> : <FileList entries={current.files} />}
      </div> : <div className="history-empty" role={failed ? 'alert' : 'status'}>{failed ? 'This record is unavailable.' : !id ? 'No record selected' : ''}</div>}
    </HistoryList>
    {current && <footer className="clipboard-detail-metadata">
      <div><span>Application</span><RecordHeading date={current.date} label={current.sourceApp?.name ?? 'Nami'} sourceApp={current.sourceApp} /></div>
      {current.width != null && current.height != null && <div><span>Dimensions</span><span>{current.width} × {current.height}</span></div>}
      {current.kind === 'files' && current.fileSize != null && <div><span>File size</span><span>{fileSize(current.fileSize)}</span></div>}
      {current.kind === 'text' && <div><span>Characters</span><span>{current.characterCount?.toLocaleString('en')}</span></div>}
    </footer>}
    <RecordFeedback message={feedback} />
  </section>;
}
