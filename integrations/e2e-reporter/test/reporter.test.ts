import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { FinishedRun } from 'e2e';
import { ciMeta, postRun, teamHub, type ReporterDeps } from '../src/index.ts';

const TOKEN = 'th_0123456789abcdef-secret';

function finished(report: object = { schemaVersion: 'report-1', run: { id: 'run-1', results: [] } }): FinishedRun {
  return { report, status: 'passed', exitCode: 0, projectRoot: '/work', reportPath: undefined, artifactsRoot: '/work/.e2e/results', aiTracePath: undefined, traces: new Map(), redact: (text: string) => text } as unknown as FinishedRun;
}

interface Call {
  url: string;
  init: RequestInit;
}

function deps(env: NodeJS.ProcessEnv, answer: (call: Call) => Response | Promise<Response>, files: Record<string, string> = {}): ReporterDeps & { calls: Call[] } {
  const calls: Call[] = [];

  return {
    calls,
    env,
    readFile: async (file) => files[file] ?? Promise.reject(new Error(`no file ${file}`)),
    fetch: (async (url: string | URL | Request, init?: RequestInit) => {
      const call = { url: String(url), init: init ?? {} };
      calls.push(call);

      return answer(call);
    }) as typeof fetch,
  };
}

const accepted = (): Response => Response.json({ created: true, url: 'https://hub.example.com/p/WEB/tests?run=7', linkedIssues: ['WEB-4', 'WEB-11'] });
const options = { url: 'https://hub.example.com/', project: 'WEB', token: TOKEN };
const signal = new AbortController().signal;

