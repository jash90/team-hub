// An example of agent steps, apart from the rest of the suite: it needs a model, so it is
// skipped unless E2E_AGENT_MODEL names one (see e2e.config.ts and the README).
import { describe, test } from '@e2e-dev/web';
import { expect } from 'e2e';

const model = process.env.E2E_AGENT_MODEL;

describe('Agent example', { session: 'admin', tags: ['agent'], skip: model ? false : 'no model is configured (set E2E_AGENT_MODEL)' }, () => {
  test('an agent comments on an issue', async ({ app, agent, screen }) => {
    const comment = `Checked by the e2e suite at ${new Date().toISOString()}`;

    await app.open('/issue/WEB-13');
    await agent.act('add the comment {comment} to this issue', { params: { comment } });

    await agent.assert('the newest comment on the issue is from Bartek Admin');
    await expect(screen.getByText(comment)).toBeVisible();
  });
});
