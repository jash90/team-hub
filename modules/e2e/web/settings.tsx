import { useEffect, useState, type FormEvent } from 'react';

import { api } from '@/api.ts';
import type { E2eSettings } from '@/bindings/E2eSettings.ts';
import type { Project } from '@/bindings/Project.ts';
import { Button, ErrorNote, Field, SettingsCard, Spinner, Tag, useBusy } from '@/components/ui.tsx';
import { t } from '@/i18n/index.ts';

import { VIDEO_MODES, useTestSettings } from './jobs.tsx';
import { BASE } from './shared.tsx';

const TRACE_MODES: [string, string][] = [
  ['on', 'A trace page for every test'],
  ['retain-on-failure', 'A trace page for tests that did not pass'],
  ['off', 'No trace pages'],
];

type Variable = { name: string; value: string };
type Plain = { repo: string; ref: string; directory: string; installCommand: string; runArgs: string; video: string; trace: string; timeoutMinutes: string };

function plainOf(settings: E2eSettings): Plain {
  return {
    repo: settings.repo ?? '',
    ref: settings.ref,
    directory: settings.directory,
    installCommand: settings.installCommand,
    runArgs: settings.runArgs,
    video: settings.video,
    trace: settings.trace,
    timeoutMinutes: String(settings.timeoutMinutes),
  };
}

/** Plain variables as rows of name and value. */
function Variables({ rows, disabled, onChange }: { rows: Variable[]; disabled: boolean; onChange: (rows: Variable[]) => void }) {
  const set = (index: number, change: Partial<Variable>) => onChange(rows.map((row, at) => (at === index ? { ...row, ...change } : row)));

  return (
    <div className="e2e-vars">
      {rows.map((row, index) => (
        <div key={index} className="e2e-var">
          <input className="input input-mono" value={row.name} placeholder="APP_URL" aria-label={t('Variable name')} disabled={disabled} onChange={(event) => set(index, { name: event.target.value })} />
          <input className="input input-mono" value={row.value} placeholder="https://staging.example.com" aria-label={t('Variable value')} disabled={disabled} onChange={(event) => set(index, { value: event.target.value })} />
          {!disabled && <Button variant="ghost" icon="trash" aria-label={t('Remove variable')} title={t('Remove variable')} onClick={() => onChange(rows.filter((_, at) => at !== index))} />}
        </div>
      ))}
      {!disabled && (
        <div>
          <Button size="sm" icon="plus" onClick={() => onChange([...rows, { name: '', value: '' }])}>
            {t('Add variable')}
          </Button>
        </div>
      )}
      {disabled && rows.length === 0 && <span className="muted">{t('None.')}</span>}
    </div>
  );
}

/**
 * Secrets by name. A stored value is never shown: it can be replaced or removed. `changes`
 * holds what the next save does: a new value for a name, or null to remove it.
 */
function Secrets({ stored, changes, disabled, onChange }: { stored: string[]; changes: Record<string, string | null>; disabled: boolean; onChange: (changes: Record<string, string | null>) => void }) {
  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const [replacing, setReplacing] = useState<string | null>(null);
  const added = Object.keys(changes).filter((key) => !stored.includes(key) && changes[key] !== null);
  const without = (key: string) => Object.fromEntries(Object.entries(changes).filter(([other]) => other !== key));

  function add(): void {
    onChange({ ...changes, [name.trim()]: value });
    setName('');
    setValue('');
  }

  return (
    <div className="e2e-vars">
      {[...stored, ...added].map((key) => {
        const change = changes[key];
        const isStored = stored.includes(key);

        return (
          <div key={key} className="e2e-secret">
            <span className="mono e2e-secret-name">{key}</span>
            {change === null ? <Tag tone="red">{t('will be removed')}</Tag> : change !== undefined ? <Tag tone="amber">{isStored ? t('will be replaced') : t('new, not saved yet')}</Tag> : <Tag tone="green">{t('set')}</Tag>}
            {replacing === key && (
              <input
                className="input input-mono e2e-secret-value"
                type="password"
                autoComplete="new-password"
                autoFocus
                placeholder={t('New value')}
                aria-label={t('New value of {name}', { name: key })}
                value={change ?? ''}
                onChange={(event) => onChange(event.target.value ? { ...changes, [key]: event.target.value } : without(key))}
              />
            )}
            <span className="filter-spacer" />
            {!disabled && change === undefined && replacing !== key && (
              <>
                <Button size="sm" onClick={() => setReplacing(key)}>
                  {t('Replace')}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => onChange({ ...changes, [key]: null })}>
                  {t('Remove')}
                </Button>
              </>
            )}
            {!disabled && (change !== undefined || replacing === key) && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setReplacing(null);
                  onChange(without(key));
                }}
              >
                {t('Undo')}
              </Button>
            )}
          </div>
        );
      })}
      {stored.length + added.length === 0 && disabled && <span className="muted">{t('None.')}</span>}
      {!disabled && (
        <div className="e2e-var">
          <input className="input input-mono" value={name} placeholder="PAYMENT_API_KEY" aria-label={t('Secret name')} onChange={(event) => setName(event.target.value)} />
          <input className="input input-mono" type="password" autoComplete="new-password" value={value} placeholder={t('Value')} aria-label={t('Secret value')} onChange={(event) => setValue(event.target.value)} />
          <Button icon="plus" disabled={!name.trim() || !value} onClick={add}>
            {t('Add secret')}
          </Button>
        </div>
      )}
    </div>
  );
}

