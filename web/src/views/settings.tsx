import { useState, type FormEvent } from 'react';

import { api } from '../api.ts';
import type { ApiToken } from '../bindings/ApiToken.ts';
import type { ModuleInfo } from '../bindings/ModuleInfo.ts';
import type { PersonalKeyState } from '../bindings/PersonalKeyState.ts';
import type { User } from '../bindings/User.ts';
import { Avatar, Button, Dialog, Field, Picker, SettingsCard, Tabs, Tag, attempt, copyText, toast, useBusy } from '../components/ui.tsx';
import { currentLanguage, formatDate, setLanguage, t, timeAgo } from '../i18n/index.ts';
import { translateServer } from '../i18n/server.ts';
import { useModules } from '../modules.ts';
import { navigate } from '../router.tsx';
import { useRemote, useSession } from '../session.tsx';

export function SettingsView({ tab }: { tab?: string }) {
  const { user } = useSession();
  const modules = useModules();
  const admin = user.role === 'admin';
  const moduleTabs = modules.flatMap((module) => module.settings ?? []).filter((entry) => admin || !entry.adminOnly);
  const tabs = [
    { id: 'profile', label: t('Profile') },
    { id: 'tokens', label: t('API tokens') },
    { id: 'keys', label: t('My keys') },
    ...(admin
      ? [
          { id: 'users', label: t('People') },
          { id: 'modules', label: t('Modules') },
        ]
      : []),
    ...moduleTabs.map((entry) => ({ id: entry.id, label: entry.label() })),
  ];
  const active = tabs.some((candidate) => candidate.id === tab) ? (tab as string) : 'profile';

  return (
    <div className="view view-narrow">
      <header className="view-head">
        <div>
          <h1>{t('Settings')}</h1>
        </div>
      </header>
      <Tabs tabs={tabs} active={active} onChange={(id) => navigate(`/settings/${id}`)} />
      <div className="settings-body">
        {active === 'profile' && <Profile />}
        {active === 'tokens' && <Tokens />}
        {active === 'keys' && <Keys />}
        {active === 'users' && admin && <People />}
        {active === 'modules' && admin && <Modules />}
        {moduleTabs.find((entry) => entry.id === active)?.render()}
      </div>
    </div>
  );
}

function Profile() {
  const { user, boot, refresh } = useSession();
  const me = boot.users.find((candidate) => candidate.id === user.id);
  const [form, setForm] = useState({ name: user.name, email: user.email, githubLogin: me?.githubLogin ?? '' });
  const [password, setPassword] = useState('');
  const [currentPassword, setCurrentPassword] = useState('');
  const [busy, run] = useBusy();

  function save(event: FormEvent): void {
    event.preventDefault();
    void run(async () => {
      await api.patch(`/api/users/${user.id}`, form);
      await refresh();
    }, t('Saved'));
  }

  return (
    <>
      <SettingsCard title={t('Your account')}>
        <form className="form" onSubmit={save}>
          <div className="form-row">
            <Field label={t('Name')}>
              <input className="input" required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
            </Field>
            <Field label={t('E-mail')}>
              <input className="input" type="email" required value={form.email} onChange={(event) => setForm({ ...form, email: event.target.value })} />
            </Field>
          </div>
          <Field label={t('GitHub login')} hint={t('Lets commits and pull requests be recognised as yours.')}>
            <input className="input" value={form.githubLogin} onChange={(event) => setForm({ ...form, githubLogin: event.target.value })} />
          </Field>
          <div>
            <Button variant="primary" type="submit" disabled={busy}>
              {t('Save')}
            </Button>
          </div>
        </form>
      </SettingsCard>
      <SettingsCard title={t('Password')}>
        <form
          className="inline-form"
          onSubmit={(event) => {
            event.preventDefault();
            void run(() => api.patch(`/api/users/${user.id}`, { password, currentPassword }), t('Password changed')).then(() => {
              setPassword('');
              setCurrentPassword('');
            });
          }}
        >
          <input className="input" type="password" placeholder={t('Current password')} autoComplete="current-password" value={currentPassword} onChange={(event) => setCurrentPassword(event.target.value)} />
          <input className="input" type="password" minLength={8} placeholder={t('New password')} autoComplete="new-password" value={password} onChange={(event) => setPassword(event.target.value)} />
          <Button type="submit" disabled={busy || password.length < 8 || !currentPassword}>
            {t('Change password')}
          </Button>
        </form>
      </SettingsCard>
      <SettingsCard title={t('Language')}>
        <Picker
          value={currentLanguage()}
          onChange={setLanguage}
          options={[
            { value: 'en', label: 'English' },
            { value: 'pl', label: 'Polski' },
          ]}
        />
      </SettingsCard>
    </>
  );
}

