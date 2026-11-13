import { useState, type ReactNode } from 'react';

import { Icon, type IconName } from '../icon.tsx';
import { Button } from './button.tsx';
import { Popover } from './popover.tsx';

export type MenuItem = { label: string; icon?: IconName; onSelect: () => void; danger?: boolean; disabled?: boolean };

export function Menu({ items, label, icon = 'dots', children, align = 'end' }: { items: MenuItem[]; label: string; icon?: IconName; children?: ReactNode; align?: 'start' | 'end' }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);

  return (
    <>
      <Button variant="ghost" icon={children ? undefined : icon} aria-label={label} title={label} onClick={(event) => setAnchor(anchor ? null : event.currentTarget)}>
        {children}
      </Button>
      {anchor && (
        <Popover anchor={anchor} onClose={() => setAnchor(null)} align={align}>
          <div className="menu" role="menu">
            {items.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                disabled={item.disabled}
                className={item.danger ? 'menu-item menu-danger' : 'menu-item'}
                onClick={() => {
                  setAnchor(null);
                  item.onSelect();
                }}
              >
                {item.icon && <Icon name={item.icon} />}
                {item.label}
              </button>
            ))}
          </div>
        </Popover>
      )}
    </>
  );
}
