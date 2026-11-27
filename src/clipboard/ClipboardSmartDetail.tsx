import { useEffect, useId, useMemo, useState, type CSSProperties } from 'react';
import { CircleCheck, Copy, Download, ExternalLink, FolderOpen, ScanQrCode } from 'lucide-react';
import type { Translate } from '../shared/i18n';
import { useActionDone } from '../shared/useActionDone';
import { HistoryIcon } from '../translation/HistoryIcon';
import { FileIcon, type FileEntry } from './ClipboardMedia';
import { encodeQrCode, type QrMatrix } from './qrEncoder';
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
  type SmartJwt,
  type SmartMath,
  type SmartNumber,
  type SmartTimestamp,
  type SmartUrl,
} from './smartDetail';

function useCopiedKey() {
  const { doneKey, run } = useActionDone();
  return {
    copied: doneKey,
    copyValue: (key: string, value: string, onCopy: (text: string) => Promise<boolean>) =>
      run(key, () => onCopy(value), { lockWhileDone: true }),
    runAction: (key: string, action: () => boolean | void | Promise<boolean | void>) => run(key, action),
  };
}

function SmartCopyButton({ active, disabled, label, tooltip, onClick }: {
  active: boolean;
  disabled: boolean;
  label: string;
  tooltip: string;
  onClick: () => void;
}) {
  return <button
    type="button"
    className={`icon-button clipboard-smart-icon-button${active ? ' record-copied' : ''}`}
    disabled={disabled}
    aria-label={label}
    data-tooltip={tooltip}
    onClick={onClick}
  >
    {active ? <CircleCheck className="copy-success" aria-hidden="true" /> : <HistoryIcon name="doc.on.doc" />}
  </button>;
}

export function ClipboardColorCard({ color }: { color: SmartColor }) {
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
      </div>)}
    </div>
  </div>;
}

export type JsonViewMode = 'original' | 'pretty' | 'minified';

export function ClipboardJsonBar({ mode, onModeChange, t }: {
  mode: JsonViewMode;
  onModeChange: (mode: JsonViewMode) => void;
  t: Translate;
}) {
  return <div className="clipboard-smart-bar" data-tauri-drag-region="false">
    <div className="clipboard-detail-mode" role="group" aria-label="JSON">
      <button type="button" aria-pressed={mode === 'pretty'} data-tooltip={t('clipboardJsonFormat')} onClick={() => onModeChange('pretty')}>{t('clipboardJsonFormat')}</button>
      <button type="button" aria-pressed={mode === 'minified'} data-tooltip={t('clipboardJsonMinify')} onClick={() => onModeChange('minified')}>{t('clipboardJsonMinify')}</button>
      <button type="button" aria-pressed={mode === 'original'} data-tooltip={t('clipboardJsonOriginal')} onClick={() => onModeChange('original')}>{t('clipboardJsonOriginal')}</button>
    </div>
  </div>;
}

export function ClipboardTimestampCard({ timestamp, locale, t }: {
  timestamp: SmartTimestamp;
  locale: 'zh-CN' | 'en';
  t: Translate;
}) {
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
      </div>)}
      <div className="clipboard-smart-row">
        <span className="clipboard-smart-key" title={t('clipboardTimestampRelative')}>{t('clipboardTimestampRelative')}</span>
        <span className="clipboard-smart-value clipboard-smart-value-plain" title={relative}>{relative}</span>
      </div>
    </div>
  </div>;
}

export function ClipboardDateTimeCard({ dateTime, locale, t }: {
  dateTime: SmartDateTime;
  locale: 'zh-CN' | 'en';
  t: Translate;
}) {
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
      </div>)}
      <div className="clipboard-smart-row">
        <span className="clipboard-smart-key" title={t('clipboardTimestampRelative')}>{t('clipboardTimestampRelative')}</span>
        <span className="clipboard-smart-value clipboard-smart-value-plain" title={relative}>{relative}</span>
      </div>
    </div>
  </div>;
}

