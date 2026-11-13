// Where a piece of work is carried out: this computer, a runner by name, or whichever fits.
import type { Runner } from '@web/bindings/Runner.ts';
import { t } from '@web/i18n/index.ts';

export const HERE = 'here';
export const ANY = 'any';

/** What a runner is: somebody's computer, the team's shared service, or somebody's service. */
export function runnerKind(runner: Runner): string {
  return runner.interactive ? t('computer') : runner.shared ? t('shared service') : t('service');
}

/**
 * The runners that take `module`'s work (of `what`, in `project`) and that this person may
 * name: their own, and services while they are online. This computer is offered apart.
 */
export function namableRunners(runners: Runner[], here: string, me: number | undefined, module: string, what?: string, project?: string): Runner[] {
  return runners.filter((runner) => {
    const work = runner.work.find((candidate) => candidate.module === module);
    const takes = work && (!what || work.what.includes(what)) && (!project || !work.projects || work.projects.includes(project));

    return runner.id !== here && takes && (runner.userId === me || (!runner.interactive && runner.online));
  });
}

export function WhereSelect({ value, onChange, runners, anyLabel }: { value: string; onChange: (value: string) => void; runners: Runner[]; anyLabel?: string }) {
  return (
    <select className="input" value={value} onChange={(event) => onChange(event.target.value)} aria-label={t('Where it is carried out')}>
      <option value={HERE}>{t('This computer')}</option>
      {runners.map((runner) => (
        <option key={runner.id} value={runner.id}>
          {runner.name} · {runnerKind(runner)}
          {runner.online ? '' : ` · ${t('offline')}`}
        </option>
      ))}
      <option value={ANY}>{anyLabel ?? t('Any runner that fits')}</option>
    </select>
  );
}
