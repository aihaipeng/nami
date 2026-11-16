import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ClipboardRecord } from './useClipboard';

export function useClipboardImageText(records: ClipboardRecord[], enabled: boolean) {
  const [results, setResults] = useState(new Map<string, boolean>());
  const ids = JSON.stringify(records.filter((record) => record.kind === 'image' || record.kind === 'files').map(({ id, kind }) => [id, kind]));
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    void (async () => {
      for (const [recordId, kind] of JSON.parse(ids) as [string, string][]) {
        if (disposed) break;
        if (kind !== 'files' && results.has(recordId)) continue;
        try {
          const hasText = await invoke<boolean>('clipboard_record_has_text', { recordId });
          if (disposed) break;
          setResults((previous) => {
            const next = new Map(previous);
            next.set(recordId, hasText);
            if (next.size > 128) next.delete(next.keys().next().value!);
            return next;
          });
        } catch {
          if (!disposed && kind === 'files') setResults((previous) => {
            const next = new Map(previous);
            next.delete(recordId);
            return next;
          });
        }
      }
    })();
    return () => { disposed = true; };
  }, [ids, enabled]);
  return results;
}
