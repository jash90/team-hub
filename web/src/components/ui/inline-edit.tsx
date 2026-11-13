import { useEffect, useState } from 'react';

/** Text that turns into an input when clicked; Enter or leaving saves, Escape cancels. */
export function InlineEdit({ value, onSave, className, placeholder, multiline }: { value: string; onSave: (value: string) => void; className?: string; placeholder?: string; multiline?: boolean }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);

  useEffect(() => setDraft(value), [value]);

  function finish(save: boolean): void {
    setEditing(false);

    if (save && draft.trim() && draft.trim() !== value) {
      onSave(draft.trim());
    } else {
      setDraft(value);
    }
  }

  if (!editing) {
    return (
      <button type="button" className={`inline-edit ${className ?? ''}`} onClick={() => setEditing(true)}>
        {value || <span className="muted">{placeholder}</span>}
      </button>
    );
  }

  const shared = {
    autoFocus: true,
    className: `input inline-edit-input ${className ?? ''}`,
    value: draft,
    onBlur: () => finish(true),
  };

  return multiline ? (
    <textarea
      {...shared}
      rows={2}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' && !event.shiftKey) {
          event.preventDefault();
          finish(true);
        } else if (event.key === 'Escape') {
          finish(false);
        }
      }}
    />
  ) : (
    <input
      {...shared}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          finish(true);
        } else if (event.key === 'Escape') {
          finish(false);
        }
      }}
    />
  );
}
