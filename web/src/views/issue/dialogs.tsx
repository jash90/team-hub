import { useState } from 'react';

import { api } from '../../api.ts';
import type { Issue } from '../../bindings/Issue.ts';
import type { QuickHit } from '../../bindings/QuickHit.ts';
import { Button, Dialog, Field, Picker, useBusy } from '../../components/ui.tsx';
import { formatMinutes, t } from '../../i18n/index.ts';
import { useRemote } from '../../session.tsx';

export function LinkDialog({ issue, onClose, onDone }: { issue: Issue; onClose: () => void; onDone: () => void }) {
  const [kind, setKind] = useState('blocks');
  const [target, setTarget] = useState('');
  const [busy, run] = useBusy();
  const hits = useRemote(() => (target.trim().length > 1 ? api.get<QuickHit[]>(`/api/quick?q=${encodeURIComponent(target)}`) : Promise.resolve([])), [target]);
  const kinds = [
    { value: 'blocks', label: t('blocks') },
    { value: 'blocked', label: t('is blocked by') },
    { value: 'relates', label: t('relates to') },
    { value: 'duplicates', label: t('duplicates') },
  ];

  function save(): void {
    const key = target.trim().toUpperCase();

    void run(async () => {
      if (kind === 'blocked') {
        await api.post(`/api/issues/${key}/links`, { type: 'blocks', targetKey: issue.key });
      } else {
        await api.post(`/api/issues/${issue.key}/links`, { type: kind, targetKey: key });
      }

      onDone();
      onClose();
    });
  }

  return (
    <Dialog
      title={t('Link {key}', { key: issue.key })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" disabled={busy || !target.trim()} onClick={save}>
            {t('Link')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('This issue')}>
          <Picker value={kind} options={kinds} onChange={setKind} />
        </Field>
        <Field label={t('Issue')} hint={t('A key such as WEB-12, or words from the summary')}>
          <input className="input" autoFocus value={target} onChange={(event) => setTarget(event.target.value)} onKeyDown={(event) => event.key === 'Enter' && save()} />
        </Field>
        <div className="menu menu-flat">
          {(hits.data ?? [])
            .filter((hit) => hit.kind === 'issue' && hit.id !== issue.key)
            .slice(0, 6)
            .map((hit) => (
              <button key={hit.id} type="button" className="menu-item quick-hit" onClick={() => setTarget(hit.id)}>
                <span className="issue-key">{hit.id}</span>
                <span className="quick-hit-title">{hit.title}</span>
              </button>
            ))}
        </div>
      </div>
    </Dialog>
  );
}

export function WorkDialog({ issue, onClose, onDone }: { issue: Issue; onClose: () => void; onDone: () => void }) {
  const [spent, setSpent] = useState('');
  const [note, setNote] = useState('');
  const [estimate, setEstimate] = useState('');
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={t('Log work on {key}', { key: issue.key })}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant="primary"
            disabled={busy || (!spent.trim() && !estimate.trim())}
            onClick={() =>
              void run(async () => {
                if (spent.trim()) {
                  await api.post(`/api/issues/${issue.key}/worklogs`, { spent, note });
                }

                if (estimate.trim()) {
                  const hours = Number(estimate.replace(',', '.'));
                  await api.patch(`/api/issues/${issue.key}`, { estimateMinutes: Number.isFinite(hours) ? Math.round(hours * 60) : null });
                }

                onDone();
                onClose();
              })
            }
          >
            {t('Save')}
          </Button>
        </>
      }
    >
      <div className="form">
        <Field label={t('Time spent')} hint={t('For example 1h 30m, 45m or 2d (a day is 8 hours)')}>
          <input className="input" autoFocus value={spent} onChange={(event) => setSpent(event.target.value)} placeholder="1h 30m" />
        </Field>
        <Field label={t('What was done')}>
          <input className="input" value={note} onChange={(event) => setNote(event.target.value)} />
        </Field>
        <Field label={t('Original estimate (hours)')} hint={issue.estimateMinutes !== null ? t('Currently {estimate}', { estimate: formatMinutes(issue.estimateMinutes) }) : undefined}>
          <input className="input" inputMode="decimal" value={estimate} onChange={(event) => setEstimate(event.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}
