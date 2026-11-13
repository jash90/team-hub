import { useMemo, type ReactNode } from 'react';

import type { Bootstrap } from './bindings/Bootstrap.ts';
import type { Issue } from './bindings/Issue.ts';
import type { IssueRow } from './bindings/IssueRow.ts';
import type { Project } from './bindings/Project.ts';
import type { RunnerWork } from './bindings/RunnerWork.ts';
import type { IconName } from './components/icon.tsx';
import { addTranslations } from './i18n/index.ts';
import type { Params } from './router.tsx';
import { useSession } from './session.tsx';

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
  /** What a runner does for this module, in a few words, for the list of runners. */
  runnerWork?: (work: RunnerWork) => string;
  /** Polish for the module's phrases. */
  polish?: Record<string, string>;
};

type Screens = () => Promise<{ default: WebModule }>;

/**
 * Every module that has screens, by its id: one line each, and all the interface knows about
 * a module. The screens are a chunk of their own, fetched only when the server has the module
 * and it is switched on. The order here is the order of navigation entries, panels and tabs.
 */
const SCREENS: Record<string, Screens> = {
  github: () => import('../../modules/github/web/index.tsx'),
  wiki: () => import('../../modules/wiki/web/index.tsx'),
  confluence: () => import('../../modules/confluence/web/index.tsx'),
  jira: () => import('../../modules/jira/web/index.tsx'),
  timeline: () => import('../../modules/timeline/web/index.tsx'),
  reports: () => import('../../modules/reports/web/index.tsx'),
  e2e: () => import('../../modules/e2e/web/index.tsx'),
  agents: () => import('../../modules/agents/web/index.tsx'),
};

const loaded = new Map<string, WebModule>();
const reported = new Set<string>();

/**
 * Fetches, side by side, the screens of the modules that are switched on and not here yet.
 * The application calls this before it hands a bootstrap to the session, so a module that
 * counts as switched on has its screens: at the start, and when one is switched on later.
 * Returns the ids of the modules whose screens could not be fetched, each only the first
 * time. Such a module is left out of the interface and asked for again on the next call,
 * though a browser may remember the failure until the page is reloaded.
 */
export async function loadModules(boot: Bootstrap): Promise<string[]> {
  const wanted = boot.modules.filter((module) => module.enabled && module.id in SCREENS && !loaded.has(module.id));
  const failed: string[] = [];

  await Promise.all(
    wanted.map(async ({ id }) => {
      try {
        const module = (await SCREENS[id]()).default;

        if (module.polish) {
          addTranslations(module.polish);
        }

        loaded.set(id, module);
      } catch (error) {
        console.warn(`The screens of the module "${id}" could not be loaded`, error);

        if (!reported.has(id)) {
          reported.add(id);
          failed.push(id);
        }
      }
    }),
  );

  return failed;
}

/**
 * The modules that are compiled in and switched on. Given a project (its key or its id),
 * those that are not switched off in that project.
 */
export function useModules(project?: string | number | null): WebModule[] {
  const { boot, moduleEnabled } = useSession();

  return useMemo(() => {
    const enabled = Object.keys(SCREENS).flatMap((id) => {
      const module = moduleEnabled(id) ? loaded.get(id) : undefined;

      return module ? [module] : [];
    });

    if (project === undefined || project === null) {
      return enabled;
    }

    const off = boot.projects.find((candidate) => (typeof project === 'number' ? candidate.id === project : candidate.key === project))?.disabledModules ?? [];

    return off.length > 0 ? enabled.filter((module) => !off.includes(module.id)) : enabled;
  }, [boot, moduleEnabled, project]);
}
