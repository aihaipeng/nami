import { useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ActivityBeam } from '../shared/ActivityBeam';
import type { ClipboardRecord } from './useClipboard';

export function ClipboardNotePreview({ record, revealed, native, image }: { record: ClipboardRecord; revealed: boolean; native: boolean; image?: ReactNode }) {
  const [cached, setCached] = useState<{ record: ClipboardRecord; text: string } | null>(null);
  const [failed, setFailed] = useState<ClipboardRecord | null>(null);
  const ready = record.kind === 'image' || cached?.record === record || failed === record;
  const expanded = revealed && ready;
  useEffect(() => {
    if (!revealed) { setFailed(null); return; }
    if (!native || record.kind === 'image' || cached?.record === record) return;
    let active = true;
    setFailed(null);
    void invoke<{ text: string }>('clipboard_record_detail', { recordId: record.id }).then((detail) => {
      if (active) setCached({ record, text: detail.text });
    }).catch(() => { if (active) setFailed(record); });
    return () => { active = false; };
  }, [record, revealed, native, cached]);
  let text = cached?.record === record ? cached.text : '';
  if (record.kind === 'files' && text) {
    try { text = (JSON.parse(text) as string[]).join('\n'); } catch { /* Keep the original content visible. */ }
  }
  return <span className="clipboard-record-body" data-expanded={expanded}>
    <span className="clipboard-record-title" aria-hidden={expanded}>{record.title}</span>
    {revealed && !ready && <ActivityBeam label="Working" />}
    <span className="clipboard-record-content" aria-hidden={!expanded}><span className="clipboard-record-original">
      {failed === record ? <span role="alert">Could not load content. Hover again to retry.</span> : record.kind === 'image' ? image : <span className="history-preview">{text}</span>}
    </span></span>
  </span>;
}
