import { useEffect, useState, type FormEvent } from 'react';

import { api, query } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import type { QueryFieldInfo } from '../../bindings/QueryFieldInfo.ts';
import type { ScheduleSettings } from '../../bindings/ScheduleSettings.ts';
import type { ScheduledTask } from '../../bindings/ScheduledTask.ts';
import type { SearchResult } from '../../bindings/SearchResult.ts';
import type { TaskField } from '../../bindings/TaskField.ts';
import type { TaskKind } from '../../bindings/TaskKind.ts';
import type { TaskSlot } from '../../bindings/TaskSlot.ts';
import { QueryHints, withClause } from '../../components/query-hints.tsx';
import { Button, Dialog, Field, Picker, useBusy } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { translateServer } from '../../i18n/server.ts';
import { useRemote } from '../../session.tsx';

type Values = Record<string, unknown>;

/** What the form starts with: the task's settings, else what the kind suggests. Numbers are edited as text. */
function initialValues(kind: TaskKind, task: ScheduledTask | null): Values {
  return Object.fromEntries(
    kind.fields.map((field) => {
      const value = task?.config[field.name] ?? field.default ?? (field.type === 'boolean' ? false : '');

      return [field.name, field.type === 'number' && typeof value === 'number' ? String(value) : value];
    }),
  );
}

/** The settings as the server takes them: a number as a number, an empty one left out. */
function toConfig(kind: TaskKind, values: Values): Values {
  return Object.fromEntries(
    kind.fields.map((field) => {
      const value = values[field.name];

      return [field.name, field.type === 'number' ? (String(value ?? '').trim() === '' ? null : Number(value)) : value];
    }),
  );
}

/**
 * A query, written the way the search view writes one: the same list of fields to pick
 * from, and the server's word on whether it understands the text, while it is typed.
 */
function QueryInput({ value, onChange, required }: { value: string; onChange: (value: string) => void; required: boolean }) {
  const fields = useRemote(() => api.get<QueryFieldInfo[]>('/api/search/fields'), []);
  const [hints, setHints] = useState<HTMLElement | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    if (!value.trim()) {
      setProblem(null);

      return undefined;
    }

    const timer = window.setTimeout(() => {
      api
        .get<SearchResult>(`/api/search${query({ q: value, limit: 1 })}`)
        .then((result) => setProblem(result.error ? `${translateServer(result.error.message)} (${t('at character {position}', { position: result.error.position + 1 })})` : null))
        .catch(() => setProblem(null));
    }, 350);

    return () => window.clearTimeout(timer);
  }, [value]);

  return (
    <>
      <span className="task-query">
        <input
          className={problem ? 'input input-mono input-error' : 'input input-mono'}
          spellCheck={false}
          required={required}
          value={value}
          placeholder='status = "To Do" AND created >= -1d ORDER BY priority'
          onChange={(event) => onChange(event.target.value)}
        />
        <Button icon="info" onClick={(event) => setHints(hints ? null : event.currentTarget)}>
          {t('Fields')}
        </Button>
      </span>
      {problem && <span className="field-hint task-field-problem">{problem}</span>}
      {hints && fields.data && <QueryHints anchor={hints} fields={fields.data} onPick={(example) => onChange(withClause(value, example))} onClose={() => setHints(null)} />}
    </>
  );
}

/** One field of a kind's form. What the labels say comes from the module, in English. */
function ConfigField({ field, value, onChange }: { field: TaskField; value: unknown; onChange: (value: unknown) => void }) {
  const label = translateServer(field.label);
  const hint = field.hint ? translateServer(field.hint) : undefined;
  const text = typeof value === 'string' ? value : '';

  if (field.type === 'boolean') {
    return (
      <div className="task-check">
        <label className="check">
          <input type="checkbox" checked={value === true} onChange={(event) => onChange(event.target.checked)} />
          <span>{label}</span>
        </label>
        {hint && <span className="field-hint">{hint}</span>}
      </div>
    );
  }

  if (field.type === 'select') {
    const options = field.options.map((option) => ({ value: option.value, label: translateServer(option.label) }));

    return (
      <Field label={label} hint={hint}>
        <Picker value={text} options={field.required ? options : [{ value: '', label: t('None') }, ...options]} onChange={onChange} label={label} />
      </Field>
    );
  }

  return (
    <Field label={label} hint={hint}>
      {field.type === 'query' && <QueryInput value={text} onChange={onChange} required={field.required} />}
      {field.type === 'textarea' && <textarea className="input" rows={3} required={field.required} value={text} onChange={(event) => onChange(event.target.value)} />}
      {field.type === 'text' && <input className="input" required={field.required} value={text} onChange={(event) => onChange(event.target.value)} />}
      {field.type === 'number' && <input className="input input-number" type="number" required={field.required} value={text} onChange={(event) => onChange(event.target.value)} />}
    </Field>
  );
}

