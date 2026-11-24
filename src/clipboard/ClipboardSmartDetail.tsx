import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { CircleCheck, ExternalLink, FolderOpen, ScanQrCode } from 'lucide-react';
import type { Translate } from '../shared/i18n';
import { HistoryIcon } from '../translation/HistoryIcon';
import { FileIcon, type FileEntry } from './ClipboardMedia';
import { encodeQrCode } from './qrEncoder';
import {
  computeNextCronRuns,
  describeCron,
  formatCronRunTime,
  formatLocalTimestamp,
  formatRelativeTime,
  isOpenableQrLink,
  type SmartColor,
  type SmartCron,
  type SmartDateTime,
  type SmartDecoded,
  type SmartImageData,
  type SmartJson,
  type SmartJwt,
  type SmartMath,
  type SmartNumber,
  type SmartTimestamp,
  type SmartUrl,
} from './smartDetail';

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
        <span className="clipboard-smart-key" title={row.label}>{row.label}</span>
        <code className="clipboard-smart-value" title={row.value}>{row.value}</code>
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

export function ClipboardTimestampCard({ timestamp, locale, canCopy, onCopy, t }: {
  timestamp: SmartTimestamp;
  locale: 'zh-CN' | 'en';
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const rows = [
    { key: 'local', label: t('clipboardTimestampLocal'), value: timestamp.local },
    { key: 'iso', label: t('clipboardTimestampIso'), value: timestamp.iso },
  ];
  const relative = formatRelativeTime(timestamp.epochMs, locale);
  return <div className="clipboard-smart-card" data-tauri-drag-region="false">
    <div className="clipboard-smart-rows">
      {rows.map((row) => <div className="clipboard-smart-row" key={row.key}>
        <span className="clipboard-smart-key" title={row.label}>{row.label}</span>
        <code className="clipboard-smart-value" title={row.value}>{row.value}</code>
        <SmartCopyButton active={copied === row.key} disabled={!canCopy} label={`${t('copy')} ${row.label}`}
          onClick={() => { void copyValue(row.key, row.value, onCopy); }} />
      </div>)}
      <div className="clipboard-smart-row">
        <span className="clipboard-smart-key" title={t('clipboardTimestampRelative')}>{t('clipboardTimestampRelative')}</span>
        <span className="clipboard-smart-value clipboard-smart-value-plain" title={relative}>{relative}</span>
      </div>
    </div>
  </div>;
}

export function ClipboardDateTimeCard({ dateTime, locale, canCopy, onCopy, t }: {
  dateTime: SmartDateTime;
  locale: 'zh-CN' | 'en';
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const rows: Array<{ key: string; label: string; value: string }> = [];
  if (dateTime.showLocal) rows.push({ key: 'local', label: t('clipboardTimestampLocal'), value: dateTime.local });
  if (dateTime.showIso) rows.push({ key: 'iso', label: t('clipboardTimestampIso'), value: dateTime.iso });
  rows.push({ key: 'unix-s', label: t('clipboardUnixSeconds'), value: dateTime.unixSeconds });
  rows.push({ key: 'unix-ms', label: t('clipboardUnixMilliseconds'), value: dateTime.unixMilliseconds });
  const relative = formatRelativeTime(dateTime.epochMs, locale);
  return <div className="clipboard-smart-card" data-tauri-drag-region="false">
    <div className="clipboard-smart-rows">
      {rows.map((row) => <div className="clipboard-smart-row" key={row.key}>
        <span className="clipboard-smart-key" title={row.label}>{row.label}</span>
        <code className="clipboard-smart-value" title={row.value}>{row.value}</code>
        <SmartCopyButton active={copied === row.key} disabled={!canCopy} label={`${t('copy')} ${row.label}`}
          onClick={() => { void copyValue(row.key, row.value, onCopy); }} />
      </div>)}
      <div className="clipboard-smart-row">
        <span className="clipboard-smart-key" title={t('clipboardTimestampRelative')}>{t('clipboardTimestampRelative')}</span>
        <span className="clipboard-smart-value clipboard-smart-value-plain" title={relative}>{relative}</span>
      </div>
    </div>
  </div>;
}

export function ClipboardFilesCard({ entries, compact = false, canCopy, onCopy, onReveal, revealLabel, t }: {
  entries: ReadonlyArray<FileEntry>;
  compact?: boolean;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  onReveal?: (index: number) => void;
  revealLabel?: string;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  if (entries.length === 0) return null;
  if (compact && entries.length === 1) {
    const single = entries[0];
    return <div className="clipboard-smart-card clipboard-smart-files" data-tauri-drag-region="false">
      <div className="clipboard-smart-rows">
        <div className="clipboard-smart-row">
          <span className="clipboard-smart-key" title={t('clipboardFilePath')}>{t('clipboardFilePath')}</span>
          <code className="clipboard-smart-value" title={single.path}>{single.path}</code>
          <span className="clipboard-smart-actions">
            {onReveal && <button
              type="button"
              className="icon-button clipboard-smart-icon-button"
              data-action="reveal-file"
              aria-label={`${revealLabel}: ${single.name}`}
              title={revealLabel}
              disabled={!single.exists}
              onClick={() => onReveal(0)}
            >
              <FolderOpen aria-hidden="true" />
            </button>}
            <SmartCopyButton active={copied === 'file-0'} disabled={!canCopy} label={`${t('copy')} ${single.name}`}
              onClick={() => { void copyValue('file-0', single.path, onCopy); }} />
          </span>
        </div>
      </div>
    </div>;
  }
  return <div className="clipboard-smart-files-list" data-tauri-drag-region="false">
    {entries.map((entry, index) => {
      const key = `file-${index}`;
      return <div className="clipboard-smart-card clipboard-smart-files clipboard-smart-file-item" key={`${index}:${entry.path}`}>
        <div className="clipboard-smart-row clipboard-smart-file-head">
          <span className="clipboard-file" title={entry.path} data-missing={!entry.exists}>
            <FileIcon entry={entry} />
            <span className="clipboard-file-name">{entry.name}</span>
          </span>
          {onReveal && <button
            type="button"
            className="icon-button clipboard-smart-icon-button"
            data-action="reveal-file"
            aria-label={`${revealLabel}: ${entry.name}`}
            title={revealLabel}
            disabled={!entry.exists}
            onClick={() => onReveal(index)}
          >
            <FolderOpen aria-hidden="true" />
          </button>}
        </div>
        <div className="clipboard-smart-row clipboard-smart-row-wrap clipboard-smart-divider">
          <code className="clipboard-smart-value clipboard-smart-value-wrap" title={entry.path}>{entry.path}</code>
          <SmartCopyButton active={copied === key} disabled={!canCopy} label={`${t('copy')} ${entry.name}`}
            onClick={() => { void copyValue(key, entry.path, onCopy); }} />
        </div>
      </div>;
    })}
  </div>;
}

export function ClipboardUrlCard({ url, canCopy, onCopy, t }: {
  url: SmartUrl;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const hasTopRows = url.cleanUrl !== null || url.decodedUrl !== null;
  const remaining = url.totalParams - url.params.length;
  return <div className="clipboard-smart-card clipboard-smart-url" data-tauri-drag-region="false">
    <div className="clipboard-smart-rows">
      {hasTopRows && <div className="clipboard-smart-section">
        {url.cleanUrl !== null && <div className="clipboard-smart-row">
          <span className="clipboard-smart-key" title={t('clipboardUrlClean')}>{t('clipboardUrlClean')}</span>
          <code className="clipboard-smart-value" title={url.cleanUrl}>{url.cleanUrl}</code>
          <SmartCopyButton active={copied === 'clean'} disabled={!canCopy} label={`${t('copy')} ${t('clipboardUrlClean')}`}
            onClick={() => { void copyValue('clean', url.cleanUrl!, onCopy); }} />
        </div>}
        {url.decodedUrl !== null && <div className="clipboard-smart-row">
          <span className="clipboard-smart-key" title={t('clipboardUrlDecoded')}>{t('clipboardUrlDecoded')}</span>
          <code className="clipboard-smart-value" title={url.decodedUrl}>{url.decodedUrl}</code>
          <SmartCopyButton active={copied === 'decoded'} disabled={!canCopy} label={`${t('copy')} ${t('clipboardUrlDecoded')}`}
            onClick={() => { void copyValue('decoded', url.decodedUrl!, onCopy); }} />
        </div>}
      </div>}
      {url.params.length > 0 && <div className={`clipboard-smart-section${hasTopRows ? ' clipboard-smart-divider' : ''} clipboard-smart-params`}>
        {url.params.map((param, index) => {
          const key = `param-${index}`;
          return <div className="clipboard-smart-row" key={`${index}:${param.key}`}>
            <span className="clipboard-smart-key clipboard-smart-param-key" title={param.key}>{param.key}</span>
            <code className="clipboard-smart-value" title={param.value}>{param.value}</code>
            <SmartCopyButton active={copied === key} disabled={!canCopy} label={`${t('copy')} ${param.key}`}
              onClick={() => { void copyValue(key, param.value, onCopy); }} />
          </div>;
        })}
        {remaining > 0 && <div className="clipboard-smart-more">{t('clipboardUrlMoreParams').replace('{count}', String(remaining))}</div>}
      </div>}
    </div>
  </div>;
}

export function ClipboardJwtCard({ jwt, locale, canCopy, onCopy, t }: {
  jwt: SmartJwt;
  locale: 'zh-CN' | 'en';
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const [tab, setTab] = useState<'payload' | 'header'>('payload');
  useEffect(() => { setTab('payload'); }, [jwt]);

  const now = Date.now();
  const status = jwt.expiresAtMs !== null
    ? (jwt.expiresAtMs <= now ? 'expired' : jwt.notBeforeMs !== null && jwt.notBeforeMs > now ? 'pending' : 'valid')
    : jwt.notBeforeMs !== null && jwt.notBeforeMs > now ? 'pending' : null;
  const activeJson = tab === 'payload' ? jwt.payloadPretty : jwt.headerPretty;
  const activeLabel = tab === 'payload' ? t('clipboardJwtPayload') : t('clipboardJwtHeader');
  const metaRows: Array<{ key: string; label: string; value: string }> = [];
  if (jwt.algorithm) metaRows.push({ key: 'alg', label: 'ALG', value: jwt.algorithm });
  if (jwt.expiresAtMs !== null) {
    metaRows.push({
      key: 'exp',
      label: 'EXP',
      value: `${formatLocalTimestamp(new Date(jwt.expiresAtMs))} · ${formatRelativeTime(jwt.expiresAtMs, locale, now)}`,
    });
  }
  if (jwt.issuedAtMs !== null) metaRows.push({ key: 'iat', label: 'IAT', value: formatLocalTimestamp(new Date(jwt.issuedAtMs)) });
  if (jwt.notBeforeMs !== null) metaRows.push({ key: 'nbf', label: 'NBF', value: formatLocalTimestamp(new Date(jwt.notBeforeMs)) });

  return <div className="clipboard-smart-card clipboard-smart-jwt" data-tauri-drag-region="false">
    <div className="clipboard-smart-header">
      <div className="clipboard-detail-mode" role="group" aria-label="JWT">
        <button type="button" aria-pressed={tab === 'payload'} onClick={() => setTab('payload')}>{t('clipboardJwtPayload')}</button>
        <button type="button" aria-pressed={tab === 'header'} onClick={() => setTab('header')}>{t('clipboardJwtHeader')}</button>
      </div>
      {status && <span className={`clipboard-smart-badge clipboard-smart-badge-${status}`}>
        {t(status === 'expired' ? 'clipboardJwtExpired' : status === 'pending' ? 'clipboardJwtNotActive' : 'clipboardJwtValid')}
      </span>}
      <SmartCopyButton active={copied === tab} disabled={!canCopy} label={`${t('copy')} ${activeLabel}`}
        onClick={() => { void copyValue(tab, activeJson, onCopy); }} />
    </div>
    {metaRows.length > 0 && <div className="clipboard-smart-rows">
      {metaRows.map((row) => <div className="clipboard-smart-row" key={row.key}>
        <span className="clipboard-smart-key" title={row.label}>{row.label}</span>
        <code className="clipboard-smart-value" title={row.value}>{row.value}</code>
      </div>)}
    </div>}
    <pre className="clipboard-smart-code">{activeJson}</pre>
  </div>;
}

export function ClipboardDecodedCard({ decoded, canCopy, onCopy, t }: {
  decoded: SmartDecoded;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const [jsonView, setJsonView] = useState(decoded.jsonPretty !== null);
  useEffect(() => { setJsonView(decoded.jsonPretty !== null); }, [decoded]);

  const title = decoded.encoding === 'base64'
    ? t('clipboardDecodedBase64')
    : decoded.encoding === 'url'
      ? t('clipboardDecodedUrl')
      : t('clipboardDecodedEscape');
  const activeText = decoded.jsonPretty !== null && jsonView ? decoded.jsonPretty : decoded.text;
  const copyKey = decoded.jsonPretty !== null && jsonView ? 'json' : 'text';

  return <div className="clipboard-smart-card clipboard-smart-decoded" data-tauri-drag-region="false">
    <div className="clipboard-smart-header">
      <span className="clipboard-smart-heading">{title}</span>
      {decoded.jsonPretty !== null && <div className="clipboard-detail-mode" role="group" aria-label={title}>
        <button type="button" aria-pressed={jsonView} onClick={() => setJsonView(true)}>{t('clipboardJsonFormat')}</button>
        <button type="button" aria-pressed={!jsonView} onClick={() => setJsonView(false)}>{t('clipboardJsonOriginal')}</button>
      </div>}
      <SmartCopyButton active={copied === copyKey} disabled={!canCopy} label={`${t('copy')} ${title}`}
        onClick={() => { void copyValue(copyKey, activeText, onCopy); }} />
    </div>
    <pre className="clipboard-smart-code">{activeText}</pre>
  </div>;
}

export type ImageDataViewMode = 'preview' | 'source';

function formatEstimatedBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toLocaleString('en', { maximumFractionDigits: 1 })} KB`;
  return `${(bytes / (1024 * 1024)).toLocaleString('en', { maximumFractionDigits: 1 })} MB`;
}

export function ClipboardImageDataBar({ imageData, mode, onModeChange, dimensions, t }: {
  imageData: SmartImageData;
  mode: ImageDataViewMode;
  onModeChange: (mode: ImageDataViewMode) => void;
  dimensions: { width: number; height: number } | null;
  t: Translate;
}) {
  const meta = dimensions
    ? `${imageData.format} · ${dimensions.width} × ${dimensions.height} · ${formatEstimatedBytes(imageData.estimatedBytes)}`
    : `${imageData.format} · ${formatEstimatedBytes(imageData.estimatedBytes)}`;
  return <div className="clipboard-smart-bar" data-tauri-drag-region="false">
    <div className="clipboard-detail-mode" role="group" aria-label={imageData.format}>
      <button type="button" aria-pressed={mode === 'preview'} onClick={() => onModeChange('preview')}>{t('clipboardImagePreview')}</button>
      <button type="button" aria-pressed={mode === 'source'} onClick={() => onModeChange('source')}>{t('clipboardImageSource')}</button>
    </div>
    <span className="clipboard-smart-meta">{meta}</span>
  </div>;
}

export function ClipboardImageDataPreview({ imageData, onDimensions, t }: {
  imageData: SmartImageData;
  onDimensions?: (dims: { width: number; height: number } | null) => void;
  t: Translate;
}) {
  const [failed, setFailed] = useState(false);
  useEffect(() => { setFailed(false); }, [imageData.src]);
  return <div className="clipboard-smart-card clipboard-smart-image-card" data-tauri-drag-region="false">
    {failed
      ? <div className="clipboard-smart-image-error">{t('clipboardImagePreviewError')}</div>
      : <div className="clipboard-smart-image-stage">
        <img
          className="clipboard-smart-image-img"
          src={imageData.src}
          alt={`${imageData.format} ${t('clipboardImagePreview')}`}
          onLoad={(event) => {
            const { naturalWidth, naturalHeight } = event.currentTarget;
            onDimensions?.(naturalWidth > 0 && naturalHeight > 0 ? { width: naturalWidth, height: naturalHeight } : null);
          }}
          onError={() => {
            setFailed(true);
            onDimensions?.(null);
          }}
        />
      </div>}
  </div>;
}

export function ClipboardCronCard({ cron, locale, canCopy, onCopy, t }: {
  cron: SmartCron;
  locale: 'zh-CN' | 'en';
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const summary = useMemo(() => describeCron(cron, locale), [cron, locale]);
  const { runs, computedAt } = useMemo(() => {
    const ts = Date.now();
    return { runs: computeNextCronRuns(cron, ts, 3), computedAt: ts };
  }, [cron]);
  return <div className="clipboard-smart-card clipboard-smart-cron" data-tauri-drag-region="false">
    <div className="clipboard-smart-rows">
      <div className="clipboard-smart-row">
        <span className="clipboard-smart-key" title={t('clipboardCronSchedule')}>{t('clipboardCronSchedule')}</span>
        <span className="clipboard-smart-value clipboard-smart-value-plain" title={summary}>{summary}</span>
        <SmartCopyButton active={copied === 'summary'} disabled={!canCopy} label={`${t('copy')} ${t('clipboardCronSchedule')}`}
          onClick={() => { void copyValue('summary', summary, onCopy); }} />
      </div>
      {runs.length > 0 && <div className="clipboard-smart-section clipboard-smart-divider">
        {runs.map((run, index) => {
          const key = `run-${index}`;
          const label = t('clipboardCronNextRun').replace('{index}', String(index + 1));
          const formatted = formatCronRunTime(run);
          const display = index === 0 ? `${formatted} · ${formatRelativeTime(run.getTime(), locale, computedAt)}` : formatted;
          return <div className="clipboard-smart-row" key={key}>
            <span className="clipboard-smart-key" title={label}>{label}</span>
            <code className="clipboard-smart-value" title={display}>{display}</code>
            <SmartCopyButton active={copied === key} disabled={!canCopy} label={`${t('copy')} ${label}`}
              onClick={() => { void copyValue(key, formatted, onCopy); }} />
          </div>;
        })}
      </div>}
    </div>
  </div>;
}

export function ClipboardMathCard({ math, canCopy, onCopy, t }: {
  math: SmartMath;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const rows: Array<{ key: string; label: string; value: string }> = [
    { key: 'result', label: t('clipboardMathResult'), value: math.result },
  ];
  if (math.formatted !== null) {
    rows.push({ key: 'formatted', label: t('clipboardMathFormatted'), value: math.formatted });
  }
  return <div className="clipboard-smart-card clipboard-smart-math" data-tauri-drag-region="false">
    <div className="clipboard-smart-rows">
      {rows.map((row) => <div className="clipboard-smart-row" key={row.key}>
        <span className="clipboard-smart-key" title={row.label}>{row.label}</span>
        <code className="clipboard-smart-value" title={row.value}>{row.value}</code>
        <SmartCopyButton active={copied === row.key} disabled={!canCopy} label={`${t('copy')} ${row.label}`}
          onClick={() => { void copyValue(row.key, row.value, onCopy); }} />
      </div>)}
    </div>
  </div>;
}

export function ClipboardNumberCard({ number, canCopy, onCopy, t }: {
  number: SmartNumber;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  const baseRows = [
    { key: 'dec', label: 'DEC', value: number.dec },
    { key: 'hex', label: 'HEX', value: number.hex },
    { key: 'bin', label: 'BIN', value: number.bin },
    { key: 'oct', label: 'OCT', value: number.oct },
  ];
  const permRow = number.permissions !== null
    ? { key: 'perm', label: t('clipboardNumberPermissions'), value: number.permissions }
    : null;
  const rows = permRow
    ? (number.preferPermissionsFirst ? [permRow, ...baseRows] : [...baseRows, permRow])
    : baseRows;
  return <div className="clipboard-smart-card clipboard-smart-number" data-tauri-drag-region="false">
    <div className="clipboard-smart-rows">
      {rows.map((row) => <div className="clipboard-smart-row" key={row.key}>
        <span className="clipboard-smart-key" title={row.label}>{row.label}</span>
        <code className="clipboard-smart-value" title={row.value}>{row.value}</code>
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
