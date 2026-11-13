// Names of the app's own things, in one place. English is written here; Polish comes from the
// dictionaries (`i18n.ts`), the same way as in the web interface.
import { t } from '@web/i18n/index.ts';
import type { IconName } from '@web/components/icon.tsx';
import type { EventKind } from './bindings/EventKind.ts';
import type { ProposalKind } from './bindings/ProposalKind.ts';
import type { ProposalState } from './bindings/ProposalState.ts';
import type { Thread } from './bindings/Thread.ts';
import type { ThreadKind } from './bindings/ThreadKind.ts';
import type { ThreadState } from './bindings/ThreadState.ts';

type Tone = 'neutral' | 'blue' | 'green' | 'amber' | 'red' | 'purple';

const KINDS: Record<ThreadKind, { label: string; hint: string; icon: IconName }> = {
  implement: { label: 'Implement', hint: 'Makes the change on a branch of its own and commits it locally.', icon: 'branch' },
  review: { label: 'Review', hint: 'Reads the change on the branch against the issue. Edits nothing.', icon: 'eye' },
  analyse: { label: 'Analyse', hint: 'Reads the code and the issue and reports. Changes nothing.', icon: 'search' },
  custom: { label: 'Custom', hint: 'Does what the instructions say.', icon: 'edit' },
};

const STATES: Record<ThreadState, { label: string; tone: Tone }> = {
  queued: { label: 'waiting for a slot', tone: 'neutral' },
  running: { label: 'working', tone: 'blue' },
  waiting: { label: 'needs you', tone: 'amber' },
  done: { label: 'done|thread', tone: 'green' },
  failed: { label: 'failed|thread', tone: 'red' },
  stopped: { label: 'stopped', tone: 'neutral' },
};

const PROPOSALS: Record<ProposalKind, { label: string; icon: IconName }> = {
  comment: { label: 'Post a comment', icon: 'message' },
  transition: { label: 'Change the status', icon: 'flag' },
  create_subtasks: { label: 'Create subtasks', icon: 'layers' },
  push: { label: 'Push the branch', icon: 'upload' },
  pull_request: { label: 'Open a pull request', icon: 'pullRequest' },
};

const PROPOSAL_STATES: Record<ProposalState, { label: string; tone: Tone }> = {
  pending: { label: 'to decide', tone: 'amber' },
  approved: { label: 'approved|proposal', tone: 'green' },
  rejected: { label: 'rejected|proposal', tone: 'neutral' },
  failed: { label: 'failed|proposal', tone: 'red' },
};

export const EVENT_ICONS: Record<EventKind, IconName> = { message: 'bot', tool: 'settings', status: 'info', question: 'alert', answer: 'user', error: 'alert' };

/** A phrase may carry a context after `|` so that two meanings can be translated apart. */
function phrase(text: string): string {
  const translated = t(text);

  return translated === text ? text.split('|')[0] : translated;
}

export const kindLabel = (kind: ThreadKind) => t(KINDS[kind].label);
export const kindHint = (kind: ThreadKind) => t(KINDS[kind].hint);
export const kindIcon = (kind: ThreadKind) => KINDS[kind].icon;
export const THREAD_KINDS = Object.keys(KINDS) as ThreadKind[];
export const stateLabel = (state: ThreadState) => phrase(STATES[state].label);
export const stateTone = (state: ThreadState) => STATES[state].tone;
export const proposalLabel = (kind: ProposalKind) => t(PROPOSALS[kind].label);
export const proposalIcon = (kind: ProposalKind) => PROPOSALS[kind].icon;
export const proposalStateLabel = (state: ProposalState) => phrase(PROPOSAL_STATES[state].label);
export const proposalStateTone = (state: ProposalState) => PROPOSAL_STATES[state].tone;

/** What a thread needs from the person right now, if anything. */
export function needOf(thread: Thread): string | null {
  if (thread.pendingQuestion) {
    return t('asks a question');
  }

  if (thread.pendingProposals > 0) {
    return t('{count} to decide', { count: thread.pendingProposals });
  }

  return null;
}
