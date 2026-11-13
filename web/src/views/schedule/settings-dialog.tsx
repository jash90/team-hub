import { useState, type FormEvent } from 'react';

import { api, errorMessage } from '../../api.ts';
import type { ScheduleSettings } from '../../bindings/ScheduleSettings.ts';
import { Button, Dialog, ErrorNote, Field, toast } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { browserZone } from './times.ts';

/** The two times, the time zone and the weekend, for the whole instance; its administrators' to change. */
export function SettingsDialog({ settings, onClose, onSaved }: { settings: ScheduleSettings; onClose: () => void; onSaved: (saved: ScheduleSettings) => void }) {
  const [form, setForm] = useState(settings);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const here = browserZone();

  async function submit(event: FormEvent): Promise<void> {
    event.preventDefault();
    setBusy(true);

    try {
      // Whether the zone exists is the server's to say: it is the one that keeps time by it.
      onSaved(await api.put<ScheduleSettings>('/api/admin/schedule', form));
      toast(t('Schedule settings saved'));
      onClose();
    } catch (error) {
      setProblem(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      title={t('Schedule settings')}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="schedule-settings" disabled={busy}>
            {t('Save')}
          </Button>
        </>
      }
    >
      <form id="schedule-settings" className="form" onSubmit={(event) => void submit(event)}>
        <p className="muted">{t('These apply to every project of this instance.')}</p>
        <div className="form-row">
          <Field label={t('Morning')}>
            <input className="input" type="time" required value={form.morning} onChange={(event) => setForm({ ...form, morning: event.target.value })} />
          </Field>
          <Field label={t('After work')}>
            <input className="input" type="time" required value={form.afternoon} onChange={(event) => setForm({ ...form, afternoon: event.target.value })} />
          </Field>
        </div>
        <Field label={t('Time zone')} hint={t('The IANA name of the zone the two times are in, such as Europe/Warsaw.')}>
          <input className="input" required spellCheck={false} value={form.timezone} placeholder="Europe/Warsaw" onChange={(event) => setForm({ ...form, timezone: event.target.value })} />
        </Field>
        {here !== form.timezone.trim() && (
          <p className="muted schedule-zone-hint">
            {t('Your browser is in {zone}.', { zone: here })}{' '}
            <button type="button" className="link-button" onClick={() => setForm({ ...form, timezone: here })}>
              {t('Use it')}
            </button>
          </p>
        )}
        <label className="check">
          <input type="checkbox" checked={form.weekdaysOnly} onChange={(event) => setForm({ ...form, weekdaysOnly: event.target.checked })} />
          <span>{t('Working days only: nothing runs by itself on Saturday and Sunday')}</span>
        </label>
        {problem && <ErrorNote message={problem} />}
      </form>
    </Dialog>
  );
}
