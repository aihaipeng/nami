import { clipboardOpenError, clipboardOpenTarget } from './openTarget';
import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ViewProps } from '../App';
import { RecordActions } from '../shared/RecordActions';
import { RecordHeading } from '../shared/RecordHeading';
import { ActivityBeam } from '../shared/ActivityBeam';
import { ActionTooltipHost } from '../shared/ActionTooltip';
import { RecordFeedback } from '../shared/RecordFeedback';
import { HistoryList } from '../shared/HistoryList';
import { matchesBinding } from '../shared/shortcuts';
import { ClipboardDeleteConfirm, markedDeleteDescription } from './ClipboardDeleteConfirm';
import { ImagePreview, isSingleImage, type FileEntry } from './ClipboardMedia';
import { ClipboardRichText } from './ClipboardRichText';
import {
  ClipboardColorCard,
  ClipboardCronCard,
  ClipboardDateTimeCard,
  ClipboardDecodedCard,
  ClipboardFilesCard,
  ClipboardImageDataBar,
  ClipboardImageDataPreview,
  ClipboardImagePalette,
  ClipboardImageQrBar,
  ClipboardJsonBar,
  ClipboardJwtCard,
  ClipboardMathCard,
  ClipboardNumberCard,
  ClipboardQrPreview,
  ClipboardTimestampCard,
  ClipboardUrlCard,
  type ImageDataViewMode,
  type JsonViewMode,
} from './ClipboardSmartDetail';
import { ClipboardTextPreview } from './ClipboardTextPreview';
import { extractImagePalette } from './imagePalette';
import { packQrModules, type QrMatrix } from './qrEncoder';
import { analyzeSmartDetail, canEncodeQr, normalizeQrPayload } from './smartDetail';
import { resolveLanguage } from '../shared/i18n';
import type { ClipboardRecord } from './useClipboard';
import { clipboardProcessError, clipboardUseError } from './feedback';
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

const QR_PREVIEW_DWELL_MS = 150;

function richFormatLabel(format: NonNullable<ClipboardDetailData['richTextFormat']>) {
  return format === 'html+rtf' ? 'Rich Text (HTML + RTF)' : format === 'html' ? 'Rich Text (HTML)' : 'Rich Text (RTF)';
}

function formatSize(bytes: number) {
  return bytes < 1024 * 1024 ? `${(bytes / 1024).toLocaleString('en', { maximumFractionDigits: 1 })} KB` : `${(bytes / (1024 * 1024)).toLocaleString('en', { maximumFractionDigits: 1 })} MB`;
}