function FilePathIcon({ className }: { className?: string }) {
  const id = useId();
  const topOverlapClip = `${id}-top-overlap`;
  const upperRingMask = `${id}-upper-mask`;
  const lowerRingMask = `${id}-lower-mask`;
  return <svg
    viewBox="0 0 24 24"
    fill="none"
    className={className}
    data-icon="file-path"
    aria-hidden="true"
  >
    <defs>
      <clipPath id={topOverlapClip}>
        <rect x="7.5" y="4" width="9" height="8" />
      </clipPath>
      <mask id={upperRingMask}>
        <rect x="9.3" y="8.2" width="12.6" height="7.6" rx="3.8" stroke="#FFFFFF" strokeWidth="2.8" />
      </mask>
      <mask id={lowerRingMask}>
        <rect x="2.1" y="8.2" width="12.6" height="7.6" rx="3.8" stroke="#FFFFFF" strokeWidth="2.8" />
      </mask>
    </defs>
    <g transform="rotate(-45 12 12)">
      {/* Upper-right link (bright blue) */}
      <rect x="9.3" y="8.2" width="12.6" height="7.6" rx="3.8" stroke="#1A9DE2" strokeWidth="2.8" />
      {/* Contact shadow cast onto the upper-right link at the lower crossing */}
      <rect
        x="2.65"
        y="8.2"
        width="12.6"
        height="7.6"
        rx="3.8"
        stroke="#003E78"
        strokeOpacity="0.28"
        strokeWidth="3.2"
        mask={`url(#${upperRingMask})`}
      />
      {/* Lower-left link (deep blue) */}
      <rect x="2.1" y="8.2" width="12.6" height="7.6" rx="3.8" stroke="#0178D4" strokeWidth="2.8" />
      {/* Contact shadow cast onto the lower-left link at the upper crossing */}
      <g clipPath={`url(#${topOverlapClip})`}>
        <rect
          x="8.75"
          y="8.2"
          width="12.6"
          height="7.6"
          rx="3.8"
          stroke="#003E78"
          strokeOpacity="0.28"
          strokeWidth="3.2"
          mask={`url(#${lowerRingMask})`}
        />
        {/* Interlocking top weave: upper-right link passes over lower-left link */}
        <rect x="9.3" y="8.2" width="12.6" height="7.6" rx="3.8" stroke="#1A9DE2" strokeWidth="2.8" />
      </g>
    </g>
  </svg>;
}

