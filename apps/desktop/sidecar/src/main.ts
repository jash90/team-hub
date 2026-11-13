/**
 * Runs one turn of one agent thread through the Claude Agent SDK.
 *
 * The desktop core starts this process, writes one `start` line to its standard input and
 * reads events from its standard output, one JSON object per line (the types are generated
 * from the core: `ToSidecar` and `FromSidecar`). `stop`, or the input closing, ends the turn
 * and the process together with everything it started.
 *
 * The process holds no token of team-hub and makes no request to it: the tracker's tools are
 * relayed to the core as `request` lines and answered with `reply` lines.
 */
import { createInterface } from 'node:readline';

import { type Options, query } from '@anthropic-ai/claude-agent-sdk';

import type { AgentEvent } from '../../ui/src/bindings/AgentEvent.ts';
import type { FromSidecar } from '../../ui/src/bindings/FromSidecar.ts';
import type { ToSidecar } from '../../ui/src/bindings/ToSidecar.ts';
import { confinement, withoutMentions } from './confine.ts';
import { HUB_SERVER, hubServer, hubToolNames } from './hub-tools.ts';

type Start = Extract<ToSidecar, { type: 'start' }>;
type Reply = Extract<ToSidecar, { type: 'reply' }>;

const PROTOCOL = 1;
const DETAIL_LENGTH = 400;
const GRACE_MS = 1500;

const send = (message: FromSidecar) => process.stdout.write(`${JSON.stringify(message)}\n`);
const emit = (event: AgentEvent) => send({ type: 'event', event });

const replies = new Map<number, (reply: Reply) => void>();
const abortController = new AbortController();
let requests = 0;

const callCore = async (tool: string, input: unknown) => {
  requests += 1;

  const request = requests;
  const reply = await new Promise<Reply>((resolve) => {
    replies.set(request, resolve);
    send({ type: 'request', request, tool, input: input as never });
  });

  replies.delete(request);

  if (!reply.ok) {
    throw new Error(typeof reply.result === 'string' ? reply.result : JSON.stringify(reply.result));
  }

  return reply.result;
};

/**
 * The built-in tools named on the allow list: `Edit(//path/**)` names `Edit`. A `Read` rule
 * covers the tools that search files too, so they are offered wherever reading is.
 */
const builtInTools = (allowed: string[]) => {
  const named = allowed.map((rule) => rule.split('(')[0]).filter((name) => !name.startsWith('mcp__'));

  return [...new Set(named.flatMap((name) => (name === 'Read' ? ['Read', 'Glob', 'Grep'] : [name])))];
};

export const optionsFor = (start: Start): Options => ({
  cwd: start.cwd,
  model: start.model,
  abortController,
  maxTurns: start.max_turns,
  ...(start.resume ? { resume: start.resume } : {}),
  env: { ...process.env, ...start.env, CLAUDE_AGENT_SDK_CLIENT_APP: 'team-hub-desktop/0.1.0' },
  // Nobody is there to approve anything: what is not on the allow list is refused.
  permissionMode: 'dontAsk',
  tools: builtInTools(start.allowed_tools),
  allowedTools: [...start.allowed_tools, ...hubToolNames(start.hub_tools, start.commit_tool)],
  disallowedTools: start.denied_tools,
  hooks: confinement(start.cwd, start.allowed_tools),
  canUseTool: async (toolName) => ({
    behavior: 'deny',
    message: `${toolName} is not on this thread's allow list. Nobody can approve it here; do the task without it, or say what you would need.`,
  }),
  ...(start.sandbox
    ? {
        sandbox: {
          enabled: true,
          failIfUnavailable: true,
          allowUnsandboxedCommands: false,
          autoAllowBashIfSandboxed: true,
          network: { allowedDomains: [], strictAllowlist: true, allowLocalBinding: false },
          filesystem: { allowWrite: start.sandbox.writable, denyWrite: start.sandbox.protected, denyRead: start.sandbox.unreadable },
        },
      }
    : {}),
  systemPrompt: { type: 'preset', preset: 'claude_code', append: start.rules },
  // The person's own Claude Code settings bring their hooks, plugins and MCP servers, and a
  // repository's settings can bring hooks of its own; a thread gets none of them.
  settingSources: [],
  strictMcpConfig: true,
  mcpServers: { [HUB_SERVER]: hubServer(start.hub_tools, callCore, start.commit_tool) },
});

const run = async (start: Start) => {
  try {
    for await (const message of query({ prompt: withoutMentions(start.prompt), options: optionsFor(start) })) {
      if (message.type === 'system' && message.subtype === 'init') {
        emit({ kind: 'session', session_id: message.session_id });
      }

      if (message.type === 'assistant') {
        for (const block of message.message.content) {
          if (block.type === 'text' && block.text.trim()) {
            emit({ kind: 'assistant', text: block.text });
          }

          if (block.type === 'tool_use') {
            emit({ kind: 'tool', name: block.name, detail: JSON.stringify(block.input).slice(0, DETAIL_LENGTH) });
          }
        }
      }

      if (message.type === 'result') {
        const models = Object.values(message.modelUsage ?? {});
        const sum = (pick: (usage: (typeof models)[number]) => number) => models.reduce((total, usage) => total + pick(usage), 0);

        emit({
          kind: 'result',
          ok: message.subtype === 'success' && !message.is_error,
          summary: message.subtype === 'success' ? message.result : `${message.subtype}: ${message.errors.join('; ')}`,
          // Running totals of the session, as the SDK reports them.
          cost_usd: message.total_cost_usd,
          turns: message.num_turns,
          input_tokens: sum((usage) => usage.inputTokens + usage.cacheReadInputTokens + usage.cacheCreationInputTokens),
          output_tokens: sum((usage) => usage.outputTokens),
        });
      }
    }
  } catch (reason) {
    if (!abortController.signal.aborted) {
      emit({ kind: 'error', message: reason instanceof Error ? reason.message : String(reason) });
    }
  }
};

/** Ends this process and what it started: it leads its own process group. */
const end = (code: number) => {
  abortController.abort();

  setTimeout(() => {
    try {
      if (process.platform !== 'win32') {
        process.kill(-process.pid, 'SIGKILL');
      }
    } catch {
      // Not a group leader (started by hand): only this process ends.
    }

    process.exit(code);
  }, GRACE_MS).unref();
};

const main = async () => {
  let turn: Promise<void> | undefined;

  send({ type: 'ready', protocol: PROTOCOL });

  for await (const line of createInterface({ input: process.stdin })) {
    let message: ToSidecar;

    try {
      message = JSON.parse(line) as ToSidecar;
    } catch {
      continue;
    }

    if (message.type === 'start' && !turn) {
      turn = run(message).then(() => process.exit(0));
    }

    if (message.type === 'stop') {
      end(0);
    }

    if (message.type === 'reply') {
      replies.get(message.request)?.(message);
    }
  }

  // The input closed: whoever started this process is gone, and no agent works on unseen.
  end(turn ? 1 : 0);
};

if (import.meta.url === `file://${process.argv[1]}`) {
  void main();
}
