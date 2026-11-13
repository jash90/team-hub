import { describe, test } from '@e2e-dev/web';
import { credentials, expect } from 'e2e';

describe('Sign-in', () => {
  test('a visitor signs in and lands on their work', async ({ app, screen, browser }) => {
    const admin = credentials.user('admin');

    await app.open('/p/WEB/board');
    await expect(screen.getByRole('heading', 'Sign in')).toBeVisible();

    await screen.getByLabel('E-mail').fill(admin.username);
    await screen.getByLabel('Password').fill(admin.password);
    await screen.getByRole('button', 'Sign in').tap();

    // Signing in keeps the address the visitor asked for.
    await expect(browser).toHaveURL('/p/WEB/board');
    await expect(screen.getByRole('navigation').getByRole('link', 'Board')).toBeVisible();
  });

  test('a wrong password is refused with a message', async ({ app, screen }) => {
    await app.open('/');
    await screen.getByLabel('E-mail').fill(credentials.user('admin').username);
    await screen.getByLabel('Password').fill('not the password');
    await screen.getByRole('button', 'Sign in').tap();

    await expect(screen.getByRole('status')).toContainText('Wrong e-mail or password');
    await expect(screen.getByRole('heading', 'Sign in')).toBeVisible();
  });
});
