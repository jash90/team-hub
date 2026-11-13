import type { ButtonHTMLAttributes } from 'react';

import { Icon, type IconName } from '../icon.tsx';

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'default' | 'primary' | 'ghost' | 'danger';
  size?: 'sm' | 'md';
  icon?: IconName;
};

export function Button({ variant = 'default', size = 'md', icon, className, children, type = 'button', ...rest }: ButtonProps) {
  const classes = ['button', `button-${variant}`, `button-${size}`, !children && icon ? 'button-icon' : '', className ?? ''].filter(Boolean).join(' ');

  return (
    <button type={type} className={classes} {...rest}>
      {icon && <Icon name={icon} size={size === 'sm' ? 14 : 16} />}
      {children}
    </button>
  );
}