test('posts the report as it is, with what CI knows beside it', async () => {
  const env = {
    GITHUB_ACTIONS: 'true',
    GITHUB_WORKFLOW: 'CI',
    GITHUB_RUN_NUMBER: '128',
    GITHUB_RUN_ID: '7300157612',
    GITHUB_REPOSITORY: 'acme/sklep-web',
    GITHUB_REF: 'refs/pull/53/merge',
    GITHUB_REF_NAME: '53/merge',
    GITHUB_HEAD_REF: 'fix/WEB-11-pusty-koszyk',
    GITHUB_SHA: 'merge00000000000000000000000000000000000',
    GITHUB_EVENT_PATH: '/event.json',
  };
  const fake = deps(env, accepted, { '/event.json': JSON.stringify({ pull_request: { number: 53, head: { sha: 'head1111111111111111111111111111111111111' } } }) });
  const report = { schemaVersion: 'report-1', run: { id: 'run-1', results: [{ titlePath: ['WEB-11 pusty koszyk'] }] } };
  const rows = await postRun(finished(report), signal, options, fake);

  assert.deepEqual(rows, [{ label: 'team-hub', text: 'https://hub.example.com/p/WEB/tests?run=7 (tests verify 2 issues)' }]);
  assert.equal(fake.calls.length, 1);
  assert.equal(fake.calls[0]?.url, 'https://hub.example.com/api/m/e2e/projects/WEB/runs');
  assert.equal(fake.calls[0]?.init.method, 'POST');
  assert.deepEqual(fake.calls[0]?.init.headers, { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/json' });
  assert.equal(fake.calls[0]?.init.signal, signal);
  assert.deepEqual(JSON.parse(String(fake.calls[0]?.init.body)), {
    ...report,
    meta: {
      name: 'CI #128',
      branch: 'fix/WEB-11-pusty-koszyk',
      commit: 'head1111111111111111111111111111111111111',
      pullRequest: 53,
      ciUrl: 'https://github.com/acme/sklep-web/actions/runs/7300157612',
    },
  });
});

test('reads other CI services, and options win over the environment', async () => {
  const gitlab = { GITLAB_CI: 'true', CI_JOB_NAME: 'e2e', CI_PIPELINE_IID: '41', CI_COMMIT_REF_NAME: 'main', CI_COMMIT_SHA: 'abc', CI_MERGE_REQUEST_IID: '9', CI_JOB_URL: 'https://gitlab.example/j/1' };
  assert.deepEqual(await ciMeta(deps(gitlab, accepted)), { name: 'e2e #41', branch: 'main', commit: 'abc', pullRequest: 9, ciUrl: 'https://gitlab.example/j/1' });

  const circle = { CIRCLECI: 'true', CIRCLE_BRANCH: 'feature/x', CIRCLE_SHA1: 'def', CIRCLE_PULL_REQUEST: 'https://github.com/acme/web/pull/12' };
  assert.deepEqual(await ciMeta(deps(circle, accepted)), { branch: 'feature/x', commit: 'def', pullRequest: 12 });

  const buildkite = { BUILDKITE: 'true', BUILDKITE_BRANCH: 'main', BUILDKITE_COMMIT: 'fff', BUILDKITE_PULL_REQUEST: 'false' };
  assert.deepEqual(await ciMeta(deps(buildkite, accepted)), { branch: 'main', commit: 'fff' });

  // A push on GitHub Actions: no event file, the branch and commit of the push.
  const push = { GITHUB_ACTIONS: 'true', GITHUB_REF: 'refs/heads/main', GITHUB_REF_NAME: 'main', GITHUB_SHA: 'aaa' };
  assert.deepEqual(await ciMeta(deps(push, accepted)), { branch: 'main', commit: 'aaa' });
  assert.deepEqual(await ciMeta(deps({}, accepted)), {});

  const fake = deps(gitlab, accepted);
  await postRun(finished(), signal, { ...options, name: 'nightly', branch: 'release' }, fake);
  assert.deepEqual(JSON.parse(String(fake.calls[0]?.init.body)).meta, { name: 'nightly', branch: 'release', commit: 'abc', pullRequest: 9, ciUrl: 'https://gitlab.example/j/1' });
});

test('takes its settings from the environment when the config names none', async () => {
  const fake = deps({ TEAM_HUB_URL: 'http://localhost:8192', TEAM_HUB_PROJECT: 'HUB', TEAM_HUB_TOKEN: TOKEN }, accepted);
  await postRun(finished(), signal, { token: undefined }, fake);

  assert.equal(fake.calls[0]?.url, 'http://localhost:8192/api/m/e2e/projects/HUB/runs');
  assert.deepEqual(JSON.parse(String(fake.calls[0]?.init.body)).meta, {});
});

test('says why nothing was posted and never fails the run', async () => {
  const never = deps({}, () => assert.fail('nothing should be posted'));

  assert.deepEqual(await postRun(finished(), signal, { url: 'https://hub.example.com', project: 'WEB', token: undefined }, never), [{ label: 'team-hub', text: 'not posted: TEAM_HUB_TOKEN is not set' }]);
  assert.match((await postRun(finished(), signal, { token: TOKEN }, never))[0]?.text ?? '', /set url and project/);
  assert.match((await postRun(finished(), signal, { ...options, url: 'hub.example.com' }, never))[0]?.text ?? '', /not an http\(s\) address/);
  assert.match((await postRun(finished({ run: { explore: { goal: 'x' } } }), signal, options, never))[0]?.text ?? '', /explore run/);

  const refused = deps({}, () => Response.json({ error: { code: 'forbidden', message: 'You can only view this project' } }, { status: 403 }));
  assert.deepEqual(await postRun(finished(), signal, options, refused), [{ label: 'team-hub', text: 'not posted: HTTP 403: You can only view this project (the token’s owner may not edit this project)' }]);

  const duplicate = deps({}, () => Response.json({ created: false, url: 'https://hub.example.com/p/WEB/tests?run=7', linkedIssues: [] }));
  assert.match((await postRun(finished(), signal, options, duplicate))[0]?.text ?? '', /run=7 \(already there\)$/);
});

test('never prints the token', async () => {
  const echoing = deps({}, () => Response.json({ error: { message: `bad token ${TOKEN}` } }, { status: 401 }));
  const failing = deps({}, () => Promise.reject(new TypeError('fetch failed', { cause: new Error(`connect ECONNREFUSED with Bearer ${TOKEN}`) })));

  for (const fake of [echoing, failing]) {
    const text = (await postRun(finished(), signal, options, fake))[0]?.text ?? '';

    assert.match(text, /^not posted: /);
    assert.ok(!text.includes(TOKEN), text);
    assert.ok(text.includes('***'), text);
  }
});

test('constructing the reporter reads nothing', () => {
  const reporter = teamHub({ url: 'https://hub.example.com', project: 'WEB', token: undefined });

  assert.equal(reporter.name, 'team-hub');
  assert.equal(typeof reporter.onRunFinished, 'function');
});
