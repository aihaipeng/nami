import { useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { createPortal } from 'react-dom';

export function restorePanelFocus(trigger: HTMLElement | null) {
  if (trigger?.isConnected && !trigger.closest('[inert]')
    && getComputedStyle(trigger).visibility !== 'hidden' && getComputedStyle(trigger).display !== 'none') {
    trigger.focus({ preventScroll: true });
  } else document.querySelector<HTMLTextAreaElement>('.source-input')?.focus({ preventScroll: true });
}

export function panelHorizontalPosition(anchorLeft: number, width: number, visibleLeft: number, visibleWidth: number) {
  const maxWidth = Math.min(width, visibleWidth - 16);
  return { left: Math.max(visibleLeft + 8, Math.min(anchorLeft, visibleLeft + visibleWidth - maxWidth - 8)), maxWidth };
}

export function FloatingPanel({ anchor, returnFocus = anchor, onClose, id, label, role, width, side, children, className = '', dismissOnSettingsScroll = false }: {
  anchor: RefObject<HTMLElement | null>; returnFocus?: RefObject<HTMLElement | null>; onClose: () => void; id?: string; label?: string;
  role: 'dialog' | 'menu' | 'tooltip'; width: number; side: 'top' | 'bottom'; children: ReactNode; className?: string; dismissOnSettingsScroll?: boolean;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  const [position, setPosition] = useState({ left: 8, top: 8, maxWidth: width });

  useLayoutEffect(() => {
    const element = panel.current;
    const trigger = anchor.current;
    if (!element || !trigger) return;
    function place() {
      const rect = trigger!.getBoundingClientRect();
      const height = element!.getBoundingClientRect().height;
      const viewport = document.querySelector<HTMLElement>('.translation-shell');
      const visibleLeft = Number(viewport?.dataset.viewportLeft ?? 0);
      const visibleWidth = Number(viewport?.dataset.viewportWidth ?? innerWidth);
      const above = rect.top - height - 6;
      const below = rect.bottom + 6;
      const desired = side === 'top' ? (above >= 8 ? above : below) : (below + height <= innerHeight - 8 ? below : above);
      setPosition({ ...panelHorizontalPosition(rect.left, width, visibleLeft, visibleWidth), top: Math.max(8, Math.min(desired, innerHeight - height - 8)) });
    }
    const observer = new ResizeObserver(place);
    observer.observe(element);
    place();
    (element.querySelector<HTMLElement>('input') ?? element.querySelector<HTMLElement>('[data-active=true]') ?? element.querySelector<HTMLElement>('button:not(:disabled)'))?.focus({ preventScroll: true });
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !element.contains(event.target) && !trigger.contains(event.target)) {
        close.current();
        requestAnimationFrame(() => {
          if (document.hasFocus() && document.activeElement === document.body) restoreFocus();
        });
      }
    };
    const dismiss = () => close.current();
    const settingsScroll = dismissOnSettingsScroll ? trigger.closest('.settings-content') : null;
    const dismissForScroll = () => {
      const ownedFocus = element.contains(document.activeElement);
      dismiss();
      if (ownedFocus) restorePanelFocus(trigger);
    };
    settingsScroll?.addEventListener('scroll', dismissForScroll, { passive: true });
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('blur', dismiss);
    window.addEventListener('resize', dismiss);
    window.addEventListener('nami:window-moved', dismiss);
    return () => {
      observer.disconnect();
      settingsScroll?.removeEventListener('scroll', dismissForScroll);
      document.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('blur', dismiss);
      window.removeEventListener('resize', dismiss);
      window.removeEventListener('nami:window-moved', dismiss);
    };
  }, [anchor, side, width, dismissOnSettingsScroll]);

  function restoreFocus() {
    close.current();
    restorePanelFocus(returnFocus.current);
  }

  return createPortal(<div ref={panel} id={id} role={role} aria-label={label} className={`floating-panel ${className}`}
    style={{ ...position, width: role === 'tooltip' ? 'max-content' : width }} onKeyDown={(event) => {
      if (event.nativeEvent.isComposing) return;
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); restoreFocus(); }
      if (event.key === 'Tab') {
        if (!dismissOnSettingsScroll) event.preventDefault();
        restoreFocus();
      }
    }}>{children}</div>, document.body);
}
