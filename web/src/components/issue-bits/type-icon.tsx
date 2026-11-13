import type { ReactNode } from 'react';

import type { IssueType } from '../../bindings/IssueType.ts';

const TYPE_GLYPHS: Record<string, ReactNode> = {
  epic: <path d="M13 3 5 13h6l-1 8 8-10h-6z" fill="currentColor" stroke="none" />,
  story: <path d="M7 4h10v16l-5-4-5 4z" fill="currentColor" stroke="none" />,
  task: <path d="m6.5 12 4 4 7-8" />,
  bug: <circle cx="12" cy="12" r="5" fill="currentColor" stroke="none" />,
  subtask: (
    <>
      <rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" stroke="none" />
      <rect x="12" y="12" width="8" height="8" rx="1.5" />
    </>
  ),
};

export function TypeIcon({ type, size = 16 }: { type: IssueType | undefined; size?: number }) {
  const icon = type && TYPE_GLYPHS[type.icon] ? type.icon : 'task';

  return (
    <span className={`type-icon type-${icon}`} style={{ width: size, height: size }} title={type?.name}>
      <svg viewBox="0 0 24 24" width={size - 4} height={size - 4} fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
        {TYPE_GLYPHS[icon]}
      </svg>
    </span>
  );
}
