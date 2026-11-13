import { Empty, ErrorNote, Spinner } from '@web/components/ui.tsx';
import { countOf, t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { IssueRowLine } from '../components/bits.tsx';
import { useRemote } from '../live.ts';

/** Issues of the whole server matching words or a query in the server's query language. */
export function SearchView({ query }: { query: string }) {
  const found = useRemote(() => core.search(query), [query]);

  return (
    <div className="desk-view">
      <h1>{t('Search')}</h1>
      <p className="muted mono">{query}</p>
      {found.loading && !found.data && <Spinner />}
      {found.error && <ErrorNote message={found.error} />}
      {found.data?.result.error && <ErrorNote message={`${found.data.result.error.message} (${t('at character {position}', { position: found.data.result.error.position })})`} />}
      {found.data && !found.data.result.error && (
        <section className="panel">
          <header className="panel-head">
            <h2>{countOf(Number(found.data.result.total), 'issue', 'issues')}</h2>
          </header>
          {found.data.result.issues.length === 0 && <Empty icon="search" title={t('No issues match')} />}
          {found.data.result.issues.map((row) => (
            <IssueRowLine key={row.id} issue={row} projects={found.data?.projects ?? []} />
          ))}
        </section>
      )}
    </div>
  );
}
