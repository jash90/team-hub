import { describe, test } from '@e2e-dev/web';
import { expect } from 'e2e';

describe('Knowledge base', { session: 'admin' }, () => {
  test('a page of the shop space opens from the page tree', async ({ app, screen, browser }) => {
    await app.open('/');
    await expect(screen.getByRole('link', 'My work')).toBeVisible();

    // The knowledge base is a module; an instance built or configured without it has no such page.
    const enabled = await browser.evaluate(async () => {
      const boot = (await (await fetch('/api/bootstrap')).json()) as { modules: { id: string; enabled: boolean }[] };

      return boot.modules.some((module) => module.id === 'wiki' && module.enabled);
    });
    test.skip(!enabled, 'the knowledge base module is not part of this instance');

    await app.open('/wiki');
    await expect(screen.getByRole('heading', 'Knowledge base')).toBeVisible();
    await screen.getByRole('link', /Sklep — baza wiedzy/).first().tap();
    await expect(browser).toHaveURL('/wiki/WEB');

    await screen.getByRole('navigation', 'Pages').getByRole('link', 'Architektura sklepu').tap();
    await expect(screen.getByRole('heading', 'Architektura sklepu', { level: 1 })).toBeVisible();
  });
});