/** How a runner runs the project's suite: for project administrators; others read it. */
export function TestSettings({ project }: { project: Project }) {
  const settings = useTestSettings(project);
  const admin = project.role === 'admin';
  const [plain, setPlain] = useState<Plain | null>(null);
  const [env, setEnv] = useState<Variable[]>([]);
  const [secrets, setSecrets] = useState<Record<string, string | null>>({});
  const [busy, run] = useBusy();
  const loaded = settings.data;

  // What the server holds fills the form: at first, and again after each save.
  useEffect(() => {
    if (loaded) {
      setPlain(plainOf(loaded));
      setEnv(Object.entries(loaded.env).map(([name, value]) => ({ name, value })));
      setSecrets({});
    }
  }, [loaded]);

  if (!loaded || !plain) {
    return settings.error ? <ErrorNote message={settings.error} /> : <Spinner />;
  }

  const set = (change: Partial<Plain>) => setPlain({ ...plain, ...change });

  function save(event: FormEvent): void {
    event.preventDefault();

    if (!plain) {
      return;
    }

    void run(async () => {
      const saved = await api.put<E2eSettings>(`${BASE}/projects/${project.key}/settings`, {
        ...plain,
        repo: plain.repo.trim() || null,
        timeoutMinutes: Number(plain.timeoutMinutes) || 0,
        env: Object.fromEntries(env.filter((row) => row.name.trim()).map((row) => [row.name.trim(), row.value])),
        secrets,
      });
      settings.set(saved);
    }, t('Test settings saved'));
  }

  return (
    <SettingsCard
      title={t('Running the tests')}
      lead={t('A runner checks the repository out, installs the suite and runs it with recording on. Set the repository to let members start test jobs from the Tests page.')}
    >
      <form className="form e2e-settings" onSubmit={save}>
        <div className="form-row e2e-settings-row">
          <Field label={t('Repository')} hint={t('owner/name on GitHub. Leave empty to switch test jobs off.')}>
            <input className="input input-mono" value={plain.repo} placeholder="acme/shop" disabled={!admin} onChange={(event) => set({ repo: event.target.value })} />
          </Field>
          <Field label={t('Branch, tag or commit')} hint={t('What a job runs unless it names another.')}>
            <input className="input input-mono" value={plain.ref} placeholder="main" disabled={!admin} onChange={(event) => set({ ref: event.target.value })} />
          </Field>
        </div>
        <div className="form-row e2e-settings-row">
          <Field label={t('Directory of the suite')} hint={t('Where its package.json is, from the repository root. Empty for the root.')}>
            <input className="input input-mono" value={plain.directory} placeholder="e2e" disabled={!admin} onChange={(event) => set({ directory: event.target.value })} />
          </Field>
          <Field label={t('Install command')}>
            <input className="input input-mono" value={plain.installCommand} placeholder="npm ci" disabled={!admin} onChange={(event) => set({ installCommand: event.target.value })} />
          </Field>
        </div>
        <div className="form-row e2e-settings-row">
          <Field label={t('Extra arguments for e2e run')}>
            <input className="input input-mono" value={plain.runArgs} placeholder="--workers 2" disabled={!admin} onChange={(event) => set({ runArgs: event.target.value })} />
          </Field>
          <Field label={t('Stop a job after (minutes)')} hint={t('At most 240.')}>
            <input className="input input-number" type="number" min={1} max={240} value={plain.timeoutMinutes} disabled={!admin} onChange={(event) => set({ timeoutMinutes: event.target.value })} />
          </Field>
        </div>
        <div className="form-row e2e-settings-row">
          <Field label={t('Video')} hint={t('A recording lets people watch the test afterwards, step by step.')}>
            <select className="input" value={plain.video} disabled={!admin} onChange={(event) => set({ video: event.target.value })}>
              {VIDEO_MODES.map(([mode, label]) => (
                <option key={mode} value={mode}>
                  {t(label)}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t('Trace pages')}>
            <select className="input" value={plain.trace} disabled={!admin} onChange={(event) => set({ trace: event.target.value })}>
              {TRACE_MODES.map(([mode, label]) => (
                <option key={mode} value={mode}>
                  {t(label)}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <div className="e2e-settings-group">
          <span className="field-label">{t('Variables')}</span>
          <span className="field-hint">{t('Plain values the suite reads from its environment, such as the address of the app under test.')}</span>
          <Variables rows={env} disabled={!admin} onChange={setEnv} />
        </div>
        <div className="e2e-settings-group">
          <span className="field-label">{t('Secrets')}</span>
          <span className="field-hint">{t('Stored encrypted and never shown again. A runner receives them only when it takes a job of this project.')}</span>
          <Secrets stored={loaded.secretNames} changes={secrets} disabled={!admin} onChange={setSecrets} />
        </div>
        {admin ? (
          <div>
            <Button variant="primary" type="submit" disabled={busy}>
              {t('Save test settings')}
            </Button>
          </div>
        ) : (
          <p className="muted">{t('Only a project administrator changes these settings.')}</p>
        )}
      </form>
    </SettingsCard>
  );
}
