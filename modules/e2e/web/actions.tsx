import { useState, type FormEvent } from 'react';

import { api } from '@/api.ts';
import type { E2eIssueRef } from '@/bindings/E2eIssueRef.ts';
import type { E2eRun } from '@/bindings/E2eRun.ts';
import type { E2eRunResult } from '@/bindings/E2eRunResult.ts';
import type { E2eStatus } from '@/bindings/E2eStatus.ts';
import type { E2eTest } from '@/bindings/E2eTest.ts';
import type { IssueRow } from '@/bindings/IssueRow.ts';
import type { Project } from '@/bindings/Project.ts';
import type { IconName } from '@/components/icon.tsx';
import { openIssue } from '@/components/issue-bits.tsx';
import { Button, ConfirmDialog, Dialog, Field, type MenuItem, attempt, toast, useBusy } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';

import { BASE, runPath, shortCommit } from './shared.tsx';

/** What "Create bug" and the dialogs need to know about a failure, from a test or from a run's result. */
export type Failure = {
  testId: number;
  title: string;
  file: string;
  line: number | null;
  message: string | null;
  runId: number | null;
  branch: string | null;
  commit: string | null;
  issues: E2eIssueRef[];
};

export function bugDescription(project: Project, failure: Failure): string {
  const where = `${failure.file}${failure.line ? `:${failure.line}` : ''}`;
  const lines = [t('The end-to-end test **{title}** fails.', { title: failure.title }), '', `- ${t('File')}: \`${where}\``];

  if (failure.runId) {
    const origin = [failure.branch ? `${t('branch')} \`${failure.branch}\`` : '', failure.commit ? `commit \`${shortCommit(failure.commit)}\`` : ''].filter(Boolean).join(', ');

    lines.push(`- ${t('Run')}: [${t('Run {id}', { id: failure.runId })}](${runPath(project.key, failure.runId)})${origin ? ` (${origin})` : ''}`);
  }

  if (failure.issues.length > 0) {
    lines.push(`- ${t('Verifies')}: ${failure.issues.map((issue) => issue.key).join(', ')}`);
  }

  if (failure.message) {
    lines.push('', '```', failure.message.replace(/```/g, "'''"), '```');
  }

  return lines.join('\n');
}

export function BugDialog({ project, failure, bugTypeId, onClose, onDone }: { project: Project; failure: Failure; bugTypeId: number | null; onClose: () => void; onDone: () => void }) {
  const [summary, setSummary] = useState(() => t('Failing test: {title}', { title: failure.title.split(' > ').pop() ?? failure.title }).slice(0, 200));
  const [description, setDescription] = useState(() => bugDescription(project, failure));
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      const created = await api.post<IssueRow>('/api/issues', { projectKey: project.key, summary, description, ...(bugTypeId ? { typeId: bugTypeId } : {}) });
      await api.put(`${BASE}/tests/${failure.testId}/issues/${created.key}`, {});
      toast(t('Created {key} and linked the test to it', { key: created.key }));
      onDone();
      onClose();
      openIssue(created.key);
    });
  }

  return (
    <Dialog
      title={bugTypeId ? t('Create bug') : t('Create issue')}
      wide
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="e2e-bug-form" disabled={busy || !summary.trim()}>
            {bugTypeId ? t('Create bug') : t('Create issue')}
          </Button>
        </>
      }
    >
      <form id="e2e-bug-form" className="form" onSubmit={submit}>
        <Field label={t('Summary')}>
          <input className="input" required maxLength={250} value={summary} onChange={(event) => setSummary(event.target.value)} />
        </Field>
        <Field label={t('Description')} hint={t('The test is linked to the new issue, so its next results show up there.')}>
          <textarea className="input input-mono e2e-textarea" rows={12} value={description} onChange={(event) => setDescription(event.target.value)} />
        </Field>
      </form>
    </Dialog>
  );
}

export function LinkDialog({ project, failure, onClose, onDone }: { project: Project; failure: Failure; onClose: () => void; onDone: () => void }) {
  const [key, setKey] = useState('');
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      await api.put(`${BASE}/tests/${failure.testId}/issues/${encodeURIComponent(key.trim())}`, {});
      onDone();
      onClose();
    }, t('Test linked'));
  }

  return (
    <Dialog
      title={t('Link test to an issue')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="e2e-link-form" disabled={busy || !key.trim()}>
            {t('Link')}
          </Button>
        </>
      }
    >
      <form id="e2e-link-form" className="form" onSubmit={submit}>
        <p className="e2e-title">{failure.title}</p>
        <Field label={t('Issue key')} hint={t('A link made here stays through later runs. Tests are also linked on their own when a title or tag names an issue, like {key}-12.', { key: project.key })}>
          <input className="input input-mono" required placeholder={`${project.key}-12`} value={key} onChange={(event) => setKey(event.target.value)} />
        </Field>
      </form>
    </Dialog>
  );
}

export type Acting = { kind: 'bug' | 'link' | 'forget'; failure: Failure };

export function failureOfTest(test: E2eTest): Failure {
  return { testId: test.id, title: test.title, file: test.file, line: test.line, message: test.message, runId: test.lastRunId, branch: test.branch, commit: test.commit, issues: test.issues };
}

export function failureOfResult(result: E2eRunResult, run: E2eRun): Failure {
  return { testId: result.testId, title: result.title, file: result.file, line: result.line, message: result.message, runId: run.id, branch: run.branch, commit: run.commit, issues: result.issues };
}

export function testMenu(failure: Failure, failed: boolean, bugTypeId: number | null, act: (acting: Acting) => void): MenuItem[] {
  return [
    ...(failed ? [{ label: bugTypeId ? t('Create bug') : t('Create issue'), icon: 'plus' as IconName, onSelect: () => act({ kind: 'bug', failure }) }] : []),
    { label: t('Link to an issue…'), icon: 'link', onSelect: () => act({ kind: 'link', failure }) },
    { label: t('Forget this test'), icon: 'trash', danger: true, onSelect: () => act({ kind: 'forget', failure }) },
  ];
}

export function Actions({ project, acting, bugTypeId, onClose, onDone }: { project: Project; acting: Acting | null; bugTypeId: number | null; onClose: () => void; onDone: () => void }) {
  if (!acting) {
    return null;
  }

  if (acting.kind === 'bug') {
    return <BugDialog project={project} failure={acting.failure} bugTypeId={bugTypeId} onClose={onClose} onDone={onDone} />;
  }

  if (acting.kind === 'link') {
    return <LinkDialog project={project} failure={acting.failure} onClose={onClose} onDone={onDone} />;
  }

  return (
    <ConfirmDialog
      title={t('Forget this test?')}
      body={t('“{title}” and its results are removed, with its links to issues. If a later run still has the test, it comes back.', { title: acting.failure.title })}
      confirm={t('Forget test')}
      danger
      onClose={onClose}
      onConfirm={async () => {
        await attempt(() => api.del(`${BASE}/tests/${acting.failure.testId}`));
        onDone();
        onClose();
      }}
    />
  );
}

export function Message({ text, status }: { text: string; status: E2eStatus | null }) {
  return <pre className={`e2e-message e2e-is-${status ?? 'none'}`}>{text}</pre>;
}
