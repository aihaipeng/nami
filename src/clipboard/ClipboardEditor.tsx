import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ActivityBeam } from '../shared/ActivityBeam';

interface Detail { title: string; text: string; kind: 'text' | 'image' | 'files' }
export function ClipboardEditor({ recordId, onClose, onReturnFocus, onSaved, saveTitle, ocrLabel }: { recordId: string; ocrLabel?: string; saveTitle?: (title: string) => Promise<boolean>; onClose: () => void; onReturnFocus: () => void; onSaved?: (detail: Detail) => void }) {
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
    const load = ocrLabel
      ? invoke<string>('clipboard_record_ocr_text', { recordId }).then((text): Detail => ({ title: '', text, kind: 'image' }))
      : invoke<Detail>(saveTitle ? 'history_record_note' : 'clipboard_record_detail', { recordId });
    void load.then((value) => {
      if (!active) return;
      setDetail(value); setTitle(value.title); setText(value.text);
    }).catch(() => { if (active) setError(true); });
    return () => { active = false; };
  }, [recordId, attempt, Boolean(saveTitle), ocrLabel]);
  const focusField = ocrLabel ? 'textarea' : 'input';
  useEffect(() => { if (detail) card.current?.querySelector<HTMLElement>(focusField)?.focus({ preventScroll: true }); }, [detail, focusField]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; window.clearTimeout(timer.current); }; }, []);
  const dirty = detail !== null && (title !== detail.title || (ocrLabel || detail.kind === 'text') && text !== detail.text);
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
  useEffect(() => { (navigation ? confirmation.current?.querySelector('button') : card.current?.querySelector<HTMLElement>(focusField))?.focus({ preventScroll: true }); }, [navigation, focusField]);
  function finishNavigation() {
    onClose();
    void invoke('records_navigation_resolve', { proceed: true });
  }
  function cancelNavigation() {
    setNavigation(false);
    void invoke('records_navigation_resolve', { proceed: false });
    card.current?.querySelector<HTMLElement>(focusField)?.focus({ preventScroll: true });
  }
  function close(navigate = false) {
    if (saving || closing) return;
    setClosing(true);
    timer.current = window.setTimeout(() => { if (navigate) finishNavigation(); else { onClose(); onReturnFocus(); } }, window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 140);
  }
  const canSave = !ocrLabel || Boolean(text.trim()) && dirty;
  async function save(navigate = false) {
    if (!detail || saving || closing || !canSave) return;
    setSaving(true); setError(false);
    try {
      if (ocrLabel) await invoke('clipboard_record_update', { recordId, action: 'correct_ocr', text });
      else if (saveTitle) { if (!await saveTitle(title)) throw new Error('note_save'); }
      else await invoke('clipboard_record_update', { recordId, action: 'edit', title, ...(detail.kind === 'text' ? { text } : {}) });
      if (!mounted.current) return;
      dirtyRef.current = false;
      // Release the native navigation guard before the card stops intercepting clicks.
      await invoke('records_editor_set', { dirty: false, open: true });
      if (!mounted.current) return;
      onSaved?.({ title, text, kind: detail.kind });
      setClosing(true);
      timer.current = window.setTimeout(() => { if (navigate) finishNavigation(); else { onClose(); onReturnFocus(); } }, window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 140);
    } catch { if (mounted.current) setError(true); }
    finally { if (mounted.current) setSaving(false); }
  }
  const nonEditable = !ocrLabel && detail !== null && detail.kind !== 'text';
  const content = nonEditable ? 'Non-editable content' : text;
  return createPortal(<div className={`clipboard-editor-layer${entered && !closing ? ' open' : ''}${closing ? ' closing' : ''}`} onPointerDown={(event) => { if (event.target === event.currentTarget && !navigation) close(); }}>
    <form ref={card} className="floating-panel clipboard-editor" role="dialog" aria-modal="true" aria-label={ocrLabel || "Note"} tabIndex={-1}
      onSubmit={(event) => { event.preventDefault(); if (!navigation) void save(); }} onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); if (navigation) cancelNavigation(); else close(); }
        if (event.key === 'Enter' && !navigation && !event.shiftKey && !event.altKey && (ocrLabel ? (event.ctrlKey || event.metaKey) : (!event.ctrlKey && !event.metaKey))) {
          if (event.target instanceof HTMLButtonElement && event.target.type !== 'submit') return;
          event.preventDefault(); event.stopPropagation();
          if (!event.repeat) void save();
        }
        if (event.key === 'Tab') {
          const controls = [...(navigation ? confirmation.current! : event.currentTarget).querySelectorAll<HTMLElement>('input:not(:disabled),textarea:not(:disabled),button:not(:disabled)')];
          if (!controls.length) { event.preventDefault(); return; }
          const first = controls[0], last = controls.at(-1);
          if (event.shiftKey && (document.activeElement === first || document.activeElement === card.current)) { event.preventDefault(); last?.focus(); }
          else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
        }
      }}>
      <ActivityBeam label="Working" active={!detail && !error || saving} />
      <div aria-hidden={!detail || undefined} inert={!detail || navigation}>
        {!ocrLabel && <label><span className="clipboard-editor-label">Title</span><input aria-label="Title" maxLength={200} value={title} onChange={(event) => setTitle(event.target.value)} disabled={!detail || saving || closing} /></label>}
        <label><span className="clipboard-editor-label">{ocrLabel || "Content"}</span><textarea className={nonEditable ? 'non-editable' : undefined} aria-label={ocrLabel || "Content"} value={content} readOnly={!ocrLabel && (!!saveTitle || detail?.kind !== 'text')} onChange={(event) => { if (!nonEditable && !saveTitle) setText(event.target.value); }} disabled={nonEditable || !detail || saving || closing} /></label>
      </div>
      {error && !navigation && <p role="alert" className="clipboard-editor-error">{detail ? 'Could not save changes. Your edits are preserved.' : 'Could not load this record.'}{!detail && <button type="button" onClick={() => setAttempt((value) => value + 1)}>Retry</button>}</p>}
      <div className="clipboard-editor-actions" inert={navigation}><button type="button" onClick={() => close()} disabled={saving || closing} aria-keyshortcuts="Escape">Cancel<span className="keycaps" aria-hidden="true"><kbd>Esc</kbd></span></button><button type="submit" disabled={!canSave || !detail || saving || closing} aria-keyshortcuts={ocrLabel ? "Meta+Enter Control+Enter" : "Enter"}>Save<span className="keycaps" aria-hidden="true"><kbd>{ocrLabel ? "⌘↵" : "↵"}</kbd></span></button></div>
      {navigation && <div ref={confirmation} className="clipboard-navigation-confirm" role="alertdialog" aria-modal="true" aria-label="Save changes?">
        <strong>Save changes?</strong><p>{ocrLabel ? "Your corrected text has unsaved changes." : "Your note has unsaved changes."}</p>{error && <p role="alert" className="clipboard-editor-error">Could not save changes. Your edits are preserved.</p>}
        <div className="clipboard-editor-actions"><button type="button" disabled={saving || closing} onClick={cancelNavigation}>Cancel</button><button type="button" disabled={saving || closing} onClick={() => close(true)}>Discard</button><button type="button" disabled={!canSave || saving || closing} onClick={() => void save(true)}>Save</button></div>
      </div>}
    </form>
  </div>, document.body);
}
