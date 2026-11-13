import type { ReactNode } from 'react';

/** A titled card on a settings page: the instance's, a project's, or a module's tab on either. */
export function SettingsCard({ title, lead, children, action }: { title: string; lead?: string; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="panel settings-card">
      <header className="panel-head">
        <h2>{title}</h2>
        <span className="filter-spacer" />
        {action}
      </header>
      {lead && <p className="muted settings-lead">{lead}</p>}
      {children}
    </section>
  );
}
