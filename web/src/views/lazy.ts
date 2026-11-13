// The core views, each a chunk of its own: the browser fetches the code of a page when it
// is first shown (the page in the address right at the start, the rest when it is idle).
// The sign-in and set-up screens are small and stay with the shell.

import { lazyView } from '../lazy.tsx';

export const DashboardView = lazyView(() => import('./dashboard.tsx').then((loaded) => loaded.DashboardView));
export const ProjectsView = lazyView(() => import('./projects.tsx').then((loaded) => loaded.ProjectsView));
export const SearchView = lazyView(() => import('./search.tsx').then((loaded) => loaded.SearchView));
export const IssueView = lazyView(() => import('./issue.tsx').then((loaded) => loaded.IssueView));
export const SettingsView = lazyView(() => import('./settings.tsx').then((loaded) => loaded.SettingsView));
export const BoardView = lazyView(() => import('./board.tsx').then((loaded) => loaded.BoardView));
export const BacklogView = lazyView(() => import('./backlog.tsx').then((loaded) => loaded.BacklogView));
export const ReleasesView = lazyView(() => import('./releases.tsx').then((loaded) => loaded.ReleasesView));
export const ScheduleView = lazyView(() => import('./schedule.tsx').then((loaded) => loaded.ScheduleView));
export const RunnersView = lazyView(() => import('./runners.tsx').then((loaded) => loaded.RunnersView));
export const ProjectSettingsView = lazyView(() => import('./project-settings.tsx').then((loaded) => loaded.ProjectSettingsView));
// A dialog: nothing stands in for it while its code arrives.
export const CreateIssueDialog = lazyView(() => import('./create-issue.tsx').then((loaded) => loaded.CreateIssueDialog), null);

const ALL = [DashboardView, ProjectsView, SearchView, IssueView, SettingsView, BoardView, BacklogView, ReleasesView, ScheduleView, ProjectSettingsView, RunnersView, CreateIssueDialog];

/** Fetches the code of every core view that is not here yet. A view that fails says so when it is shown. */
export function preloadViews(): void {
  for (const view of ALL) {
    view.preload().catch(() => undefined);
  }
}
