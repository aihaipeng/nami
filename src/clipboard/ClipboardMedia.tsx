import { useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { File, Folder, Image } from 'lucide-react';
import { ActivityBeam } from '../shared/ActivityBeam';
import type { ClipboardRecord } from './useClipboard';

export interface FileEntry { path: string; name: string; exists: boolean; isDir: boolean; isImage: boolean; iconDataUrl: string | null }
export const IMAGE_HIBERNATE_MS = 20_000;

export function mediaImageUrl(recordId: string, maxPixels: number, maxHeight?: number) {
  const base = typeof window !== 'undefined'
    && window.location.protocol === 'http:'
    && window.location.hostname.endsWith('.localhost')
    ? 'http://nami-media.localhost'
    : 'nami-media://localhost';
  const height = maxHeight ? `&h=${maxHeight}` : '';
  return `${base}/image/${encodeURIComponent(recordId)}?w=${maxPixels}${height}`;
}

function useVisible(enabled: boolean, trackVisibility = false) {
  const element = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!enabled) {
      setVisible(false);
      return;
    }
    if (typeof IntersectionObserver === 'undefined') { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => {
      const intersecting = entries.some((entry) => entry.isIntersecting);
      if (trackVisibility) setVisible(intersecting);
      else if (intersecting) { setVisible(true); observer.disconnect(); }
    });
    if (element.current) observer.observe(element.current);
    return () => observer.disconnect();
  }, [enabled, trackVisibility]);
  return { element, visible };
}

function previewPixelLimit() {
  const scale = Math.max(1, window.devicePixelRatio || 1);
  return Math.min(2048, Math.max(320, Math.ceil(window.innerWidth * scale / 256) * 256));
}

function previewHeightLimit(maxDisplayHeight?: number) {
  if (!maxDisplayHeight) return undefined;
  const scale = Math.max(1, window.devicePixelRatio || 1);
  return Math.min(2048, Math.max(32, Math.ceil(maxDisplayHeight * scale)));
}

export function ImagePreview({ recordId, native, active = true, maxDisplayHeight, hibernateMs = IMAGE_HIBERNATE_MS, fallback, onImageLoad }: {
  recordId: string;
  native: boolean;
  active?: boolean;
  maxDisplayHeight?: number;
  hibernateMs?: number;
  fallback?: ReactNode;
  onImageLoad?: (img: HTMLImageElement) => void;
}) {
  const [mediaActive, setMediaActive] = useState(active);
  useEffect(() => {
    if (active) {
      setMediaActive(true);
      return;
    }
    if (!mediaActive) return;
    const timer = window.setTimeout(() => setMediaActive(false), hibernateMs);
    return () => window.clearTimeout(timer);
  }, [active, mediaActive, hibernateMs]);

  const { element, visible } = useVisible(native && mediaActive, true);
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [maxPixels, setMaxPixels] = useState(previewPixelLimit);
  const [maxHeight, setMaxHeight] = useState(() => previewHeightLimit(maxDisplayHeight));

  useEffect(() => {
    setSrc(null);
    setFailed(false);
  }, [recordId]);

  useEffect(() => {
    if (!mediaActive) {
      setSrc(null);
      setFailed(false);
    }
  }, [mediaActive]);

  useEffect(() => {
    if (!visible || !mediaActive) return;
    const update = () => {
      setMaxPixels(previewPixelLimit());
      setMaxHeight(previewHeightLimit(maxDisplayHeight));
    };
    update();
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, [visible, mediaActive, maxDisplayHeight]);

  useEffect(() => {
    if (!native || !mediaActive || !visible) return;
    const next = mediaImageUrl(recordId, maxPixels, maxHeight);
    setFailed(false);
    setSrc((current) => (current === next ? current : next));
  }, [native, mediaActive, visible, recordId, maxPixels, maxHeight]);

  return <span ref={element} className="clipboard-image-preview">{src ? <img src={src} crossOrigin={onImageLoad ? 'anonymous' : undefined} decoding="async" alt="Image" onLoad={(event) => onImageLoad?.(event.currentTarget)} onError={() => { setSrc(null); setFailed(true); }} /> : failed ? fallback ?? <span className="clipboard-media-placeholder" role="img" aria-label="Image unavailable"><Image /></span> : <span className="clipboard-media-placeholder">{mediaActive && visible ? <ActivityBeam label="Working" /> : null}</span>}</span>;
}

export function FileIcon({ entry }: { entry: FileEntry }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => { setFailed(false); }, [entry.iconDataUrl]);
  return entry.iconDataUrl && !failed ? <img src={entry.iconDataUrl} decoding="async" alt="" onError={() => setFailed(true)} /> : entry.isDir ? <Folder aria-hidden="true" /> : <File aria-hidden="true" />;
}

export function FileList({ entries, onReveal, revealLabel }: { entries: FileEntry[]; onReveal?: (index: number) => void; revealLabel?: string }) {
  return <span className="clipboard-files">{entries.map((entry, index) => {
    const content = <><FileIcon entry={entry} /><span className="clipboard-file-name">{entry.name}</span></>;
    const props = { className: 'clipboard-file', title: entry.path, 'data-missing': !entry.exists };
    return onReveal ? <button key={index} type="button" className="clipboard-file" data-missing={!entry.exists} data-tooltip={revealLabel ?? entry.path} aria-label={`${revealLabel}: ${entry.name}`} disabled={!entry.exists} onClick={() => onReveal(index)}>{content}</button>
      : <span key={index} {...props}>{content}</span>;
  })}</span>;
}

type MediaProps = { record: ClipboardRecord; native: boolean; active: boolean };

export function isSingleImage(entries: FileEntry[]) {
  return entries.length === 1 && entries[0].exists && !entries[0].isDir && entries[0].isImage;
}

function FilesPreview({ record, native, active }: MediaProps) {
  const { element, visible } = useVisible(native && active);
  const [entries, setEntries] = useState<FileEntry[] | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!native || !visible || !active) return;
    let disposed = false;
    setFailed(false);
    void invoke<FileEntry[]>('clipboard_files', { recordId: record.id }).then((files) => {
      if (disposed) return;
      setEntries(files);
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; };
  }, [native, visible, active, record.id]);
  const files = entries && <FileList entries={entries} />;
  return <span ref={element} className="clipboard-media">{failed ? <span className="clipboard-media-placeholder" role="img" aria-label="Files unavailable"><File /></span> : !entries ? <span className="clipboard-media-placeholder">{active && visible ? <ActivityBeam label="Working" /> : null}</span> : isSingleImage(entries) ? <ImagePreview recordId={record.id} native={native} active={active} maxDisplayHeight={64} fallback={files} /> : files}</span>;
}

export function ClipboardMedia(props: MediaProps) {
  return props.record.kind === 'image' ? <ImagePreview recordId={props.record.id} native={props.native} active={props.active} maxDisplayHeight={64} /> : <FilesPreview {...props} />;
}
