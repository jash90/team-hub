import { Icon } from '@/components/icon.tsx';
import { Tabs } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';
import { setParam, useSearchParams } from '@/router.tsx';
import { useProject } from '@/session.tsx';

import { polish } from './pl.ts';
import { Burndown, CreatedResolved, CumulativeFlow, CycleTime, Velocity, Workload } from './reports.tsx';
import './styles.css';

const SPRINT_REPORTS = ['burndown', 'velocity'];

function ReportsView() {
  const project = useProject();
  const wanted = useSearchParams().get('report');
  const sprints = project.boardMode !== 'kanban';
  const tabs = [
    { id: 'burndown', label: t('Burndown') },
    { id: 'velocity', label: t('Velocity') },
    { id: 'flow', label: t('Cumulative flow') },
    { id: 'pace', label: t('Created vs resolved') },
    { id: 'cycle', label: t('Cycle time') },
    { id: 'workload', label: t('Workload') },
  ].filter((tab) => sprints || !SPRINT_REPORTS.includes(tab.id));
  const active = tabs.find((tab) => tab.id === wanted)?.id ?? tabs[0].id;

  return (
    <div className="view reports">
      <header className="view-head">
        <div>
          <h1>{t('Reports')}</h1>
          <p className="muted">{t('Drawn from the history of the issues: every change of status, sprint and estimate counts from the moment it was made.')}</p>
        </div>
      </header>
      {!sprints && (
        <p className="reports-note">
          <Icon name="info" />
          {t('This project works as a kanban board, without sprints. Burndown and velocity measure sprints, so they are not shown here; flow and cycle time say the most about a kanban team.')}
        </p>
      )}
      <Tabs tabs={tabs} active={active} onChange={(id) => setParam('report', id, { replace: true })} />
      {active === 'burndown' && <Burndown project={project} />}
      {active === 'velocity' && <Velocity project={project} />}
      {active === 'flow' && <CumulativeFlow project={project} />}
      {active === 'pace' && <CreatedResolved project={project} />}
      {active === 'cycle' && <CycleTime project={project} />}
      {active === 'workload' && <Workload project={project} />}
    </div>
  );
}

export const reportsModule: WebModule = {
  id: 'reports',
  projectNav: (projectKey) => [{ id: 'reports', label: () => t('Reports'), icon: 'chart', path: `/p/${projectKey}/reports` }],
  routes: [{ pattern: '/p/:key/reports', render: (params) => <ReportsView key={params.key} /> }],
  polish,
};
