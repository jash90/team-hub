// The query the simple filters stand for.

export function quote(text: string): string {
  return /^[\p{L}\p{N}_.@+-]+$/u.test(text) ? text : `"${text.replace(/"/g, '\\"')}"`;
}

export type Chips = { status: string; assignee: string; type: string; priority: string };

export function chipsQuery(scope: string | null, chips: Chips, text: string): string {
  const parts = [
    scope ? `project = ${scope}` : '',
    chips.status ? `category = ${quote(chips.status)}` : '',
    chips.assignee === 'me' ? 'assignee = currentUser()' : chips.assignee === 'none' ? 'assignee is empty' : '',
    chips.type ? `type = ${quote(chips.type)}` : '',
    chips.priority ? `priority = ${chips.priority}` : '',
    text.trim() ? `text ~ ${quote(text.trim()).startsWith('"') ? quote(text.trim()) : `"${text.trim()}"`}` : '',
  ].filter(Boolean);

  return `${parts.join(' AND ')}${parts.length > 0 ? ' ' : ''}ORDER BY updated DESC`;
}
