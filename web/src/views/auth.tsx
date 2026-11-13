import { useState, type FormEvent, type ReactNode } from 'react';

import { api } from '../api.ts';
import { Button, Field, useBusy } from '../components/ui.tsx';
import { currentLanguage, setLanguage, t } from '../i18n/index.ts';

function AuthFrame({ title, lead, children }: { title: string; lead: string; children: ReactNode }) {
  return (
    <div className="auth">
      <div className="auth-card">
        <div className="auth-brand">
          <svg viewBox="0 0 32 32" width="36" height="36">
            <rect width="32" height="32" rx="8" fill="var(--accent)" />
            <path d="M9 16.5l4.5 4.5L23 11" fill="none" stroke="#fff" strokeWidth="3.2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          <strong>team-hub</strong>
        </div>
        <h1>{title}</h1>
        <p className="muted">{lead}</p>
        {children}
        <button type="button" className="link-button auth-language" onClick={() => setLanguage(currentLanguage() === 'pl' ? 'en' : 'pl')}>
          {currentLanguage() === 'pl' ? 'English' : 'Polski'}
        </button>
      </div>
    </div>
  );
}

export function SetupView({ onDone }: { onDone: () => Promise<void> }) {
  const [form, setForm] = useState({ name: '', email: '', password: '', demo: true });
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();
    void run(async () => {
      await api.post('/api/setup', form);
      await onDone();
    });
  }

  return (
    <AuthFrame title={t('Set up team-hub')} lead={t('Create the first account. It becomes the administrator of this instance.')}>
      <form className="form" onSubmit={submit}>
        <Field label={t('Your name')}>
          <input className="input" required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} autoComplete="name" />
        </Field>
        <Field label={t('E-mail')}>
          <input className="input" type="email" required value={form.email} onChange={(event) => setForm({ ...form, email: event.target.value })} autoComplete="email" />
        </Field>
        <Field label={t('Password')} hint={t('At least 8 characters')}>
          <input className="input" type="password" required minLength={8} value={form.password} onChange={(event) => setForm({ ...form, password: event.target.value })} autoComplete="new-password" />
        </Field>
        <label className="check">
          <input type="checkbox" checked={form.demo} onChange={(event) => setForm({ ...form, demo: event.target.checked })} />
          <span>{t('Start with example projects, a team and a few weeks of history')}</span>
        </label>
        <Button variant="primary" type="submit" disabled={busy}>
          {busy ? t('Setting up…') : t('Create account')}
        </Button>
      </form>
    </AuthFrame>
  );
}

export function LoginView({ onDone }: { onDone: () => Promise<void> }) {
  const [form, setForm] = useState({ email: '', password: '' });
  const [busy, run] = useBusy();

  function submit(event: FormEvent): void {
    event.preventDefault();
    void run(async () => {
      await api.post('/api/login', form);
      await onDone();
    });
  }

  return (
    <AuthFrame title={t('Sign in')} lead={t('Use the e-mail address and password of your account.')}>
      <form className="form" onSubmit={submit}>
        <Field label={t('E-mail')}>
          <input className="input" type="email" required value={form.email} onChange={(event) => setForm({ ...form, email: event.target.value })} autoComplete="username" />
        </Field>
        <Field label={t('Password')}>
          <input className="input" type="password" required value={form.password} onChange={(event) => setForm({ ...form, password: event.target.value })} autoComplete="current-password" />
        </Field>
        <Button variant="primary" type="submit" disabled={busy}>
          {t('Sign in')}
        </Button>
      </form>
    </AuthFrame>
  );
}