/**
 * The person's own keys (`docs/runners.md`): what a shared runner works with on their runs
 * and test jobs. A value goes in and never comes back; the page sees only that it is set.
 */
function Keys() {
  const keys = useRemote(() => api.get<PersonalKeyState[]>('/api/me/keys'), []);

  return (
    <SettingsCard
      title={t('My keys')}
      lead={t('A shared runner belongs to the team and has no keys of its own: it works on your agent runs and test jobs with yours, so your runs are billed to you and reach only what you can reach. The keys are stored encrypted, are never shown again, and leave the server only to a shared runner that has taken your work. The desktop app does not need them.')}
    >
      {keys.error && <p className="muted">{keys.error}</p>}
      {keys.data?.length === 0 && <p className="muted">{t('No module that is switched on uses personal keys.')}</p>}
      {keys.data?.map((key) => (
        <KeyRow key={key.name} state={key} onChanged={() => void keys.reload()} />
      ))}
    </SettingsCard>
  );
}

function KeyRow({ state, onChanged }: { state: PersonalKeyState; onChanged: () => void }) {
  const [value, setValue] = useState('');
  const [busy, run] = useBusy();
  const path = `/api/me/keys/${state.name}`;

  return (
    <div className="key-row">
      <div className="key-row-head">
        <strong>{translateServer(state.label)}</strong>
        {state.set ? (
          <Tag tone="green">
            {t('set')} <code>{state.tail}</code>
          </Tag>
        ) : (
          <Tag>{t('not set')}</Tag>
        )}
        {state.set && state.updatedAt && <span className="muted">{t('changed {when}', { when: timeAgo(state.updatedAt) })}</span>}
      </div>
      <p className="muted">{translateServer(state.hint)}</p>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void run(() => api.put(path, { value }), t('Key saved')).then(() => {
            setValue('');
            onChanged();
          });
        }}
      >
        <input
          className="input input-mono"
          type="password"
          autoComplete="off"
          aria-label={translateServer(state.label)}
          placeholder={state.set ? t('Paste a new value to replace it') : t('Paste the key')}
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
        <Button type="submit" icon="key" disabled={busy || !value.trim()}>
          {state.set ? t('Replace') : t('Save')}
        </Button>
        {state.set && (
          <Button variant="ghost" icon="trash" disabled={busy} onClick={() => void run(() => api.del(path), t('Key removed')).then(onChanged)}>
            {t('Remove')}
          </Button>
        )}
      </form>
    </div>
  );
}

function Tokens() {
  const tokens = useRemote(() => api.get<ApiToken[]>('/api/me/tokens'), []);
  const [name, setName] = useState('');
  const [fresh, setFresh] = useState<string | null>(null);
  const [busy, run] = useBusy();

  return (
    <SettingsCard title={t('API tokens')} lead={t('A token acts as you, with your permissions: for scripts, CI and AI agents. Send it as “Authorization: Bearer …”.')}>
      <table className="table table-plain">
        <tbody>
          {tokens.data?.map((token) => (
            <tr key={token.id}>
              <td>
                <strong>{token.name}</strong>
              </td>
              <td>
                <code>{token.hint}</code>
              </td>
              <td className="muted">
                {t('created {when}', { when: formatDate(token.createdAt) })}
                {token.lastUsedAt ? ` · ${t('last used {when}', { when: timeAgo(token.lastUsedAt) })}` : ` · ${t('never used')}`}
              </td>
              <td className="cell-actions">
                <Button size="sm" variant="ghost" icon="trash" aria-label={t('Revoke')} onClick={() => void attempt(() => api.del(`/api/me/tokens/${token.id}`), t('Token revoked')).then(() => tokens.reload())} />
              </td>
            </tr>
          ))}
          {tokens.data?.length === 0 && (
            <tr>
              <td className="muted">{t('No tokens yet.')}</td>
            </tr>
          )}
        </tbody>
      </table>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void run(async () => {
            const created = await api.post<{ token: string }>('/api/me/tokens', { name });
            setFresh(created.token);
            setName('');
            await tokens.reload();
          });
        }}
      >
        <input className="input" placeholder={t('What is it for? e.g. “CI”')} value={name} onChange={(event) => setName(event.target.value)} />
        <Button type="submit" icon="key" disabled={busy || !name.trim()}>
          {t('Create token')}
        </Button>
      </form>
      {fresh && (
        <Dialog
          title={t('Your new token')}
          onClose={() => setFresh(null)}
          footer={
            <>
              <Button icon="copy" onClick={() => void copyText(fresh)}>
                {t('Copy')}
              </Button>
              <Button variant="primary" onClick={() => setFresh(null)}>
                {t('Done')}
              </Button>
            </>
          }
        >
          <div className="form">
            <p>{t('Copy it now. It is shown only this once; afterwards only its last characters are visible.')}</p>
            <code className="code-block">{fresh}</code>
          </div>
        </Dialog>
      )}
    </SettingsCard>
  );
}

