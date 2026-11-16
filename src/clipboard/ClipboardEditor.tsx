import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ActivityBeam } from '../shared/ActivityBeam';

interface Detail { title: string; text: string; kind: 'text' | 'image' | 'files' }
export function ClipboardEditor({ recordId, onClose, onReturnFocus, onSaved }: { recordId: string; onClose: () => void; onReturnFocus: () => void; onSaved: (detail: Detail) => void }) {
  const [detail, setDetail] = useState<Detail | null>(null);
  const [title, setTitle] = useState('');
  const [text, setText] = useState('');
  const [error, setError] = useState(false);
  const [saving, setSaving] = useState(false);
  const [entered, setEntered] = useState(false);
  const [closing, setClosing] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [navigation, setNavigation] = useState(false);
  const confirmation = useRef<HTMLDivElement>(null);
  const card = useRef<HTMLFormElement>(null);
  const mounted = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useLayoutEffect(() => {
    card.current?.getBoundingClientRect();
    const frame = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(frame);
  }, []);
  useEffect(() => { card.current?.focus({ preventScroll: true }); }, []);
  useEffect(() => {
    let active = true;
    setError(false);
    void invoke<Detail>('clipboard_record_detail', { recordId }).then((value) => {
      if (!active) return;
      setDetail(value); setTitle(value.title); setText(value.text);
    }).catch(() => { if (active) setError(true); });
    return () => { active = false; };
  }, [recordId, attempt]);
  useEffect(() => { if (detail) card.current?.querySelector('input')?.focus({ preventScroll: true }); }, [detail]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; window.clearTimeout(timer.current); }; }, []);
  const dirty = detail !== null && (title !== detail.title || detail.kind === 'text' && text !== detail.text);
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  useEffect(() => {
    void invoke('records_editor_set', { dirty, open: true }).catch(() => { if (mounted.current) setError(true); });
  }, [dirty]);
  useEffect(() => {
    let disposed = false;
    const subscription = listen('records_navigation_requested', () => {
      if (disposed) return;
      if (dirtyRef.current) setNavigation(true);
      else void invoke('records_navigation_resolve', { proceed: true });
    });
    return () => {
      disposed = true;
      void subscription.then((remove) => remove()).catch(() => {});
      void invoke('records_editor_set', { dirty: false, open: false }).catch(() => {});
    };
  }, []);
  useEffect(() => { (navigation ? confirmation.current?.querySelector('button') : card.current?.querySelector('input'))?.focus({ preventScroll: true }); }, [navigation]);
  function finishNavigation() {
    onClose();
    void invoke('records_navigation_resolve', { proceed: true });
  }
  function cancelNavigation() {
    setNavigation(false);
    void invoke('records_navigation_resolve', { proceed: false });
    card.current?.querySelector('input')?.focus({ preventScroll: true });
  }
  function close(navigate = false) {
    if (saving || closing) return;
    setClosing(true);
    timer.current = window.setTimeout(() => { if (navigate) finishNavigation(); else { onClose(); onReturnFocus(); } }, window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 140);
  }
  async function save(navigate = false) {
    if (!detail || saving || closing) return;
    setSaving(true); setError(false);
    try {
      await invoke('clipboard_record_update', { recordId, action: 'edit', title, ...(detail.kind === 'text' ? { text } : {}) });
      if (!mounted.current) return;
      onSaved({ title, text, kind: detail.kind });
      setClosing(true);
      timer.current = window.setTimeout(() => { if (navigate) finishNavigation(); else { onClose(); onReturnFocus(); } }, window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 140);
    } catch { if (mounted.current) setError(true); }
    finally { if (mounted.current) setSaving(false); }
  }
  const nonEditable = detail !== null && detail.kind !== 'text';
  const content = nonEditable ? 'Non-editable content' : text;
  return createPortal(<div className={`clipboard-editor-layer${entered && !closing ? ' open' : ''}${closing ? ' closing' : ''}`} onPointerDown={(event) => { if (event.target === event.currentTarget && !navigation) close(); }}>
    <form ref={card} className="floating-panel clipboard-editor" role="dialog" aria-modal="true" aria-label="Note" tabIndex={-1}
      onSubmit={(event) => { event.preventDefault(); void save(); }} onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); if (navigation) cancelNavigation(); else close(); }
        if (event.key === 'Tab') {
          const controls = [...(navigation ? confirmation.current! : event.currentTarget).querySelectorAll<HTMLElement>('input:not(:disabled),textarea:not(:disabled),button:not(:disabled)')];
          if (!controls.length) { event.preventDefault(); return; }
          const first = controls[0], last = controls.at(-1);
          if (event.shiftKey && (document.activeElement === first || document.activeElement === card.current)) { event.preventDefault(); last?.focus(); }
          else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
        }
      }}>
      {(!detail && !error || saving) && <ActivityBeam label="Working" />}
      <div aria-hidden={!detail || undefined} inert={!detail || navigation}>
        <label><span className="clipboard-editor-label">Title</span><input aria-label="Title" maxLength={200} value={title} onChange={(event) => setTitle(event.target.value)} disabled={!detail || saving || closing} /></label>
        <label><span className="clipboard-editor-label">Content</span><textarea className={nonEditable ? 'non-editable' : undefined} aria-label="Content" value={content} readOnly={detail?.kind !== 'text'} onChange={(event) => { if (!nonEditable) setText(event.target.value); }} disabled={nonEditable || !detail || saving || closing} /></label>
      </div>
      {error && !navigation && <p role="alert" className="clipboard-editor-error">{detail ? 'Could not save changes. Your edits are preserved.' : 'Could not load this record.'}{!detail && <button type="button" onClick={() => setAttempt((value) => value + 1)}>Retry</button>}</p>}
      <div className="clipboard-editor-actions" inert={navigation}><button type="button" onClick={() => close()} disabled={saving || closing}>Cancel</button><button type="submit" disabled={!detail || saving || closing}>Save</button></div>
      {navigation && <div ref={confirmation} className="clipboard-navigation-confirm" role="alertdialog" aria-modal="true" aria-label="Save changes?">
        <strong>Save changes?</strong><p>Your note has unsaved changes.</p>{error && <p role="alert" className="clipboard-editor-error">Could not save changes. Your edits are preserved.</p>}
        <div className="clipboard-editor-actions"><button type="button" disabled={saving || closing} onClick={cancelNavigation}>Cancel</button><button type="button" disabled={saving || closing} onClick={() => close(true)}>Discard</button><button type="button" disabled={saving || closing} onClick={() => void save(true)}>Save</button></div>
      </div>}
    </form>
  </div>, document.body);
}