export function ClipboardFilesCard({ entries, compact = false, canCopy, onCopy, onReveal, revealLabel, t }: {
  entries: ReadonlyArray<FileEntry>;
  compact?: boolean;
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  onReveal?: (index: number) => boolean | void | Promise<boolean | void>;
  revealLabel?: string;
  t: Translate;
}) {
  const { copied, copyValue, runAction } = useCopiedKey();
  if (entries.length === 0) return null;
  if (compact && entries.length === 1) {
    const single = entries[0];
    const revealDone = copied === 'reveal-0';
    return <div className="clipboard-smart-card clipboard-smart-files" data-tauri-drag-region="false">
      <div className="clipboard-smart-row clipboard-smart-row-wrap clipboard-smart-file-path" data-missing={!single.exists || undefined}>
        <FilePathIcon className="clipboard-smart-file-path-icon" />
        {single.exists
          ? <code className="clipboard-smart-value clipboard-smart-value-wrap" title={single.path}>{single.path}</code>
          : <span className="clipboard-smart-value clipboard-smart-value-plain clipboard-smart-value-wrap clipboard-smart-value-missing">{t('clipboardFilePathUnavailable')}</span>}
        <span className="clipboard-smart-actions">
          <SmartCopyButton
            active={copied === 'file-0'}
            disabled={!canCopy || !single.exists}
            label={`${t('copy')} ${single.name}`}
            tooltip={t('copy')}
            onClick={() => { void copyValue('file-0', single.path, onCopy); }}
          />
          {onReveal && <button
            type="button"
            className={`icon-button clipboard-smart-icon-button${revealDone ? ' record-copied' : ''}`}
            data-action="reveal-file"
            aria-label={`${revealLabel}: ${single.name}`}
            data-tooltip={revealLabel}
            disabled={!single.exists}
            onClick={() => { void runAction('reveal-0', () => onReveal(0)); }}
          >
            {revealDone ? <CircleCheck className="copy-success" aria-hidden="true" /> : <FolderOpen aria-hidden="true" />}
          </button>}
        </span>
      </div>
    </div>;
  }
  return <div className="clipboard-smart-files-list" data-tauri-drag-region="false">
    {entries.map((entry, index) => {
      const key = `file-${index}`;
      const revealKey = `reveal-${index}`;
      const revealDone = copied === revealKey;
      return <div className="clipboard-smart-card clipboard-smart-files clipboard-smart-file-item" key={`${index}:${entry.path}`}>
        <div className="clipboard-smart-row clipboard-smart-file-head">
          <span className="clipboard-file" title={entry.exists ? entry.path : undefined} data-missing={!entry.exists}>
            <FileIcon entry={entry} />
            <span className="clipboard-file-name">{entry.name}</span>
          </span>
          {onReveal && <button
            type="button"
            className={`icon-button clipboard-smart-icon-button${revealDone ? ' record-copied' : ''}`}
            data-action="reveal-file"
            aria-label={`${revealLabel}: ${entry.name}`}
            data-tooltip={revealLabel}
            disabled={!entry.exists}
            onClick={() => { void runAction(revealKey, () => onReveal(index)); }}
          >
            {revealDone ? <CircleCheck className="copy-success" aria-hidden="true" /> : <FolderOpen aria-hidden="true" />}
          </button>}
        </div>
        <div className="clipboard-smart-row clipboard-smart-row-wrap clipboard-smart-divider clipboard-smart-file-path" data-missing={!entry.exists || undefined}>
          <FilePathIcon className="clipboard-smart-file-path-icon" />
          {entry.exists
            ? <code className="clipboard-smart-value clipboard-smart-value-wrap" title={entry.path}>{entry.path}</code>
            : <span className="clipboard-smart-value clipboard-smart-value-plain clipboard-smart-value-wrap clipboard-smart-value-missing">{t('clipboardFilePathUnavailable')}</span>}
          <SmartCopyButton
            active={copied === key}
            disabled={!canCopy || !entry.exists}
            label={`${t('copy')} ${entry.name}`}
            tooltip={t('copy')}
            onClick={() => { void copyValue(key, entry.path, onCopy); }}
          />
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
          <SmartCopyButton active={copied === 'clean'} disabled={!canCopy} label={`${t('copy')} ${t('clipboardUrlClean')}`} tooltip={t('copy')}
            onClick={() => { void copyValue('clean', url.cleanUrl!, onCopy); }} />
        </div>}
        {url.decodedUrl !== null && <div className="clipboard-smart-row">
          <span className="clipboard-smart-key" title={t('clipboardUrlDecoded')}>{t('clipboardUrlDecoded')}</span>
          <code className="clipboard-smart-value" title={url.decodedUrl}>{url.decodedUrl}</code>
          <SmartCopyButton active={copied === 'decoded'} disabled={!canCopy} label={`${t('copy')} ${t('clipboardUrlDecoded')}`} tooltip={t('copy')}
            onClick={() => { void copyValue('decoded', url.decodedUrl!, onCopy); }} />
        </div>}
      </div>}
      {url.params.length > 0 && <div className={`clipboard-smart-section${hasTopRows ? ' clipboard-smart-divider' : ''} clipboard-smart-params`}>
        {url.params.map((param, index) => {
          const linkParam = isOpenableQrLink(param.value);
          const key = `param-${index}`;
          return <div className="clipboard-smart-row" key={`${index}:${param.key}`}>
            <span className="clipboard-smart-key clipboard-smart-param-key" title={param.key}>{param.key}</span>
            <code className="clipboard-smart-value" title={param.value}>{param.value}</code>
            {linkParam && <SmartCopyButton active={copied === key} disabled={!canCopy} label={`${t('copy')} ${param.key}`} tooltip={t('copy')}
              onClick={() => { void copyValue(key, param.value.trim(), onCopy); }} />}
          </div>;
        })}
        {remaining > 0 && <div className="clipboard-smart-more">{t('clipboardUrlMoreParams').replace('{count}', String(remaining))}</div>}
      </div>}
    </div>
  </div>;
}

export function ClipboardJwtCard({ jwt, locale, t }: {
  jwt: SmartJwt;
  locale: 'zh-CN' | 'en';
  t: Translate;
}) {
  const [tab, setTab] = useState<'payload' | 'header'>('payload');
  useEffect(() => { setTab('payload'); }, [jwt]);

  const now = Date.now();
  const status = jwt.expiresAtMs !== null
    ? (jwt.expiresAtMs <= now ? 'expired' : jwt.notBeforeMs !== null && jwt.notBeforeMs > now ? 'pending' : 'valid')
    : jwt.notBeforeMs !== null && jwt.notBeforeMs > now ? 'pending' : null;
  const activeJson = tab === 'payload' ? jwt.payloadPretty : jwt.headerPretty;
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
        <button type="button" aria-pressed={tab === 'payload'} data-tooltip={t('clipboardJwtPayload')} onClick={() => setTab('payload')}>{t('clipboardJwtPayload')}</button>
        <button type="button" aria-pressed={tab === 'header'} data-tooltip={t('clipboardJwtHeader')} onClick={() => setTab('header')}>{t('clipboardJwtHeader')}</button>
      </div>
      {status && <span className={`clipboard-smart-badge clipboard-smart-badge-${status}`}>
        {t(status === 'expired' ? 'clipboardJwtExpired' : status === 'pending' ? 'clipboardJwtNotActive' : 'clipboardJwtValid')}
      </span>}
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

export function ClipboardDecodedCard({ decoded, t }: {
  decoded: SmartDecoded;
  t: Translate;
}) {
  const [jsonView, setJsonView] = useState(decoded.jsonPretty !== null);
  useEffect(() => { setJsonView(decoded.jsonPretty !== null); }, [decoded]);

  const title = decoded.encoding === 'base64'
    ? t('clipboardDecodedBase64')
    : decoded.encoding === 'url'
      ? t('clipboardDecodedUrl')
      : t('clipboardDecodedEscape');
  const activeText = decoded.jsonPretty !== null && jsonView ? decoded.jsonPretty : decoded.text;

  return <div className="clipboard-smart-card clipboard-smart-decoded" data-tauri-drag-region="false">
    <div className="clipboard-smart-header">
      <span className="clipboard-smart-heading">{title}</span>
      {decoded.jsonPretty !== null && <div className="clipboard-detail-mode" role="group" aria-label={title}>
        <button type="button" aria-pressed={jsonView} data-tooltip={t('clipboardJsonFormat')} onClick={() => setJsonView(true)}>{t('clipboardJsonFormat')}</button>
        <button type="button" aria-pressed={!jsonView} data-tooltip={t('clipboardJsonOriginal')} onClick={() => setJsonView(false)}>{t('clipboardJsonOriginal')}</button>
      </div>}
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
      <button type="button" aria-pressed={mode === 'preview'} data-tooltip={t('clipboardImagePreview')} onClick={() => onModeChange('preview')}>{t('clipboardImagePreview')}</button>
      <button type="button" aria-pressed={mode === 'source'} data-tooltip={t('clipboardImageSource')} onClick={() => onModeChange('source')}>{t('clipboardImageSource')}</button>
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

export function ClipboardCronCard({ cron, locale, t }: {
  cron: SmartCron;
  locale: 'zh-CN' | 'en';
  t: Translate;
}) {
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
          </div>;
        })}
      </div>}
    </div>
  </div>;
}

