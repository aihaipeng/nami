import { useId, useLayoutEffect, useRef } from 'react';
import type { Translate } from '../shared/i18n';

export function markedDeleteDescription(t: Translate, record: { isFavorite: boolean; isPinned: boolean }) {
  if (record.isPinned && record.isFavorite) return t('clipboardDeletePinnedFavorite');
  if (record.isPinned) return t('clipboardDeletePinned');
  return t('clipboardDeleteFavorite');
}

export function ClipboardDeleteConfirm({ t, description, onCancel, onConfirm }: {
  t: Translate;
  description: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const cancelRef = useRef(onCancel);
  const confirmRef = useRef(onConfirm);
  cancelRef.current = onCancel;
  confirmRef.current = onConfirm;
  useLayoutEffect(() => {
    const element = dialog.current!;
    const trigger = document.activeElement;
    element.showModal();
    element.focus({ preventScroll: true });
    const onKey = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key !== 'Escape' && event.key !== 'Enter') return;
      const modified = event.metaKey || event.ctrlKey || event.altKey || event.shiftKey;
      if (event.key === 'Enter' && !modified && !event.repeat && event.target instanceof HTMLButtonElement && element.contains(event.target)) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat || modified) return;
      if (event.key === 'Escape') cancelRef.current();
      else confirmRef.current();
    };
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      element.close();
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus({ preventScroll: true });
    };
  }, []);
  return <dialog ref={dialog} className="clipboard-delete-confirm" role="alertdialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId}
    onCancel={(event) => { event.preventDefault(); cancelRef.current(); }}>
    <strong id={titleId}>{t('clipboardDeleteMarkedTitle')}</strong>
    <p id={descriptionId}>{description}</p>
    <div className="clipboard-delete-confirm-actions">
      <button type="button" aria-keyshortcuts="Escape" onClick={onCancel}>{t('clipboardCancel')}<span className="keycaps" aria-hidden="true"><kbd>Esc</kbd></span></button>
      <button type="button" className="clipboard-delete-confirm-action" aria-keyshortcuts="Enter" onClick={onConfirm}>{t('clipboardDeleteConfirm')}<span className="keycaps" aria-hidden="true"><kbd>↵</kbd></span></button>
    </div>
  </dialog>;
}
