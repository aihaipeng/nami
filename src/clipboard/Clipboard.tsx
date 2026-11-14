import { useRef, useState, type KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ViewProps } from '../App';
import type { ClipboardResult } from '../shared/contracts';
import { ClipboardList, Copy, List, Pin, Search, Star } from 'lucide-react';
import { clipboardSamples } from '../shared/input';
import type { MessageKey } from '../shared/i18n';
import './clipboard.css';

export function Clipboard({ t, snapshot, command, native, activity }: ViewProps) {
  const [query, setQuery] = useState('');
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [selected, setSelected] = useState<string>(clipboardSamples[0].id);
  const [feedback, setFeedback] = useState<MessageKey | null>(null);
  const operation = useRef(0);
  const rowRefs = useRef(new Map<string, HTMLButtonElement>());
  const search = useRef<HTMLInputElement>(null);
  const samples = favoritesOnly ? [] : clipboardSamples.filter((sample) => sample.text.toLocaleLowerCase().includes(query.toLocaleLowerCase()));

  async function useSample(sampleId: string, action: 'copy' | 'paste') {
    if (!native) return;
    const current = ++operation.current;
    activity(1);
    setSelected(sampleId);
    setFeedback(null);
    try {
      const result = await invoke<ClipboardResult>('clipboard_record_use', { sampleId, action });
      if (operation.current !== current) return;
      if (result.copyStatus === 'superseded') setFeedback('clipboardChanged');
      else if (result.status === 'dispatched') setFeedback('pasteDone');
      else if (result.status === 'copied') setFeedback('copyDone');
      else setFeedback(result.copyStatus === 'current' ? 'pasteManual' : 'copyFailed');
    } catch {
      if (operation.current === current) setFeedback('nativeError');
    } finally {
      activity(-1);
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'Escape' || (event.metaKey && event.key.toLowerCase() === 'w')) {
      event.preventDefault(); operation.current += 1;
      void command('window_hide', { region: 'clipboard' });
    }
  }

  function focusRow(id: string) {
    setSelected(id); rowRefs.current.get(id)?.focus();
  }

  return <section className="clipboard-panel" aria-label={t('clipboard')} onKeyDown={keydown}>
    <div className="drag-strip" data-tauri-drag-region />
    <header className="history-toolbar clipboard-toolbar">
      <label className="history-search clipboard-search"><Search aria-hidden="true" />
        <input ref={search} aria-label={t('searchClipboard')} placeholder={t('searchClipboard')} value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => {
          if (event.key === 'ArrowDown' && !event.nativeEvent.isComposing && samples[0]) { event.preventDefault(); focusRow(samples[0].id); }
        }} />
      </label>
      <button type="button" className="icon-button" aria-label="All clipboard records" aria-pressed={!favoritesOnly} onClick={() => setFavoritesOnly(false)}><List aria-hidden="true" /></button>
      <button type="button" className="icon-button" aria-label="Favorite clipboard records" aria-pressed={favoritesOnly} onClick={() => setFavoritesOnly(true)}><Star aria-hidden="true" /></button>
      <button type="button" className="icon-button" aria-label={snapshot.window.clipboardPinned ? t('unpin') : t('pin')} aria-pressed={snapshot.window.clipboardPinned} onClick={() => void command('window_pin', { region: 'clipboard', pinned: !snapshot.window.clipboardPinned })}><Pin aria-hidden="true" /></button>
    </header>
    <div className="clipboard-list" aria-label={t('sample')}>
      {samples.map((sample, index) => <article className={selected === sample.id ? 'clipboard-row selected' : 'clipboard-row'} key={sample.id}>
        <button type="button" ref={(element) => { if (element) rowRefs.current.set(sample.id, element); else rowRefs.current.delete(sample.id); }} className="record-body" disabled={!native} aria-label={`${t('sample')}: ${sample.text}`} onFocus={() => setSelected(sample.id)} onClick={() => void useSample(sample.id, 'paste')} onKeyDown={(event) => {
          if (event.key === 'ArrowDown' && samples[index + 1]) { event.preventDefault(); focusRow(samples[index + 1].id); }
          else if (event.key === 'ArrowUp') { event.preventDefault(); if (samples[index - 1]) focusRow(samples[index - 1].id); else search.current?.focus(); }
        }}>
          <span className="record-icon"><ClipboardList aria-hidden="true" /></span>
          <span className="record-content"><span className="record-meta">{t('text')}<span>{t('sample')}</span></span><span className="record-text">{sample.text}</span></span>
        </button>
        <button type="button" className="clipboard-icon-button record-copy" disabled={!native} aria-label={`${t('copy')}: ${sample.text}`} onClick={() => void useSample(sample.id, 'copy')}><Copy aria-hidden="true" /></button>
      </article>)}
      {!samples.length && <div className="clipboard-empty"><p>{favoritesOnly ? 'No favorites yet' : t('noMatches')}</p></div>}
    </div>
    {feedback && <div className="operation-feedback clipboard-feedback" role="status">{t(feedback)}</div>}
    <details className="sample-notice clipboard-notice"><summary>{t('sampleMode')}</summary><p>{t('sampleNotice')}</p></details>
  </section>;
}
