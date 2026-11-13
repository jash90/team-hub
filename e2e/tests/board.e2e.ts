import { describe, test } from '@e2e-dev/web';
import { expect } from 'e2e';

describe('Board', { session: 'admin' }, () => {
  test('the board shows the active sprint with its columns', async ({ app, screen }) => {
    await app.open('/p/WEB/board');

    await expect(screen.getByRole('heading', /^WEB Sprint \d+$/, { level: 1 })).toBeVisible();
    await expect(screen.getByRole('button', 'Complete sprint')).toBeVisible();

    for (const column of ['To Do', 'In Progress', 'In Review', 'Done']) {
      await expect(screen.getByText(column).first()).toBeVisible();
    }
  });

  test('WEB-9 the card of an issue opens the issue beside the board', async ({ app, screen, browser }) => {
    await app.open('/p/WEB/board');
    await screen.getByRole('button', /Płatność kartą przez bramkę .*WEB-9\b/).tap();

    const issue = screen.getByRole('dialog', 'WEB-9');
    await expect(issue).toBeVisible();
    await expect(issue.getByRole('button', 'Płatność kartą przez bramkę')).toBeVisible();
    await expect(issue.getByRole('button', 'Status')).toHaveText('In Review', { ignoreCase: true });
    await expect(browser).toHaveURL(/\/p\/WEB\/board\?issue=WEB-9$/);

    await issue.getByRole('button', 'Close').tap();
    await expect(issue).toBeHidden();
    await expect(browser).toHaveURL('/p/WEB/board');
  });
});
