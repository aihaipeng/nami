// Adapted from EcoPaste NoteContentSwitcher; retains Nami's lazy content loading. See licenses/EcoPaste.txt.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { motion, useReducedMotion } from 'motion/react';
import { invoke } from '@tauri-apps/api/core';
import { ActivityBeam } from '../shared/ActivityBeam';
import type { ClipboardRecord } from './useClipboard';

export function ClipboardNotePreview({ record, revealed, native, media }: { record: ClipboardRecord; revealed: boolean; native: boolean; media?: ReactNode }) {
  const [cached, setCached] = useState<{ record: ClipboardRecord; text: string } | null>(null);
  const [failed, setFailed] = useState<ClipboardRecord | null>(null);
  const ready = record.kind !== 'text' || cached?.record === record || failed === record;
  const expanded = revealed && ready;
  const reduceMotion = useReducedMotion();
  const title = useRef<HTMLSpanElement>(null);
  const content = useRef<HTMLSpanElement>(null);
  const [height, setHeight] = useState<number | 'auto'>('auto');
  const transition = { duration: reduceMotion ? 0 : .18, ease: 'easeOut' } as const;
  useLayoutEffect(() => {
    const node = expanded ? content.current : title.current;
    if (!node) return;
    const measure = () => setHeight(node.getBoundingClientRect().height);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [expanded]);
  useEffect(() => {
    if (!revealed) { setFailed(null); return; }
    if (!native || record.kind !== 'text' || cached?.record === record) return;
    let active = true;
    setFailed(null);
    void invoke<{ text: string }>('clipboard_record_detail', { recordId: record.id }).then((detail) => {
      if (active) setCached({ record, text: detail.text });
    }).catch(() => { if (active) setFailed(record); });
    return () => { active = false; };
  }, [record, revealed, native, cached]);
  const text = cached?.record === record ? cached.text : '';
  return <motion.span className="clipboard-record-body" data-expanded={expanded} initial={false} animate={{ height }} transition={transition}>
    <motion.span ref={title} className="clipboard-record-title" aria-hidden={expanded} initial={false} animate={{ opacity: expanded ? 0 : 1 }} transition={transition}>{record.title}</motion.span>
    <ActivityBeam label="Working" active={revealed && !ready} />
    <motion.span ref={content} className="clipboard-record-content" aria-hidden={!expanded} initial={false} animate={{ opacity: expanded ? 1 : 0 }} transition={transition}><span className="clipboard-record-original">
      {failed === record ? <span role="alert">Could not load content. Hover again to retry.</span> : record.kind !== 'text' ? media : <span className="history-preview history-text-preview">{text}</span>}
    </span></motion.span>
  </motion.span>;
}
