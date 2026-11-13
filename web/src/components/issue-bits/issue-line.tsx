import type { ReactNode } from 'react';

import type { IssueRow } from '../../bindings/IssueRow.ts';
import type { Project } from '../../bindings/Project.ts';
import { useSession } from '../../session.tsx';
import { Avatar } from '../ui/avatar.tsx';
import { DueDate } from './due-date.tsx';
import { IssueKey } from './issue-key.tsx';
import { ModuleBadges } from './module-badges.tsx';
import { PriorityIcon } from './priority.tsx';
import { StatusPill } from './status-pill.tsx';
import { TypeIcon } from './type-icon.tsx';

type LineProps = {
  issue: IssueRow;
  project?: Project;
  /** Extra content at the end of the line. */
  trailing?: ReactNode;
  /** Leaves out labels and the epic, where the line has little room. */
  compact?: boolean;
};

/** One issue on one line: lists on the dashboard, in the backlog, under an epic. */
export function IssueLine({ issue, project, trailing, compact }: LineProps) {
  const { userById } = useSession();
  const status = project?.statuses.find((candidate) => candidate.id === issue.statusId);
  const type = project?.types.find((candidate) => candidate.id === issue.typeId);

  return (
    <div className="issue-line">
      {project && <TypeIcon type={type} />}
      <IssueKey issue={issue} done={status?.category === 'done'} />
      <span className="issue-line-summary" title={issue.summary}>
        {issue.summary}
      </span>
      <span className="issue-line-meta">
        {!compact && issue.parentKey && type?.level === 'standard' && <span className="epic-chip" title={issue.parentSummary ?? ''}>{issue.parentSummary}</span>}
        {!compact &&
          issue.labels.slice(0, 3).map((label) => (
            <span key={label} className="label">
              {label}
            </span>
          ))}
        <ModuleBadges issue={issue} />
        <DueDate date={issue.dueDate} done={status?.category === 'done'} short />
        {status && <StatusPill status={status} />}
        {issue.points !== null && <span className="points">{issue.points}</span>}
        <PriorityIcon priority={issue.priority} />
        <Avatar user={userById(issue.assigneeId)} size={22} />
        {trailing}
      </span>
    </div>
  );
}
