import { useState, type ReactNode } from 'react';

import { Button, Dialog, Field, Picker, attempt, useBusy } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { useProjectReload, useSession } from '../../session.tsx';

export function Card({ title, lead, children }: { title: string; lead?: string; children: ReactNode }) {
  return (
    <section className="panel settings-card">
      <header className="panel-head">
        <h2>{title}</h2>
      </header>
      {lead && <p className="muted settings-lead">{lead}</p>}
      {children}
    </section>
  );
}

/** Runs a configuration change and reloads the project so every view sees it. */
export function useChange(): (work: () => Promise<unknown>, done?: string) => Promise<void> {
  const reload = useProjectReload();
  const { refresh } = useSession();

  return async (work, done) => {
    await attempt(work, done);
    await reload();
    await refresh();
  };
}

export function RemoveDialog({ what, options, onClose, onConfirm }: { what: string; options: { id: number; name: string }[]; onClose: () => void; onConfirm: (moveTo: number) => Promise<void> }) {
  const [moveTo, setMoveTo] = useState(options[0]?.id ?? 0);
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={t('Delete “{name}”?', { name: what })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await onConfirm(moveTo);
                onClose();
              })
            }
          >
            {t('Delete')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('Issues that have it now get')}>
          <Picker value={moveTo} options={options.map((option) => ({ value: option.id, label: option.name }))} onChange={setMoveTo} />
        </Field>
      </div>
    </Dialog>
  );
}
