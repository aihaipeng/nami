// Adapted from EcoPaste ClipboardQuickActions; modified for Nami. See licenses/EcoPaste.txt.
import type { ViewProps } from '../App';
import type { ComponentProps } from 'react';
import { ExternalLink, FolderOpen, ScanQrCode } from 'lucide-react';
import { AnimatePresence, motion, useIsPresent, useReducedMotion } from 'motion/react';
import { HistoryIcon } from '../translation/HistoryIcon';

export function RecordActions({ t, favorite, pinned, native, deleteLabel, onOpenTarget, onQrCode, onUpdate, onTranslate, translateActive = false, onOcr, onNote, busy = false, retainDelete = false, allowMarkedDelete = false, tabIndex }: Pick<ViewProps, 't' | 'native'> & {
  favorite: boolean; pinned: boolean; deleteLabel: string;
  onOpenTarget?: { kind: 'file' | 'link' | 'email'; label: string; onClick: () => void };
  onQrCode?: { active: boolean; label: string; onClick: () => void };
  onUpdate: (action: 'favorite' | 'pin' | 'delete') => void;
  onTranslate?: () => void; translateActive?: boolean; onOcr?: () => void; onNote?: () => void; busy?: boolean; retainDelete?: boolean; allowMarkedDelete?: boolean; tabIndex?: number;
}) {
  const reduceMotion = useReducedMotion();
  const protectedRecord = !allowMarkedDelete && (favorite || pinned);
  const animatedDelete = !retainDelete && !allowMarkedDelete;
  const staticActions = [
    ...(onOpenTarget ? [{ id: 'openTarget', label: onOpenTarget.label, customIcon: onOpenTarget.kind === 'file' ? <FolderOpen aria-hidden="true" /> : <ExternalLink aria-hidden="true" />, onClick: onOpenTarget.onClick }] : []),
    ...(onQrCode ? [{ id: 'qrcode', label: onQrCode.label, customIcon: <ScanQrCode aria-hidden="true" />, pressed: onQrCode.active, onClick: onQrCode.onClick }] : []),
    ...(onTranslate ? [{ id: 'translate', label: 'Translate', description: t('clipboardTranslatePrivacy'), icon: 'languages' as const, pressed: translateActive, disabled: busy, onClick: onTranslate }] : []),
    ...(onOcr ? [{ id: 'ocr', label: 'Extract text', description: t('clipboardOcrPrivacy'), icon: 'ocr' as const, disabled: busy, onClick: onOcr }] : []),
    ...(onNote ? [{ id: 'note', label: 'Note', icon: 'note' as const, onClick: onNote }] : []),
    { id: 'favorite', label: favorite ? t('unfavorite') : t('favorite'), icon: 'star' as const, pressed: favorite, onClick: () => onUpdate('favorite') },
    { id: 'pin', label: pinned ? t('unpinRecord') : t('pinRecord'), icon: pinned ? 'pin.fill' as const : 'pin' as const, pressed: pinned, onClick: () => onUpdate('pin') },
    ...(!animatedDelete && (!protectedRecord || retainDelete) ? [{ id: 'delete', label: deleteLabel, icon: 'trash' as const, disabled: protectedRecord || busy, onClick: () => onUpdate('delete') }] : []),
  ];
  const renderButton = (action: typeof staticActions[number]) => {
    const description = 'description' in action ? action.description : undefined;
    const activeToggle = (action.id === 'qrcode' || action.id === 'translate') && 'pressed' in action && action.pressed;
    return <ActionButton className={`icon-button${action.id === 'delete' ? ' history-delete' : action.id === 'favorite' && favorite ? ' record-favorite' : activeToggle ? ' record-active' : ''}`} title={description ? `${action.label} — ${description}` : action.label} aria-label={action.label} aria-description={description}
      tabIndex={tabIndex} aria-pressed={'pressed' in action ? action.pressed : undefined} disabled={!native || ('disabled' in action && action.disabled)} onClick={action.onClick}>{'customIcon' in action ? action.customIcon : <HistoryIcon name={action.icon} />}</ActionButton>;
  };
  const buttons = staticActions.map((action) => <span key={action.id} className="history-action" data-action={action.id}>{renderButton(action)}</span>);
  return <div className="history-row-actions" role="group" aria-label={t('actions')} onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()} onPointerDown={(event) => event.stopPropagation()} onMouseDown={(event) => {
    event.stopPropagation();
    if ((event.target as HTMLElement).closest('button')) event.preventDefault();
  }}>
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
  return <button {...props} disabled={props.disabled || !present} aria-hidden={!present} inert={!present} tabIndex={present ? props.tabIndex : -1} onClick={present ? props.onClick : undefined} />;
}
