import { clipboardOpenError, clipboardOpenTarget } from './openTarget';
import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ViewProps } from '../App';
import { RecordActions } from '../shared/RecordActions';
import { RecordHeading } from '../shared/RecordHeading';
import { ActivityBeam } from '../shared/ActivityBeam';
import { RecordFeedback } from '../shared/RecordFeedback';
import { HistoryList } from '../shared/HistoryList';
import { FileList, ImagePreview, isSingleImage, type FileEntry } from './ClipboardMedia';
import { ClipboardRichText } from './ClipboardRichText';
import {
  ClipboardColorCard,
  ClipboardDateTimeCard,
  ClipboardDecodedCard,
  ClipboardFilesCard,
  ClipboardImageQrBar,
  ClipboardJsonBar,
  ClipboardJwtCard,
  ClipboardQrPreview,
  ClipboardTimestampCard,
  ClipboardUrlCard,
  type JsonViewMode,
} from './ClipboardSmartDetail';
import { ClipboardTextPreview } from './ClipboardTextPreview';
import { analyzeSmartDetail, canEncodeQr } from './smartDetail';
import { resolveLanguage } from '../shared/i18n';
import { useRecordsPreviewShortcut } from '../shared/recordPreviewShortcut';
import type { ClipboardRecord } from './useClipboard';
import { clipboardProcessError, clipboardUseError, clipboardUseFeedback } from './feedback';
import type { ClipboardResult } from '../shared/contracts';
import './clipboard.css';

export interface ClipboardDetailData extends ClipboardRecord {
  width: number | null;
  height: number | null;
  size: number | null;
  characterCount: number | null;
  richTextFormat?: 'html' | 'rtf' | 'html+rtf';
  richTextHtml?: string;
  files: FileEntry[];
  imageIndexStatus?: 'unindexed' | 'pending' | 'failed' | 'empty' | 'indexed';
}

