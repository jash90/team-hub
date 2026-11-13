import type { User } from '../../bindings/User.ts';
import { t } from '../../i18n/index.ts';
import { Icon } from '../icon.tsx';

function initials(name: string): string {
  const parts = name.trim().split(/\s+/);

  return ((parts[0]?.[0] ?? '') + (parts.length > 1 ? (parts[parts.length - 1][0] ?? '') : '')).toUpperCase();
}

export function Avatar({ user, size = 24, label }: { user: User | undefined; size?: number; label?: string }) {
  const style = { width: size, height: size, fontSize: Math.round(size * 0.42), background: user?.color ?? 'var(--line-strong)' };

  return (
    <span className={user ? 'avatar' : 'avatar avatar-none'} style={style} title={label ?? user?.name ?? t('Unassigned')}>
      {user ? initials(user.name) : <Icon name="user" size={Math.round(size * 0.6)} />}
    </span>
  );
}
