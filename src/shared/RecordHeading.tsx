import type { CSSProperties } from 'react';
import fallbackIcon from '../assets/history/reference-nami.png';

export function RecordHeading({ date: value, label, sourceApp }: {
  date: string;
  label: string;
  sourceApp?: { name: string; iconDataUrl?: string | null } | null;
}) {
  const date = new Date(value);
  const two = (part: number) => String(part).padStart(2, '0');
  const time = `${date.toDateString() === new Date().toDateString() ? '' : `${two(date.getMonth() + 1)}-${two(date.getDate())} `}${two(date.getHours())}:${two(date.getMinutes())}`;
  const name = sourceApp?.name ?? 'Nami';
  return <span className="history-row-heading">
    {sourceApp?.iconDataUrl
      ? <img className="history-origin" src={sourceApp.iconDataUrl} alt={name} title={name} />
      : <span className="history-origin history-symbol" role="img" aria-label={name} title={name} style={{ '--symbol-url': `url("${fallbackIcon}")`, borderRadius: 0, background: '#fff' } as CSSProperties} />}
    <span className="history-languages" title={label}>{label}</span>
    <time dateTime={value}>{time}</time>
  </span>;
}