export function ClipboardMathCard({ math, t }: {
  math: SmartMath;
  t: Translate;
}) {
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
      </div>)}
    </div>
  </div>;
}

export function ClipboardNumberCard({ number, t }: {
  number: SmartNumber;
  t: Translate;
}) {
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
      </div>)}
    </div>
  </div>;
}

export function ClipboardQrPreview({
  text,
  recordId,
  canCopy = false,
  canSave = false,
  canEnlarge = false,
  onCopyImage,
  onSaveImage,
  onEnlarge,
  t,
}: {
  text: string;
  recordId?: string;
  canCopy?: boolean;
  canSave?: boolean;
  canEnlarge?: boolean;
  onCopyImage?: (matrix: QrMatrix) => Promise<boolean>;
  onSaveImage?: (matrix: QrMatrix) => Promise<boolean>;
  onEnlarge?: (matrix: QrMatrix) => Promise<boolean>;
  t: Translate;
}) {
  const { doneKey, run } = useActionDone(`${recordId ?? ''}:${text}`);
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
  const previewSize = matrix.version >= 25 ? 300 : matrix.version >= 13 ? 260 : 210;
  const enlargeable = canEnlarge && Boolean(onEnlarge);
  const actionsVisible = Boolean(onCopyImage || onSaveImage);
  const qrSvg = <svg
    className="clipboard-qr-svg"
    style={{ '--qr-size': `${previewSize}px` } as CSSProperties}
    data-enlargeable={enlargeable || undefined}
    data-qr-version={matrix.version}
    data-qr-ec={matrix.ecLevel}
    viewBox={`${-quiet} ${-quiet} ${viewSize} ${viewSize}`}
    shapeRendering="crispEdges"
    role="img"
    aria-label="QR Code">
    <rect x={-quiet} y={-quiet} width={viewSize} height={viewSize} fill="#ffffff" />
    <path d={path} fill="#111111" />
  </svg>;
  return <div className="clipboard-qr-preview" data-tauri-drag-region="false">
    {enlargeable ? <button
      type="button"
      className="clipboard-qr-trigger"
      style={{ '--qr-size': `${previewSize}px` } as CSSProperties}
      aria-label={t('clipboardQrEnlarge')}
      onClick={() => { void onEnlarge!(matrix); }}>
      {qrSvg}
    </button> : qrSvg}
    {actionsVisible && <div className="clipboard-qr-actions" role="toolbar" aria-label={t('clipboardQrDetected')}>
      {onCopyImage && <button
        type="button"
        className={`icon-button clipboard-qr-action${doneKey === 'copy' ? ' record-copied' : ''}`}
        disabled={!canCopy}
        aria-label={t('clipboardQrCopyImage')}
        data-tooltip={t('clipboardQrCopyImage')}
        onClick={() => { void run('copy', () => onCopyImage(matrix), { lockWhileDone: true }); }}>
        {doneKey === 'copy' ? <CircleCheck className="copy-success" aria-hidden="true" /> : <Copy aria-hidden="true" />}
      </button>}
      {onSaveImage && <button
        type="button"
        className={`icon-button clipboard-qr-action${doneKey === 'save' ? ' record-copied' : ''}`}
        disabled={!canSave}
        aria-label={t('clipboardQrSaveImage')}
        data-tooltip={t('clipboardQrSaveImage')}
        onClick={() => { void run('save', () => onSaveImage(matrix)); }}>
        {doneKey === 'save' ? <CircleCheck className="copy-success" aria-hidden="true" /> : <Download aria-hidden="true" />}
      </button>}
    </div>}
  </div>;
}

