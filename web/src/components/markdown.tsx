import { useMemo, useState, type KeyboardEvent } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { t } from '../i18n/index.ts';
import { Link } from '../router.tsx';
import { useSession } from '../session.tsx';

const CODE = /(```[\s\S]*?```|`[^`\n]*`|\[[^\]]*\]\([^)]*\))/g;

/**
 * Turns issue keys of known projects and `[[wiki links]]` into Markdown links, leaving code
 * and existing links alone.
 */
export function linkify(text: string, projectKeys: string[]): string {
  const keys = projectKeys.length > 0 ? new RegExp(`(?<![A-Za-z0-9/\\[-])(${projectKeys.join('|')})-(\\d+)(?![A-Za-z0-9])`, 'g') : null;

  return text
    .split(CODE)
    .map((part, index) => {
      if (index % 2 === 1) {
        return part;
      }

      // `[[Title]]`, `[[SPACE:Title]]` and `[[Title|shown text]]`; only a prefix shaped like a space key names a space.
      let linked = part.replace(/\[\[([^\]|]+?)(?:\|([^\]]+?))?\]\]/g, (_whole, target: string, shown?: string) => {
        const colon = target.indexOf(':');
        const [space, title] = colon > 0 && /^[A-Z][A-Z0-9]{1,9}$/.test(target.slice(0, colon).trim()) ? [target.slice(0, colon), target.slice(colon + 1)] : ['', target];

        return `[${(shown ?? title).trim()}](/wiki-go?title=${encodeURIComponent(title.trim())}${space ? `&space=${encodeURIComponent(space.trim())}` : ''})`;
      });

      if (keys) {
        linked = linked.replace(keys, (key: string) => `[${key}](/issue/${key})`);
      }

      return linked;
    })
    .join('');
}

export function Markdown({ text, className }: { text: string; className?: string }) {
  const { boot } = useSession();
  const source = useMemo(
    () =>
      linkify(
        text,
        boot.projects.map((project) => project.key),
      ),
    [text, boot.projects],
  );

  return (
    <div className={`markdown ${className ?? ''}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) =>
            href?.startsWith('/') ? (
              <Link href={href}>{children}</Link>
            ) : (
              <a href={href} target="_blank" rel="noreferrer noopener">
                {children}
              </a>
            ),
          img: ({ alt }) => <span className="muted">[{alt || t('image')}]</span>,
        }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}

type EditorProps = {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  rows?: number;
  /** Ctrl/⌘+Enter. */
  onSubmit?: () => void;
  autoFocus?: boolean;
};

export function MarkdownEditor({ value, onChange, placeholder, rows = 6, onSubmit, autoFocus }: EditorProps) {
  const [preview, setPreview] = useState(false);

  function onKey(event: KeyboardEvent<HTMLTextAreaElement>): void {
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      onSubmit?.();
    }
  }

  return (
    <div className="editor">
      <div className="editor-tabs">
        <button type="button" className={preview ? 'editor-tab' : 'editor-tab editor-tab-active'} onClick={() => setPreview(false)}>
          {t('Write')}
        </button>
        <button type="button" className={preview ? 'editor-tab editor-tab-active' : 'editor-tab'} onClick={() => setPreview(true)}>
          {t('Preview')}
        </button>
        <span className="editor-hint muted">Markdown</span>
      </div>
      {preview ? (
        <div className="editor-preview">{value.trim() ? <Markdown text={value} /> : <span className="muted">{t('Nothing to preview')}</span>}</div>
      ) : (
        <textarea
          className="input editor-area"
          rows={rows}
          value={value}
          placeholder={placeholder}
          autoFocus={autoFocus}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={onKey}
        />
      )}
    </div>
  );
}
