import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type ChromeAction = 'source' | 'translate';
export interface ChromeState {
  sourceLabel: string;
  sourceWidth: number;
  translateLabel: string;
  canTranslate: boolean;
}

export function useNativeChrome(native: boolean, state: ChromeState, onAction: (action: ChromeAction) => void) {
  const [connected, setConnected] = useState(false);
  const [ready, setReady] = useState(false);
  const action = useRef(onAction);
  action.current = onAction;
  useEffect(() => {
    if (!native) { setConnected(false); setReady(false); return; }
    let disposed = false;
    const remove = listen<ChromeAction>('translation_chrome_action', ({ payload }) => {
      if (!disposed && (payload === 'source' || payload === 'translate')) action.current(payload);
    });
    void remove.then(() => { if (!disposed) setConnected(true); }).catch(() => {});
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native]);
  const { sourceLabel, sourceWidth, translateLabel, canTranslate } = state;
  useEffect(() => {
    if (!native || !connected) return;
    let disposed = false;
    void invoke<boolean>('translation_chrome_update', {
      state: { sourceLabel, sourceWidth, translateLabel, canTranslate },
    }).then((installed) => { if (!disposed && installed === true) setReady(true); }).catch(() => {});
    return () => { disposed = true; };
  }, [native, connected, sourceLabel, sourceWidth, translateLabel, canTranslate]);
  return native && ready;
}
