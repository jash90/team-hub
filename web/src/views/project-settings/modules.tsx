import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import { t } from '../../i18n/index.ts';
import { useSession } from '../../session.tsx';
import { Card, useChange } from './parts.tsx';

/** The modules that are on for the instance, each with a switch for this project. */
export function Modules({ project }: { project: Project }) {
  const { boot } = useSession();
  const change = useChange();
  const modules = boot.modules.filter((module) => module.enabled);

  return (
    <Card title={t('Modules')} lead={t('A module switched off here keeps its data, but its navigation, its panel on issues and its marks on cards disappear from this project. Other projects are not affected.')}>
      <div className="module-list">
        {modules.map((module) => (
          <div key={module.id} className="module-row">
            <div>
              <strong>{module.name}</strong>
              <p className="muted">{module.description}</p>
            </div>
            <label className="switch">
              <input
                type="checkbox"
                checked={!project.disabledModules.includes(module.id)}
                aria-label={module.name}
                onChange={(event) => void change(() => api.put(`/api/projects/${project.key}/modules/${module.id}`, { enabled: event.target.checked }))}
              />
              <span />
            </label>
          </div>
        ))}
        {modules.length === 0 && <p className="muted">{t('No modules are switched on for this instance.')}</p>}
      </div>
    </Card>
  );
}
