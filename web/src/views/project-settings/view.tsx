import { Tabs } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { useModules } from '../../modules.ts';
import { navigate } from '../../router.tsx';
import { useProject } from '../../session.tsx';
import { Components } from './components.tsx';
import { General } from './general.tsx';
import { Members } from './members.tsx';
import { Modules } from './modules.tsx';
import { Types } from './types.tsx';
import { Workflow } from './workflow.tsx';

export function ProjectSettingsView({ tab }: { tab?: string }) {
  const project = useProject();
  const modules = useModules(project.key);
  const admin = project.role === 'admin';
  const moduleTabs = modules.flatMap((module) => module.projectSettings ?? []);
  const tabs = [
    { id: 'general', label: t('General') },
    { id: 'workflow', label: t('Workflow') },
    { id: 'types', label: t('Issue types') },
    { id: 'components', label: t('Components') },
    { id: 'members', label: t('Members') },
    { id: 'modules', label: t('Modules') },
    ...moduleTabs.map((entry) => ({ id: entry.id, label: entry.label() })),
  ];
  const active = tabs.some((candidate) => candidate.id === tab) ? (tab as string) : 'general';
  const fromModule = moduleTabs.find((entry) => entry.id === active);

  return (
    <div className="view view-narrow">
      <header className="view-head">
        <div>
          <h1>{t('Project settings')}</h1>
          <p className="muted">
            {project.name} · {project.key}
          </p>
        </div>
      </header>
      {!admin && <p className="note">{t('Only project administrators can change these settings.')}</p>}
      <Tabs tabs={tabs} active={active} onChange={(id) => navigate(`/p/${project.key}/settings/${id}`)} />
      <fieldset className="settings-body" disabled={!admin && !fromModule}>
        {active === 'general' && <General project={project} />}
        {active === 'workflow' && <Workflow project={project} />}
        {active === 'types' && <Types project={project} />}
        {active === 'components' && <Components project={project} />}
        {active === 'members' && <Members project={project} />}
        {active === 'modules' && <Modules project={project} />}
      </fieldset>
      {fromModule?.render(project)}
    </div>
  );
}
