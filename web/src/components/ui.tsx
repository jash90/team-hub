import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ButtonHTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';

import { errorMessage } from '../api.ts';
import type { User } from '../bindings/User.ts';
import { t } from '../i18n/index.ts';
import { Icon, type IconName } from './icon.tsx';

// ---------- buttons ----------

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'default' | 'primary' | 'ghost' | 'danger';
  size?: 'sm' | 'md';
  icon?: IconName;
};

export function Button({ variant = 'default', size = 'md', icon, className, children, type = 'button', ...rest }: ButtonProps) {
  const classes = ['button', `button-${variant}`, `button-${size}`, !children && icon ? 'button-icon' : '', className ?? ''].filter(Boolean).join(' ');

  return (
    <button type={type} className={classes} {...rest}>
      {icon && <Icon name={icon} size={size === 'sm' ? 14 : 16} />}
      {children}
    </button>
  );
}

// ---------- toasts ----------

type Toast = { id: number; text: string; kind: 'info' | 'error' };

let toasts: Toast[] = [];
let nextToast = 1;
const toastListeners = new Set<() => void>();

function publishToasts(next: Toast[]): void {
  toasts = next;

  for (const listener of toastListeners) {
    listener();
  }
}

export function toast(text: string, kind: Toast['kind'] = 'info'): void {
  const id = nextToast++;
  publishToasts([...toasts, { id, text, kind }]);
  window.setTimeout(() => publishToasts(toasts.filter((item) => item.id !== id)), kind === 'error' ? 7000 : 3500);
}

export function Toaster() {
  const items = useSyncExternalStore(
    (listener) => {
      toastListeners.add(listener);

      return () => toastListeners.delete(listener);
    },
    () => toasts,
  );

  return (
    <div className="toaster" role="status" aria-live="polite">
      {items.map((item) => (
        <div key={item.id} className={`toast toast-${item.kind}`}>
          <Icon name={item.kind === 'error' ? 'alert' : 'check'} />
          <span>{item.text}</span>
        </div>
      ))}
    </div>
  );
}

/** Runs something that talks to the server; a failure becomes a toast instead of an unhandled rejection. */
export async function attempt<T>(work: () => Promise<T>, done?: string): Promise<T | undefined> {
  try {
    const result = await work();

    if (done) {
      toast(done);
    }

    return result;
  } catch (error) {
    toast(errorMessage(error), 'error');

    return undefined;
  }
}

/** `[busy, run]`: `run` wraps `attempt` and tells whether it is still going. */
export function useBusy(): [boolean, <T>(work: () => Promise<T>, done?: string) => Promise<T | undefined>] {
  const [busy, setBusy] = useState(false);
  const run = useCallback(async <T,>(work: () => Promise<T>, done?: string) => {
    setBusy(true);

    try {
      return await attempt(work, done);
    } finally {
      setBusy(false);
    }
  }, []);

  return [busy, run];
}

// ---------- people ----------

function initials(name: string): string {
  const parts = name.trim().split(/\s+/);

  return ((parts[0]?.[0] ?? '') + (parts.length > 1 ? (parts[parts.length - 1][0] ?? '') : '')).toUpperCase();
}

export function Avatar({ user, size = 24, label }: { user: User | undefined; size?: number; label?: string }) {
  const style = { width: size, height: size, fontSize: Math.round(size * 0.42), background: user?.color ?? 'var(--line-strong)' };

  return (
    <span className={user ? 'avatar' : 'avatar avatar-none'} style={style} title={label ?? user?.name ?? t('Unassigned')}>
      {user ? initials(user.name) : <Icon name="user" size={Math.round(size * 0.6)} />}
    </span>
  );
}

// ---------- layout bits ----------

export function Spinner({ label }: { label?: string }) {
  return (
    <div className="spinner-row">
      <span className="spinner" />
      {label && <span className="muted">{label}</span>}
    </div>
  );
}

