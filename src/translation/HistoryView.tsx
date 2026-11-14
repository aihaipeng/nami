import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { ViewProps } from '../App';
import { HistoryPanel } from './HistoryPanel';
import { HistoryDivider } from './HistoryDivider';
import type { HistorySelection } from './history';

export function HistoryView({ t, snapshot, command, native }: ViewProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    const remove = listen<HistorySelection>('history_sample_selected', ({ payload }) => {
      if (!disposed) setSelectedId(payload.sampleId);
    });
    return () => { disposed = true; void remove.then((unlisten) => unlisten()); };
  }, []);

  return <div className="history-view" aria-hidden={!snapshot.window.historyVisible} inert={!snapshot.window.historyVisible} onKeyDown={(event) => {
    if (event.defaultPrevented || event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape' || (event.metaKey && event.key.toLowerCase() === 'w')) {
      event.preventDefault(); void command('window_hide', { region: 'translation' });
    }
  }}>
    <HistoryPanel t={t} snapshot={snapshot} command={command} native={native} selectedId={selectedId}
      onSelect={(record, retranslate = false) => { void command('history_sample_select', { sampleId: record.id, retranslate }); }}
      onKeyDown={() => {}} />
    {snapshot.window.translationVisible && <HistoryDivider t={t} width={snapshot.window.historyWidth} command={command} />}
  </div>;
}
