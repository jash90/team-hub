import { api } from '@/api.ts';
import type { AgentRun } from '@/bindings/AgentRun.ts';
import type { AgentRunner } from '@/bindings/AgentRunner.ts';
import { canEdit } from '@/components/issue-bits.tsx';
import { Empty, ErrorNote, Picker, Spinner } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { useRemote, useSession, useStored } from '@/session.tsx';

import { BASE, RunCard, RunnerLine, isFinished, needsPerson, useAgentSettings, useRuns } from './runs.tsx';

const RECENT = 20;

/** Everything agents are doing across the projects the viewer can see. */
export function AgentsPage() {
  const { boot, userById } = useSession();
  const settings = useAgentSettings();
  const [project, setProject] = useStored<string | null>('agents-project', null);
  const [mine, setMine] = useStored('agents-mine', false);
  const runs = useRuns(`/runs?limit=300${mine ? '&mine=true' : ''}${project ? `&project=${project}` : ''}`, [project, mine], () => true);
  const runners = useRemote(() => api.get<AgentRunner[]>(`${BASE}/runners`), [], (event) => event.kind === 'agent.run');
  const mayDecide = (run: AgentRun) => canEdit(boot.projects.find((candidate) => candidate.key === run.projectKey) ?? { role: null });
  const all = runs.data ?? [];
  const needing = all.filter(needsPerson);
  const rest = all.filter((run) => !needsPerson(run));
  const groups: { id: string; title: string; runs: AgentRun[]; empty?: string; open?: boolean }[] = [
    { id: 'needs', title: t('Needs you'), runs: needing, empty: t('Nothing is waiting for a decision or an answer.'), open: true },
    { id: 'running', title: t('Working'), runs: rest.filter((run) => run.state === 'claimed' || run.state === 'running' || run.state === 'waiting') },
    { id: 'queued', title: t('Queued'), runs: rest.filter((run) => run.state === 'queued') },
    { id: 'recent', title: t('Recent'), runs: rest.filter((run) => isFinished(run.state)).slice(0, RECENT) },
  ];

  return (
    <div className="view agents-page">
      <header className="view-head">
        <div>
          <h1>{t('AI agents')}</h1>
          <p className="muted">{t('Work handed to agents: what waits for a person, what is being done, and who is doing it. Start a run from an issue’s page.')}</p>
        </div>
      </header>
      <div className="filter-bar">
        <Picker
          label={t('Project')}
          value={project}
          options={[{ value: null as string | null, label: t('All projects') }, ...boot.projects.map((candidate) => ({ value: candidate.key as string | null, label: `${candidate.key} · ${candidate.name}` }))]}
          onChange={setProject}
        />
        <div className="agents-switch" role="group" aria-label={t('Whose runs')}>
          <button type="button" aria-pressed={!mine} className={mine ? undefined : 'agents-switch-on'} onClick={() => setMine(false)}>
            {t('Everyone’s')}
          </button>
          <button type="button" aria-pressed={mine} className={mine ? 'agents-switch-on' : undefined} onClick={() => setMine(true)}>
            {t('Asked by me')}
          </button>
        </div>
      </div>
      {runs.error && !runs.data && <ErrorNote message={runs.error} />}
      {!runs.data && !runs.error && <Spinner />}
      {runs.data && all.length === 0 && (
        <Empty icon="bot" title={t('No agent has been given work yet')}>
          {t('Open an issue and use the “AI agent” section: refine its description, break it down, or delegate the implementation.')}
        </Empty>
      )}
      {runs.data &&
        all.length > 0 &&
        groups.map(
          (group) =>
            (group.runs.length > 0 || group.empty) && (
              <section key={group.id} className="panel agents-group-panel">
                <header className="panel-head">
                  <h2>{group.title}</h2>
                  <span className="tab-count">{group.runs.length}</span>
                </header>
                {group.runs.length === 0 && <p className="muted panel-empty">{group.empty}</p>}
                {group.runs.map((run) => (
                  <RunCard key={`${group.id}-${run.id}`} run={run} canDecide={mayDecide(run)} settings={settings.data} open={group.open} showIssue onChange={() => void runs.reload()} />
                ))}
              </section>
            ),
        )}
      <section className="panel agents-group-panel">
        <header className="panel-head">
          <h2>{t('Runners')}</h2>
          <span className="tab-count">{runners.data?.filter((runner) => runner.online).length ?? 0}</span>
        </header>
        <div className="agents-runner-list">
          {settings.data && (
            <div className="agents-runner">
              <span className={settings.data.serverStatus === 'ready' ? 'agents-dot agents-dot-on' : 'agents-dot'} />
              <strong>{t('Server')}</strong>
              <span className="muted">
                {settings.data.serverStatus === 'ready'
                  ? settings.data.serverKinds.join(', ')
                  : settings.data.serverStatus === 'no_key'
                    ? t('no model key is set (Settings → AI agents)')
                    : t('this month’s spending cap is used up')}
              </span>
            </div>
          )}
          {runners.data?.map((runner) => (
            <RunnerLine key={runner.id} runner={runner} owner={userById(runner.userId)?.name} />
          ))}
          {runners.data?.length === 0 && <p className="muted">{t('No desktop app or coding agent has connected as a runner yet.')}</p>}
        </div>
      </section>
    </div>
  );
}
