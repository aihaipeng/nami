import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { historySamples, type HistorySnapshot } from './history';

export function useHistory(native: boolean) {
  const [snapshot, setSnapshot] = useState<HistorySnapshot>({ revision: -1, records: native ? [] : historySamples, selectedId: null });
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(native);
  const active = useRef(false);
  const revision = useRef(-1);

  function accept(next: HistorySnapshot) {
    if (!active.current || next.revision <= revision.current) return;
    revision.current = next.revision;
    setSnapshot(next);
    setLoading(false);
  }

  useEffect(() => {
    active.current = true;
    if (!native) return () => { active.current = false; };
    let disposed = false;
    let unlisten: (() => void) | undefined;
    async function connect() {
      const remove = await listen<HistorySnapshot>('history_changed', ({ payload }) => {
        if (!disposed) accept(payload);
      });
      if (disposed) { remove(); return; }
      unlisten = remove;
      const initial = await invoke<HistorySnapshot>('history_snapshot');
      if (!disposed) accept(initial);
    }
    void connect().catch(() => { if (!disposed) { setError(true); setLoading(false); } });
    return () => { disposed = true; active.current = false; unlisten?.(); };
  }, [native]);

  async function update(recordId: string, action: 'favorite' | 'pin' | 'delete') {
    if (!native) return;
    setError(false);
    try {
      const next = await invoke<HistorySnapshot>('history_record_update', { recordId, action });
      accept(next);
    } catch { if (active.current) setError(true); }
  }

  return { snapshot, error, loading, update };
}
