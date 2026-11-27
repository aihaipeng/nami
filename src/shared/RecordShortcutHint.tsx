export function RecordShortcutHint({ keys, separator = ' ', decorative }: {
  keys: readonly string[]; separator?: string; decorative?: boolean;
}) {
  const key = keys.at(-1) ?? '';
  return <kbd className="record-shortcut-hint" aria-hidden={decorative}
    data-wide-single={keys.length === 1 && key.length > 1}>
    {keys.length > 1 && <span className="record-shortcut-modifiers">
      {keys.slice(0, -1).map((modifier) => <span key={modifier}>{modifier}{separator}</span>)}
    </span>}
    <span className="record-shortcut-key">{key}</span>
  </kbd>;
}
