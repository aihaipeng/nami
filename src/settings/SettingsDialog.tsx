import { useId, useLayoutEffect, useRef, type KeyboardEventHandler, type ReactNode } from 'react';

export function SettingsDialog({ title, description, onClose, onKeyDown, children, className, role = 'alertdialog' }: {
  className?: string; role?: 'dialog' | 'alertdialog'; title: string; description?: string; onClose: () => void; onKeyDown?: KeyboardEventHandler<HTMLDialogElement>; children: ReactNode;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const id = useId();
  useLayoutEffect(() => {
    const element = dialog.current!;
    const trigger = document.activeElement;
    element.showModal();
    element.querySelector<HTMLElement>('[data-initial-focus]')?.focus({ preventScroll: true });
    return () => {
      element.close();
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus({ preventScroll: true });
    };
  }, []);
  return <dialog ref={dialog} className={className ? `settings-dialog ${className}` : 'settings-dialog'} role={role}
    aria-labelledby={`${id}-title`} aria-describedby={description ? `${id}-description` : undefined}
    onKeyDown={onKeyDown} onCancel={(event) => { event.preventDefault(); onClose(); }}>
    <h3 id={`${id}-title`}>{title}</h3>
    {description && <p id={`${id}-description`}>{description}</p>}
    {children}
  </dialog>;
}
