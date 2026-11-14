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

export function editSource(original: string, nextDisplay: string) {
  const previous = displaySource(original);
  let start = 0;
  while (start < previous.length && start < nextDisplay.length && previous[start] === nextDisplay[start]) start += 1;
  let end = previous.length;
  let nextEnd = nextDisplay.length;
  while (end > start && nextEnd > start && previous[end - 1] === nextDisplay[nextEnd - 1]) { end -= 1; nextEnd -= 1; }
  return original.slice(0, originalOffset(original, start)) + nextDisplay.slice(start, nextEnd) + original.slice(originalOffset(original, end));
}
