import { t } from '../../i18n/index.ts';

export const PRIORITIES = [
  { value: 1, name: 'Highest' },
  { value: 2, name: 'High' },
  { value: 3, name: 'Medium' },
  { value: 4, name: 'Low' },
  { value: 5, name: 'Lowest' },
];

export function priorityName(value: number): string {
  return t(PRIORITIES.find((priority) => priority.value === value)?.name ?? 'Medium');
}

const PRIORITY_PATHS: Record<number, string[]> = {
  1: ['m6 11 6-6 6 6', 'm6 18 6-6 6 6'],
  2: ['m6 15 6-6 6 6'],
  3: ['M6 9h12', 'M6 15h12'],
  4: ['m6 9 6 6 6-6'],
  5: ['m6 6 6 6 6-6', 'm6 13 6 6 6-6'],
};

export function PriorityIcon({ priority, size = 16 }: { priority: number; size?: number }) {
  return (
    <span className={`priority priority-${priority}`} title={`${t('Priority')}: ${priorityName(priority)}`}>
      <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round">
        {(PRIORITY_PATHS[priority] ?? PRIORITY_PATHS[3]).map((path) => (
          <path key={path} d={path} />
        ))}
      </svg>
    </span>
  );
}
