import { test } from '@e2e-dev/web';
import { credentials, expect } from 'e2e';

// Signs in once; the other tests start from this session.
test.setup('sign in as the administrator', { sessions: ['admin'] }, async ({ app, screen, session }) => {
  const admin = credentials.user('admin');

  await app.open('/');
  await screen.getByLabel('E-mail').fill(admin.username);
  await screen.getByLabel('Password').fill(admin.password);
  await screen.getByRole('button', 'Sign in').tap();

  await expect(screen.getByRole('link', 'My work')).toBeVisible();
  await session.save('admin');
});
