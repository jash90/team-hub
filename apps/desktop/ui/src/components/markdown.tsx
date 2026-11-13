import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { openExternal } from '../bridge.ts';

/**
 * Markdown from the tracker or from an agent. Raw HTML in it is shown as text, never run;
 * links open in the browser, not in the app's window; images are not fetched.
 */
export function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={className ? `markdown ${className}` : 'markdown'}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={(event) => {
                event.preventDefault();

                if (href) {
                  void openExternal(href);
                }
              }}
            >
              {children}
            </a>
          ),
          img: ({ alt }) => <span className="muted">[{alt || 'image'}]</span>,
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
