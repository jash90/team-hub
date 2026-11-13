import type { QueryFieldInfo } from '../bindings/QueryFieldInfo.ts';
import { t } from '../i18n/index.ts';
import { Popover } from './ui.tsx';

const FUNCTIONS = ['currentUser()', 'openSprints()', 'now()', 'startOfDay()', 'startOfWeek()', 'startOfMonth()'];

/** The fields of the query language with an example each, for wherever a query is written. */
export function QueryHints({ anchor, fields, onPick, onClose }: { anchor: HTMLElement; fields: QueryFieldInfo[]; onPick: (example: string) => void; onClose: () => void }) {
  return (
    <Popover anchor={anchor} onClose={onClose} align="end" width={460}>
      <div className="hints">
        <p className="muted">{t('Combine clauses with AND, OR, NOT and parentheses; finish with ORDER BY. Click an example to add it.')}</p>
        <div className="hints-grid">
          {fields.map((field) => (
            <button key={field.name} type="button" className="hint" onClick={() => onPick(field.example)}>
              <strong>{field.name}</strong>
              <code>{field.example}</code>
            </button>
          ))}
        </div>
        <p className="muted">
          {t('Functions')}: {FUNCTIONS.join(', ')} · {t('dates')}: 2026-11-30, -7d, 2w
        </p>
      </div>
    </Popover>
  );
}

/** The query with one more clause in front, as clicking an example adds it. */
export function withClause(query: string, example: string): string {
  return query.trim() ? `${example} AND ${query}` : example;
}
