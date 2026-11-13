import { t } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';

import { AgentsPage } from './page.tsx';
import { Badges, DashboardPart, IssuePanel } from './panel.tsx';
import { polish } from './pl.ts';
import { AgentsSettings, ProjectTrust } from './settings.tsx';
import './styles.css';

const agentsModule: WebModule = {
  id: 'agents',
  globalNav: () => [{ id: 'agents', label: () => t('AI agents'), icon: 'bot', path: '/agents' }],
  routes: [{ pattern: '/agents', render: () => <AgentsPage /> }],
  badges: Badges,
  issuePanel: IssuePanel,
  dashboard: DashboardPart,
  projectSettings: [{ id: 'agents', label: () => t('AI agent'), render: (project) => <ProjectTrust project={project} /> }],
  settings: [{ id: 'agents', label: () => t('AI agents'), render: () => <AgentsSettings /> }],
  runnerWork: (work) => t('Agent runs: {kinds}', { kinds: work.what.map((kind) => t(kind)).join(', ') }),
  polish,
};

export default agentsModule;
