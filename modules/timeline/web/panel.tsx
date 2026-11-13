import { api } from '@/api.ts';
import type { Issue } from '@/bindings/Issue.ts';
import type { TimelineSpan } from '@/bindings/TimelineSpan.ts';
import { canEdit } from '@/components/issue-bits.tsx';
import { attempt } from '@/components/ui.tsx';
import { formatDate, t } from '@/i18n/index.ts';
import { Link } from '@/router.tsx';
import { useSession } from '@/session.tsx';

/**
 * The issue's place on the timeline: its start date, which people who may edit set here
 * (the due date has its field among the issue's own), and the way to the project's timeline.
 */
export function IssuePanel({ issue, reload }: { issue: Issue; reload: () => void }) {
  const { boot } = useSession();
  const span = issue.panels.timeline as TimelineSpan | undefined;

  if (!span) {
    return null;
  }

  const project = boot.projects.find((candidate) => candidate.id === issue.projectId);
  const editable = project !== undefined && !project.archived && canEdit(project);
  const estimates = [
    span.derived.start && span.start ? t('starts {date}', { date: formatDate(span.start) }) : null,
    span.derived.due && span.due ? t('due {date}', { date: formatDate(span.due) }) : null,
  ].flatMap((text) => (text ? [text] : []));

  async function setStart(start: string | null): Promise<void> {
    await attempt(() => api.put(`/api/m/timeline/issues/${issue.key}/span`, { start, due: issue.dueDate }));
    reload();
  }

  return (
    <section className="issue-section">
      <div className="section-head">
        <h3>{t('Timeline')}</h3>
        <span className="filter-spacer" />
        <Link href={`/p/${issue.projectKey}/timeline`} className="timeline-more">
          {t('Open the timeline')}
        </Link>
      </div>
      <div className="timeline-panel">
        <label className="timeline-panel-field">
          <span className="muted">{t('Start date')}</span>
          <input
            type="date"
            className="input input-inline"
            value={span.derived.start ? '' : (span.start ?? '')}
            disabled={!editable}
            onChange={(event) => void setStart(event.target.value || null)}
          />
        </label>
        <span className="timeline-panel-field">
          <span className="muted">{t('Due date')}</span>
          <span>{issue.dueDate ? formatDate(issue.dueDate) : t('not set')}</span>
        </span>
      </div>
      {estimates.length > 0 && <p className="muted timeline-panel-note">{t('Estimated from its issues: {dates}', { dates: estimates.join(', ') })}</p>}
    </section>
  );
}
