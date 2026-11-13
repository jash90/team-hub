// The commands of the core, typed. The names and arguments are those of
// `hub-desktop-core/src/commands.rs`; the types are generated from the Rust ones.
import type { ConnectionStatus } from './bindings/ConnectionStatus.ts';
import type { DashboardView } from './bindings/DashboardView.ts';
import type { IssueView } from './bindings/IssueView.ts';
import type { NewThread } from './bindings/NewThread.ts';
import type { Proposal } from './bindings/Proposal.ts';
import type { PullRequestPreview } from './bindings/PullRequestPreview.ts';
import type { PullRequestResult } from './bindings/PullRequestResult.ts';
import type { PushPreview } from './bindings/PushPreview.ts';
import type { PushResult } from './bindings/PushResult.ts';
import type { RepoCheck } from './bindings/RepoCheck.ts';
import type { RepoMapping } from './bindings/RepoMapping.ts';
import type { RepoStatus } from './bindings/RepoStatus.ts';
import type { SearchView } from './bindings/SearchView.ts';
import type { Settings } from './bindings/Settings.ts';
import type { Thread } from './bindings/Thread.ts';
import type { ThreadDetail } from './bindings/ThreadDetail.ts';
import type { ThreadDiff } from './bindings/ThreadDiff.ts';
import type { WorkspaceView } from './bindings/WorkspaceView.ts';
import type { AgentRun } from '@web/bindings/AgentRun.ts';
import type { AgentRunDetail } from '@web/bindings/AgentRunDetail.ts';
import type { E2eJob } from '@web/bindings/E2eJob.ts';
import type { E2eJobDetail } from '@web/bindings/E2eJobDetail.ts';
import type { E2eSettings } from '@web/bindings/E2eSettings.ts';
import type { GithubProjectCode } from '@web/bindings/GithubProjectCode.ts';
import type { Runner } from '@web/bindings/Runner.ts';
import type { Delegation } from './bindings/Delegation.ts';
import type { NewTestJob } from './bindings/NewTestJob.ts';
import { call } from './bridge.ts';

export const core = {
  status: () => call<ConnectionStatus>('status'),
  connect: (url: string, token: string) => call<ConnectionStatus>('connect', { url, token }),
  disconnect: () => call<ConnectionStatus>('disconnect'),
  settings: () => call<Settings>('settings'),
  saveSettings: (settings: Settings) => call<Settings>('save_settings', settings),
  models: () => call<string[]>('models'),
  workspace: () => call<WorkspaceView>('workspace'),
  dashboard: () => call<DashboardView>('dashboard'),
  issue: (key: string) => call<IssueView>('issue', { key }),
  search: (query: string) => call<SearchView>('search', { query }),
  comment: (key: string, body: string) => call<IssueView>('comment', { key, body }),
  transition: (key: string, statusId: number) => call<IssueView>('transition', { key, statusId }),
  assignToMe: (key: string) => call<IssueView>('assign_to_me', { key }),
  logWork: (key: string, spent: string, note: string) => call<IssueView>('log_work', { key, spent, note }),
  projectCode: (key: string) => call<GithubProjectCode | null>('project_code', { key }),
  repos: () => call<RepoStatus[]>('repos'),
  checkRepo: (path: string, githubRepo: string | null) => call<RepoCheck>('check_repo', { path, githubRepo }),
  mapRepo: (projectKey: string, path: string, githubRepo: string | null) => call<RepoMapping>('map_repo', { projectKey, path, githubRepo }),
  unmapRepo: (id: number) => call<null>('unmap_repo', { id }),
  threads: () => call<Thread[]>('threads'),
  thread: (id: number) => call<ThreadDetail>('thread', { id }),
  startThread: (thread: NewThread) => call<Thread>('start_thread', thread),
  stopThread: (id: number) => call<Thread>('stop_thread', { id }),
  sayToThread: (id: number, text: string | null) => call<Thread>('say_to_thread', { id, text }),
  threadDiff: (id: number) => call<ThreadDiff>('thread_diff', { id }),
  removeWorktree: (id: number) => call<Thread>('remove_worktree', { id }),
  forgetThread: (id: number) => call<null>('forget_thread', { id }),
  pendingProposals: () => call<Proposal[]>('pending_proposals'),
  decideProposal: (id: number, approve: boolean, note: string | null) => call<Proposal>('decide_proposal', { id, approve, note }),
  pushPreview: (id: number) => call<PushPreview>('push_preview', { id }),
  push: (id: number, commit: string, confirmedFindings: boolean) => call<PushResult>('push', { id, commit, confirmedFindings }),
  pullRequestPreview: (id: number) => call<PullRequestPreview>('pull_request_preview', { id }),
  openPullRequest: (id: number, title: string, body: string) => call<PullRequestResult>('open_pull_request', { id, title, body }),
  recordPullRequest: (id: number, url: string) => call<Thread>('record_pull_request', { id, url }),
  // The same runners and work as the web shows (docs/runners.md).
  runs: (mine: boolean) => call<AgentRun[]>('runs', { mine }),
  runners: () => call<Runner[]>('runners'),
  delegate: (work: Delegation) => call<AgentRun>('delegate', work),
  runDetail: (id: number) => call<AgentRunDetail>('run_detail', { id }),
  decideOnRun: (id: number, approve: boolean, note: string | null) => call<AgentRunDetail>('decide_on_run', { id, approve, note }),
  cancelRun: (id: number) => call<AgentRun>('cancel_run', { id }),
  testSettings: (key: string) => call<E2eSettings | null>('test_settings', { key }),
  testJobs: (key: string) => call<E2eJob[]>('test_jobs', { key }),
  startTestJob: (job: NewTestJob) => call<E2eJob>('start_test_job', job),
  testJob: (id: number, after: number) => call<E2eJobDetail>('test_job', { id, after }),
  cancelTestJob: (id: number) => call<E2eJob>('cancel_test_job', { id }),
};
