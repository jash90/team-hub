import { Icon } from '@web/components/icon.tsx';
import { Button } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import { core } from '../api.ts';
import { useRemote } from '../live.ts';
import { useSession } from '../session.tsx';

/** What the middle shows before anything is selected: where to start. */
export function HomeView() {
  const { go, me, status } = useSession();
  const repos = useRemote(() => core.repos(), []);
  const nothingMapped = repos.data?.length === 0;

  return (
    <div className="desk-view desk-home">
      <h1>{me ? t('Hello, {name}', { name: me.name.split(' ')[0] }) : 'team-hub'}</h1>
      <p className="muted">{t('Pick an issue on the right to read it, or a thread on the left to see what its agent did. Nothing an agent proposes happens before you approve it.')}</p>
      <div className="desk-steps">
        <button type="button" className="desk-step" onClick={() => go({ name: 'repos' })}>
          <Icon name="folder" size={18} />
          <strong>{t('Repositories')}</strong>
          <span className="muted">{nothingMapped ? t('Start here: tie a project to its clone on this computer.') : t('{count} mapped', { count: repos.data?.length ?? 0 })}</span>
        </button>
        <button type="button" className="desk-step" onClick={() => go({ name: 'new-thread' })}>
          <Icon name="bot" size={18} />
          <strong>{t('New thread')}</strong>
          <span className="muted">{t('Hand an issue to an agent: implement, review, analyse.')}</span>
        </button>
        <button type="button" className="desk-step" onClick={() => go({ name: 'settings' })}>
          <Icon name="settings" size={18} />
          <strong>{t('Settings')}</strong>
          <span className="muted">{status.runner.enabled ? t('This computer takes runs delegated on the web.') : t('This computer does not take runs delegated on the web.')}</span>
        </button>
      </div>
      {!status.online && <div className="note">{t('The server cannot be reached. You see the copy stored on this computer; threads keep working locally.')}</div>}
      <div>
        <Button variant="ghost" icon="refresh" onClick={() => window.location.reload()}>
          {t('Reload')}
        </Button>
      </div>
    </div>
  );
}
