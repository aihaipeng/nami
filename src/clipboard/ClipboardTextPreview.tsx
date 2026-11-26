import type { CSSProperties } from 'react';
import { Link, Mail } from 'lucide-react';
import { recognizeClipboardText } from './contentRecognition';

export function ClipboardTextPreview({
  text,
  className,
  as = 'span',
  'data-tauri-drag-region': dragRegion,
}: {
  text: string;
  className: string;
  as?: 'span' | 'div';
  'data-tauri-drag-region'?: string;
}) {
  const { leading, trimmed, trailing, match } = recognizeClipboardText(text);
  const Tag = as;
  const kind = match.kind === 'plain' ? undefined : match.kind;

  if (match.kind === 'color') {
    return (
      <Tag className={className} data-content-kind={kind} data-tauri-drag-region={dragRegion}>
        {leading}
        <span
          className="clipboard-color-swatch"
          data-alpha={match.hasAlpha || undefined}
          style={{ '--swatch-color': match.cssColor } as CSSProperties}
          aria-hidden="true"
        />
        {trimmed}
        {trailing}
      </Tag>
    );
  }

  if (match.kind === 'link') {
    return (
      <Tag className={className} data-content-kind={kind} data-tauri-drag-region={dragRegion}>
        {leading}
        <Link className="clipboard-inline-icon" aria-hidden="true" />
        {match.prefix && <span className="clipboard-link-muted">{match.prefix}</span>}
        <span className="clipboard-link-host">{match.host}</span>
        {match.rest && <span className="clipboard-link-rest">{match.rest}</span>}
        {trailing}
      </Tag>
    );
  }

  if (match.kind === 'email') {
    return (
      <Tag className={className} data-content-kind={kind} data-tauri-drag-region={dragRegion}>
        {leading}
        <Mail className="clipboard-inline-icon" aria-hidden="true" />
        <span className="clipboard-link-host">{match.local}</span>
        <span className="clipboard-link-muted">@</span>
        <span>{match.domain}</span>
        {trailing}
      </Tag>
    );
  }

  return (
    <Tag className={className} data-content-kind={kind} data-tauri-drag-region={dragRegion}>
      {text}
    </Tag>
  );
}
