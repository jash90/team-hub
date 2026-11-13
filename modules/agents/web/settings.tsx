import { useState, type FormEvent } from 'react';

import { api } from '@/api.ts';
import type { AgentInfo } from '@/bindings/AgentInfo.ts';
import type { AgentProposalKind } from '@/bindings/AgentProposalKind.ts';
import type { AgentRunKind } from '@/bindings/AgentRunKind.ts';
import type { AgentRunner } from '@/bindings/AgentRunner.ts';
import type { AgentSettings } from '@/bindings/AgentSettings.ts';
import type { AgentToolInfo } from '@/bindings/AgentToolInfo.ts';
import type { AgentTrust } from '@/bindings/AgentTrust.ts';
import type { Project } from '@/bindings/Project.ts';
import type { JsonValue } from '@/bindings/serde_json/JsonValue.ts';
import { Button, ErrorNote, Field, Picker, Spinner, Tag, attempt, copyText, useBusy } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import { Link } from '@/router.tsx';
import { useRemote, useSession } from '@/session.tsx';
import { SettingsCard } from '@/views/settings.tsx';

import { BASE, RunnerLine, kindLabel, proposalLabel, useAgentSettings } from './runs.tsx';

const OWN_MODULE = 'agents';

type Schema = { [key: string]: JsonValue };
type Argument = { name: string; type: string; required: boolean; description: string };

function asSchema(value: JsonValue | undefined): Schema | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? value : null;
}

/** "string", "number or null", "list of string": a JSON Schema type the way a person says it. */
function typeText(schema: Schema): string {
  const names = (Array.isArray(schema.type) ? schema.type : [schema.type]).filter((name): name is string => typeof name === 'string');
  const items = asSchema(schema.items);
  const described = names.map((name) => (name === 'array' && items ? t('list of {type}', { type: typeText(items) }) : name));

  return described.length > 0 ? described.join(` ${t('or')} `) : t('any');
}

/** The arguments a tool takes, read from the JSON Schema of its input. */
function argumentsOf(schema: JsonValue): Argument[] {
  const root = asSchema(schema);
  const properties = asSchema(root?.properties);
  const required = Array.isArray(root?.required) ? root.required : [];

  const fields = Object.entries(properties ?? {}).map(([name, value]) => {
    const property = asSchema(value) ?? {};
    const choices = Array.isArray(property.enum) ? property.enum.map(String) : [];
    const description = typeof property.description === 'string' ? property.description : '';

    return {
      name,
      type: typeText(property),
      required: required.includes(name),
      description: [description, choices.length > 0 ? t('One of: {values}', { values: choices.join(', ') }) : ''].filter(Boolean).join('. '),
    };
  });

  // A schema lists properties in no particular order; what must be given comes first.
  return [...fields.filter((field) => field.required), ...fields.filter((field) => !field.required)];
}

function CopyRow({ text, label }: { text: string; label: string }) {
  return (
    <span className="copy-row">
      <code className="code-block">{text}</code>
      <Button icon="copy" aria-label={label} title={label} onClick={() => void copyText(text)} />
    </span>
  );
}

