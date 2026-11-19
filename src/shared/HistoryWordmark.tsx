import type { CSSProperties } from 'react';
import wordmark from '../assets/history/reference/nami-wordmark.svg';

export function HistoryWordmark() {
  return <span className="history-wordmark history-wordmark-art" role="img" aria-label="Nami" data-tauri-drag-region
    style={{ '--wordmark-url': `url("${wordmark}")`, width: 44, height: 20 } as CSSProperties} />;
}
