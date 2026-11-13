import { useState, type FormEvent } from 'react';

import { api } from '@/api.ts';
import type { ConfluenceImport } from '@/bindings/ConfluenceImport.ts';
import type { ConfluenceImportReport } from '@/bindings/ConfluenceImportReport.ts';
import type { ConfluenceRemoteSpace } from '@/bindings/ConfluenceRemoteSpace.ts';
import type { ConfluenceSettings } from '@/bindings/ConfluenceSettings.ts';
import type { ConfluenceTestResult } from '@/bindings/ConfluenceTestResult.ts';
import { Icon } from '@/components/icon.tsx';
import { Button, ErrorNote, Field, Picker, Spinner, Tag, attempt, toast, useBusy } from '@/components/ui.tsx';
import { formatDateTime, t, timeAgo } from '@/i18n/index.ts';
import type { WebModule } from '@/modules.ts';
import { Link } from '@/router.tsx';
import { useRemote } from '@/session.tsx';
import { SettingsCard } from '@/views/settings.tsx';

import { polish } from './pl.ts';
import './styles.css';

const BASE = '/api/m/confluence';
const SPACE_KEY = /^[A-Z][A-Z0-9]{1,9}$/;

/** What a run did, in numbers, with the pages that failed or are gone spelled out. */
function Report({ report }: { report: ConfluenceImportReport }) {
  const nothing = report.created + report.updated + report.unchanged + report.skipped + report.failed.length + report.gone.length === 0;

  return (
    <div className="confluence-report">
      <div className="confluence-counts">
        <Tag tone={report.created > 0 ? 'green' : 'neutral'}>{t('{count} created', { count: report.created })}</Tag>
        <Tag tone={report.updated > 0 ? 'blue' : 'neutral'}>{t('{count} updated', { count: report.updated })}</Tag>
        <Tag>{t('{count} unchanged', { count: report.unchanged })}</Tag>
        <Tag title={t('Excluded pages with everything under them, and pages detached here')}>{t('{count} skipped', { count: report.skipped })}</Tag>
        {report.gone.length > 0 && <Tag tone="amber">{t('{count} gone from Confluence', { count: report.gone.length })}</Tag>}
        {report.failed.length > 0 && <Tag tone="red">{t('{count} failed', { count: report.failed.length })}</Tag>}
        {nothing && <span className="muted">{t('The space has no pages.')}</span>}
      </div>
      {report.failed.length > 0 && (
        <details className="confluence-details" open>
          <summary>{t('Pages that could not be stored')}</summary>
          <ul>
            {report.failed.map((failure, index) => (
              <li key={index}>
                <strong>{failure.title || t('(no title)')}</strong>: {failure.reason}
              </li>
            ))}
          </ul>
        </details>
      )}
      {report.gone.length > 0 && (
        <details className="confluence-details">
          <summary>{t('Pages Confluence no longer has (kept here as they are)')}</summary>
          <ul>
            {report.gone.map((page) => (
              <li key={page.pageId}>
                <Link href={`/wiki/${report.wikiSpaceKey}/${page.pageId}`}>{page.title}</Link>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

function Working({ text }: { text: string }) {
  return (
    <div className="confluence-working" role="status">
      <Spinner />
      <span>
        <strong>{text}</strong> {t('Every page is fetched and converted inside this one request, so this can take a while for a large space. Keep this tab open.')}
      </span>
    </div>
  );
}

function Connection({ settings, onSaved }: { settings: ConfluenceSettings; onSaved: (settings: ConfluenceSettings) => void }) {
  const [form, setForm] = useState({ baseUrl: settings.baseUrl, email: settings.email, token: '' });
  const [result, setResult] = useState<ConfluenceTestResult | null>(null);
  const [testing, setTesting] = useState(false);
  const [busy, run] = useBusy();
  const changed = form.baseUrl.trim() !== settings.baseUrl || form.email.trim() !== settings.email || form.token.trim() !== '';

  function save(event: FormEvent): void {
    event.preventDefault();

    void run(async () => {
      onSaved(await api.put<ConfluenceSettings>(`${BASE}/settings`, form));
      setForm({ ...form, token: '' });
      setResult(null);
    }, t('Saved'));
  }

  async function test(): Promise<void> {
    setTesting(true);
    setResult(null);
    setResult((await attempt(() => api.post<ConfluenceTestResult>(`${BASE}/test`, {}))) ?? null);
    setTesting(false);
  }

  return (
    <SettingsCard
      title={t('Connection')}
      lead={t('The address and one token for the whole instance. The token is stored encrypted and never shown again; imports can read whatever its account can read in Confluence.')}
    >
      <form className="form" onSubmit={save}>
        <Field label={t('Confluence address')} hint={t('Cloud: https://your-site.atlassian.net/wiki (with /wiki). Server and Data Center: the address people open, with its context path.')}>
          <input className="input input-mono" value={form.baseUrl} placeholder="https://example.atlassian.net/wiki" onChange={(event) => setForm({ ...form, baseUrl: event.target.value })} />
        </Field>
        <div className="form-row">
          <Field label={t('E-mail')} hint={t('Cloud: the e-mail of the account the API token belongs to. Leave empty for a personal access token (Server, Data Center).')}>
            <input className="input" type="email" autoComplete="off" value={form.email} onChange={(event) => setForm({ ...form, email: event.target.value })} />
          </Field>
          <Field label={t('Token')} hint={settings.tokenHint ? t('A token ending in {hint} is stored. Leave blank to keep it.', { hint: settings.tokenHint }) : t('No token is stored.')}>
            <input className="input input-mono" type="password" autoComplete="new-password" value={form.token} onChange={(event) => setForm({ ...form, token: event.target.value })} />
          </Field>
        </div>
        <div className="inline-form confluence-actions">
          <Button variant="primary" type="submit" disabled={busy || !changed}>
            {t('Save')}
          </Button>
          <Button icon="play" disabled={testing || busy || changed || !settings.configured} title={changed ? t('Save first: the test uses what is stored') : undefined} onClick={() => void test()}>
            {testing ? t('Testing…') : t('Test connection')}
          </Button>
          {settings.tokenHint && (
            <Button variant="danger" disabled={busy} onClick={() => void run(async () => onSaved(await api.put<ConfluenceSettings>(`${BASE}/settings`, { baseUrl: settings.baseUrl, email: settings.email, clearToken: true })), t('Token removed'))}>
              {t('Remove token')}
            </Button>
          )}
        </div>
        {result && (
          <div className={result.ok ? 'confluence-answer confluence-answer-ok' : 'confluence-answer confluence-answer-failed'} role="status">
            <Icon name={result.ok ? 'check' : 'alert'} />
            <span>
              <strong>{result.ok ? t('It works.') : t('It does not work.')}</strong> {result.message}
            </span>
          </div>
        )}
      </form>
    </SettingsCard>
  );
}

function ImportForm({ onDone }: { onDone: () => void }) {
  const remote = useRemote(() => api.get<ConfluenceRemoteSpace[]>(`${BASE}/spaces`), []);
  const [spaceKey, setSpaceKey] = useState<string | null>(null);
  const [target, setTarget] = useState('');
  const [rootPageId, setRootPageId] = useState('');
  const [exclude, setExclude] = useState('');
  const [running, setRunning] = useState(false);
  const [report, setReport] = useState<ConfluenceImportReport | null>(null);
  const suggested = spaceKey && SPACE_KEY.test(spaceKey.toUpperCase()) ? spaceKey.toUpperCase() : '';
  const targetKey = target || suggested;

  async function submit(event: FormEvent): Promise<void> {
    event.preventDefault();
    setRunning(true);
    setReport(null);

    const done = await attempt(() =>
      api.post<ConfluenceImportReport>(`${BASE}/import`, {
        spaceKey,
        targetSpaceKey: targetKey,
        ...(rootPageId.trim() ? { rootPageId: rootPageId.trim() } : {}),
        exclude: exclude
          .split('\n')
          .map((line) => line.trim())
          .filter(Boolean),
      }),
    );

    setRunning(false);

    if (done) {
      setReport(done);
      toast(t('Import finished'));
      onDone();
    }
  }

  return (
    <SettingsCard
      title={t('Import a space')}
      lead={t('Copies the pages of a Confluence space into a knowledge base space as a read-only mirror. Running it again later brings the copy up to date.')}
      action={
        <Button size="sm" icon="refresh" disabled={remote.loading || running} onClick={() => void remote.reload()}>
          {t('Reload spaces')}
        </Button>
      }
    >
      {remote.error && <ErrorNote message={remote.error} />}
      {!remote.data && !remote.error && <Spinner label={t('Asking Confluence for its spaces…')} />}
      {remote.data && (
        <form className="form" onSubmit={(event) => void submit(event)}>
          <fieldset className="confluence-fields" disabled={running}>
            <div className="form-row">
              <Field label={t('Confluence space')}>
                <Picker
                  value={spaceKey}
                  disabled={running}
                  placeholder={t('Choose a space…')}
                  options={remote.data.map((space) => ({ value: space.key as string | null, label: `${space.name} (${space.key})` }))}
                  onChange={(key) => {
                    setSpaceKey(key);
                    setTarget('');
                  }}
                />
              </Field>
              <Field label={t('Into the space with the key')} hint={t('Created if it does not exist. 2–10 letters and digits. A space that is not a Confluence import is never overwritten.')}>
                <input className="input input-mono" maxLength={10} value={targetKey} placeholder="DOCS" onChange={(event) => setTarget(event.target.value.toUpperCase().replace(/[^A-Z0-9]/g, ''))} />
              </Field>
            </div>
            <Field label={t('Only under the page with this id (optional)')} hint={t('The number in the address of the page in Confluence. Empty imports the whole space.')}>
              <input className="input input-mono confluence-short" inputMode="numeric" value={rootPageId} onChange={(event) => setRootPageId(event.target.value)} />
            </Field>
            <Field label={t('Leave out (optional)')} hint={t('One page per line, by title or by id. Everything under such a page is left out too.')}>
              <textarea className="input" rows={3} value={exclude} onChange={(event) => setExclude(event.target.value)} />
            </Field>
          </fieldset>
          <div className="inline-form confluence-actions">
            <Button variant="primary" type="submit" icon="download" disabled={running || !spaceKey || !SPACE_KEY.test(targetKey)}>
              {running ? t('Importing…') : t('Import')}
            </Button>
          </div>
          {running && <Working text={t('Importing {key}…', { key: spaceKey ?? '' })} />}
          {report && (
            <div className="confluence-result">
              <p>
                {t('Imported into')} <Link href={`/wiki/${report.wikiSpaceKey}`}>{report.wikiSpaceKey}</Link>:
              </p>
              <Report report={report} />
            </div>
          )}
        </form>
      )}
    </SettingsCard>
  );
}

function Imports({ imports, reload }: { imports: ConfluenceImport[]; reload: () => Promise<void> }) {
  const [syncing, setSyncing] = useState<string | null>(null);

  async function sync(key: string): Promise<void> {
    setSyncing(key);

    const report = await attempt(() => api.post<ConfluenceImportReport>(`${BASE}/imports/${key}/sync`, {}));

    if (report) {
      toast(t('{key} is up to date', { key }));
    }

    await reload();
    setSyncing(null);
  }

  return (
    <SettingsCard title={t('Imported spaces')} lead={t('A sync fetches the space again: new pages are added, changed ones get a new version, unchanged and detached ones are left alone.')}>
      {imports.length === 0 && <p className="muted">{t('Nothing has been imported yet.')}</p>}
      {syncing && <Working text={t('Syncing {key}…', { key: syncing })} />}
      {imports.map((entry) => (
        <div key={entry.wikiSpaceKey} className="confluence-import">
          <div className="confluence-import-head">
            <span className="project-badge">{entry.wikiSpaceKey}</span>
            <Link href={`/wiki/${entry.wikiSpaceKey}`} className="confluence-import-name">
              {entry.wikiSpaceName}
            </Link>
            <span className="muted">
              {t('from the Confluence space {key}', { key: entry.spaceKey })}
              {entry.rootPageId && ` · ${t('under page {id}', { id: entry.rootPageId })}`}
              {entry.exclude.length > 0 && ` · ${t('leaving out: {list}', { list: entry.exclude.join(', ') })}`}
            </span>
            <span className="filter-spacer" />
            <Button size="sm" icon="refresh" disabled={syncing !== null} onClick={() => void sync(entry.wikiSpaceKey)}>
              {syncing === entry.wikiSpaceKey ? t('Syncing…') : t('Sync now')}
            </Button>
          </div>
          <p className="muted confluence-import-when">
            {entry.lastRunAt ? (
              <span title={formatDateTime(entry.lastRunAt)}>{t('Last synced {when}', { when: timeAgo(entry.lastRunAt) })}</span>
            ) : (
              t('Never synced')
            )}
            {entry.baseUrl && ` · ${entry.baseUrl}`}
          </p>
          {entry.lastReport && <Report report={entry.lastReport} />}
        </div>
      ))}
    </SettingsCard>
  );
}

function AdminSettings() {
  const settings = useRemote(() => api.get<ConfluenceSettings>(`${BASE}/settings`), []);
  const imports = useRemote(() => api.get<ConfluenceImport[]>(`${BASE}/imports`), [], (event) => event.kind === 'confluence.imported');

  if (!settings.data) {
    return settings.error ? <ErrorNote message={settings.error} /> : <Spinner />;
  }

  return (
    <>
      <Connection settings={settings.data} onSaved={settings.set} />
      {settings.data.configured ? (
        <ImportForm key={`${settings.data.baseUrl}|${settings.data.email}|${settings.data.tokenHint}`} onDone={() => void imports.reload()} />
      ) : (
        <SettingsCard title={t('Import a space')}>
          <p className="muted">{t('Store the address and a token above first.')}</p>
        </SettingsCard>
      )}
      {imports.error && <ErrorNote message={imports.error} />}
      {imports.data && <Imports imports={imports.data} reload={imports.reload} />}
    </>
  );
}

export const confluenceModule: WebModule = {
  id: 'confluence',
  settings: [{ id: 'confluence', label: () => 'Confluence', adminOnly: true, render: () => <AdminSettings /> }],
  polish,
};
