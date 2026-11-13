import { useState, type KeyboardEvent, type ReactNode } from 'react';

import { t } from '../../i18n/index.ts';
import { Icon } from '../icon.tsx';
import { Popover } from './popover.tsx';

export type Option<T> = { value: T; label: string; render?: ReactNode; keywords?: string };

type PickerProps<T> = {
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
  /** What the closed control shows; defaults to the selected option. */
  display?: ReactNode;
  placeholder?: string;
  disabled?: boolean;
  variant?: 'field' | 'inline';
  label?: string;
  align?: 'start' | 'end';
};

function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/ł/gi, 'l')
    .toLowerCase();
}

/** A select with search, for values that are more than plain text (people, statuses, sprints). */
export function Picker<T extends string | number | null>({ value, options, onChange, display, placeholder, disabled, variant = 'field', label, align }: PickerProps<T>) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [filter, setFilter] = useState('');
  const [cursor, setCursor] = useState(0);
  const selected = options.find((option) => option.value === value);
  const wanted = fold(filter);
  const visible = wanted ? options.filter((option) => fold(`${option.label} ${option.keywords ?? ''}`).includes(wanted)) : options;

  function close(): void {
    setAnchor(null);
    setFilter('');
    setCursor(0);
  }

  function choose(option: Option<T>): void {
    close();

    if (option.value !== value) {
      onChange(option.value);
    }
  }

  function onKey(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setCursor((current) => Math.min(current + 1, visible.length - 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setCursor((current) => Math.max(current - 1, 0));
    } else if (event.key === 'Enter' && visible[cursor]) {
      event.preventDefault();
      choose(visible[cursor]);
    }
  }

  return (
    <>
      <button
        type="button"
        className={`picker picker-${variant}`}
        disabled={disabled}
        aria-label={label}
        aria-haspopup="listbox"
        onClick={(event) => (anchor ? close() : setAnchor(event.currentTarget))}
      >
        <span className="picker-value">{display ?? selected?.render ?? selected?.label ?? <span className="muted">{placeholder ?? t('Choose…')}</span>}</span>
        {variant === 'field' && <Icon name="chevronDown" size={14} className="picker-caret" />}
      </button>
      {anchor && (
        <Popover anchor={anchor} onClose={close} align={align} width={Math.max(220, anchor.offsetWidth)}>
          <div className="picker-panel">
            {options.length > 7 && (
              <input
                autoFocus
                className="input picker-filter"
                placeholder={t('Filter…')}
                value={filter}
                onChange={(event) => {
                  setFilter(event.target.value);
                  setCursor(0);
                }}
                onKeyDown={onKey}
              />
            )}
            <div className="picker-options" role="listbox">
              {visible.map((option, index) => (
                <button
                  key={String(option.value)}
                  type="button"
                  role="option"
                  aria-selected={option.value === value}
                  className={['picker-option', index === cursor && options.length > 7 ? 'picker-cursor' : '', option.value === value ? 'picker-selected' : ''].join(' ')}
                  onMouseEnter={() => setCursor(index)}
                  onClick={() => choose(option)}
                >
                  <span className="picker-option-body">{option.render ?? option.label}</span>
                  {option.value === value && <Icon name="check" size={14} />}
                </button>
              ))}
              {visible.length === 0 && <div className="picker-none muted">{t('Nothing matches')}</div>}
            </div>
          </div>
        </Popover>
      )}
    </>
  );
}
