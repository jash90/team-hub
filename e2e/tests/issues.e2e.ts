import { describe, test } from '@e2e-dev/web';
import { expect } from 'e2e';

describe('Issues', { session: 'admin' }, () => {
  test('a new issue is created from the header and can be deleted again', async ({ app, screen }) => {
    const summary = `e2e: issue created by the suite ${Date.now()}`;

    await app.open('/p/WEB/backlog');
    await screen.getByRole('banner').getByRole('button', 'Create').tap();

    const form = screen.getByRole('dialog', 'Create issue');
    await form.getByRole('textbox', 'Summary').fill(summary);
    await form.getByRole('button', 'Create').tap();

    // The new issue opens beside the backlog.
    const issue = screen.getByRole('dialog', /^WEB-\d+$/);
    await expect(screen.getByRole('status').filter({ hasText: /WEB-\d+ created/ })).toBeVisible();
    await expect(issue.getByRole('button', summary)).toBeVisible();
    await expect(issue.getByRole('button', 'Status')).toHaveText('To Do', { ignoreCase: true });

    // Leave the demo instance as it was.
    await issue.getByRole('button', 'More').tap();
    await screen.getByRole('menuitem', 'Delete issue').tap();
    await screen.getByRole('dialog', /^Delete WEB-\d+\?$/).getByRole('button', 'Delete issue').tap();

    await expect(issue).toBeHidden();
    await expect(screen.getByText(summary)).toHaveCount(0);
  });

  test('WEB-16 an issue moves to In Progress and back to To Do', async ({ app, screen }) => {
    await app.open('/issue/WEB-16');
    const status = screen.getByRole('button', 'Status');

    await status.tap();
    await screen.getByRole('option', 'In Progress').tap();
    await expect(status).toHaveText('In Progress', { ignoreCase: true });

    // The move is kept by the server, not only shown.
    await app.open('/issue/WEB-16');
    await expect(status).toHaveText('In Progress', { ignoreCase: true });

    await status.tap();
    await screen.getByRole('option', 'To Do').tap();
    await expect(status).toHaveText('To Do', { ignoreCase: true });
  });
});
