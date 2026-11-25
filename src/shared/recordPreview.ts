export function historyPreview(text: string) {
  return [...text.replace(/\r\n/g, '\n').trim()].slice(0, 256).join('');
}

export function historyScrollTarget(scrollTop: number, viewport: { top: number; bottom: number }, row: { top: number; bottom: number }) {
  if (row.top < viewport.top) return Math.max(0, scrollTop + row.top - viewport.top);
  if (row.bottom > viewport.bottom) return Math.max(0, scrollTop + row.bottom - viewport.bottom);
  return null;
}
