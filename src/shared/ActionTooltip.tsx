import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';

type TooltipTarget = {
  element: HTMLElement;
  text: string;
  container: HTMLElement;
};

type TooltipLayout = {
  left: number;
  top: number;
  side: 'top' | 'bottom';
  arrowX: number;
  ready: boolean;
};

const EDGE_MARGIN = 8;
const GAP = 7;

const mountedHosts = new Set<symbol>();
let currentOwner: symbol | null = null;
const hostSubscribers = new Set<() => void>();

function notifyHostChange() {
  hostSubscribers.forEach((listener) => listener());
}

function resolveTooltipButton(node: EventTarget | null): HTMLElement | null {
  if (!(node instanceof Element)) return null;
  const button = node.closest<HTMLElement>('button[data-tooltip]');
  if (!button) return null;
  if (button instanceof HTMLButtonElement && button.disabled) return null;
  if (button.getAttribute('aria-disabled') === 'true' || button.hidden) return null;
  const text = button.dataset.tooltip?.trim();
  return text ? button : null;
}

function tooltipContainer(element: HTMLElement): HTMLElement {
  return element.closest<HTMLElement>('dialog[open]') ?? document.body;
}

export function ActionTooltipHost() {
  const token = useRef(Symbol('action-tooltip-host'));
  const [owner, setOwner] = useState(false);

  useEffect(() => {
    const id = token.current;
    mountedHosts.add(id);
    if (currentOwner === null) currentOwner = id;
    const sync = () => setOwner(currentOwner === id);
    hostSubscribers.add(sync);
    notifyHostChange();
    return () => {
      hostSubscribers.delete(sync);
      mountedHosts.delete(id);
      if (currentOwner === id) {
        currentOwner = mountedHosts.values().next().value ?? null;
        notifyHostChange();
      }
    };
  }, []);

  return owner ? <ActionTooltipPortal /> : null;
}

