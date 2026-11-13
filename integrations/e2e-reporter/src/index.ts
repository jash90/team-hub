/**
 * Reporter for the e2e testing framework that posts each finished run to team-hub, where
 * every test is linked to the issues it verifies.
 *
 * ```ts
 * import { teamHub } from 'team-hub-e2e-reporter';
 *
 * export default {
 *   targets,
 *   reporters: ['list', teamHub({ url: 'https://hub.example.com', project: 'WEB', token: process.env.TEAM_HUB_TOKEN })],
 * } satisfies E2EConfig;
 * ```
 */

import { readFile } from 'node:fs/promises';
import type { FinishedRun, Reporter, ReporterSummary } from 'e2e';

const SUMMARY_LABEL = 'team-hub';

export interface TeamHubOptions {
  /** Where team-hub lives, such as `https://hub.example.com`. Default: `TEAM_HUB_URL`. */
  url?: string | undefined;
  /** The key of the project the tests belong to, such as `WEB`. Default: `TEAM_HUB_PROJECT`. */
  project?: string | undefined;
  /**
   * An API token of somebody who may edit the project (team-hub: Settings → API tokens).
   * Default: `TEAM_HUB_TOKEN`. Without one the run is not posted, and the run is not failed for it.
   */
  token?: string | undefined;
  /** What the run is called in team-hub. Default: the CI workflow and run number, such as `CI #128`. */
  name?: string | undefined;
  /** Overrides what the CI environment says about the run. */
  branch?: string | undefined;
  commit?: string | undefined;
  pullRequest?: number | undefined;
  ciUrl?: string | undefined;
}

/** What team-hub is told about a run beside the report. */
export interface RunMeta {
  name?: string;
  branch?: string;
  commit?: string;
  pullRequest?: number;
  ciUrl?: string;
}

/** What the reporter touches outside itself, so tests run it against fakes. */
export interface ReporterDeps {
  readonly fetch: typeof fetch;
  readonly env: NodeJS.ProcessEnv;
  readonly readFile: (file: string) => Promise<string>;
}

/**
 * The team-hub reporter. Everything it reads (the token, the CI environment) is read when
 * the run finishes, never when the config loads, so constructing it has no side effects.
 * It never changes the run's result: when posting fails, the summary says why.
 */
export function teamHub(options: TeamHubOptions = {}): Reporter {
  return {
    name: 'team-hub',
    onRunFinished: (run, signal) => postRun(run, signal, options, { fetch: globalThis.fetch, env: process.env, readFile: (file) => readFile(file, 'utf8') }),
  };
}

/** A variable's trimmed value; unset and blank both read as undefined. */
function value(env: NodeJS.ProcessEnv, name: string): string | undefined {
  const raw = env[name];

  return raw === undefined || raw.trim() === '' ? undefined : raw.trim();
}

function positive(text: string | undefined): number | undefined {
  // A pull request arrives as `12`, as `refs/pull/12/merge`, or as a link ending in `/12`.
  const match = text === undefined ? null : /(?:^|\/)(\d+)(?:\/(?:merge|head))?$/.exec(text.trim());
  const number = match === null ? Number.NaN : Number(match[1]);

  return Number.isInteger(number) && number > 0 ? number : undefined;
}

function compact(meta: { [Key in keyof RunMeta]: RunMeta[Key] | undefined }): RunMeta {
  return Object.fromEntries(Object.entries(meta).filter(([, entry]) => entry !== undefined && entry !== '')) as RunMeta;
}

/** The pull request of a GitHub Actions job, from the event payload: its number and its head commit. */
async function githubPullRequest(deps: ReporterDeps): Promise<{ number?: number; head?: string }> {
  const path = value(deps.env, 'GITHUB_EVENT_PATH');

  if (path === undefined) {
    return {};
  }

  try {
    const payload: unknown = JSON.parse(await deps.readFile(path));
    const pull = typeof payload === 'object' && payload !== null ? (payload as Record<string, unknown>)['pull_request'] : undefined;

    if (typeof pull !== 'object' || pull === null) {
      return {};
    }

    const number = (pull as Record<string, unknown>)['number'];
    const head = (pull as Record<string, unknown>)['head'];
    const sha = typeof head === 'object' && head !== null ? (head as Record<string, unknown>)['sha'] : undefined;

    return { ...(typeof number === 'number' && number > 0 ? { number } : {}), ...(typeof sha === 'string' ? { head: sha } : {}) };
  } catch {
    return {};
  }
}

/**
 * Branch, commit, pull request, CI link and a name for the run, from the variables the CI
 * service sets: GitHub Actions, GitLab CI/CD, Bitrise, Codemagic, CircleCI, Buildkite,
 * Azure Pipelines, Jenkins. Outside CI it is empty, and team-hub falls back to the commit
 * and branch the report itself carries.
 */