function People() {
  const { boot, refresh, user: me } = useSession();
  const [adding, setAdding] = useState(false);

  async function change(user: User, patch: Record<string, unknown>): Promise<void> {
    await attempt(() => api.patch(`/api/users/${user.id}`, patch));
    await refresh();
  }

  return (
    <SettingsCard
      title={t('People')}
      lead={t('Everyone with an account on this instance.')}
      action={
        <Button icon="plus" onClick={() => setAdding(true)}>
          {t('Add person')}
        </Button>
      }
    >
      <table className="table table-plain">
        <tbody>
          {boot.users.map((user) => (
            <tr key={user.id} className={user.active ? undefined : 'row-muted'}>
              <td>
                <span className="with-avatar">
                  <Avatar user={user} size={26} />
                  <span>
                    <strong>{user.name}</strong>
                    <br />
                    <span className="muted">{user.email}</span>
                  </span>
                </span>
              </td>
              <td>
                <Picker
                  value={user.role}
                  disabled={user.id === me.id}
                  options={[
                    { value: 'admin', label: t('Administrator') },
                    { value: 'member', label: t('Member') },
                  ]}
                  onChange={(role) => void change(user, { role })}
                />
              </td>
              <td>{user.active ? <Tag tone="green">{t('active')}</Tag> : <Tag>{t('deactivated')}</Tag>}</td>
              <td className="cell-actions">
                {user.id !== me.id && (
                  <Button size="sm" onClick={() => void change(user, { active: !user.active })}>
                    {user.active ? t('Deactivate') : t('Reactivate')}
                  </Button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {adding && <AddPersonDialog onClose={() => setAdding(false)} />}
    </SettingsCard>
  );
}

function AddPersonDialog({ onClose }: { onClose: () => void }) {
  const { refresh } = useSession();
  const [form, setForm] = useState({ name: '', email: '', password: '', role: 'member' });
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();
    void run(async () => {
      await api.post('/api/users', form);
      await refresh();
      onClose();
    }, t('Account created'));
  }

  return (
    <Dialog
      title={t('Add person')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="add-person" disabled={busy}>
            {t('Create account')}
          </Button>
        </>
      }
    >
      <form id="add-person" className="form" onSubmit={submit}>
        <Field label={t('Name')}>
          <input className="input" autoFocus required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
        </Field>
        <Field label={t('E-mail')}>
          <input className="input" type="email" required value={form.email} onChange={(event) => setForm({ ...form, email: event.target.value })} />
        </Field>
        <Field label={t('Initial password')} hint={t('Give it to them directly; they can change it in their profile.')}>
          <input className="input" required minLength={8} value={form.password} onChange={(event) => setForm({ ...form, password: event.target.value })} />
        </Field>
        <Field label={t('Role')}>
          <Picker
            value={form.role}
            options={[
              { value: 'member', label: t('Member') },
              { value: 'admin', label: t('Administrator') },
            ]}
            onChange={(role) => setForm({ ...form, role })}
          />
        </Field>
      </form>
    </Dialog>
  );
}

function Modules() {
  const { boot, refresh } = useSession();

  async function toggle(module: ModuleInfo): Promise<void> {
    await attempt(() => api.put(`/api/admin/modules/${module.id}`, { enabled: !module.enabled }));
    await refresh();
    toast(module.enabled ? t('{name} switched off', { name: translateServer(module.name) }) : t('{name} switched on', { name: translateServer(module.name) }));
  }

  return (
    <SettingsCard title={t('Modules')} lead={t('Everything beyond the tracker itself is a module. A module that is off keeps its data but shows nothing and receives nothing.')}>
      <div className="module-list">
        {boot.modules.map((module) => (
          <div key={module.id} className="module-row">
            <div>
              <strong>{translateServer(module.name)}</strong> <span className="muted">v{module.version}</span>
              {module.dependsOn.length > 0 && (
                <span className="muted">
                  {' '}
                  · {t('needs')} {module.dependsOn.join(', ')}
                </span>
              )}
              <p className="muted">{translateServer(module.description)}</p>
            </div>
            <label className="switch">
              <input type="checkbox" checked={module.enabled} onChange={() => void toggle(module)} aria-label={translateServer(module.name)} />
              <span />
            </label>
          </div>
        ))}
      </div>
    </SettingsCard>
  );
}