export function ClipboardImagePalette({ colors, canCopy, onCopy, t }: {
  colors: string[];
  canCopy: boolean;
  onCopy: (text: string) => Promise<boolean>;
  t: Translate;
}) {
  const { copied, copyValue } = useCopiedKey();
  if (!colors.length) return null;
  return <div className="clipboard-smart-card clipboard-smart-palette" role="group" aria-label={t('clipboardImagePalette')} data-tauri-drag-region="false">
    <div className="clipboard-palette-grid">
      {colors.map((hex) => {
        const active = copied === hex;
        return <button
          key={hex}
          type="button"
          className={`clipboard-palette-swatch${active ? ' record-copied' : ''}`}
          disabled={!canCopy}
          aria-label={`${t('copy')} ${hex}`}
          data-tooltip={`${t('copy')} ${hex}`}
          onClick={() => { void copyValue(hex, hex, onCopy); }}
        >
          <span className="clipboard-palette-chip" style={{ '--swatch-color': hex } as CSSProperties} aria-hidden="true">
            {active && <span className="clipboard-palette-check"><CircleCheck className="copy-success" /></span>}
          </span>
          <code className="clipboard-palette-hex">{hex}</code>
        </button>;
      })}
    </div>
  </div>;
}

export function ClipboardImageQrBar({ codes, canCopy, canOpen, onCopy, onOpenUrl, t }: {
  codes: string[];
  canCopy: boolean;
  canOpen: boolean;
  onCopy: (text: string) => Promise<boolean>;
  onOpenUrl: (url: string) => Promise<boolean | void>;
  t: Translate;
}) {
  const { copied, copyValue, runAction } = useCopiedKey();
  if (!codes.length) return null;
  return <div className="clipboard-smart-card clipboard-smart-qr-list" data-tauri-drag-region="false">
    {codes.map((code, index) => {
      const isLink = isOpenableQrLink(code);
      const canOpenLink = canOpen && isLink;
      const key = `qr-${index}`;
      const openKey = `qr-open-${index}`;
      const openDone = copied === openKey;
      return <div className="clipboard-smart-row clipboard-smart-row-wrap" key={`${index}:${code}`}>
        <ScanQrCode className="clipboard-smart-icon" aria-hidden="true" />
        <code className="clipboard-smart-value clipboard-smart-value-wrap" title={code}>{code}</code>
        <span className="clipboard-smart-actions">
          <SmartCopyButton active={copied === key} disabled={!canCopy} label={`${t('copy')} ${t('clipboardQrDetected')}`} tooltip={t('copy')}
            onClick={() => { void copyValue(key, code.trim(), onCopy); }} />
          {canOpenLink && <button type="button" className={`icon-button clipboard-smart-icon-button${openDone ? ' record-copied' : ''}`}
            aria-label={t('clipboardQrOpenLink')} data-tooltip={t('clipboardQrOpenLink')}
            onClick={() => { void runAction(openKey, () => onOpenUrl(code.trim())); }}>
            {openDone ? <CircleCheck className="copy-success" aria-hidden="true" /> : <ExternalLink aria-hidden="true" />}
          </button>}
        </span>
      </div>;
    })}
  </div>;
}
