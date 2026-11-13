import { useState } from 'react';

import { api } from '../../api.ts';
import type { Project } from '../../bindings/Project.ts';
import type { ProjectRole } from '../../bindings/ProjectRole.ts';
import { Avatar, Button, Picker, Tag, toast } from '../../components/ui.tsx';
import { t } from '../../i18n/index.ts';
import { useSession } from '../../session.tsx';
import { Card, useChange } from './parts.tsx';

const ROLES: { value: ProjectRole; label: string }[] = [
  { value: 'admin', label: 'Administrator' },
  { value: 'member', label: 'Member' },
  { value: 'viewer', label: 'Viewer' },
];

export function Members({ project }: { project: Project }) {
  const { boot, userById } = useSession();
  const change = useChange();
  const [adding, setAdding] = useState<number | null>(null);
  const base = `/api/projects/${project.key}/members`;
  const outsiders = boot.users.filter((user) => user.active && !project.members.some((member) => member.userId === user.id));

  return (
    <Card
      title={t('Members')}
      lead={project.isPrivate ? t('Only these people can see this project.') : t('Everyone signed in can work in this project. Listing someone here changes what they may do: viewers only read, administrators configure.')}
    >
      <table className="table table-plain">
        <tbody>
          {project.members.map((member) => {
            const person = userById(member.userId);

            return (
              <tr key={member.userId}>
                <td>
                  <span className="with-avatar">
                    <Avatar user={person} size={24} />
                    {person?.name ?? '?'}
                    {project.leadId === member.userId && <Tag tone="purple">{t('lead')}</Tag>}
                  </span>
                </td>
                <td>
                  <Picker value={member.role} options={ROLES.map((role) => ({ value: role.value, label: t(role.label) }))} onChange={(role) => void change(() => api.put(`${base}/${member.userId}`, { role }))} />
                </td>
                <td className="cell-actions">
                  <Button size="sm" variant="ghost" icon="x" aria-label={t('Remove')} onClick={() => void change(() => api.put(`${base}/${member.userId}`, { role: null }))} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {outsiders.length > 0 && (
        <div className="inline-form">
          <Picker value={adding} placeholder={t('Add a person…')} options={outsiders.map((user) => ({ value: user.id as number | null, label: user.name, keywords: user.email }))} onChange={setAdding} />
          <Button
            icon="plus"
            disabled={adding === null}
            onClick={() =>
              void change(() => api.put(`${base}/${adding}`, { role: 'member' })).then(() => {
                setAdding(null);
                toast(t('Added'));
              })
            }
          >
            {t('Add')}
          </Button>
        </div>
      )}
    </Card>
  );
}
