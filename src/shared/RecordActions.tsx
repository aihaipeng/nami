// Adapted from EcoPaste ClipboardQuickActions; modified for Nami. See licenses/EcoPaste.txt.
import type { ViewProps } from '../App';
import { useEffect, useRef, useState, type ComponentProps } from 'react';
import { CircleCheck } from 'lucide-react';
import { AnimatePresence, motion, useIsPresent, useReducedMotion } from 'motion/react';
import { HistoryIcon } from '../translation/HistoryIcon';

export function RecordActions({ t, favorite, pinned, native, canCopy, copyLabel, deleteLabel, onCopy, onUpdate, onTranslate, onOcr, onNote, busy = false, retainDelete = false, allowMarkedDelete = false, recordId }: Pick<ViewProps, 't' | 'native'> & {
  favorite: boolean; pinned: boolean; canCopy?: boolean;
  copyLabel?: string; deleteLabel: string;
  onCopy?: () => void | boolean | Promise<boolean>; onUpdate: (action: 'favorite' | 'pin' | 'delete') => void;
  onTranslate?: () => void; onOcr?: () => void; onNote?: () => void; busy?: boolean; retainDelete?: boolean; allowMarkedDelete?: boolean; recordId?: string;
}) {
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const copyGeneration = useRef(0);
  const copying = useRef(false);
  useEffect(() => {
    ++copyGeneration.current; copying.current = false; setCopied(false);
    return () => { ++copyGeneration.current; clearTimeout(copyTimer.current); };
  }, [recordId]);
  async function copy() {
    if (copying.current || copied) return;
    copying.current = true;
    const generation = copyGeneration.current;
    try {
      if (await onCopy?.() === true && generation === copyGeneration.current) {
        setCopied(true);
        copyTimer.current = setTimeout(() => setCopied(false), 1000);
      }
    } finally { if (generation === copyGeneration.current) copying.current = false; }
  }
  const reduceMotion = useReducedMotion();
  const protectedRecord = !allowMarkedDelete && (favorite || pinned);
  const animatedDelete = !retainDelete && !allowMarkedDelete;
  const staticActions = [
    ...(onCopy ? [{ id: 'copy', label: copyLabel ?? t('copy'), icon: 'doc.on.doc' as const, disabled: !canCopy, onClick: () => { void copy(); } }] : []),
    ...(onTranslate ? [{ id: 'translate', label: 'Translate', description: t('clipboardTranslatePrivacy'), icon: 'languages' as const, disabled: busy, onClick: onTranslate }] : []),
    ...(onOcr ? [{ id: 'ocr', label: 'Extract text', description: t('clipboardOcrPrivacy'), icon: 'ocr' as const, disabled: busy, onClick: onOcr }] : []),
    ...(onNote ? [{ id: 'note', label: 'Note', icon: 'note' as const, onClick: onNote }] : []),
    { id: 'favorite', label: favorite ? t('unfavorite') : t('favorite'), icon: 'star' as const, pressed: favorite, onClick: () => onUpdate('favorite') },
    { id: 'pin', label: pinned ? t('unpinRecord') : t('pinRecord'), icon: pinned ? 'pin.fill' as const : 'pin' as const, pressed: pinned, onClick: () => onUpdate('pin') },
    ...(!animatedDelete && (!protectedRecord || retainDelete) ? [{ id: 'delete', label: deleteLabel, icon: 'trash' as const, disabled: protectedRecord || busy, onClick: () => onUpdate('delete') }] : []),
  ];
  const renderButton = (action: typeof staticActions[number]) => {
    const description = 'description' in action ? action.description : undefined;
    return <ActionButton className={`icon-button${action.id === 'delete' ? ' history-delete' : action.id === 'favorite' && favorite ? ' record-favorite' : action.id === 'copy' && copied ? ' record-copied' : ''}`} title={description ? `${action.label} — ${description}` : action.label} aria-label={action.label} aria-description={description}
      aria-pressed={'pressed' in action ? action.pressed : undefined} disabled={!native || ('disabled' in action && action.disabled)} onClick={action.onClick}>{action.id === 'copy' && copied ? <CircleCheck className="copy-success" aria-hidden="true" /> : <HistoryIcon name={action.icon} />}</ActionButton>;
  };
  const buttons = staticActions.map((action) => <span key={action.id} className="history-action" data-action={action.id}>{renderButton(action)}</span>);
  return <div className="history-row-actions" role="group" aria-label={t('actions')} onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onPointerDown={(event) => event.stopPropagation()} onMouseDown={(event) => event.stopPropagation()}>
    {buttons}
    {animatedDelete && <AnimatePresence initial={false}>
      {!protectedRecord && <motion.span key="delete" className="history-action" data-action="delete"
        initial={reduceMotion ? false : { opacity: 0, scale: .9, width: 0, x: 4 }}
        animate={{ opacity: 1, scale: 1, width: 24, x: 0 }}
        exit={reduceMotion ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, scale: .9, width: 0, x: 4 }}
        transition={{ duration: reduceMotion ? 0 : .16, ease: 'easeOut' }}>{renderButton({ id: 'delete', label: deleteLabel, icon: 'trash', disabled: protectedRecord || busy, onClick: () => onUpdate('delete') })}</motion.span>}
    </AnimatePresence>}
  </div>;
}

function ActionButton(props: ComponentProps<'button'>) {
  const present = useIsPresent();
  return <button {...props} disabled={props.disabled || !present} aria-hidden={!present} inert={!present} tabIndex={present ? undefined : -1} onClick={present ? props.onClick : undefined} />;
}
