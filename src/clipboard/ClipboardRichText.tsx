import { useMemo, type MouseEvent } from 'react';

const ALLOWED_TAGS = new Set([
  'a', 'b', 'blockquote', 'br', 'caption', 'code', 'dd', 'del', 'div', 'dl', 'dt',
  'em', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'hr', 'i', 'ins', 'li', 'mark', 'ol',
  'p', 'pre', 's', 'span', 'strong', 'sub', 'sup', 'table', 'tbody', 'td', 'tfoot',
  'th', 'thead', 'tr', 'u', 'ul',
]);

const DROP_SUBTREE_TAGS = new Set([
  'applet', 'audio', 'button', 'canvas', 'embed', 'form', 'frame', 'frameset', 'head',
  'iframe', 'img', 'input', 'link', 'map', 'math', 'meta', 'noscript', 'object',
  'option', 'optgroup', 'picture', 'script', 'select', 'source', 'style', 'svg',
  'template', 'textarea', 'title', 'track', 'video',
]);

const SAFE_COLOR_KEYWORDS = new Set([
  'transparent', 'black', 'white', 'red', 'green', 'blue', 'yellow', 'orange',
  'purple', 'pink', 'gray', 'grey', 'silver', 'maroon', 'navy', 'teal', 'olive',
  'aqua', 'cyan', 'magenta', 'fuchsia', 'lime', 'brown', 'gold', 'indigo', 'violet',
]);

function isSafeColor(value: string) {
  if (/^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(value)) return true;
  if (/^(rgba?|hsla?)\([0-9\s,./%+-]+\)$/i.test(value)) return true;
  return SAFE_COLOR_KEYWORDS.has(value);
}

function sanitizeInlineStyle(raw: string): string | null {
  const parts: string[] = [];
  for (const decl of raw.split(';')) {
    const colon = decl.indexOf(':');
    if (colon <= 0) continue;
    const prop = decl.slice(0, colon).trim().toLowerCase();
    const val = decl.slice(colon + 1).replace(/!important\s*$/i, '').trim().toLowerCase();
    if (!val || val.length > 64 || /url|expression|import|var\(|\\|@/i.test(val)) continue;
    if ((prop === 'color' || prop === 'background-color') && isSafeColor(val)) {
      parts.push(`${prop}:${val}`);
    } else if (prop === 'font-weight' && (/^(normal|bold|bolder|lighter)$/.test(val) || /^[1-9]00$/.test(val))) {
      parts.push(`${prop}:${val}`);
    } else if (prop === 'font-style' && /^(normal|italic|oblique)$/.test(val)) {
      parts.push(`${prop}:${val}`);
    } else if (prop === 'text-decoration' && val.split(/\s+/).every((t) => /^(none|underline|line-through|overline)$/.test(t))) {
      parts.push(`${prop}:${val}`);
    } else if (prop === 'text-align' && /^(left|right|center|justify)$/.test(val)) {
      parts.push(`${prop}:${val}`);
    }
  }
  return parts.length ? parts.join(';') : null;
}

function sanitizeHref(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || trimmed.length > 4096 || /[\x00-\x1f\x7f]/.test(trimmed)) return null;
  return /^(https?:\/\/|mailto:)/i.test(trimmed) ? trimmed : null;
}

export function sanitizeRichHtmlFragment(html: string): string {
  if (!html || typeof DOMParser === 'undefined') return '';
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const out = document.implementation.createHTMLDocument('');

  function cloneNode(node: Node, depth: number): Node | null {
    if (depth > 64) return null;
    if (node.nodeType === Node.TEXT_NODE) {
      return out.createTextNode(node.textContent ?? '');
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return null;
    const el = node as Element;
    const tag = el.tagName.toLowerCase();
    if (DROP_SUBTREE_TAGS.has(tag)) return null;
    if (!ALLOWED_TAGS.has(tag)) {
      const fragment = out.createDocumentFragment();
      for (const child of Array.from(el.childNodes)) {
        const cloned = cloneNode(child, depth + 1);
        if (cloned) fragment.appendChild(cloned);
      }
      return fragment;
    }
    const clean = out.createElement(tag);
    if (tag === 'a') {
      const href = sanitizeHref(el.getAttribute('href') ?? '');
      if (href) {
        clean.setAttribute('href', href);
        clean.setAttribute('rel', 'noopener noreferrer');
      }
    } else if (tag === 'td' || tag === 'th') {
      for (const attr of ['colspan', 'rowspan'] as const) {
        const value = Number(el.getAttribute(attr));
        if (Number.isInteger(value) && value >= 1 && value <= 100) {
          clean.setAttribute(attr, String(value));
        }
      }
    }
    const style = el.getAttribute('style');
    if (style) {
      const safe = sanitizeInlineStyle(style);
      if (safe) clean.setAttribute('style', safe);
    }
    for (const child of Array.from(el.childNodes)) {
      const cloned = cloneNode(child, depth + 1);
      if (cloned) clean.appendChild(cloned);
    }
    return clean;
  }

  const container = out.createElement('div');
  for (const child of Array.from(doc.body.childNodes)) {
    const cloned = cloneNode(child, 0);
    if (cloned) container.appendChild(cloned);
  }
  return container.innerHTML;
}

export function ClipboardRichText({ html }: { html: string }) {
  const safeHtml = useMemo(() => sanitizeRichHtmlFragment(html), [html]);
  function onClick(event: MouseEvent<HTMLDivElement>) {
    if ((event.target as Element | null)?.closest('a')) {
      event.preventDefault();
    }
  }
  return (
    <div
      className="clipboard-detail-rich"
      data-tauri-drag-region="false"
      onClick={onClick}
      dangerouslySetInnerHTML={{ __html: safeHtml }}
    />
  );
}
