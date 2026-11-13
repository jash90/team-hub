import type { Status } from '../../bindings/Status.ts';
import type { StatusCategory } from '../../bindings/StatusCategory.ts';
import { t } from '../../i18n/index.ts';

export function categoryLabel(category: StatusCategory): string {
  return category === 'todo' ? t('To do') : category === 'done' ? t('Done') : t('In progress');
}

export function StatusPill({ status }: { status: Status | undefined }) {
  return <span className={`status status-${status?.category ?? 'todo'}`}>{status?.name ?? '?'}</span>;
}
