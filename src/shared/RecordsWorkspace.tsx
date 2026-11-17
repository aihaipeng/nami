import { useEffect } from 'react';
import type { ViewProps } from '../App';
import { Clipboard } from '../clipboard/Clipboard';
import { HistoryPanel } from '../translation/HistoryPanel';
import type { HistoryRecord } from '../translation/history';

export function RecordsWorkspace({ selectedId = null, onSelect, ...props }: ViewProps & {
  selectedId?: string | null;
  onSelect?: (record: HistoryRecord, retranslate?: boolean, preview?: boolean) => void;
}) {
  const { snapshot, command } = props;
  const view = snapshot.window.recordView ?? 'clipboard';
  const visible = snapshot.window.historyVisible;
  useEffect(() => {
    if (!visible || view !== 'translation') return;
    const dismiss = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
      if (event.key === 'Escape' || ((snapshot.capabilities.primaryModifier === 'meta' ? event.metaKey : event.ctrlKey) && event.key.toLowerCase() === 'w')) {
        event.preventDefault(); void command('window_hide', { region: 'translation' });
      }
    };
    window.addEventListener('keydown', dismiss);
    return () => window.removeEventListener('keydown', dismiss);
  }, [visible, view, snapshot.capabilities.primaryModifier, command]);
  return <div className="records-workspace" data-record-view={view}>
    <div className="record-view" hidden={view !== 'clipboard'} inert={!visible || view !== 'clipboard'}>
      <Clipboard {...props} active={visible && view === 'clipboard'} />
    </div>
    <div className="record-view" hidden={view !== 'translation'} inert={!visible || view !== 'translation'}>
      <HistoryPanel {...props} active={visible && view === 'translation'} selectedId={selectedId}
        onSelect={onSelect ?? ((record, retranslate = false, preview = false) => { void command('history_sample_select', { sampleId: record.id, retranslate, ...(preview ? { preview: true } : {}) }); })}
        onKeyDown={() => {}} />
    </div>
  </div>;
}