function richFormatLabel(format: NonNullable<ClipboardDetailData['richTextFormat']>) {
  return format === 'html+rtf' ? 'Rich Text (HTML, RTF)' : format === 'html' ? 'Rich Text (HTML)' : 'Rich Text (RTF)';
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
  const [textViewMode, setTextViewMode] = useState<'rich' | 'plain'>('rich');
  const [jsonMode, setJsonMode] = useState<JsonViewMode>('original');
  const [showQr, setShowQr] = useState(false);
  const [qrCodes, setQrCodes] = useState<string[]>([]);
  const [openTargetKind, setOpenTargetKind] = useState<'file' | 'link' | 'email' | null>(null);
  const operation = useRef(0);
  const feedbackOperation = useRef(0);
  const processing = useRef(false);
  const list = useRef<HTMLDivElement>(null);
  const pinned = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ++operation.current; processing.current = false; setBusy(false); setFeedback(null); setFailed(null); setTextViewMode('rich'); setJsonMode('original'); setShowQr(false); setQrCodes([]); setOpenTargetKind(null);
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
  const singleImage = shown?.kind === 'image' || shown?.kind === 'files' && isSingleImage(shown.files);
  const canTranslate = shown?.kind === 'text' ? !!shown.text.trim() : snapshot.capabilities.imageOcr && singleImage && shown?.imageIndexStatus !== 'empty';
  const canOcr = canTranslate && shown?.kind !== 'text' && snapshot.capabilities.clipboardWrite;
  const [indexingId, setIndexingId] = useState<string | null>(null);
  const indexStatus = current && (indexingId === current.id ? 'pending' : current.imageIndexStatus);
  const smart = useMemo(() => (current?.kind === 'text' ? analyzeSmartDetail(current.text) : null), [current?.kind, current?.text]);
  const canShowQr = Boolean(current?.kind === 'text' && canEncodeQr(current.text));
  const singleExistingFile = Boolean((current?.kind === 'files' || current?.kind === 'image') && current.files.length === 1 && current.files[0].exists);
  useEffect(() => {
    if (!active || !native || !current || !snapshot.capabilities.clipboardOpenTarget) { setOpenTargetKind(null); return; }
    if (singleExistingFile) { setOpenTargetKind('file'); return; }
    if (current.kind !== 'text') { setOpenTargetKind(null); return; }
    let disposed = false;
    void clipboardOpenTarget(current.id, false)
      .then((kind) => { if (!disposed) setOpenTargetKind(kind === 'file' || kind === 'link' || kind === 'email' ? kind : null); })
      .catch(() => { if (!disposed) setOpenTargetKind(null); });
    return () => { disposed = true; };
  }, [active, native, current?.id, current?.kind, current?.text, singleExistingFile, snapshot.capabilities.clipboardOpenTarget]);
  useEffect(() => {
    if (!active || !native || !current || !singleImage || !snapshot.capabilities.imageOcr) { setQrCodes([]); return; }
    let disposed = false;
    void invoke<string[]>('clipboard_image_qr_codes', { recordId: current.id })
      .then((codes) => { if (!disposed) setQrCodes(Array.isArray(codes) ? codes : []); })
      .catch(() => { if (!disposed) setQrCodes([]); });
    return () => { disposed = true; };
  }, [active, native, current?.id, singleImage, snapshot.capabilities.imageOcr]);
  async function openCurrentTarget() {
    if (!current || loading) return;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try { await clipboardOpenTarget(current.id, true, singleExistingFile ? 0 : undefined); }
    catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t(clipboardOpenError(error)));
    }
  }
  async function copyDerivedText(text: string) {
    if (!native || !snapshot.capabilities.clipboardWrite) return false;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try {
      await invoke('clipboard_text_copy', { text });
      return operation.current === revision;
    } catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t(clipboardUseError(error)));
      return false;
    }
  }
  async function openQrUrl(url: string) {
    if (!native || !snapshot.capabilities.clipboardOpenTarget) return;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try { await invoke('clipboard_open_url', { url }); }
    catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t(clipboardOpenError(error)));
    }
  }
  async function indexImage(recordId: string) {
    const revision = operation.current;
    setIndexingId(recordId);
    setFeedback(null);
    try { await invoke('clipboard_record_process', { recordId, action: 'index' }); }
    catch { if (operation.current === revision) setFeedback(t('clipboardImageIndexFailed')); }
    finally { setIndexingId((id) => id === recordId ? null : id); }
  }
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
      if (operation.current === revision && feedbackOperation.current === feedbackRevision && String(error) !== 'cancelled') setFeedback(type === 'copy' ? t(clipboardUseError(error)) : process ? clipboardProcessError(t, error, type) : t('nativeError'));
    } finally {
      if (process && operation.current === revision) { processing.current = false; setBusy(false); }
    }
    return false;
  }
  useRecordsPreviewShortcut({
    active: active && !snapshot.window.recordsEditorOpen,
    excludedContainers: '.records-workspace, .translation-panel, .preview-bar',
    snapshot,
    native,
    onPreview: () => { void command('records_details_toggle'); },
  });
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
  const hasRichPreview = Boolean(current?.kind === 'text' && current.richTextHtml);
  const showingRich = hasRichPreview && textViewMode === 'rich';
  const displayedText = current?.kind === 'text' && smart?.kind === 'json'
    ? (jsonMode === 'pretty' ? smart.pretty : jsonMode === 'minified' ? smart.minified : current.text)
    : current?.text ?? '';
  const canCopyDerived = native && current !== null && !loading && snapshot.capabilities.clipboardWrite;
  const language = resolveLanguage(snapshot.locale);
  const mac = snapshot.capabilities.platform === 'macos';
  const openTargetAction = current && openTargetKind ? {
    kind: openTargetKind,
    label: openTargetKind === 'file' ? t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager')
      : openTargetKind === 'link' ? t('clipboardQrOpenLink') : t('clipboardShortcutOpenTarget'),
    onClick: () => { void openCurrentTarget(); },
  } : undefined;
  const qrAction = canShowQr ? {
    active: showQr,
    label: t(showQr ? 'clipboardQrHide' : 'clipboardQrGenerate'),
    onClick: () => setShowQr((value) => !value),
  } : undefined;
  return <section inert={snapshot.window.recordsEditorOpen} className="history-panel clipboard-detail" data-qr-view={(showQr && canShowQr) || undefined} aria-label="Clipboard details" aria-busy={loading}>
    <ActivityBeam label={t('working')} active={active && loading} />
    <header className="clipboard-detail-toolbar" data-tauri-drag-region>
      {hasRichPreview && !showQr && <div className="clipboard-detail-mode" role="group" aria-label={t('clipboardRichPreviewMode')} data-tauri-drag-region="false">
        <button type="button" aria-pressed={showingRich} onClick={() => setTextViewMode('rich')}>{t('clipboardRichView')}</button>
        <button type="button" aria-pressed={!showingRich} onClick={() => setTextViewMode('plain')}>{t('clipboardPlainView')}</button>
      </div>}
      {shown && <RecordActions allowMarkedDelete recordId={shown.id} t={t} native={native && current !== null && !loading} favorite={shown.isFavorite} pinned={shown.isPinned} deleteLabel={t('deleteClipboard')} canCopy={snapshot.capabilities.clipboardWrite} onCopy={() => action('copy')}
        onOpenTarget={openTargetAction} onQrCode={qrAction}
        onTranslate={canTranslate ? () => { void action('translate'); } : undefined} onOcr={canOcr ? () => { void action('ocr'); } : undefined}
        onNote={() => { if (current) void command('clipboard_note_open', { recordId: current.id }); }} onUpdate={(type) => { void action(type); }} busy={busy} retainDelete />}
    </header>
    <HistoryList scrollOnly active={active} list={list} pinnedList={pinned} pinned={[]} onNearEnd={() => {}}>
      {current ? <div className="clipboard-detail-content" key={current.id}>
        {current.kind === 'text'
          ? (showQr && canShowQr
            ? <ClipboardQrPreview text={current.text} />
            : showingRich && current.richTextHtml
              ? <ClipboardRichText html={current.richTextHtml} />
              : <>
                {smart?.kind === 'json' && <ClipboardJsonBar json={smart} mode={jsonMode} onModeChange={setJsonMode} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
                <ClipboardTextPreview as="div" text={displayedText} className="clipboard-detail-text" data-tauri-drag-region="false" />
                {smart?.kind === 'color' && <ClipboardColorCard color={smart} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
                {smart?.kind === 'timestamp' && <ClipboardTimestampCard timestamp={smart} locale={language} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
                {smart?.kind === 'datetime' && <ClipboardDateTimeCard dateTime={smart} locale={language} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
                {smart?.kind === 'url' && <ClipboardUrlCard url={smart} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
                {smart?.kind === 'jwt' && <ClipboardJwtCard jwt={smart} locale={language} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
                {smart?.kind === 'decoded' && <ClipboardDecodedCard decoded={smart} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
              </>)
          : singleImage
            ? <>
              <ImagePreview key={current.id} recordId={current.id} native={native} />
              <ClipboardImageQrBar codes={qrCodes} canCopy={canCopyDerived} canOpen={native && snapshot.capabilities.clipboardOpenTarget} onCopy={copyDerivedText} onOpenUrl={openQrUrl} t={t} />
              {current.files.length > 0 && <ClipboardFilesCard entries={current.files} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
            </>
            : current.files.length > 1
              ? <ClipboardFilesCard entries={current.files} canCopy={canCopyDerived} onCopy={copyDerivedText}
                onReveal={snapshot.capabilities.clipboardOpenTarget ? (index) => { void revealFile(index); } : undefined}
                revealLabel={t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager')} t={t} />
              : <>
                <FileList entries={current.files} />
                {current.files.length > 0 && <ClipboardFilesCard entries={current.files} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
              </>}
      </div> : <div className="history-empty" role={failed ? 'alert' : 'status'}>{failed ? t(failed === 'unavailable' ? 'clipboardDetailUnavailable' : 'clipboardDetailLoadError') : !id ? 'No record selected' : ''}{failed === 'retry' && <button onClick={() => setAttempt((value) => value + 1)}>{t('retry')}</button>}</div>}
    </HistoryList>
    {current && (indexStatus === 'unindexed' || indexStatus === 'failed') && <div className="clipboard-image-index-action">
      <button type="button" disabled={loading || busy} onClick={() => { void indexImage(current.id); }}>{t(indexStatus === 'failed' ? 'retry' : 'clipboardIndexImage')}</button>
    </div>}
    {current && <footer className="clipboard-detail-metadata">
      <div><span>Application</span><RecordHeading date={current.date} label={current.sourceApp?.name ?? 'Nami'} sourceApp={current.sourceApp} /></div>
      {current.width != null && current.height != null && <div><span>Dimensions</span><span>{current.width} × {current.height}</span></div>}
      {current.size != null && <div><span>Size</span><span>{formatSize(current.size)}</span></div>}
      {current.kind === 'text' && <div><span>Characters</span><span>{current.characterCount?.toLocaleString('en')}</span></div>}
      {current.kind === 'text' && current.richTextFormat && <div><span>{t('clipboardDetailFormat')}</span><span>{richFormatLabel(current.richTextFormat)}</span></div>}
    </footer>}
    <RecordFeedback message={feedback} />
  </section>;
}