export function Empty({ icon, title, children }: { icon?: IconName; title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      {icon && <Icon name={icon} size={28} />}
      <strong>{title}</strong>
      {children && <div className="muted">{children}</div>}
    </div>
  );
}

export function ErrorNote({ message }: { message: string }) {
  return (
    <div className="note note-error" role="alert">
      <Icon name="alert" />
      <span>{message}</span>
    </div>
  );
}

export function Tag({ children, tone = 'neutral', title }: { children: ReactNode; tone?: 'neutral' | 'blue' | 'green' | 'amber' | 'red' | 'purple'; title?: string }) {
  return (
    <span className={`tag tag-${tone}`} title={title}>
      {children}
    </span>
  );
}

export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      {children}
      {hint && <span className="field-hint">{hint}</span>}
    </label>
  );
}

export function ProgressBar({ done, inProgress = 0, total }: { done: number; inProgress?: number; total: number }) {
  const share = (value: number) => (total > 0 ? `${(value / total) * 100}%` : '0%');

  return (
    <div className="progress" role="img" aria-label={`${done} / ${total}`}>
      <span className="progress-done" style={{ width: share(done) }} />
      <span className="progress-doing" style={{ width: share(inProgress) }} />
    </div>
  );
}

export type TabItem = { id: string; label: string; count?: number };

export function Tabs({ tabs, active, onChange }: { tabs: TabItem[]; active: string; onChange: (id: string) => void }) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((tab) => (
        <button key={tab.id} type="button" role="tab" aria-selected={tab.id === active} className={tab.id === active ? 'tab tab-active' : 'tab'} onClick={() => onChange(tab.id)}>
          {tab.label}
          {tab.count !== undefined && <span className="tab-count">{tab.count}</span>}
        </button>
      ))}
    </div>
  );
}

// ---------- overlays ----------

function useEscape(onEscape: () => void, active = true): void {
  useEffect(() => {
    if (!active) {
      return undefined;
    }

    const handle = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation();
        onEscape();
      }
    };

    window.addEventListener('keydown', handle, true);

    return () => window.removeEventListener('keydown', handle, true);
  }, [onEscape, active]);
}

