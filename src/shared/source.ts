export function displaySource(value: string) {
  return value.replace(/\r\n|\r|\n/g, ' ');
}

function originalOffset(value: string, displayedOffset: number) {
  let original = 0;
  for (let displayed = 0; displayed < displayedOffset && original < value.length; displayed += 1) {
    original += value[original] === '\r' && value[original + 1] === '\n' ? 2 : 1;
  }
  return original;
}

const SOURCE_DRAG_EDGE = 24;
const SOURCE_DRAG_MAX_STEP = 24;

export function sourceDragDelta(pointerX: number, left: number, right: number) {
  const intoRight = pointerX - (right - SOURCE_DRAG_EDGE);
  const intoLeft = left + SOURCE_DRAG_EDGE - pointerX;
  if (intoRight > 0) return Math.min(SOURCE_DRAG_MAX_STEP, Math.ceil(intoRight / 2));
  if (intoLeft > 0) return -Math.min(SOURCE_DRAG_MAX_STEP, Math.ceil(intoLeft / 2));
  return 0;
}

export function sourceIndexAt(text: string, target: number, width: (prefix: string) => number) {
  if (target <= 0 || text.length === 0) return 0;
  if (width(text) <= target) return text.length;
  let low = 0;
  let high = text.length;
  while (low < high) {
    const mid = Math.floor((low + high + 1) / 2);
    if (width(text.slice(0, mid)) <= target) low = mid;
    else high = mid - 1;
  }
  return low;
}

export interface SourceDragField {
  value: string;
  scrollLeft: number;
  scrollWidth: number;
  clientWidth: number;
  setSelectionRange(start: number, end: number, direction?: 'forward' | 'backward' | 'none'): void;
}

export function dragSourceSelection(field: SourceDragField, options: {
  pointerX: number;
  left: number;
  right: number;
  paddingLeft: number;
  anchor: number;
  width: (prefix: string) => number;
}) {
  const maxScroll = Math.max(0, field.scrollWidth - field.clientWidth);
  field.scrollLeft = Math.min(maxScroll, Math.max(0, field.scrollLeft + sourceDragDelta(options.pointerX, options.left, options.right)));
  const visibleX = Math.min(options.right - 1, Math.max(options.left + 1, options.pointerX));
  const index = sourceIndexAt(field.value, visibleX - options.left + field.scrollLeft - options.paddingLeft, options.width);
  field.setSelectionRange(Math.min(options.anchor, index), Math.max(options.anchor, index), index < options.anchor ? 'backward' : 'forward');
}

export function editSource(original: string, nextDisplay: string) {
  const previous = displaySource(original);
  let start = 0;
  while (start < previous.length && start < nextDisplay.length && previous[start] === nextDisplay[start]) start += 1;
  let end = previous.length;
  let nextEnd = nextDisplay.length;
  while (end > start && nextEnd > start && previous[end - 1] === nextDisplay[nextEnd - 1]) { end -= 1; nextEnd -= 1; }
  return original.slice(0, originalOffset(original, start)) + nextDisplay.slice(start, nextEnd) + original.slice(originalOffset(original, end));
}
