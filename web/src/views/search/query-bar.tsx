import { useState, type Dispatch, type SetStateAction } from 'react';

import type { QueryFieldInfo } from '../../bindings/QueryFieldInfo.ts';
import { Icon } from '../../components/icon.tsx';
import { PRIORITIES } from '../../components/issue-bits.tsx';
import { QueryHints, withClause } from '../../components/query-hints.tsx';
import { Button, Picker } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import type { Remote } from '../../session.tsx';
import type { Chips } from './query.ts';

type QueryBarProps = {
  advanced: boolean;
  setAdvanced: (advanced: boolean) => void;
  text: string;
  setText: Dispatch<SetStateAction<string>>;
  /** The query that ran last did not parse. */
  error: boolean;
  run: (query: string) => void;
  chips: Chips;
  words: string;
  setWords: (words: string) => void;
  applyChips: (chips: Chips, words: string) => void;
  fields: Remote<QueryFieldInfo[]>;
};

/** Where the query comes from: the simple filters, or the query language with its list of fields. */
export function QueryBar({ advanced, setAdvanced, text, setText, error, run, chips, words, setWords, applyChips, fields }: QueryBarProps) {
  const [hints, setHints] = useState<HTMLElement | null>(null);

  return (
    <>
      <div className="search-box">
        {advanced ? (
          <form
            className="search-query"
            onSubmit={(event) => {
              event.preventDefault();
              run(text);
            }}
          >
            <input
              className={error ? 'input input-mono input-error' : 'input input-mono'}
              spellCheck={false}
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder='project = WEB AND status = "In Progress" ORDER BY priority DESC'
              aria-label={t('Query')}
            />
            <Button variant="primary" type="submit" icon="search">
              {t('Search')}
            </Button>
            <Button icon="info" onClick={(event) => setHints(hints ? null : event.currentTarget)}>
              {t('Fields')}
            </Button>
          </form>
        ) : (
          <div className="search-chips">
            <div className="filter-search">
              <Icon name="search" />
              <input className="input" placeholder={t('Words in the summary, description or comments')} value={words} onChange={(event) => setWords(event.target.value)} onKeyDown={(event) => event.key === 'Enter' && applyChips(chips, words)} />
            </div>
            <Picker
              value={chips.status}
              onChange={(status) => applyChips({ ...chips, status }, words)}
              options={[
                { value: '', label: t('Any status') },
                { value: 'todo', label: t('To do') },
                { value: 'in progress', label: t('In progress') },
                { value: 'done', label: t('Done') },
              ]}
            />
            <Picker
              value={chips.assignee}
              onChange={(assignee) => applyChips({ ...chips, assignee }, words)}
              options={[
                { value: '', label: t('Anyone') },
                { value: 'me', label: t('Assigned to me') },
                { value: 'none', label: t('Unassigned') },
              ]}
            />
            <Picker
              value={chips.type}
              onChange={(type) => applyChips({ ...chips, type }, words)}
              options={[{ value: '', label: t('Any type') }, ...['Epic', 'Story', 'Task', 'Bug', 'Subtask'].map((name) => ({ value: name, label: name }))]}
            />
            <Picker
              value={chips.priority}
              onChange={(priority) => applyChips({ ...chips, priority }, words)}
              options={[{ value: '', label: t('Any priority') }, ...PRIORITIES.map((priority) => ({ value: priority.name, label: t(priority.name) }))]}
            />
          </div>
        )}
        <button type="button" className="link-button" onClick={() => setAdvanced(!advanced)}>
          {advanced ? t('Simple filters') : t('Write a query')}
        </button>
      </div>

      {hints && fields.data && <QueryHints anchor={hints} fields={fields.data} onPick={(example) => setText((current) => withClause(current, example))} onClose={() => setHints(null)} />}
    </>
  );
}
