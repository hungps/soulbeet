// The dashboard tab lives in the URL, so a reload has to land on the same tab.
// The router rewrites any URL its route does not spell out itself, which used
// to drop ?tab= and, on a cold load, bounce the whole page back to /.

import { expect, test } from '@playwright/test';
import { copyFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { freshSession } from '../helpers/app.js';
import { e2eRoot, musicDirOnHost } from '../helpers/env.js';
import { glassAtlas } from '../fixtures/dataset.js';

test('puts the track number right next to the play button', async ({ context, page }) => {
  const { folder } = await freshSession(context, page, 'library-number');

  // The scan indexes a folder from its files and tags, so a fixture copied in
  // is enough to get a track row without waiting on a download.
  const albumDir = join(musicDirOnHost, folder.name, glassAtlas.artist, glassAtlas.title);
  mkdirSync(albumDir, { recursive: true });
  copyFileSync(
    join(e2eRoot, '.fixtures', 'audio', `${glassAtlas.tracks[0]!.mbid}.flac`),
    join(albumDir, '01 Northern Line.flac'),
  );

  await page.getByRole('link', { name: 'Dashboard' }).click();
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('button', { name: 'Scan all' }).click();

  const release = page.getByText(glassAtlas.title).first();
  await expect(release).toBeVisible({ timeout: 60_000 });
  await release.click();

  const row = page
    .locator('div.flex.items-center.gap-3')
    .filter({ hasText: 'Northern Line' })
    .last();
  const number = row.locator('span').filter({ hasText: /^01$/ }).first();
  await expect(number).toHaveClass(/\bw-4\b/);
  await expect(number).toHaveClass(/\btext-center\b/);
});

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
