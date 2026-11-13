import type { AgentConfig, E2EConfig } from 'e2e';
import { web } from '@e2e-dev/web';
import { teamHub } from 'team-hub-e2e-reporter';

import { localChromium } from './support/chromium.ts';

/** The team-hub instance under test: a demo instance (set up with example data). */
const appUrl = process.env.TEAM_HUB_URL ?? 'http://localhost:8192';
/** A Chromium already on the machine; unset, the engine launches the one Playwright installed. */
const chromium = process.env.CHROMIUM_EXECUTABLE;
/** Only the example in tests/agent.example.e2e.ts uses a model; without one it is skipped. */
const model = process.env.E2E_AGENT_MODEL;

async function agents(id: string): Promise<Record<string, AgentConfig>> {
  // `npm install -D ai` first: the package is not needed for the rest of the suite.
  const provider = 'ai';
  const { gateway } = (await import(provider)) as { gateway: (id: string) => NonNullable<AgentConfig['model']> };

  return { default: { model: gateway(id) } };
}

export default {
  projectId: 'team-hub',
  targets: [
    {
      name: 'chromium',
      engine: web({ viewport: { width: 1440, height: 900 }, locale: 'en-GB', ...(chromium ? { connect: localChromium(chromium) } : {}) }),
      app: { url: appUrl },
    },
  ],
  timeout: 60_000,
  // `--reporter json` replaces `list` and `junit` for one run; the team-hub reporter always runs.
  // It posts the finished run to the instance under test when TEAM_HUB_TOKEN is set.
  reporters: ['list', 'junit', teamHub({ url: process.env.TEAM_HUB_REPORT_URL ?? appUrl, project: process.env.TEAM_HUB_PROJECT ?? 'WEB', token: process.env.TEAM_HUB_TOKEN })],
  credentials: {
    // The account the demo instance was set up with; E2E_USER_ADMIN_USERNAME and _PASSWORD replace it.
    admin: { username: 'admin@example.com', password: 'correct horse' },
  },
  ...(model ? { agents: await agents(model) } : {}),
} satisfies E2EConfig;
