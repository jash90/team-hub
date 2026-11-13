import { formatDate, formatShortDate, t } from '../../i18n/index.ts';
import { Icon } from '../icon.tsx';

export function DueDate({ date, done, short }: { date: string | null; done?: boolean; short?: boolean }) {
  if (!date) {
    return null;
  }

  const overdue = !done && date < new Date().toISOString().slice(0, 10);

  return (
    <span className={overdue ? 'due due-late' : 'due'} title={t('Due date')}>
      <Icon name="calendar" size={12} />
      {short ? formatShortDate(date) : formatDate(date)}
    </span>
  );
}
