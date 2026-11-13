import type { ReactNode } from 'react';

import type { Issue } from './bindings/Issue.ts';
import type { IssueRow } from './bindings/IssueRow.ts';
import type { Project } from './bindings/Project.ts';
import type { IconName } from './components/icon.tsx';
import { addTranslations } from './i18n/index.ts';
import type { Params } from './router.tsx';
import { useSession } from './session.tsx';

import { agentsModule } from '../../modules/agents/web/index.tsx';
import { confluenceModule } from '../../modules/confluence/web/index.tsx';
import { e2eModule } from '../../modules/e2e/web/index.tsx';
import { githubModule } from '../../modules/github/web/index.tsx';
import { reportsModule } from '../../modules/reports/web/index.tsx';
import { wikiModule } from '../../modules/wiki/web/index.tsx';

export type NavEntry = { id: string; label: () => string; icon: IconName; path: string };

/**
 * The web half of a module. Everything is optional: a module shows up only in the places
 * it has something for.
 */
export type WebModule = {
  /** The same id as the server half. */
  id: string;
  /** Entries in a project's navigation. */
  projectNav?: (projectKey: string) => NavEntry[];
  /** Entries in the main navigation. */
  globalNav?: () => NavEntry[];
  /** Full pages; a pattern under `/p/:key/` renders inside the project. */
  routes?: { pattern: string; render: (params: Params) => ReactNode }[];
  /** Small marks on an issue's card or row (it is given the module's `ext` entry). */
  badges?: (props: { issue: IssueRow }) => ReactNode;
  /** A panel on the issue page, given `issue.panels[id]`. */
  issuePanel?: (props: { issue: Issue; reload: () => void }) => ReactNode;
  /** A section of "My work", given `dashboard.ext[id]`. */
  dashboard?: (props: { data: unknown }) => ReactNode;
  /** Tabs in a project's settings. */
  projectSettings?: { id: string; label: () => string; render: (project: Project) => ReactNode }[];
  /** Tabs in the instance settings. */
  settings?: { id: string; label: () => string; adminOnly?: boolean; render: () => ReactNode }[];
  /** Polish for the module's phrases. */
  polish?: Record<string, string>;
};

const ALL: WebModule[] = [githubModule, wikiModule, confluenceModule, reportsModule, e2eModule, agentsModule];

for (const module of ALL) {
  if (module.polish) {
    addTranslations(module.polish);
  }
}

/** The modules that are compiled in and switched on. */
export function useModules(): WebModule[] {
  const { moduleEnabled } = useSession();

  return ALL.filter((module) => moduleEnabled(module.id));
}
