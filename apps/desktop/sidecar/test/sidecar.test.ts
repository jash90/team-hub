// What can be checked of the helper without a model: its side of the protocol, the options it
// hands to the Agent SDK, and that it ends when asked or when its parent is gone.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createInterface } from 'node:readline';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import type { ToSidecar } from '../../ui/src/bindings/ToSidecar.ts';
import { hubToolNames, PROPOSE_TOOLS, shapeOf } from '../src/hub-tools.ts';
import { optionsFor } from '../src/main.ts';

type Start = Extract<ToSidecar, { type: 'start' }>;

const MAIN = fileURLToPath(new URL('../src/main.ts', import.meta.url));

const start = (over: Partial<Start> = {}): Start => ({
  type: 'start',
  thread: 7,
  prompt: 'Issue WEB-1: Login',
  cwd: '/data/worktrees/web/web-1-t7',
  model: 'claude-opus-5-5',
  resume: null,
  max_turns: 40,
  rules: 'Work only inside the current directory.',
  allowed_tools: ['Read', 'Glob', 'Grep', 'Edit(//data/worktrees/web/web-1-t7/**)', 'Write(//data/worktrees/web/web-1-t7/**)', 'Bash'],
  denied_tools: ['Agent', 'Task', 'Bash(git push:*)'],
  sandbox: { writable: ['/repos/web/.git/objects'], protected: ['/repos/web/.git/hooks'], unreadable: ['/home/me/.ssh'] },
  hub_tools: [{ name: 'get_issue', description: 'One issue', input_schema: { type: 'object', properties: { key: { type: 'string' } }, required: ['key'] } }],
  env: { EXTRA: '1' },
  ...over,
});

test('a thread gets only what its allow list names, and nothing is left to ask about', () => {
  const options = optionsFor(start());

  assert.equal(options.permissionMode, 'dontAsk');
  assert.deepEqual(options.tools, ['Read', 'Glob', 'Grep', 'Edit', 'Write', 'Bash']);
  assert.ok(options.allowedTools?.includes('Edit(//data/worktrees/web/web-1-t7/**)'));
  assert.ok(!options.allowedTools?.includes('Edit'));
  assert.ok(options.allowedTools?.includes('mcp__team-hub__get_issue'));
  assert.ok(options.allowedTools?.includes('mcp__team-hub__propose_push'));
  assert.deepEqual(options.disallowedTools, ['Agent', 'Task', 'Bash(git push:*)']);
  assert.deepEqual(options.settingSources, []);
  assert.equal(options.strictMcpConfig, true);
  assert.deepEqual(Object.keys(options.mcpServers ?? {}), ['team-hub']);
  assert.equal(options.resume, undefined);
  assert.equal(optionsFor(start({ resume: 'session-1' })).resume, 'session-1');
});

test('the callback for anything else refuses', async () => {
  const decision = await optionsFor(start()).canUseTool?.('WebFetch', {}, { signal: new AbortController().signal, toolUseID: 't' } as never);

  assert.equal(decision?.behavior, 'deny');
});

test('the shell is confined: no network, writes only where the core says', () => {
  const sandbox = optionsFor(start()).sandbox;

  assert.equal(sandbox?.enabled, true);
  assert.equal(sandbox?.failIfUnavailable, true);
  assert.equal(sandbox?.allowUnsandboxedCommands, false);
  assert.deepEqual(sandbox?.network, { allowedDomains: [], strictAllowlist: true, allowLocalBinding: false });
  assert.deepEqual(sandbox?.filesystem, { allowWrite: ['/repos/web/.git/objects'], denyWrite: ['/repos/web/.git/hooks'], denyRead: ['/home/me/.ssh'] });
  assert.equal(optionsFor(start({ sandbox: null })).sandbox, undefined);
});

test('without a shell on the allow list the shell is not even offered', () => {
  const options = optionsFor(start({ allowed_tools: ['Read', 'Glob', 'Grep'], sandbox: null }));

  assert.deepEqual(options.tools, ['Read', 'Glob', 'Grep']);
});

test('tool names are those the agent sees, reads first', () => {
  const names = hubToolNames(start().hub_tools);

  assert.equal(names[0], 'mcp__team-hub__get_issue');
  assert.equal(names.length, 1 + PROPOSE_TOOLS.length);
  assert.ok(names.includes('mcp__team-hub__ask_question'));
});

test("a server tool's JSON Schema becomes a shape, and an unreadable one is accepted as it comes", () => {
  const shape = shapeOf({ type: 'object', properties: { query: { type: 'string' }, limit: { type: 'integer' } }, required: ['query'] });

  assert.deepEqual(Object.keys(shape).sort(), ['limit', 'query']);
  assert.deepEqual(Object.keys(shapeOf({ type: 'object', properties: { key: { type: 'made-up' } } })), ['key']);
  assert.deepEqual(shapeOf(null), {});
});

const launch = () => {
  const child = spawn(process.execPath, [MAIN], { stdio: ['pipe', 'pipe', 'inherit'], detached: true });
  const lines = createInterface({ input: child.stdout });

  return { child, firstLine: once(lines, 'line').then(([line]) => JSON.parse(line as string) as unknown) };
};

test('the helper says it is ready and ends when its input closes', async () => {
  const { child, firstLine } = launch();

  assert.deepEqual(await firstLine, { type: 'ready', protocol: 1 });

  child.stdin.end();

  const [code, signal] = await once(child, 'exit');

  assert.ok(code === 0 || signal === 'SIGKILL', `ended with ${code} ${signal}`);
});

test('the helper ignores what is not a message and ends on stop', async () => {
  const { child, firstLine } = launch();

  await firstLine;
  child.stdin.write('not json\n{"type":"reply","request":99,"ok":true,"result":null}\n{"type":"stop"}\n');

  const [code, signal] = await once(child, 'exit');

  assert.ok(code === 0 || signal === 'SIGKILL', `ended with ${code} ${signal}`);
});
