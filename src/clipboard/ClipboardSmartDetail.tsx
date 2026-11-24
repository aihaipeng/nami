import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { CircleCheck, ExternalLink, ScanQrCode } from 'lucide-react';
import type { Translate } from '../shared/i18n';
import { HistoryIcon } from '../translation/HistoryIcon';
import { encodeQrCode } from './qrEncoder';
import { isOpenableQrLink, type SmartColor, type SmartJson, type SmartTimestamp } from './smartDetail';

function useCopiedKey() {
  const [copied, setCopied] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const copying = useRef<string | null>(null);
  const generation = useRef(0);
  useEffect(() => () => { ++generation.current; clearTimeout(timer.current); }, []);
  async function copyValue(key: string, value: string, onCopy: (text: string) => Promise<boolean>) {
    if (copying.current === key || copied === key) return;
    copying.current = key;
    const current = generation.current;
    try {
      if (await onCopy(value) && current === generation.current) {
        setCopied(key);
        clearTimeout(timer.current);
        timer.current = setTimeout(() => { setCopied((value) => value === key ? null : value); }, 1000);
      }
    } finally {
      if (current === generation.current && copying.current === key) copying.current = null;
    }
  }
  return { copied, copyValue };
}

function SmartCopyButton({ active, disabled, label, onClick }: {
  active: boolean;
  disabled: boolean;
  label: string;
  onClick: () => void;
}) {
  return <button
    type="button"
    className={`icon-button clipboard-smart-icon-button${active ? ' record-copied' : ''}`}
    disabled={disabled}
    aria-label={label}
    title={label}
    onClick={onClick}
  >
    {active ? <CircleCheck className="copy-success" aria-hidden="true" /> : <HistoryIcon name="doc.on.doc" />}
  </button>;
}

export function ClipboardColorCard({ color, canCopy, onCopy, t }: {
  color: SmartColor;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const rows = [
    { key: 'hex', label: 'HEX', value: color.hex },
    { key: 'rgb', label: 'RGB', value: color.rgb },
    { key: 'hsl', label: 'HSL', value: color.hsl },
  ];
  return <div className="clipboard-smart-card clipboard-smart-color" data-tauri-drag-region="false">
    <div className="clipboard-smart-color-preview" data-alpha={color.hasAlpha || undefined} style={{ '--swatch-color': color.cssColor } as CSSProperties} aria-hidden="true" />
    <div className="clipboard-smart-rows">
      {rows.map((row) => <div className="clipboard-smart-row" key={row.key}>
        <span className="clipboard-smart-key">{row.label}</span>
        <code className="clipboard-smart-value">{row.value}</code>
        <SmartCopyButton active={copied === row.key} disabled={!canCopy} label={`${t('copy')} ${row.label}`}
          onClick={() => { void copyValue(row.key, row.value, onCopy); }} />
      </div>)}
    </div>
  </div>;
}

export type JsonViewMode = 'original' | 'pretty' | 'minified';

export function ClipboardJsonBar({ json, mode, onModeChange, canCopy, onCopy, t }: {
  json: SmartJson;
  mode: JsonViewMode;
  onModeChange: (mode: JsonViewMode) => void;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const activeText = mode === 'pretty' ? json.pretty : mode === 'minified' ? json.minified : null;
  return <div className="clipboard-smart-bar" data-tauri-drag-region="false">
    <div className="clipboard-detail-mode" role="group" aria-label="JSON">
      <button type="button" aria-pressed={mode === 'pretty'} onClick={() => onModeChange('pretty')}>{t('clipboardJsonFormat')}</button>
      <button type="button" aria-pressed={mode === 'minified'} onClick={() => onModeChange('minified')}>{t('clipboardJsonMinify')}</button>
      <button type="button" aria-pressed={mode === 'original'} onClick={() => onModeChange('original')}>{t('clipboardJsonOriginal')}</button>
    </div>
    {activeText !== null && <SmartCopyButton active={copied === mode} disabled={!canCopy} label={`${t('copy')} JSON`}
      onClick={() => { void copyValue(mode, activeText, onCopy); }} />}
  </div>;
}

export function ClipboardTimestampCard({ timestamp, canCopy, onCopy, t }: {
  timestamp: SmartTimestamp;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const rows = [
    { key: 'local', label: t('clipboardTimestampLocal'), value: timestamp.local },
    { key: 'iso', label: t('clipboardTimestampIso'), value: timestamp.iso },
  ];
  return <div className="clipboard-smart-card" data-tauri-drag-region="false">
    <div className="clipboard-smart-rows">
      {rows.map((row) => <div className="clipboard-smart-row" key={row.key}>
        <span className="clipboard-smart-key">{row.label}</span>
        <code className="clipboard-smart-value">{row.value}</code>
        <SmartCopyButton active={copied === row.key} disabled={!canCopy} label={`${t('copy')} ${row.label}`}
          onClick={() => { void copyValue(row.key, row.value, onCopy); }} />
      </div>)}
    </div>
  </div>;
}

export function ClipboardQrPreview({ text }: { text: string }) {
  const matrix = useMemo(() => encodeQrCode(text.trim()), [text]);
  const path = useMemo(() => {
    if (!matrix) return '';
    const segments: string[] = [];
    for (let r = 0; r < matrix.size; r += 1) {
      let c = 0;
      while (c < matrix.size) {
        if (!matrix.modules[r * matrix.size + c]) { c += 1; continue; }
        const start = c;
        while (c < matrix.size && matrix.modules[r * matrix.size + c]) c += 1;
        segments.push(`M${start} ${r}h${c - start}v1h-${c - start}z`);
      }
    }
    return segments.join('');
  }, [matrix]);
  if (!matrix) return null;
  const quiet = 4;
  const viewSize = matrix.size + quiet * 2;
  return <div className="clipboard-qr-preview" data-tauri-drag-region="false">
    <svg className="clipboard-qr-svg" viewBox={`${-quiet} ${-quiet} ${viewSize} ${viewSize}`}
      shapeRendering="crispEdges" role="img" aria-label="QR Code">
      <rect x={-quiet} y={-quiet} width={viewSize} height={viewSize} fill="#ffffff" />
      <path d={path} fill="#111111" />
    </svg>
  </div>;
}

export function ClipboardImageQrBar({ codes, canCopy, canOpen, onCopy, onOpenUrl, t }: {
  codes: string[];
  canCopy: boolean;
  canOpen: boolean;
  onCopy: (text: string) => Promise<boolean>;
  onOpenUrl: (url: string) => Promise<void>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  if (!codes.length) return null;
  return <div className="clipboard-smart-card clipboard-smart-qr-list" data-tauri-drag-region="false">
    {codes.map((code, index) => {
      const link = canOpen && isOpenableQrLink(code);
      const key = `qr-${index}`;
      return <div className="clipboard-smart-row" key={`${index}:${code}`}>
        <ScanQrCode className="clipboard-smart-icon" aria-hidden="true" />
        <code className="clipboard-smart-value clipboard-smart-value-wrap" title={code}>{code}</code>
        <SmartCopyButton active={copied === key} disabled={!canCopy} label={`${t('copy')} ${t('clipboardQrDetected')}`}
          onClick={() => { void copyValue(key, code, onCopy); }} />
        {link && <button type="button" className="icon-button clipboard-smart-icon-button"
          aria-label={t('clipboardQrOpenLink')} title={t('clipboardQrOpenLink')}
          onClick={() => { void onOpenUrl(code.trim()); }}>
          <ExternalLink aria-hidden="true" />
        </button>}
      </div>;
    })}
  </div>;
}
