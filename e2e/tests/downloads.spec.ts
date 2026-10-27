// The core pipeline, end to end: UI search -> auto source selection ->
// slskd stub delivers real FLACs -> the monitor picks them up -> real beets
// autotags against the MusicBrainz stub -> files land in the library tree.

import { expect, test } from '@playwright/test';
import {
  freshSession,
  openDownloadsPanel,
  performSearch,
  queueDot,
  uniqueFolder,
  waitForTransferRow,
} from '../helpers/app.js';
import { expectFileAppears, importedTrackPath, listFilesRecursive } from '../helpers/files.js';
import { glassAtlas, senders, thimbleHymn } from '../fixtures/dataset.js';
import { basename } from 'node:path';
import { downloadsDirOnHost } from '../helpers/env.js';

test.describe.configure({ timeout: 180_000 });

test('downloads and imports a single track', async ({ context, page }) => {
  const { folder } = await freshSession(context, page, 'happy-track');

  await performSearch(page, 'Northern Line', 'TRACK');

  const row = page.locator('li').filter({ hasText: 'Northern Line' }).first();
  await row.getByRole('button', { name: 'Download' }).click();

  await openDownloadsPanel(page);
  const item = await waitForTransferRow(
    page,
    'Music\\FLAC\\Static Harbor\\Glass Atlas\\01 - Northern Line.flac',
  );

  // Badge walks the pipeline and ends at LIB (imported into the library).
  await expect(item.getByText('LIB', { exact: true })).toBeVisible({ timeout: 120_000 });

  const imported = importedTrackPath(
    folder.name,
    glassAtlas.artist,
    glassAtlas.title,
    'Northern Line',
    'flac',
  );
  await expectFileAppears(imported, 30_000);

  // beets moved (not copied) the file out of its batch folder, and slskd never
  // had to write a renamed {name}_{ticks} copy next to an existing one.
  const leftovers = listFilesRecursive(downloadsDirOnHost).filter((path) => {
    const name = basename(path);
    return name === '01 - Northern Line.flac' || /_\d{18}\./.test(name);
  });
  expect(leftovers).toEqual([]);
});

test('lists a clicked download in the queue before a source is picked', async ({
  context,
  page,
}) => {
  await freshSession(context, page, uniqueFolder('happy-queued'));

  await performSearch(page, thimbleHymn.title, 'ALBUM');
  await page
    .locator('li')
    .filter({ hasText: thimbleHymn.title })
    .first()
    .getByRole('button', { name: 'Download' })
    .click();

  // The request is queued the moment it is clicked, so the navbar says a
  // download is running and the drawer lists it without waiting for a
  // transfer: the user can leave the search page straight away.
  await expect(queueDot(page)).toBeVisible({ timeout: 20_000 });
  await openDownloadsPanel(page);
  const drawer = page.locator('div.bg-beet-panel').filter({ hasText: 'Active Transfers' });
  await expect(drawer.getByText(thimbleHymn.title).first()).toBeVisible();
});

test('downloads and imports a full album from the album card', async ({ context, page }) => {
  const { folder } = await freshSession(context, page, 'happy-album');

  await performSearch(page, senders.title, 'ALBUM');

  const card = page.locator('li').filter({ hasText: senders.title }).first();
  await card.getByRole('button', { name: 'Download' }).click();

  for (const track of senders.tracks) {
    const imported = importedTrackPath(
      folder.name,
      senders.artist,
      senders.title,
      track.title,
      'flac',
    );
    await expectFileAppears(imported, 150_000);
  }
});
