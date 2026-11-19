import { useEffect, useId, useRef, useState } from 'react';
import { Check, ChevronDown, Search } from 'lucide-react';
import { FloatingPanel, restorePanelFocus } from './FloatingPanel';
import { matchesLanguage } from './languages';
import type { Translate } from './i18n';

export function SearchableDropdown({ value, onChange, label, items, t, side = 'top', open, onOpenChange, searchable = true, id: triggerId, capsule = false, settings = false, menuWidth = 200, hiddenTrigger = false, description, valueLabel }: {
  value: string; onChange: (value: string) => void; label: string; items: readonly { code: string; name: string }[];
  t: Translate; side?: 'top' | 'bottom'; open: boolean; onOpenChange: (open: boolean) => void; searchable?: boolean; id?: string; capsule?: boolean; settings?: boolean; menuWidth?: number;
  hiddenTrigger?: boolean; description?: string; valueLabel?: string;
}) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(value);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const id = useId();
  const options = items;
  const visible = options.filter((option) => matchesLanguage(option.code, option.name, query));
  const current = visible.findIndex((option) => option.code === active);
  const selected = valueLabel ?? options.find((option) => option.code === value)?.name ?? value;

  useEffect(() => {
    if (!open) { setQuery(''); setActive(value); }
  }, [open, value]);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>('[data-active=true]')?.scrollIntoView({ block: 'nearest' });
  }, [active, open]);

  function choose(code: string) {
    onChange(code);
    onOpenChange(false);
    restorePanelFocus(trigger.current);
  }

  return <>
    <button ref={trigger} id={triggerId} type="button" className={settings ? 'language-picker settings-dropdown' : capsule ? 'language-picker dropdown-capsule' : 'language-picker'} aria-label={`${label}: ${selected}`}
      aria-hidden={hiddenTrigger || undefined} inert={hiddenTrigger} tabIndex={hiddenTrigger ? -1 : undefined}
      title={description} aria-description={description} aria-haspopup="dialog" aria-expanded={open} onClick={() => onOpenChange(!open)}>
      <span>{selected}</span><ChevronDown aria-hidden="true" />
    </button>
    {open && <FloatingPanel anchor={trigger} onClose={() => onOpenChange(false)} label={label} role="dialog" width={menuWidth} side={side} className={settings ? 'language-popover settings-popover' : 'language-popover'} dismissOnSettingsScroll={settings}>
      {searchable && <label className="language-search"><Search aria-hidden="true" /><input role="combobox" aria-label={t('searchLanguage')}
        aria-expanded="true" aria-controls={id} aria-activedescendant={current >= 0 ? `${id}-${current}` : undefined}
        placeholder="Search..." value={query} onChange={(event) => {
          const query = event.target.value;
          setQuery(query);
          const matches = options.filter((option) => matchesLanguage(option.code, option.name, query));
          setActive(matches.find((option) => option.code === value)?.code ?? matches[0]?.code ?? '');
        }} onKeyDown={(event) => {
          if (event.nativeEvent.isComposing || event.keyCode === 229 || event.metaKey) return;
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            const index = Math.max(0, Math.min(visible.length - 1, current + (event.key === 'ArrowDown' ? 1 : -1)));
            setActive(visible[index]?.code ?? '');
          } else if (event.key === 'Enter') {
            event.preventDefault();
            if (visible[current]) choose(visible[current].code);
          }
        }} /></label>}
      <div ref={list} id={id} role="listbox" aria-label={label} className="language-options" onKeyDown={(event) => {
        if (searchable || event.nativeEvent.isComposing) return;
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault();
          const index = Math.max(0, Math.min(visible.length - 1, current + (event.key === 'ArrowDown' ? 1 : -1)));
          setActive(visible[index]?.code ?? '');
          event.currentTarget.querySelectorAll<HTMLButtonElement>('button')[index]?.focus();
        } else if (event.key === 'Enter' && visible[current]) {
          event.preventDefault();
          choose(visible[current].code);
        }
      }}>
        {visible.map((option, index) => <button key={option.code} id={`${id}-${index}`} type="button" role="option"
          aria-selected={value === option.code} data-active={active === option.code} tabIndex={-1}
          onMouseDown={(event) => event.preventDefault()} onClick={() => choose(option.code)}>
          <span>{option.name}</span>{value === option.code && <Check aria-hidden="true" />}
        </button>)}
        {!visible.length && <div className="language-no-results">{t('noMatches')}</div>}
      </div>
    </FloatingPanel>}
  </>;
}
