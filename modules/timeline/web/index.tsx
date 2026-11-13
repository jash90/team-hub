import { t } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';

import { TimelineView } from './page.tsx';
import { IssuePanel } from './panel.tsx';
import { polish } from './pl.ts';
import './styles.css';

const timelineModule: WebModule = {
  id: 'timeline',
  projectNav: (projectKey) => [{ id: 'timeline', label: () => t('Timeline'), icon: 'calendar', path: `/p/${projectKey}/timeline` }],
  routes: [{ pattern: '/p/:key/timeline', render: (params) => <TimelineView key={params.key} /> }],
  issuePanel: IssuePanel,
  polish,
};

export default timelineModule;
