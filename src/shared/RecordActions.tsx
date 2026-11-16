import type { ViewProps } from '../App';
import { HistoryIcon } from '../translation/HistoryIcon';

export function RecordActions({ t, favorite, pinned, native, canCopy, copyLabel, deleteLabel, onCopy, onUpdate }: Pick<ViewProps, 't' | 'native'> & {
  favorite: boolean; pinned: boolean; canCopy: boolean;
  copyLabel: string; deleteLabel: string;
  onCopy: () => void; onUpdate: (action: 'favorite' | 'pin' | 'delete') => void;
}) {
  return <div className="history-row-actions" role="group" aria-label={t('actions')}>
    <button className="icon-button" title={copyLabel} aria-label={copyLabel} disabled={!native || !canCopy} onClick={onCopy}><HistoryIcon name="doc.on.doc" /></button>
    <button className="icon-button" title={favorite ? t('unfavorite') : t('favorite')} aria-label={favorite ? t('unfavorite') : t('favorite')} aria-pressed={favorite} disabled={!native} onClick={() => onUpdate('favorite')}><HistoryIcon name={favorite ? 'star.fill' : 'star'} /></button>
    <button className="icon-button" title={pinned ? t('unpinRecord') : t('pinRecord')} aria-label={pinned ? t('unpinRecord') : t('pinRecord')} aria-pressed={pinned} disabled={!native} onClick={() => onUpdate('pin')}><HistoryIcon name={pinned ? 'pin.fill' : 'pin'} /></button>
    {!favorite && !pinned && <button className="icon-button history-delete" title={deleteLabel} aria-label={deleteLabel} disabled={!native} onClick={() => onUpdate('delete')}><HistoryIcon name="trash" /></button>}
  </div>;
}
