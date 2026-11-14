import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type ChromeAction = 'source' | 'translate' | 'actions';
export interface ChromeState {
  sourceLabel: string;
  sourceWidth: number;
  translateLabel: string;
  actionsLabel: string;
  canTranslate: boolean;
  actionsVisible: boolean;
}

export function useNativeChrome(native: boolean, state: ChromeState, onAction: (action: ChromeAction) => void) {
  const [connected, setConnected] = useState(false);
  const [ready, setReady] = useState(false);
  const action = useRef(onAction);
  action.current = onAction;
  useEffect(() => {
    if (!native) return;
    let disposed = false;
    const remove = listen<ChromeAction>('translation_chrome_action', ({ payload }) => {
      if (!disposed && ['source', 'translate', 'actions'].includes(payload)) action.current(payload);
    });
    void remove.then(() => { if (!disposed) setConnected(true); }).catch(() => {});
    return () => { disposed = true; void remove.then((unlisten) => unlisten()).catch(() => {}); };
  }, [native]);
  const { sourceLabel, sourceWidth, translateLabel, actionsLabel, canTranslate, actionsVisible } = state;
  useEffect(() => {
    if (!connected) return;
    let disposed = false;
    void invoke<boolean>('translation_chrome_update', {
      state: { sourceLabel, sourceWidth, translateLabel, actionsLabel, canTranslate, actionsVisible },
    }).then((installed) => { if (!disposed && installed === true) setReady(true); }).catch(() => {});
    return () => { disposed = true; };
  }, [connected, sourceLabel, sourceWidth, translateLabel, actionsLabel, canTranslate, actionsVisible]);
  return ready;
}
