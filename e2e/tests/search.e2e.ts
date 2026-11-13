import { describe, test } from '@e2e-dev/web';
import { expect } from 'e2e';

describe('Search', { session: 'admin' }, () => {
  test('WEB-11 a query finds the finished bugs of the shop', async ({ app, screen, browser }) => {
    await app.open('/search');
    await screen.getByRole('button', 'Write a query').tap();
    await screen.getByRole('textbox', 'Query').fill('project = WEB AND type = Bug AND category = done ORDER BY key');
    await screen.getByRole('button', 'Search').tap();

    await expect(screen.getByText('2 issues')).toBeVisible();
    await expect(screen.getByRole('link', 'WEB-6')).toBeVisible();
    await expect(screen.getByRole('link', 'WEB-11')).toBeVisible();
    await expect(screen.getByRole('link', 'WEB-9')).toHaveCount(0);
    await expect(browser).toHaveURL(/\/search\?q=/);
  });

  test('a query that cannot be understood says where it went wrong', async ({ app, screen }) => {
    await app.open('/search');
    await screen.getByRole('button', 'Write a query').tap();
    await screen.getByRole('textbox', 'Query').fill('colour = blue');
    await screen.getByRole('button', 'Search').tap();

    await expect(screen.getByText('Unknown field "colour"', { exact: false })).toBeVisible();
  });
});
