import { useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { File, Folder, Image } from 'lucide-react';
import { ActivityBeam } from '../shared/ActivityBeam';
import type { ClipboardRecord } from './useClipboard';

export interface FileEntry { path: string; name: string; exists: boolean; isDir: boolean; isImage: boolean; iconDataUrl: string | null }

function useVisible(native: boolean, trackVisibility = false) {
  const element = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!native) return;
    if (typeof IntersectionObserver === 'undefined') { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => {
      const intersecting = entries.some((entry) => entry.isIntersecting);
      if (trackVisibility) setVisible(intersecting);
      else if (intersecting) { setVisible(true); observer.disconnect(); }
    });
    if (element.current) observer.observe(element.current);
    return () => observer.disconnect();
  }, [native, trackVisibility]);
  return { element, visible };
}

function previewPixelLimit() {
  const scale = Math.max(1, window.devicePixelRatio || 1);
  return Math.min(2048, Math.max(320, Math.ceil(window.innerWidth * scale / 256) * 256));
}

export function ImagePreview({ recordId, native, fallback }: { recordId: string; native: boolean; fallback?: ReactNode }) {
  const { element, visible } = useVisible(native, true);
  const loaded = useRef<string | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [maxPixels, setMaxPixels] = useState(previewPixelLimit);
  useEffect(() => {
    if (!visible) return;
    const update = () => setMaxPixels(previewPixelLimit());
    update();
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, [visible]);
  useEffect(() => () => { if (url) URL.revokeObjectURL(url); }, [url]);
  useEffect(() => {
    if (!visible) return;
    const key = `${recordId}:${maxPixels}`;
    if (loaded.current === key) return;
    let disposed = false;
    setFailed(false);
    void invoke<{ bytes: number[]; mime: string }>('clipboard_image', { recordId, maxPixels }).then((image) => {
      if (disposed) return;
      loaded.current = key;
      setUrl(URL.createObjectURL(new Blob([new Uint8Array(image.bytes)], { type: image.mime })));
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; };
  }, [visible, recordId, maxPixels]);
  return <span ref={element} className="clipboard-image-preview">{url ? <img src={url} alt="Image" onError={() => { loaded.current = null; setUrl(null); setFailed(true); }} /> : failed ? fallback ?? <span className="clipboard-media-placeholder" role="img" aria-label="Image unavailable"><Image /></span> : <span className="clipboard-media-placeholder"><ActivityBeam label="Working" /></span>}</span>;
}

function FileIcon({ entry }: { entry: FileEntry }) {
  const [failed, setFailed] = useState(false);
  return entry.iconDataUrl && !failed ? <img src={entry.iconDataUrl} alt="" onError={() => setFailed(true)} /> : entry.isDir ? <Folder aria-hidden="true" /> : <File aria-hidden="true" />;
}

export function FileList({ entries }: { entries: FileEntry[] }) {
  return <span className="clipboard-files">{entries.map((entry) => <span key={entry.path} className="clipboard-file" title={entry.path} data-missing={!entry.exists}>
    <FileIcon entry={entry} />
    <span className="clipboard-file-name">{entry.name}</span>
  </span>)}</span>;
}

function FilesPreview({ record, native }: { record: ClipboardRecord; native: boolean }) {
  const { element, visible } = useVisible(native);
  const [entries, setEntries] = useState<FileEntry[] | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!visible || record.kind !== 'files') return;
    let active = true;
    setEntries(null); setFailed(false);
    void invoke<FileEntry[]>('clipboard_files', { recordId: record.id }).then((files) => { if (active) setEntries(files); }).catch(() => { if (active) setFailed(true); });
    return () => { active = false; };
  }, [visible, record.id, record.kind]);
  const files = entries && <FileList entries={entries} />;
  return <span ref={element} className="clipboard-media">{failed ? <span className="clipboard-media-placeholder" role="img" aria-label="Files unavailable"><File /></span> : !entries ? <span className="clipboard-media-placeholder"><ActivityBeam label="Working" /></span> : entries.length === 1 && entries[0].exists && entries[0].isImage ? <ImagePreview recordId={record.id} native={native} fallback={files} /> : files}</span>;
}

export function ClipboardMedia({ record, native }: { record: ClipboardRecord; native: boolean }) {
  return record.kind === 'image' ? <ImagePreview recordId={record.id} native={native} /> : <FilesPreview record={record} native={native} />;
}
