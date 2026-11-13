import { useState } from 'react';

import type { AgentRun } from '@/bindings/AgentRun.ts';
import type { GithubIssuePanel } from '@/bindings/GithubIssuePanel.ts';
import type { Issue } from '@/bindings/Issue.ts';
import type { IssueRow } from '@/bindings/IssueRow.ts';
import { Icon } from '@/components/icon.tsx';
import { IssueKey, canEdit } from '@/components/issue-bits.tsx';
import { Button } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { useSession } from '@/session.tsx';

import { RunCard, StartDialog, needsPerson, useAgentSettings, useRuns, type StartPreset } from './runs.tsx';

function presets(): StartPreset[] {
  return [
    { kind: 'refine', title: t('Refine description'), hint: t('The agent rewrites the description so the work can be picked up without questions, and proposes it as a change you can compare with the present text.') },
    {
      kind: 'refine',
      title: t('Acceptance criteria'),
      instructions: t('Add acceptance criteria as a checklist. Leave the rest of the description as it is.'),
      hint: t('The agent proposes a description with acceptance criteria added; you see the difference before anything changes.'),
    },
    { kind: 'breakdown', title: t('Break down into subtasks'), hint: t('The agent proposes a list of subtasks. You can edit the list before any of them is created.') },
    { kind: 'summarize', title: t('Summarise'), hint: t('The agent reads the description, the comments and what is known about code and tests, and writes where the issue stands. Nothing is changed.') },
    { kind: 'triage', title: t('Triage'), hint: t('The agent suggests a priority and labels, and points out what is missing or duplicated. Each suggestion is yours to approve.') },
    { kind: 'implement', title: t('Delegate implementation…'), hint: t('A runner with the repository on its machine makes the change on a branch and opens a pull request. It reports here as it goes.') },
    { kind: 'custom', title: t('Custom…'), hint: t('Say what you want done with this issue. The agent can read it and propose changes; it cannot change anything by itself.') },
  ];
}

/** The "AI agent" section of the issue page: hand work to an agent, and follow what it does. */
export function IssuePanel({ issue, reload }: { issue: Issue; reload: () => void }) {
  const { boot } = useSession();
  const settings = useAgentSettings();
  const [preset, setPreset] = useState<StartPreset | null>(null);
  const runs = useRuns(`/issues/${issue.key}/runs`, [issue.key], (issueId) => issueId === issue.id);
  const listed = runs.data ?? ((issue.panels.agents as { runs?: AgentRun[] } | undefined)?.runs ?? []);
  const editable = canEdit(boot.projects.find((project) => project.id === issue.projectId) ?? { role: null });
  const repo = (issue.panels.github as GithubIssuePanel | undefined)?.repos[0]?.fullName ?? '';

  // An approved proposal changes the issue itself, so the page around the panel is read again.
  function changed(): void {
    void runs.reload();
    reload();
  }

  return (
    <section className="issue-section agents-panel">
      <div className="section-head">
        <h3>{t('AI agent')}</h3>
      </div>
      {editable ? (
        <div className="agents-actions">
          {presets().map((option) => (
            <Button key={option.title} size="sm" icon={option.kind === 'implement' ? 'branch' : option.kind === 'custom' ? 'edit' : 'bot'} onClick={() => setPreset(option)}>
              {option.title}
            </Button>
          ))}
        </div>
      ) : (
        listed.length === 0 && <p className="muted">{t('Nobody has handed this issue to an agent.')}</p>
      )}
      {listed.map((run, index) => (
        <RunCard key={run.id} run={run} canDecide={editable} settings={settings.data} open={index === 0 && listed.length === 1} onChange={changed} />
      ))}
      {preset && <StartDialog issueKey={issue.key} projectKey={issue.projectKey} preset={preset} repo={repo} onClose={() => setPreset(null)} onStarted={changed} />}
    </section>
  );
}

/** A mark on cards and rows while an agent works on the issue, or waits for someone. */
export function Badges({ issue }: { issue: IssueRow }) {
  const state = issue.ext.agents as { working: number; waiting: number } | undefined;

  if (!state) {
    return null;
  }

  return state.waiting > 0 ? (
    <span className="agents-badge agents-badge-waiting" title={t('An AI agent waits for a decision or an answer')}>
      <Icon name="bot" size={12} />!
    </span>
  ) : (
    <span className="agents-badge" title={t('An AI agent is working on this')}>
      <Icon name="bot" size={12} />
    </span>
  );
}

/** "My work": runs the viewer asked for that cannot go on without them. */
export function DashboardPart({ data }: { data: unknown }) {
  const waiting = (data as { waiting: AgentRun[] }).waiting.filter(needsPerson);

  if (waiting.length === 0) {
    return null;
  }

  return (
    <section className="panel">
      <header className="panel-head">
        <h2>{t('Waiting for your decision')}</h2>
        <span className="tab-count">{waiting.length}</span>
      </header>
      {waiting.map((run) => (
        <div key={run.id} className="agents-waiting-row">
          <Icon name="bot" size={15} />
          <IssueKey issue={{ key: run.issueKey }} />
          <span className="agents-run-summary">{run.issueSummary}</span>
          <span className="muted">{run.pendingQuestion ? t('asks a question') : run.pendingProposals === 1 ? t('1 proposal') : t('{count} proposals', { count: run.pendingProposals })}</span>
        </div>
      ))}
    </section>
  );
}