function ActionTooltipPortal() {
  const tooltipId = useId();
  const tipRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef<HTMLElement | null>(null);
  const [target, setTarget] = useState<TooltipTarget | null>(null);
  const [layout, setLayout] = useState<TooltipLayout>({ left: EDGE_MARGIN, top: EDGE_MARGIN, side: 'top', arrowX: 16, ready: false });

  useEffect(() => {
    function clear() {
      if (activeRef.current === null) return;
      activeRef.current = null;
      setTarget(null);
    }

    function show(button: HTMLElement) {
      const text = button.dataset.tooltip?.trim();
      if (!text) {
        clear();
        return;
      }
      const container = tooltipContainer(button);
      if (activeRef.current === button) {
        setTarget((current) => (current && current.element === button && current.text === text && current.container === container
          ? current
          : { element: button, text, container }));
        return;
      }
      activeRef.current = button;
      setLayout((current) => ({ ...current, ready: false }));
      setTarget({ element: button, text, container });
    }

    function handleOver(event: Event) {
      if (typeof PointerEvent !== 'undefined' && event instanceof PointerEvent && event.pointerType === 'touch') return;
      const button = resolveTooltipButton(event.target);
      if (!button) return;
      show(button);
    }

    function handleOut(event: MouseEvent) {
      const current = activeRef.current;
      if (!current) return;
      const related = event.relatedTarget;
      if (related instanceof Node && current.contains(related)) return;
      const next = resolveTooltipButton(related);
      if (next) show(next);
      else clear();
    }

    function handleLeaveDocument() {
      clear();
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape' || event.key === 'Tab' || event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        clear();
      }
    }

    document.addEventListener('pointerover', handleOver, true);
    document.addEventListener('mouseover', handleOver, true);
    document.addEventListener('pointerout', handleOut, true);
    document.addEventListener('mouseout', handleOut, true);
    document.addEventListener('pointerdown', clear, true);
    document.addEventListener('mousedown', clear, true);
    document.addEventListener('click', clear, true);
    document.addEventListener('mouseleave', handleLeaveDocument);
    window.addEventListener('scroll', clear, true);
    window.addEventListener('wheel', clear, { passive: true, capture: true });
    window.addEventListener('keydown', handleKeyDown, true);
    window.addEventListener('blur', clear);
    window.addEventListener('resize', clear);
    window.addEventListener('nami:window-moved', clear);

    return () => {
      document.removeEventListener('pointerover', handleOver, true);
      document.removeEventListener('mouseover', handleOver, true);
      document.removeEventListener('pointerout', handleOut, true);
      document.removeEventListener('mouseout', handleOut, true);
      document.removeEventListener('pointerdown', clear, true);
      document.removeEventListener('mousedown', clear, true);
      document.removeEventListener('click', clear, true);
      document.removeEventListener('mouseleave', handleLeaveDocument);
      window.removeEventListener('scroll', clear, true);
      window.removeEventListener('wheel', clear, true);
      window.removeEventListener('keydown', handleKeyDown, true);
      window.removeEventListener('blur', clear);
      window.removeEventListener('resize', clear);
      window.removeEventListener('nami:window-moved', clear);
    };
  }, []);

  useEffect(() => {
    const element = target?.element;
    if (!element) return;
    if (!element.isConnected) {
      activeRef.current = null;
      setTarget(null);
      return;
    }
    const sync = () => {
      if (!element.isConnected || !resolveTooltipButton(element)) {
        activeRef.current = null;
        setTarget(null);
        return;
      }
      const text = element.dataset.tooltip?.trim() ?? '';
      if (!text) {
        activeRef.current = null;
        setTarget(null);
        return;
      }
      setTarget((current) => (current && current.element === element && current.text === text ? current : { element, text, container: tooltipContainer(element) }));
    };
    const observer = new MutationObserver(sync);
    observer.observe(element, { attributes: true, attributeFilter: ['data-tooltip', 'disabled', 'aria-disabled', 'hidden'] });
    return () => observer.disconnect();
  }, [target?.element]);

  useLayoutEffect(() => {
    const tip = tipRef.current;
    const element = target?.element;
    if (!tip || !element || !element.isConnected) return;

    function place() {
      if (!tip || !element || !element.isConnected) return;
      const anchor = element.getBoundingClientRect();
      const box = tip.getBoundingClientRect();
      const viewportWidth = window.innerWidth || document.documentElement.clientWidth || 1024;
      const viewportHeight = window.innerHeight || document.documentElement.clientHeight || 768;
      const tipWidth = box.width;
      const tipHeight = box.height;

      const above = anchor.top - tipHeight - GAP;
      const below = anchor.bottom + GAP;
      const preferBottom = element.dataset.tooltipSide === 'bottom';
      const fitsAbove = above >= EDGE_MARGIN;
      const fitsBelow = below + tipHeight <= viewportHeight - EDGE_MARGIN;
      const side: 'top' | 'bottom' = preferBottom
        ? (fitsBelow || !fitsAbove ? 'bottom' : 'top')
        : (fitsAbove || !fitsBelow ? 'top' : 'bottom');
      const rawTop = side === 'top' ? above : below;
      const top = Math.max(EDGE_MARGIN, Math.min(rawTop, viewportHeight - tipHeight - EDGE_MARGIN));

      const anchorCenterX = anchor.left + anchor.width / 2;
      const maxLeft = Math.max(EDGE_MARGIN, viewportWidth - tipWidth - EDGE_MARGIN);
      const left = Math.max(EDGE_MARGIN, Math.min(anchorCenterX - tipWidth / 2, maxLeft));
      const arrowPadding = Math.min(11, Math.max(6, tipWidth / 2));
      const arrowX = Math.max(arrowPadding, Math.min(tipWidth - arrowPadding, anchorCenterX - left));

      setLayout({ left, top, side, arrowX, ready: true });
    }

    place();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(place);
    observer.observe(tip);
    return () => observer.disconnect();
  }, [target]);

  if (!target || !target.element.isConnected) return null;

  const lines = target.text.split('\n').filter(Boolean);
  const style = {
    left: `${layout.left}px`,
    top: `${layout.top}px`,
    '--tooltip-arrow-x': `${layout.arrowX}px`,
  } as CSSProperties;

  return createPortal(
    <div
      ref={tipRef}
      id={tooltipId}
      role="tooltip"
      className="action-tooltip"
      data-side={layout.side}
      data-ready={layout.ready || undefined}
      style={style}
    >
      {lines.length > 1
        ? <span className="action-tooltip-lines">
          <span className="action-tooltip-title">{lines[0]}</span>
          {lines.slice(1).map((line, index) => <span className="action-tooltip-desc" key={index}>{line}</span>)}
        </span>
        : lines[0]}
    </div>,
    target.container,
  );
}