export function Dialog({ title, onClose, children, footer, wide }: { title: string; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean }) {
  const panel = useRef<HTMLDivElement>(null);
  useEscape(onClose);

  useEffect(() => {
    const first = panel.current?.querySelector<HTMLElement>('[autofocus], input:not([type=hidden]), textarea, select');
    first?.focus();
  }, []);

  return createPortal(
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className={wide ? 'dialog dialog-wide' : 'dialog'} role="dialog" aria-modal="true" aria-label={title} ref={panel}>
        <header className="dialog-head">
          <h2>{title}</h2>
          <Button variant="ghost" icon="x" onClick={onClose} aria-label={t('Close')} />
        </header>
        <div className="dialog-body">{children}</div>
        {footer && <footer className="dialog-foot">{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}

export function Drawer({ onClose, children, label }: { onClose: () => void; children: ReactNode; label: string }) {
  useEscape(onClose);

  return createPortal(
    <div className="overlay overlay-side" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <aside className="drawer" role="dialog" aria-modal="true" aria-label={label}>
        {children}
      </aside>
    </div>,
    document.body,
  );
}

type PopoverProps = {
  anchor: HTMLElement | null;
  onClose: () => void;
  children: ReactNode;
  align?: 'start' | 'end';
  width?: number;
};

/** A panel attached to an element; closes on a click elsewhere or Escape, and stays on screen. */
export function Popover({ anchor, onClose, children, align = 'start', width }: PopoverProps) {
  const panel = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
  useEscape(onClose);

  useLayoutEffect(() => {
    if (!anchor || !panel.current) {
      return;
    }

    const box = anchor.getBoundingClientRect();
    const own = panel.current.getBoundingClientRect();
    const margin = 8;
    let left = align === 'end' ? box.right - own.width : box.left;
    let top = box.bottom + 4;

    left = Math.max(margin, Math.min(left, window.innerWidth - own.width - margin));

    if (top + own.height > window.innerHeight - margin && box.top - own.height - 4 > margin) {
      top = box.top - own.height - 4;
    }

    setPosition({ top, left });
  }, [anchor, align, children]);

  useEffect(() => {
    const handle = (event: MouseEvent) => {
      const target = event.target as Node;

      if (!panel.current?.contains(target) && !anchor?.contains(target)) {
        onClose();
      }
    };

    document.addEventListener('mousedown', handle);

    return () => document.removeEventListener('mousedown', handle);
  }, [anchor, onClose]);

  return createPortal(
    <div ref={panel} className="popover" style={{ top: position?.top ?? -9999, left: position?.left ?? -9999, width, visibility: position ? 'visible' : 'hidden' }}>
      {children}
    </div>,
    document.body,
  );
}

export type MenuItem = { label: string; icon?: IconName; onSelect: () => void; danger?: boolean; disabled?: boolean };

export function Menu({ items, label, icon = 'dots', children, align = 'end' }: { items: MenuItem[]; label: string; icon?: IconName; children?: ReactNode; align?: 'start' | 'end' }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);

  return (
    <>
      <Button variant="ghost" icon={children ? undefined : icon} aria-label={label} title={label} onClick={(event) => setAnchor(anchor ? null : event.currentTarget)}>
        {children}
      </Button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} align={align}>
          <div className="menu" role="menu">
            {items.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                disabled={item.disabled}
                className={item.danger ? 'menu-item menu-danger' : 'menu-item'}
                onClick={() => {
                  setAnchor(null);
                  item.onSelect();
                }}
              >
                {item.icon && <Icon name={item.icon} />}
                {item.label}
              </button>
            ))}
          </div>
        </Popover>
      )}
    </>
  );
}

// ---------- picker ----------

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

// ---------- confirm ----------

export function ConfirmDialog({ title, body, confirm, danger, onConfirm, onClose }: { title: string; body: ReactNode; confirm: string; danger?: boolean; onConfirm: () => void | Promise<void>; onClose: () => void }) {
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={title}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant={danger ? 'danger' : 'primary'}
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await onConfirm();
                onClose();
              })
            }
          >
            {confirm}
          </Button>
        </>
      }
    >
      <div className="dialog-text">{body}</div>
    </Dialog>
  );
}

/** Text that turns into an input when clicked; Enter or leaving saves, Escape cancels. */
export function InlineEdit({ value, onSave, className, placeholder, multiline }: { value: string; onSave: (value: string) => void; className?: string; placeholder?: string; multiline?: boolean }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);

  useEffect(() => setDraft(value), [value]);

  function finish(save: boolean): void {
    setEditing(false);

    if (save && draft.trim() && draft.trim() !== value) {
      onSave(draft.trim());
    } else {
      setDraft(value);
    }
  }

  if (!editing) {
    return (
      <button type="button" className={`inline-edit ${className ?? ''}`} onClick={() => setEditing(true)}>
        {value || <span className="muted">{placeholder}</span>}
      </button>
    );
  }

  const shared = {
    autoFocus: true,
    className: `input inline-edit-input ${className ?? ''}`,
    value: draft,
    onBlur: () => finish(true),
  };

  return multiline ? (
    <textarea
      {...shared}
      rows={2}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' && !event.shiftKey) {
          event.preventDefault();
          finish(true);
        } else if (event.key === 'Escape') {
          finish(false);
        }
      }}
    />
  ) : (
    <input
      {...shared}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          finish(true);
        } else if (event.key === 'Escape') {
          finish(false);
        }
      }}
    />
  );
}

export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement('textarea');
    area.value = text;
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.append(area);
    area.select();
    document.execCommand('copy');
    area.remove();
  }

  toast(t('Copied'));
}
