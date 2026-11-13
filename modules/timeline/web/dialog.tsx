import { useState, type FormEvent } from 'react';

import type { TimelineDerived } from '@/bindings/TimelineDerived.ts';
import { Button, Dialog, ErrorNote, Field } from '@/components/ui.tsx';
import { formatDate, t } from '@/i18n/index.ts';

/** An issue whose dates are being set. Children carry no estimates, so `derived` is theirs to leave out. */
export type DatesTarget = { key: string; summary: string; start: string | null; due: string | null; derived?: TimelineDerived };

type DatesDialogProps = {
  target: DatesTarget;
  /** Called with the dates to store; null clears one. */
  onSave: (start: string | null, due: string | null) => void;
  onClose: () => void;
};

/**
 * The dates of one issue as two fields: the way to plan without dragging. An estimated date
 * is shown as a hint, not as the field's value, so saving does not turn an estimate into a plan.
 */
export function DatesDialog({ target, onSave, onClose }: DatesDialogProps) {
  const own = { start: target.derived?.start ? null : target.start, due: target.derived?.due ? null : target.due };
  const [start, setStart] = useState(own.start ?? '');
  const [due, setDue] = useState(own.due ?? '');
  const backwards = start !== '' && due !== '' && start > due;
  const estimate = (derived: boolean | undefined, date: string | null) => (derived && date ? t('Estimated from its issues: {date}', { date: formatDate(date) }) : undefined);

  function save(nextStart: string | null, nextDue: string | null): void {
    onSave(nextStart, nextDue);
    onClose();
  }

  function submit(event: FormEvent): void {
    event.preventDefault();

    if (!backwards) {
      save(start || null, due || null);
    }
  }

  return (
    <Dialog
      title={t('Dates of {key}', { key: target.key })}
      onClose={onClose}
      footer={
        <>
          <Button className="dialog-foot-left" onClick={() => save(null, null)} disabled={own.start === null && own.due === null}>
            {t('Clear')}
          </Button>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="timeline-dates" disabled={backwards}>
            {t('Save')}
          </Button>
        </>
      }
    >
      <form id="timeline-dates" className="form" onSubmit={submit}>
        <p className="timeline-dialog-summary">{target.summary}</p>
        <div className="form-row">
          <Field label={t('Start date')} hint={estimate(target.derived?.start, target.start)}>
            <input type="date" className="input" value={start} onChange={(event) => setStart(event.target.value)} />
          </Field>
          <Field label={t('Due date')} hint={estimate(target.derived?.due, target.due)}>
            <input type="date" className="input" value={due} onChange={(event) => setDue(event.target.value)} />
          </Field>
        </div>
        {backwards && <ErrorNote message={t('The start date is after the due date')} />}
      </form>
    </Dialog>
  );
}