type TaskDialogProps = {
  project: Project;
  /** The kinds this project can run. */
  kinds: TaskKind[];
  settings: ScheduleSettings | undefined;
  /** The task to change; null for a new one. */
  task: ScheduledTask | null;
  onClose: () => void;
  onSaved: () => Promise<void>;
};

/** A new task: first what kind of thing to run, then that kind's form. Or an existing task's form. */
export function TaskDialog({ project, kinds, settings, task, onClose, onSaved }: TaskDialogProps) {
  const [kind, setKind] = useState<TaskKind | null>(() => (task ? (kinds.find((candidate) => candidate.id === task.kind) ?? null) : null));
  const [name, setName] = useState(task?.name ?? '');
  const [slot, setSlot] = useState<TaskSlot>(task?.slot ?? 'morning');
  const [values, setValues] = useState<Values>(() => (kind ? initialValues(kind, task) : {}));
  const [busy, run] = useBusy();

  function choose(chosen: TaskKind): void {
    setKind(chosen);
    setValues(initialValues(chosen, null));
  }

  function submit(event: FormEvent): void {
    event.preventDefault();

    if (!kind) {
      return;
    }

    void run(async () => {
      const body = { name, slot, config: toConfig(kind, values) };

      if (task) {
        await api.patch(`/api/tasks/${task.id}`, body);
      } else {
        await api.post(`/api/projects/${project.key}/tasks`, { ...body, kind: kind.id });
      }

      await onSaved();
      onClose();
    });
  }

  if (!kind) {
    return (
      <Dialog title={t('New task')} onClose={onClose}>
        <div className="form">
          <p className="muted">{t('What should run?')}</p>
          {kinds.map((candidate) => (
            <button key={candidate.id} type="button" className="task-kind" onClick={() => choose(candidate)}>
              <strong>{translateServer(candidate.name)}</strong>
              <span className="muted">{translateServer(candidate.description)}</span>
            </button>
          ))}
          {kinds.length === 0 && <p className="muted">{t('No module that is switched on in this project has anything to run on a schedule.')}</p>}
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={task ? t('Edit task') : t('New task')}
      onClose={onClose}
      wide
      footer={
        <>
          <span className="muted dialog-foot-left">{t('The task will run with your permissions.')}</span>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button variant="primary" type="submit" form="task-form" disabled={busy || !name.trim()}>
            {t('Save')}
          </Button>
        </>
      }
    >
      <form id="task-form" className="form" onSubmit={submit}>
        <div className="task-kind-chosen">
          <div>
            <strong>{translateServer(kind.name)}</strong>
            <p className="muted">{translateServer(kind.description)}</p>
          </div>
          {!task && kinds.length > 1 && (
            <button type="button" className="link-button" onClick={() => setKind(null)}>
              {t('Choose something else')}
            </button>
          )}
        </div>
        <div className="form-row">
          <Field label={t('Name')}>
            <input className="input" autoFocus required value={name} placeholder={t('Morning triage')} onChange={(event) => setName(event.target.value)} />
          </Field>
          <Field label={t('When')}>
            <Picker
              value={slot}
              onChange={setSlot}
              label={t('When')}
              options={[
                { value: 'morning', label: settings ? t('Morning, {time}', { time: settings.morning }) : t('Morning') },
                { value: 'afternoon', label: settings ? t('After work, {time}', { time: settings.afternoon }) : t('After work') },
                { value: 'on_demand', label: t('On demand') },
              ]}
            />
          </Field>
        </div>
        {kind.fields.map((field) => (
          <ConfigField key={field.name} field={field} value={values[field.name]} onChange={(value) => setValues((current) => ({ ...current, [field.name]: value }))} />
        ))}
      </form>
    </Dialog>
  );
}
