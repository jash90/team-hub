import type { ReactNode } from 'react';

import { Icon, type IconName } from '../icon.tsx';

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