function Tool({ tool }: { tool: AgentToolInfo }) {
  const fields = argumentsOf(tool.inputSchema);

  return (
    <article className="agents-tool">
      <header>
        <code className="agents-tool-name">{tool.name}</code>
        {tool.readOnly ? <Tag title={t('Calling it changes nothing')}>{t('read-only')}</Tag> : <Tag tone="amber" title={t('Calling it changes data, as the person whose token the agent uses')}>{t('writes')}</Tag>}
      </header>
      <p>{tool.description}</p>
      {fields.length === 0 ? (
        <p className="muted">{t('Takes no arguments.')}</p>
      ) : (
        <table className="table table-plain agents-arguments">
          <thead>
            <tr>
              <th>{t('Argument')}</th>
              <th>{t('Type')}</th>
              <th>{t('Description')}</th>
            </tr>
          </thead>
          <tbody>
            {fields.map((field) => (
              <tr key={field.name}>
                <td>
                  <code>{field.name}</code>
                  {field.required && <span className="agents-required">{t('required')}</span>}
                </td>
                <td className="muted">{field.type}</td>
                <td>{field.description}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </article>
  );
}

export function AgentsSettings() {
  const info = useRemote(() => api.get<AgentInfo>('/api/m/agents/info'), []);
  const tools = useRemote(() => api.get<AgentToolInfo[]>('/api/m/agents/tools'), []);
  const { boot } = useSession();
  const groups = [...new Set((tools.data ?? []).map((tool) => tool.module))];
  const moduleName = (id: string) => (id === OWN_MODULE ? t('Issues and projects') : (boot.modules.find((module) => module.id === id)?.name ?? id));

  return (
    <>
      <ServerSettings />
      <SettingsCard
        title={t('Connect an AI agent')}
        lead={t('team-hub speaks MCP (Model Context Protocol), the standard way for coding agents such as Claude Code to use the tools of other systems. A connected agent can read, create and update issues from inside a repository, as you and with your permissions.')}
      >
        {info.error && <ErrorNote message={info.error} />}
        {!info.data && !info.error && <Spinner />}
        {info.data && (
          <ol className="agents-steps">
            <li>
              <strong>{t('Create an API token')}</strong>
              <p className="muted">
                {t('The agent signs in with a token of yours. A token is shown once, when you create it.')} <Link href="/settings/tokens">{t('Open Settings → API tokens')}</Link>
              </p>
            </li>
            <li>
              <strong>{t('Register the server with Claude Code')}</strong>
              <p className="muted">{t('Run this in the repository, with your token in place of <token>.')}</p>
              <CopyRow text={info.data.exampleCommand} label={t('Copy the command')} />
            </li>
            <li>
              <strong>{t('Or point any other MCP client at the endpoint')}</strong>
              <p className="muted">{t('Streamable HTTP, with the header Authorization: Bearer <token>.')}</p>
              <CopyRow text={info.data.endpoint} label={t('Copy the address')} />
            </li>
          </ol>
        )}
      </SettingsCard>

      <RunnersCard />

      <SettingsCard
        title={t('Tools')}
        lead={t('What a connected agent can call. Every tool runs as the person whose token the agent uses; modules that are switched on add their own.')}
        action={tools.data && <span className="tab-count">{tools.data.length}</span>}
      >
        {tools.error && <ErrorNote message={tools.error} />}
        {!tools.data && !tools.error && <Spinner />}
        {groups.map((group) => (
          <section key={group} className="agents-group">
            <h3>{moduleName(group)}</h3>
            {(tools.data ?? [])
              .filter((tool) => tool.module === group)
              .map((tool) => (
                <Tool key={tool.name} tool={tool} />
              ))}
          </section>
        ))}
      </SettingsCard>
    </>
  );
}

const SERVER_KINDS: AgentRunKind[] = ['refine', 'breakdown', 'summarize', 'triage', 'custom'];

function statusText(settings: AgentSettings): string {
  if (settings.serverStatus === 'no_key') {
    return t('No model key is set, so runs for the server wait in the queue.');
  }

  if (settings.serverStatus === 'cap_reached') {
    return t('This month’s spending cap is used up; the server takes no more runs until next month or a higher cap.');
  }

  return t('The server carries out runs with {model}.', { model: settings.models.find((model) => model.id === settings.defaultModel)?.name ?? settings.defaultModel });
}

function dollars(amount: number): string {
  return `$${amount.toFixed(amount < 1 ? 4 : 2)}`;
}

/** The server as an agent: which model it asks, with whose key, for what, up to how much. */
function ServerSettings() {
  const settings = useAgentSettings();
  const { user } = useSession();
  const admin = user.role === 'admin';
  const [apiKey, setApiKey] = useState('');
  const [apiUrl, setApiUrl] = useState<string | null>(null);
  const [cap, setCap] = useState<string | null>(null);
  const [busy, run] = useBusy();

  if (!settings.data) {
    return settings.error ? <ErrorNote message={settings.error} /> : <Spinner />;
  }

  const data = settings.data;
  const save = (change: Record<string, unknown>, done?: string) => run(async () => settings.set(await api.put<AgentSettings>(`${BASE}/settings`, change)), done);

  function submit(event: FormEvent): void {
    event.preventDefault();
    const amount = cap === null ? undefined : cap.trim() === '' ? null : Number(cap.replace(',', '.'));

    void save({ apiKey, apiUrl: apiUrl ?? undefined, monthlyCapUsd: amount }, t('Saved')).then(() => {
      setApiKey('');
      setApiUrl(null);
      setCap(null);
    });
  }

  return (
    <SettingsCard
      title={t('The server’s own agent')}
      lead={t('Work that needs no repository (refining a description, breaking an issue down, summarising, triage) is done by the server itself with a Claude model. It only ever proposes; people decide.')}
      action={<Tag tone={data.serverStatus === 'ready' ? 'green' : 'amber'}>{data.serverStatus === 'ready' ? t('ready for runs') : t('not taking runs')}</Tag>}
    >
      <p className="muted settings-lead">
        {statusText(data)} {t('Spent this month: {spent}', { spent: dollars(data.spentThisMonthUsd) })}
        {data.monthlyCapUsd !== null && ` / ${dollars(data.monthlyCapUsd)}`}
      </p>
      {admin && (
        <form className="form" onSubmit={submit}>
          <Field label={t('Claude API key')} hint={data.keyHint ? t('A key ending in {hint} is stored, encrypted. Leave blank to keep it.', { hint: data.keyHint }) : t('No key is stored. Create one in the Claude Console.')}>
            <input className="input input-mono" type="password" autoComplete="off" value={apiKey} placeholder="sk-ant-…" onChange={(event) => setApiKey(event.target.value)} />
          </Field>
          <Field label={t('Default model')} hint={t('Each run can choose another. Prices are per million tokens, input / output.')}>
            <Picker
              value={data.defaultModel}
              options={data.models.map((model) => ({ value: model.id, label: `${model.name} · $${model.inputPrice} / $${model.outputPrice}` }))}
              onChange={(defaultModel) => void save({ defaultModel })}
            />
          </Field>
          <div className="agents-kinds">
            <span className="field-label">{t('What the server may do')}</span>
            {SERVER_KINDS.map((kind) => (
              <label key={kind} className="check">
                <input
                  type="checkbox"
                  checked={data.serverKinds.includes(kind)}
                  disabled={busy}
                  onChange={(event) => void save({ serverKinds: SERVER_KINDS.filter((other) => (other === kind ? event.target.checked : data.serverKinds.includes(other))) })}
                />
                <span>{kindLabel(kind)}</span>
              </label>
            ))}
          </div>
          <Field label={t('Monthly spending cap (US dollars)')} hint={t('Once this month’s runs have cost this much, the server takes no more. Blank means no cap.')}>
            <input className="input input-number" inputMode="decimal" value={cap ?? (data.monthlyCapUsd === null ? '' : String(data.monthlyCapUsd))} onChange={(event) => setCap(event.target.value)} />
          </Field>
          <Field label={t('API address')} hint={t('Only change this to go through a gateway of your own. It must be https.')}>
            <input className="input input-mono" value={apiUrl ?? data.apiUrl} onChange={(event) => setApiUrl(event.target.value)} />
          </Field>
          <div className="inline-form">
            <Button variant="primary" type="submit" disabled={busy || (!apiKey.trim() && apiUrl === null && cap === null)}>
              {t('Save')}
            </Button>
            {data.keyConfigured && (
              <Button variant="danger" disabled={busy} onClick={() => void save({ clearApiKey: true }, t('Key removed'))}>
                {t('Remove key')}
              </Button>
            )}
          </div>
        </form>
      )}
    </SettingsCard>
  );
}

/** Runners: the programs that carry out work in code. */
function RunnersCard() {
  const runners = useRemote(() => api.get<AgentRunner[]>(`${BASE}/runners`), [], (event) => event.kind === 'agent.run');
  const { userById } = useSession();

  return (
    <SettingsCard
      title={t('Runners')}
      lead={t('Work in a repository (implementing an issue, reviewing a pull request) is carried out by a runner on someone’s machine: the team-hub desktop app, or a coding agent connected as above, which finds its work with the list_my_runs tool. A runner signs in with your API token and takes the runs you delegate, or anyone’s that are marked as shared.')}
    >
      {runners.error && <ErrorNote message={runners.error} />}
      {runners.data?.length === 0 && <p className="muted">{t('No runner has connected yet.')}</p>}
      {runners.data?.map((runner) => (
        <RunnerLine key={runner.id} runner={runner} owner={userById(runner.userId)?.name} />
      ))}
    </SettingsCard>
  );
}

const TRUSTABLE: AgentProposalKind[] = ['comment', 'transition', 'update_fields', 'create_subtasks', 'create_issue', 'log_work'];

/** Which kinds of proposal a project applies without waiting for a person. */
export function ProjectTrust({ project }: { project: Project }) {
  const trust = useRemote(() => api.get<AgentTrust>(`${BASE}/projects/${project.key}/trust`), [project.key]);
  const admin = project.role === 'admin';

  if (!trust.data) {
    return trust.error ? <ErrorNote message={trust.error} /> : <Spinner />;
  }

  const kinds = trust.data.kinds;

  return (
    <fieldset className="settings-body" disabled={!admin}>
      <SettingsCard
        title={t('What agents may do without asking')}
        lead={t('An agent proposes and a person decides. For the kinds ticked here, this project skips the decision: the proposal is applied at once, in the name of whoever asked for the run. Leave everything unticked to review every change.')}
      >
        <div className="agents-kinds">
          {TRUSTABLE.map((kind) => (
            <label key={kind} className="check">
              <input
                type="checkbox"
                checked={kinds.includes(kind)}
                onChange={(event) =>
                  void attempt(() => api.put<AgentTrust>(`${BASE}/projects/${project.key}/trust`, { kinds: TRUSTABLE.filter((other) => (other === kind ? event.target.checked : kinds.includes(other))) })).then(
                    (saved) => saved && trust.set(saved),
                  )
                }
              />
              <span>{proposalLabel(kind)}</span>
            </label>
          ))}
        </div>
        {kinds.length > 0 && (
          <p className="agents-hold">
            {t('Trusted changes still appear in each issue’s history, labelled as the agent’s.')}
          </p>
        )}
      </SettingsCard>
    </fieldset>
  );
}
