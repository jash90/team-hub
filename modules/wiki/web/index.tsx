import { t } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';

import { GoView } from './go.tsx';
import { HomeView } from './home.tsx';
import { IssuePanel } from './issue-panel.tsx';
import { spaceOfProject } from './lib.ts';
import type { PageMode } from './page.tsx';
import { polish } from './pl.ts';
import { SpaceView } from './space.tsx';
import './styles.css';

function pageId(text: string | undefined): number | null {
  const id = Number(text);

  return text && Number.isInteger(id) && id > 0 ? id : null;
}

function modeOf(text: string | undefined): PageMode {
  return text === 'edit' || text === 'history' ? text : 'view';
}

const wikiModule: WebModule = {
  id: 'wiki',
  globalNav: () => [{ id: 'wiki', label: () => t('Knowledge base'), icon: 'book', path: '/wiki' }],
  // The navigation cannot wait for the list of spaces: once it has been loaded the entry
  // leads to the project's space, before that to the list, which sends the person on.
  projectNav: (projectKey) => {
    const space = spaceOfProject(projectKey);

    return [{ id: 'wiki', label: () => t('Knowledge'), icon: 'page', path: space ? `/wiki/${space}` : `/wiki?project=${projectKey}` }];
  },
  routes: [
    { pattern: '/wiki-go', render: () => <GoView /> },
    { pattern: '/wiki', render: () => <HomeView /> },
    { pattern: '/wiki/:space', render: (params) => <SpaceView spaceKey={params.space} pageId={null} mode="view" /> },
    { pattern: '/wiki/:space/:page/:mode?', render: (params) => <SpaceView spaceKey={params.space} pageId={pageId(params.page)} mode={modeOf(params.mode)} /> },
  ],
  issuePanel: IssuePanel,
  polish,
};

export default wikiModule;
