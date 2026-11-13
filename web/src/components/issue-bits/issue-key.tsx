import { Link, setParam } from '../../router.tsx';

/** Opens the issue beside the current view. */
export function openIssue(key: string): void {
  setParam('issue', key);
}

export function IssueKey({ issue, done }: { issue: { key: string }; done?: boolean }) {
  return (
    <Link
      href={`/issue/${issue.key}`}
      className={done ? 'issue-key issue-key-done' : 'issue-key'}
      onClick={(event) => {
        if (event.button === 0 && !event.metaKey && !event.ctrlKey) {
          event.preventDefault();
          openIssue(issue.key);
        }
      }}
    >
      {issue.key}
    </Link>
  );
}
