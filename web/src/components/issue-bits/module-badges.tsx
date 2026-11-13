import type { IssueRow } from '../../bindings/IssueRow.ts';
import { useModules } from '../../modules.ts';

/** Marks from every module that is on in the issue's project (pull request state, drift, …). */
export function ModuleBadges({ issue }: { issue: IssueRow }) {
  const modules = useModules(issue.projectId);

  return (
    <>
      {modules.map((module) => {
        const Badges = module.badges;

        return Badges ? <Badges key={module.id} issue={issue} /> : null;
      })}
    </>
  );
}
