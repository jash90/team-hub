import { useEffect, useState } from 'react';

import { api } from '../../api.ts';
import type { IssueRow } from '../../bindings/IssueRow.ts';
import type { Project } from '../../bindings/Project.ts';
import { DueDate, IssueKey, ModuleBadges, PriorityIcon, StatusPill, TypeIcon } from '../../components/issue-bits.tsx';
import { Avatar } from '../../components/ui.tsx';
import { t, timeAgo } from '../../i18n/index.ts';
import { useSession } from '../../session.tsx';

/** Project configurations by id, loaded as rows from those projects show up. */
export function useProjects(ids: number[]): Map<number, Project> {
  const { boot } = useSession();
  const [projects, setProjects] = useState<Map<number, Project>>(new Map());
  const wanted = ids.join(',');

  useEffect(() => {
    for (const id of ids) {
      const summary = boot.projects.find((project) => project.id === id);

      if (summary && !projects.has(id)) {
        void api.get<Project>(`/api/projects/${summary.key}`).then((project) => setProjects((current) => new Map(current).set(id, project)));
      }
    }
    // Only the set of ids matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wanted]);

  return projects;
}

export function ResultsTable({ issues, projects, selected, setSelected }: { issues: IssueRow[]; projects: Map<number, Project>; selected: string[]; setSelected: (keys: string[]) => void }) {
  const allSelected = issues.length > 0 && selected.length === issues.length;

  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            <th className="cell-check">
              <input type="checkbox" checked={allSelected} aria-label={t('Select all')} onChange={() => setSelected(allSelected ? [] : issues.map((issue) => issue.key))} />
            </th>
            <th>{t('Key')}</th>
            <th>{t('Summary')}</th>
            <th>{t('Status')}</th>
            <th>{t('Assignee')}</th>
            <th className="cell-narrow">{t('Priority')}</th>
            <th className="cell-narrow">{t('Points')}</th>
            <th>{t('Updated')}</th>
          </tr>
        </thead>
        <tbody>
          {issues.map((issue) => (
            <ResultRow key={issue.id} issue={issue} project={projects.get(issue.projectId)} selected={selected.includes(issue.key)} onToggle={() => setSelected(selected.includes(issue.key) ? selected.filter((key) => key !== issue.key) : [...selected, issue.key])} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ResultRow({ issue, project, selected, onToggle }: { issue: IssueRow; project: Project | undefined; selected: boolean; onToggle: () => void }) {
  const { userById } = useSession();
  const status = project?.statuses.find((candidate) => candidate.id === issue.statusId);
  const assignee = userById(issue.assigneeId);

  return (
    <tr className={selected ? 'row-selected' : undefined}>
      <td className="cell-check">
        <input type="checkbox" checked={selected} onChange={onToggle} aria-label={issue.key} />
      </td>
      <td className="cell-key">
        <span className="with-avatar">
          <TypeIcon type={project?.types.find((candidate) => candidate.id === issue.typeId)} />
          <IssueKey issue={issue} done={status?.category === 'done'} />
        </span>
      </td>
      <td className="cell-summary">
        <span className="summary-text">{issue.summary}</span>
        <span className="summary-extra">
          {issue.labels.map((label) => (
            <span key={label} className="label">
              {label}
            </span>
          ))}
          <ModuleBadges issue={issue} />
          <DueDate date={issue.dueDate} done={status?.category === 'done'} />
        </span>
      </td>
      <td>{status ? <StatusPill status={status} /> : null}</td>
      <td className="cell-date">
        <span className="with-avatar">
          <Avatar user={assignee} size={20} />
          <span className={assignee ? '' : 'muted'}>{assignee?.name ?? t('Unassigned')}</span>
        </span>
      </td>
      <td className="cell-narrow">
        <PriorityIcon priority={issue.priority} />
      </td>
      <td className="cell-narrow">{issue.points ?? ''}</td>
      <td className="muted cell-date">{timeAgo(issue.updatedAt)}</td>
    </tr>
  );
}
