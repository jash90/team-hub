import { useState } from 'react';

import { Icon } from '@web/components/icon.tsx';
import { Tag } from '@web/components/ui.tsx';
import { t } from '@web/i18n/index.ts';
import type { CommitInfo } from '../bindings/CommitInfo.ts';
import type { PatchFile } from '../bindings/PatchFile.ts';

const STATUS_TONE: Record<string, 'green' | 'red' | 'blue' | 'amber' | 'neutral'> = { added: 'green', untracked: 'amber', deleted: 'red', renamed: 'blue', modified: 'neutral' };
const STATUS_TEXT: Record<string, string> = { added: 'added|file', untracked: 'not committed', deleted: 'deleted|file', renamed: 'renamed', modified: 'changed|file' };

function statusText(status: string): string {
  const text = STATUS_TEXT[status] ?? status;
  const translated = t(text);

  return translated === text ? text.split('|')[0] : translated;
}

function FileDiff({ file, open, onToggle }: { file: PatchFile; open: boolean; onToggle: () => void }) {
  return (
    <div className="desk-file">
      <button type="button" className="desk-file-head" onClick={onToggle} aria-expanded={open}>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={14} />
        <span className="mono desk-file-path" title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}>
          {file.path}
        </span>
        <Tag tone={STATUS_TONE[file.status] ?? 'neutral'}>{statusText(file.status)}</Tag>
        <span className="desk-grow" />
        {file.binary ? (
          <span className="muted">{t('binary')}</span>
        ) : (
          <span className="mono">
            <span className="desk-plus">+{file.additions}</span> <span className="desk-minus">−{file.deletions}</span>
          </span>
        )}
      </button>
      {open && !file.binary && (
        <div className="desk-patch">
          {file.hunks.length === 0 && <div className="muted desk-patch-empty">{t('No text changes.')}</div>}
          {file.hunks.map((hunk) => (
            <table key={hunk.header} className="desk-hunk">
              <tbody>
                <tr className="desk-hunk-head">
                  <td colSpan={3}>{hunk.header}</td>
                </tr>
                {hunk.lines.map((line, index) => (
                  <tr key={index} className={`desk-line desk-line-${line.kind}`}>
                    <td className="desk-line-no">{line.oldLine ?? ''}</td>
                    <td className="desk-line-no">{line.newLine ?? ''}</td>
                    <td className="desk-line-text">
                      {line.kind === 'add' ? '+' : line.kind === 'del' ? '−' : ' '}
                      {line.text}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ))}
        </div>
      )}
    </div>
  );
}

/** The files of a diff, each one folding open to its unified diff. */
export function DiffFiles({ files }: { files: PatchFile[] }) {
  const [open, setOpen] = useState<Set<string>>(() => new Set(files.length <= 3 ? files.map((file) => file.path) : []));

  if (files.length === 0) {
    return <div className="panel-empty muted">{t('No changes.')}</div>;
  }

  return (
    <div className="desk-files">
      {files.map((file) => (
        <FileDiff
          key={file.path}
          file={file}
          open={open.has(file.path)}
          onToggle={() => {
            const next = new Set(open);

            if (!next.delete(file.path)) {
              next.add(file.path);
            }

            setOpen(next);
          }}
        />
      ))}
    </div>
  );
}

export function CommitList({ commits }: { commits: CommitInfo[] }) {
  if (commits.length === 0) {
    return <div className="panel-empty muted">{t('No commits yet.')}</div>;
  }

  return (
    <div>
      {commits.map((commit) => (
        <div key={commit.sha} className="list-row desk-commit">
          <Icon name="commit" size={14} />
          <code title={commit.sha}>{commit.short}</code>
          <span className="desk-commit-subject">{commit.subject}</span>
          <span className="muted">{commit.author}</span>
        </div>
      ))}
    </div>
  );
}
