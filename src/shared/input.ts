export function inputWithinLimits(text: string): boolean {
  if (text.length > 100_000) return false;
  const lines = text.split(/\r\n|\n|\r/);
  return lines.length <= 2_000 && lines.every((line) => line.length <= 5_000);
}

export const clipboardSamples = [
  { id: 'welcome', text: 'Hello from Nami — synthetic clipboard sample.' },
  { id: 'multiline', text: '第一行 / First line\n\n第三行 / Third line\n' },
  { id: 'unicode', text: '中文、English、emoji 🌊 — 100%_literal' },
] as const;
