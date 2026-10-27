// The dashboard tab lives in the URL, so a reload has to land on the same tab.
// The router rewrites any URL its route does not spell out itself, which used
// to drop ?tab= and, on a cold load, bounce the whole page back to /.

import { expect, test } from '@playwright/test';
import { freshSession } from '../helpers/app.js';

test('keeps the tab it was opened on across a reload', async ({ context, page }) => {
  await freshSession(context, page, 'dashboard-tab');

  await page.getByRole('link', { name: 'Dashboard' }).click();
  await expect(page).toHaveURL(/\/dashboard\?tab=overview$/);

  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await expect(page).toHaveURL(/\/dashboard\?tab=library$/);
  await expect(page.getByRole('button', { name: 'Scan all' })).toBeVisible();

  await page.reload();
  await expect(page).toHaveURL(/\/dashboard\?tab=library$/);
  await expect(page.getByRole('button', { name: 'Scan all' })).toBeVisible();

  // A link to a tab, as a bookmark or a reload of one, opens on that tab.
  await page.goto('/dashboard?tab=history');
  await expect(page).toHaveURL(/\/dashboard\?tab=history$/);
  await expect(page.getByText('No deletion history yet.')).toBeVisible();
});