export function ClipboardDetail({ snapshot, native, t, command }: ViewProps) {
  const id = snapshot.window.clipboardDetailId;
  const active = snapshot.window.clipboardVisible && snapshot.window.clipboardDetailVisible;
  const [detail, setDetail] = useState<ClipboardDetailData | null>(null);
  const [failed, setFailed] = useState<'unavailable' | 'retry' | null>(null);
  const [loading, setLoading] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [textViewMode, setTextViewMode] = useState<'rich' | 'plain'>('rich');
  const [jsonMode, setJsonMode] = useState<JsonViewMode>('original');
  const [imageDataMode, setImageDataMode] = useState<ImageDataViewMode>('preview');
  const [imageDataDims, setImageDataDims] = useState<{ width: number; height: number } | null>(null);
  const [showQr, setShowQr] = useState(false);
  const [qrCodes, setQrCodes] = useState<string[]>([]);
  const [imagePalette, setImagePalette] = useState<string[]>([]);
  const [openTargetKind, setOpenTargetKind] = useState<'file' | 'link' | 'email' | null>(null);
  const [recognizedEmpty, setRecognizedEmpty] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const operation = useRef(0);
  const feedbackOperation = useRef(0);
  const processing = useRef(false);
  const paletteRecord = useRef<string | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const pinned = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ++operation.current; processing.current = false; paletteRecord.current = null; setBusy(false); setFeedback(null); setFailed(null); setRecognizedEmpty(false); setConfirmDelete(false); setTextViewMode('rich'); setJsonMode('original'); setImageDataMode('preview'); setImageDataDims(null); setShowQr(false); setQrCodes([]); setImagePalette([]); setOpenTargetKind(null);
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
  const canTranslate = shown?.kind === 'text' ? !!shown.text.trim() : shown?.kind === 'image' && snapshot.capabilities.imageOcr && shown.imageIndexStatus !== 'empty' && !recognizedEmpty;
  const canOcr = canTranslate && shown?.kind === 'image';
  const smart = useMemo(() => (current?.kind === 'text' ? analyzeSmartDetail(current.text) : null), [current?.kind, current?.text]);
  const hasRichPreview = Boolean(current?.kind === 'text' && current.richTextHtml);
  const showingRich = hasRichPreview && textViewMode === 'rich';
  const displayedText = current?.kind === 'text' && smart?.kind === 'json'
    ? (jsonMode === 'pretty' ? smart.pretty : jsonMode === 'minified' ? smart.minified : current.text)
    : current?.text ?? '';
  const imageDataSrc = smart?.kind === 'imageData' ? smart.src : null;
  useEffect(() => { setImageDataDims(null); }, [imageDataSrc]);
  const qrPayload = useMemo(() => (current?.kind === 'text' ? normalizeQrPayload(displayedText) : ''), [current?.kind, displayedText]);
  const canShowQr = Boolean(current?.kind === 'text' && canEncodeQr(displayedText));
  useEffect(() => {
    if (!active || !native || !current || current.kind !== 'text' || !snapshot.capabilities.clipboardOpenTarget) { setOpenTargetKind(null); return; }
    let disposed = false;
    void clipboardOpenTarget(current.id, false)
      .then((kind) => { if (!disposed) setOpenTargetKind(kind === 'file' || kind === 'link' || kind === 'email' ? kind : null); })
      .catch(() => { if (!disposed) setOpenTargetKind(null); });
    return () => { disposed = true; };
  }, [active, native, current?.id, current?.kind, current?.text, snapshot.capabilities.clipboardOpenTarget]);
  useEffect(() => {
    if (!singleImage) { paletteRecord.current = null; setImagePalette([]); }
  }, [singleImage]);
  useEffect(() => {
    if (!active || !native || !current || current.kind !== 'image' || !snapshot.capabilities.imageOcr) { setQrCodes([]); return; }
    let disposed = false;
    const timer = window.setTimeout(() => {
      void invoke<string[]>('clipboard_image_qr_codes', { recordId: current.id })
        .then((codes) => { if (!disposed) setQrCodes(Array.isArray(codes) ? codes : []); })
        .catch(() => { if (!disposed) setQrCodes([]); });
    }, QR_PREVIEW_DWELL_MS);
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [active, native, current?.id, current?.kind, snapshot.capabilities.imageOcr, snapshot.window.recordsSession]);
  async function openCurrentTarget(): Promise<boolean> {
    if (!current || loading) return false;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try {
      await clipboardOpenTarget(current.id, true);
      return operation.current === revision;
    } catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t(clipboardOpenError(error)));
      return false;
    }
  }
  async function copyDerivedText(text: string) {
    if (!native || !current || !snapshot.capabilities.clipboardWrite) return false;
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
  async function openQrUrl(url: string): Promise<boolean> {
    if (!native || !snapshot.capabilities.clipboardOpenTarget) return false;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try {
      await invoke('clipboard_open_url', { url });
      return operation.current === revision;
    } catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t(clipboardOpenError(error)));
      return false;
    }
  }
  async function runQrCommand<T>(name: string, matrix: QrMatrix): Promise<{ ok: boolean; value?: T }> {
    if (!native || !current || loading) return { ok: false };
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try {
      const value = await invoke<T>(name, { size: matrix.size, modules: packQrModules(matrix) });
      return { ok: operation.current === revision, value };
    } catch {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t('clipboardQrActionFailed'));
      return { ok: false };
    }
  }
  async function copyQrImage(matrix: QrMatrix): Promise<boolean> {
    if (!snapshot.capabilities.clipboardWrite) return false;
    return (await runQrCommand('clipboard_qr_image_copy', matrix)).ok;
  }
  async function saveQrImage(matrix: QrMatrix): Promise<boolean> {
    if (!snapshot.capabilities.localBackup) return false;
    const result = await runQrCommand<boolean>('clipboard_qr_image_save', matrix);
    return result.ok && result.value !== false;
  }
  async function enlargeQrImage(matrix: QrMatrix): Promise<boolean> {
    if (!snapshot.capabilities.nativeChrome) return false;
    return (await runQrCommand('clipboard_qr_enlarge', matrix)).ok;
  }
  async function revealFile(index: number): Promise<boolean> {
    if (!current || loading) return false;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    setFeedback(null);
    try {
      await clipboardOpenTarget(current.id, true, index);
      return operation.current === revision;
    } catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision) setFeedback(t(clipboardOpenError(error)));
      return false;
    }
  }
  async function action(type: 'translate' | 'ocr' | 'favorite' | 'pin' | 'delete'): Promise<boolean> {
    const process = type === 'translate' || type === 'ocr';
    if (!current || loading || processing.current && (process || type === 'delete')) return false;
    const revision = operation.current;
    const feedbackRevision = ++feedbackOperation.current;
    if (process) { processing.current = true; setBusy(true); }
    setFeedback(null);
    try {
      if (process) await invoke('clipboard_record_process', { recordId: current.id, action: type });
      else await invoke('clipboard_record_update', { recordId: current.id, action: type });
      return operation.current === revision;
    } catch (error) {
      if (operation.current === revision && feedbackOperation.current === feedbackRevision && String(error) !== 'cancelled') {
        const message = process ? clipboardProcessError(error, type) : t('nativeError');
        if (process && message === null && current.kind === 'image') setRecognizedEmpty(true);
        else if (message) setFeedback(message);
      }
      return false;
    } finally {
      if (process && operation.current === revision) { processing.current = false; setBusy(false); }
    }
  }
  function requestDelete() {
    if (!current || loading || processing.current) return;
    if (current.isFavorite || current.isPinned) setConfirmDelete(true);
    else void action('delete');
  }
  useEffect(() => {
    if (!active || snapshot.window.recordsEditorOpen) return;
    const dismiss = (event: KeyboardEvent) => {
      if (confirmDelete || event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
      if (matchesBinding({ code: 'Escape', modifiers: [] }, event)) {
        event.preventDefault();
        if (event.repeat) return;
        if (snapshot.window.clipboardTranslationVisible) void command('clipboard_translation_close');
        else void command('window_hide', { region: 'clipboard' });
      }
    };
    window.addEventListener('keydown', dismiss);
    return () => window.removeEventListener('keydown', dismiss);
  }, [active, confirmDelete, snapshot.window.recordsEditorOpen, snapshot.window.clipboardTranslationVisible, command]);
  const canCopyDerived = native && current !== null && snapshot.capabilities.clipboardWrite;
  const qrInteractive = native && current !== null && !loading;
  const language = resolveLanguage(snapshot.locale);
  const mac = snapshot.capabilities.platform === 'macos';
  const openTargetAction = current && openTargetKind ? {
    kind: openTargetKind,
    label: openTargetKind === 'file' ? t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager')
      : openTargetKind === 'link' ? t('clipboardQrOpenLink') : t('clipboardShortcutOpenTarget'),
    tooltip: openTargetKind === 'email' ? t('clipboardOpenEmail') : undefined,
    onClick: () => openCurrentTarget(),
  } : undefined;
  const qrAction = canShowQr ? {
    active: showQr,
    label: t(showQr ? 'clipboardQrHide' : 'clipboardQrGenerate'),
    onClick: () => { setShowQr((value) => !value); return true; },
  } : undefined;
  return <section inert={snapshot.window.recordsEditorOpen} className="history-panel clipboard-detail" data-qr-view={(showQr && canShowQr) || undefined} aria-label="Clipboard details" aria-busy={loading}>
    <ActivityBeam label={t('working')} active={active && loading} />
    <header className="clipboard-detail-toolbar" data-tauri-drag-region>
      {hasRichPreview && <div className="clipboard-detail-mode" role="group" aria-label={t('clipboardRichPreviewMode')} data-tauri-drag-region="false">
        <button type="button" aria-pressed={!showQr && showingRich} data-tooltip={t('clipboardRichView')} data-tooltip-side="bottom" onClick={() => { setTextViewMode('rich'); setShowQr(false); }}>{t('clipboardRichView')}</button>
        <button type="button" aria-pressed={!showQr && !showingRich} data-tooltip={t('clipboardPlainView')} data-tooltip-side="bottom" onClick={() => { setTextViewMode('plain'); setShowQr(false); }}>{t('clipboardPlainView')}</button>
      </div>}
      {shown && <RecordActions recordId={shown.id} allowMarkedDelete t={t} native={native && current !== null && !loading} favorite={shown.isFavorite} pinned={shown.isPinned} deleteLabel={t('deleteClipboard')}
        tooltipSide="bottom" onOpenTarget={openTargetAction} onQrCode={qrAction}
        translateActive={snapshot.window.clipboardTranslationVisible && snapshot.window.clipboardTranslationRecordId === shown.id}
        onTranslate={canTranslate ? () => action('translate') : undefined} onOcr={canOcr ? () => action('ocr') : undefined}
        onNote={() => { if (current) void command('clipboard_note_open', { recordId: current.id }); }} onUpdate={(type) => { if (type === 'delete') { requestDelete(); return false; } return action(type); }} busy={busy} retainDelete />}
    </header>
    <HistoryList scrollOnly active={active} list={list} pinnedList={pinned} pinned={[]} onNearEnd={() => {}}>
      {current ? <div className="clipboard-detail-content" key={current.id}>
        {current.kind === 'text'
          ? (showQr && canShowQr
            ? <ClipboardQrPreview
              text={qrPayload}
              recordId={current.id}
              canCopy={qrInteractive && snapshot.capabilities.clipboardWrite}
              canSave={qrInteractive && snapshot.capabilities.localBackup}
              canEnlarge={qrInteractive && snapshot.capabilities.nativeChrome}
              onCopyImage={copyQrImage}
              onSaveImage={saveQrImage}
              onEnlarge={enlargeQrImage}
              t={t}
            />
            : showingRich && current.richTextHtml
              ? <ClipboardRichText html={current.richTextHtml} />
              : <>
                {smart?.kind === 'json' && <ClipboardJsonBar mode={jsonMode} onModeChange={setJsonMode} t={t} />}
                {smart?.kind === 'imageData' && <ClipboardImageDataBar imageData={smart} mode={imageDataMode} onModeChange={setImageDataMode} dimensions={imageDataDims} t={t} />}
                {smart?.kind === 'imageData' && imageDataMode === 'preview'
                  ? <ClipboardImageDataPreview imageData={smart} onDimensions={setImageDataDims} t={t} />
                  : <ClipboardTextPreview as="div" text={displayedText} className="clipboard-detail-text" data-tauri-drag-region="false" />}
                {smart?.kind === 'color' && <ClipboardColorCard color={smart} />}
                {smart?.kind === 'timestamp' && <ClipboardTimestampCard timestamp={smart} locale={language} t={t} />}
                {smart?.kind === 'datetime' && <ClipboardDateTimeCard dateTime={smart} locale={language} t={t} />}
                {smart?.kind === 'url' && <ClipboardUrlCard url={smart} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />}
                {smart?.kind === 'jwt' && <ClipboardJwtCard jwt={smart} locale={language} t={t} />}
                {smart?.kind === 'cron' && <ClipboardCronCard cron={smart} locale={language} t={t} />}
                {smart?.kind === 'math' && <ClipboardMathCard math={smart} t={t} />}
                {smart?.kind === 'number' && <ClipboardNumberCard number={smart} t={t} />}
                {smart?.kind === 'decoded' && <ClipboardDecodedCard decoded={smart} t={t} />}
              </>)
          : singleImage
            ? <>
              <ImagePreview key={current.id} recordId={current.id} native={native} active={active} onImageLoad={(img) => {
                if (paletteRecord.current === current.id) return;
                paletteRecord.current = current.id;
                setImagePalette(extractImagePalette(img));
              }} />
              <ClipboardImagePalette colors={imagePalette} canCopy={canCopyDerived} onCopy={copyDerivedText} t={t} />
              {current.kind === 'image' && <ClipboardImageQrBar codes={qrCodes} canCopy={canCopyDerived} canOpen={native && snapshot.capabilities.clipboardOpenTarget} onCopy={copyDerivedText} onOpenUrl={openQrUrl} t={t} />}
              {current.files.length > 0 && <ClipboardFilesCard entries={current.files} compact canCopy={canCopyDerived} onCopy={copyDerivedText}
                onReveal={snapshot.capabilities.clipboardOpenTarget ? (index) => revealFile(index) : undefined}
                revealLabel={t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager')} t={t} />}
            </>
            : <ClipboardFilesCard entries={current.files} canCopy={canCopyDerived} onCopy={copyDerivedText}
              onReveal={snapshot.capabilities.clipboardOpenTarget ? (index) => revealFile(index) : undefined}
              revealLabel={t(mac ? 'clipboardRevealFile' : 'clipboardRevealInFileManager')} t={t} />}
      </div> : <div className="history-empty" role={failed ? 'alert' : 'status'}>{failed ? t(failed === 'unavailable' ? 'clipboardDetailUnavailable' : 'clipboardDetailLoadError') : !id ? 'No record selected' : ''}{failed === 'retry' && <button type="button" data-tooltip={t('retry')} onClick={() => setAttempt((value) => value + 1)}>{t('retry')}</button>}</div>}
    </HistoryList>
    {current && <footer className="clipboard-detail-metadata">
      <div><span>Application</span><RecordHeading date={current.date} label={current.sourceApp?.name ?? 'Nami'} sourceApp={current.sourceApp} /></div>
      {current.width != null && current.height != null && <div><span>Dimensions</span><span>{current.width} × {current.height}</span></div>}
      {current.size != null && <div><span>Size</span><span>{formatSize(current.size)}</span></div>}
      {current.kind === 'text' && <div><span>Characters</span><span>{current.characterCount?.toLocaleString('en')}</span></div>}
      {current.kind === 'text' && current.richTextFormat && <div><span>{t('clipboardDetailFormat')}</span><span>{richFormatLabel(current.richTextFormat)}</span></div>}
    </footer>}
    {confirmDelete && current && <ClipboardDeleteConfirm t={t} description={markedDeleteDescription(t, current)} onCancel={() => setConfirmDelete(false)} onConfirm={() => { setConfirmDelete(false); void action('delete'); }} />}
    <RecordFeedback message={feedback} />
    <ActionTooltipHost />
  </section>;
}