export async function ciMeta(deps: ReporterDeps): Promise<RunMeta> {
  const env = (name: string): string | undefined => value(deps.env, name);

  if (env('GITHUB_ACTIONS') !== undefined) {
    const pull = await githubPullRequest(deps);
    const server = (env('GITHUB_SERVER_URL') ?? 'https://github.com').replace(/\/+$/, '');
    const repository = env('GITHUB_REPOSITORY');
    const runId = env('GITHUB_RUN_ID');
    const number = env('GITHUB_RUN_NUMBER');

    return compact({
      name: env('GITHUB_WORKFLOW') === undefined ? undefined : `${env('GITHUB_WORKFLOW')}${number === undefined ? '' : ` #${number}`}`,
      // On a pull request GITHUB_REF_NAME is `12/merge` and GITHUB_SHA the merge commit; the head is what was written.
      branch: env('GITHUB_HEAD_REF') ?? env('GITHUB_REF_NAME'),
      commit: pull.head ?? env('GITHUB_SHA'),
      pullRequest: pull.number ?? (env('GITHUB_REF')?.startsWith('refs/pull/') ? positive(env('GITHUB_REF')) : undefined),
      ciUrl: repository === undefined || runId === undefined ? undefined : `${server}/${repository}/actions/runs/${runId}`,
    });
  }

  if (env('GITLAB_CI') !== undefined) {
    return compact({
      name: env('CI_JOB_NAME') === undefined ? undefined : `${env('CI_JOB_NAME')}${env('CI_PIPELINE_IID') === undefined ? '' : ` #${env('CI_PIPELINE_IID')}`}`,
      branch: env('CI_MERGE_REQUEST_SOURCE_BRANCH_NAME') ?? env('CI_COMMIT_REF_NAME'),
      commit: env('CI_COMMIT_SHA'),
      pullRequest: positive(env('CI_MERGE_REQUEST_IID')),
      ciUrl: env('CI_JOB_URL') ?? env('CI_PIPELINE_URL'),
    });
  }

  if (env('BITRISE_IO') !== undefined) {
    return compact({
      name: env('BITRISE_TRIGGERED_WORKFLOW_TITLE') === undefined ? undefined : `${env('BITRISE_TRIGGERED_WORKFLOW_TITLE')} #${env('BITRISE_BUILD_NUMBER') ?? ''}`.trim(),
      branch: env('BITRISE_GIT_BRANCH'),
      commit: env('BITRISE_GIT_COMMIT') ?? env('GIT_CLONE_COMMIT_HASH'),
      pullRequest: positive(env('BITRISE_PULL_REQUEST')),
      ciUrl: env('BITRISE_BUILD_URL'),
    });
  }

  if (env('CM_BUILD_ID') !== undefined) {
    return compact({
      name: env('CM_WORKFLOW_NAME') ?? env('FCI_WORKFLOW_NAME'),
      branch: env('CM_BRANCH'),
      commit: env('CM_COMMIT'),
      pullRequest: positive(env('CM_PULL_REQUEST_NUMBER')),
      ciUrl: env('CM_PROJECT_ID') === undefined ? undefined : `https://codemagic.io/app/${env('CM_PROJECT_ID')}/build/${env('CM_BUILD_ID')}`,
    });
  }

  if (env('CIRCLECI') !== undefined) {
    return compact({
      name: env('CIRCLE_JOB') === undefined ? undefined : `${env('CIRCLE_JOB')} #${env('CIRCLE_BUILD_NUM') ?? ''}`.trim(),
      branch: env('CIRCLE_BRANCH'),
      commit: env('CIRCLE_SHA1'),
      pullRequest: positive(env('CIRCLE_PR_NUMBER') ?? env('CIRCLE_PULL_REQUEST')),
      ciUrl: env('CIRCLE_BUILD_URL'),
    });
  }

  if (env('BUILDKITE') !== undefined) {
    return compact({
      name: env('BUILDKITE_LABEL') ?? env('BUILDKITE_PIPELINE_SLUG'),
      branch: env('BUILDKITE_BRANCH'),
      commit: env('BUILDKITE_COMMIT'),
      pullRequest: positive(env('BUILDKITE_PULL_REQUEST')),
      ciUrl: env('BUILDKITE_BUILD_URL'),
    });
  }

  if (env('TF_BUILD') !== undefined) {
    const collection = env('SYSTEM_TEAMFOUNDATIONCOLLECTIONURI');
    const project = env('SYSTEM_TEAMPROJECT');
    const buildId = env('BUILD_BUILDID');

    return compact({
      name: env('BUILD_DEFINITIONNAME') === undefined ? undefined : `${env('BUILD_DEFINITIONNAME')} #${env('BUILD_BUILDNUMBER') ?? ''}`.trim(),
      branch: env('SYSTEM_PULLREQUEST_SOURCEBRANCH')?.replace(/^refs\/heads\//, '') ?? env('BUILD_SOURCEBRANCHNAME'),
      commit: env('SYSTEM_PULLREQUEST_SOURCECOMMITID') ?? env('BUILD_SOURCEVERSION'),
      pullRequest: positive(env('SYSTEM_PULLREQUEST_PULLREQUESTNUMBER') ?? env('SYSTEM_PULLREQUEST_PULLREQUESTID')),
      ciUrl: collection === undefined || project === undefined || buildId === undefined ? undefined : `${collection.replace(/\/+$/, '')}/${encodeURIComponent(project)}/_build/results?buildId=${buildId}`,
    });
  }

  if (env('JENKINS_URL') !== undefined) {
    return compact({
      name: env('JOB_NAME') === undefined ? undefined : `${env('JOB_NAME')} #${env('BUILD_NUMBER') ?? ''}`.trim(),
      branch: env('CHANGE_BRANCH') ?? env('BRANCH_NAME') ?? env('GIT_BRANCH')?.replace(/^origin\//, ''),
      commit: env('GIT_COMMIT'),
      pullRequest: positive(env('CHANGE_ID')),
      ciUrl: env('BUILD_URL'),
    });
  }

  return {};
}

/** `text` without the token, so nothing the reporter prints can carry it. */
function withoutToken(text: string, token: string): string {
  return token === '' ? text : text.split(token).join('***');
}

async function refusal(response: Response): Promise<string> {
  let reason = '';

  try {
    const body: unknown = JSON.parse(await response.text());
    const error = typeof body === 'object' && body !== null ? (body as Record<string, unknown>)['error'] : undefined;
    const message = typeof error === 'object' && error !== null ? (error as Record<string, unknown>)['message'] : undefined;

    reason = typeof message === 'string' ? message : '';
  } catch {
    // The status alone says enough.
  }

  const hint = response.status === 401 ? ' (is TEAM_HUB_TOKEN a valid API token?)' : response.status === 403 ? ' (the token’s owner may not edit this project)' : '';

  return `HTTP ${response.status}${reason === '' ? '' : `: ${reason}`}${hint}`;
}

/**
 * Posts one finished run and resolves with the summary rows that say where it went, or why
 * it did not go anywhere. It never throws and never prints the token.
 */
export async function postRun(run: FinishedRun, signal: AbortSignal, options: TeamHubOptions, deps: ReporterDeps): Promise<ReporterSummary> {
  const row = (text: string): ReporterSummary => [{ label: SUMMARY_LABEL, text }];
  const token = options.token?.trim() || value(deps.env, 'TEAM_HUB_TOKEN');
  const base = (options.url?.trim() || value(deps.env, 'TEAM_HUB_URL'))?.replace(/\/+$/, '');
  const project = options.project?.trim() || value(deps.env, 'TEAM_HUB_PROJECT');

  if (run.report.run.explore !== undefined) {
    return row('not posted: an explore run has no test results');
  }

  if (token === undefined) {
    return row('not posted: TEAM_HUB_TOKEN is not set');
  }

  if (base === undefined || project === undefined) {
    return row('not posted: set url and project in teamHub({ … }), or TEAM_HUB_URL and TEAM_HUB_PROJECT');
  }

  if (!/^https?:\/\//.test(base)) {
    return row(`not posted: ${base} is not an http(s) address`);
  }

  try {
    const meta = compact({
      ...(await ciMeta(deps)),
      ...compact({ name: options.name, branch: options.branch, commit: options.commit, pullRequest: options.pullRequest, ciUrl: options.ciUrl }),
    });
    const response = await deps.fetch(`${base}/api/m/e2e/projects/${encodeURIComponent(project)}/runs`, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      // The report as the framework wrote it, with what CI knows about the run beside it.
      body: JSON.stringify({ ...run.report, meta }),
      signal,
    });

    if (!response.ok) {
      return row(withoutToken(`not posted: ${await refusal(response)}`, token));
    }

    const result = (await response.json()) as { url?: unknown; created?: unknown; linkedIssues?: unknown };
    const issues = Array.isArray(result.linkedIssues) ? result.linkedIssues.length : 0;
    const where = typeof result.url === 'string' ? result.url : `${base}/p/${project}/tests`;
    const note = result.created === false ? ' (already there)' : issues > 0 ? ` (tests verify ${issues} issue${issues === 1 ? '' : 's'})` : '';

    return row(`${where}${note}`);
  } catch (error) {
    const reason = error instanceof Error ? (error.cause instanceof Error ? `${error.message}: ${error.cause.message}` : error.message) : String(error);

    return row(withoutToken(`not posted: ${reason}`, token));
  }
}
