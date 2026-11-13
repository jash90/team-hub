import { api } from '@/api.ts';
import type { AgentInfo } from '@/bindings/AgentInfo.ts';
import type { AgentToolInfo } from '@/bindings/AgentToolInfo.ts';
import type { JsonValue } from '@/bindings/serde_json/JsonValue.ts';
import { Button, ErrorNote, Spinner, Tag, copyText } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';
import { Link } from '@/router.tsx';
import { useRemote, useSession } from '@/session.tsx';
import { SettingsCard } from '@/views/settings.tsx';

import { polish } from './pl.ts';
import './styles.css';

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

function AgentsSettings() {
  const info = useRemote(() => api.get<AgentInfo>('/api/m/agents/info'), []);
  const tools = useRemote(() => api.get<AgentToolInfo[]>('/api/m/agents/tools'), []);
  const { boot } = useSession();
  const groups = [...new Set((tools.data ?? []).map((tool) => tool.module))];
  const moduleName = (id: string) => (id === OWN_MODULE ? t('Issues and projects') : (boot.modules.find((module) => module.id === id)?.name ?? id));

  return (
    <>
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

export const agentsModule: WebModule = {
  id: OWN_MODULE,
  settings: [{ id: 'agents', label: () => t('AI agents'), render: () => <AgentsSettings /> }],
  polish,
};
