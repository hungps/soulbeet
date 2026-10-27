// The import queue end to end: one import at a time per library with
// waiting rows, repeat clicks that join the running download, an import
// pinned onto the release or recording selected in search, and a MusicBrainz
// outage reported as a failure.
// These tests script the MusicBrainz stub and reset the stubs, so the project
// runs serially after everything else. Each test downloads releases no other
// spec touches: the app's transfer registry outlives a stub reset.

import type { BrowserContext, Locator, Page } from '@playwright/test';
import { expect, test } from '@playwright/test';
import { basename, join } from 'node:path';
import {
  expandTransferGroups,
  freshSession,
  openDownloadsPanel,
  performSearch,
  setAlbumMode,
  uniqueFolder,
  waitForTransferRow,
} from '../helpers/app.js';
import { mbRequests, resetStubs, setMbMode, stubState } from '../helpers/control.js';
import { downloadsDirOnHost, musicDirOnHost } from '../helpers/env.js';
import {
  expectFileAppears,
  importedTrackPath,
  libraryItems,
  listFilesRecursive,
} from '../helpers/files.js';
import { weakTitle } from '../stubs/src/mb-server.js';
import {
  ashLullaby,
  copperSeason,
  ferryLights,
  findPeer,
  quietRelay,
  saltRoads,
  tinMoons,
  tornMap,
  vesperBells,
  type FixtureRelease,
  type PeerShare,
} from '../fixtures/dataset.js';

test.describe.configure({ timeout: 240_000 });

test.afterEach(async () => {
  await resetStubs();
});

/** The release's files on collector_01, the source auto-select picks. */
function sharesOf(release: FixtureRelease): PeerShare[] {
  const mbids = new Set(release.tracks.map((track) => track.mbid));
  return (findPeer('collector_01')?.shares ?? []).filter((share) =>
    mbids.has(share.recordingMbid),
  );
}

/**
 * The Download button of the search result titled exactly `title`: "Cinder
 * Psalm" is also part of the MB-only "Cinder Psalm (Choir Version)", which no
 * peer shares.
 */
function downloadButton(page: Page, title: string) {
  return page
    .locator('li')
    .filter({ has: page.getByText(title, { exact: true }) })
    .getByRole('button', { name: 'Download' });
}

function drawer(page: Page) {
  return page.locator('div.bg-beet-panel').filter({ hasText: 'Active Transfers' });
}

async function expectAllInLibrary(page: Page, release: FixtureRelease, folderName: string) {
  for (const share of sharesOf(release)) {
    const row = await waitForTransferRow(page, share.remotePath, 180_000);
    await expect(row.getByText('LIB', { exact: true })).toBeVisible({ timeout: 180_000 });
  }
  for (const track of release.tracks) {
    await expectFileAppears(
      importedTrackPath(folderName, release.artist, release.title, track.title, 'flac'),
      30_000,
    );
  }
}

/**
 * Downloads the single's track with MusicBrainz weak for beets, which is what
 * makes a search of beets' own end "No close match". The request pins the
 * recording selected in search, so beets looks that recording up and imports
 * it with the pick instead.
 */
async function pinnedRow(
  context: BrowserContext,
  page: Page,
  release: FixtureRelease,
  folderName: string,
) {
  await setMbMode('weak');
  const { folder } = await freshSession(context, page, folderName);

  await performSearch(page, release.title, 'TRACK');
  const asked = (await mbRequests()).length;
  await downloadButton(page, release.tracks[0]!.title).click();
  await openDownloadsPanel(page);

  const [share] = sharesOf(release);
  const row = await waitForTransferRow(page, share!.remotePath);
  await expect(row.getByText('LIB', { exact: true })).toBeVisible({ timeout: 120_000 });
  return { folder, row, asked };
}

/** No Retry import, Import as-is or matches: the row kept nothing to import. */
async function expectNothingKept(row: Locator) {
  await expect(row.getByRole('button', { name: 'Retry import' })).toHaveCount(0);
  await expect(row.getByRole('button', { name: 'Import as-is' })).toHaveCount(0);
  await expect(row.getByRole('button', { name: /^(Show|Hide) matches/ })).toHaveCount(0);
}

/** The paths beets asked MusicBrainz for after the first `from`. */
async function mbPathsSince(from: number): Promise<string[]> {
  return (await mbRequests()).slice(from);
}

test('every row of a singleton batch ends at LIB', async ({ context, page }) => {
  const { folder } = await freshSession(context, page, 'imports-singleton');

  await performSearch(page, copperSeason.title, 'ALBUM');
  await downloadButton(page, copperSeason.title).click();
  await openDownloadsPanel(page);

  await expectAllInLibrary(page, copperSeason, folder.name);
});

test('two albums to one library show WAIT then LIB', async ({ context, page }) => {
  // Every beets lookup takes 3 s, so the imports queue up behind each other.
  await setMbMode('normal', 3000);
  const { folder } = await freshSession(context, page, 'imports-wait');

  await performSearch(page, ferryLights.title, 'ALBUM');
  await downloadButton(page, ferryLights.title).click();
  await performSearch(page, saltRoads.title, 'ALBUM');
  await downloadButton(page, saltRoads.title).click();
  await openDownloadsPanel(page);

  const waiting = drawer(page)
    .locator('div.group')
    .filter({ hasText: 'Waiting for library (1 ahead)' })
    .filter({ has: page.getByText('WAIT', { exact: true }) });
  await expect(async () => {
    await expandTransferGroups(page);
    await expect(waiting.first()).toBeVisible();
  }).toPass({ timeout: 120_000 });

  await expectAllInLibrary(page, ferryLights, folder.name);
  await expectAllInLibrary(page, saltRoads, folder.name);
});

test('rapid re-clicks give one transfer and no tick copies', async ({ context, page }) => {
  const { folder } = await freshSession(context, page, 'imports-reclick');

  await performSearch(page, tinMoons.title, 'ALBUM');
  let requests = 0;
  page.on('request', (request) => {
    if (request.method() === 'POST' && request.url().endsWith('/api/auto-download')) {
      requests++;
    }
  });
  // The icon disables itself once it re-renders, so a user's fast clicks are
  // dispatched in one go: each reaches the handler and sends a request.
  await downloadButton(page, tinMoons.title).evaluate((button: HTMLElement) => {
    button.click();
    button.click();
    button.click();
  });
  await expect.poll(() => requests).toBe(3);
  await openDownloadsPanel(page);
  await expectAllInLibrary(page, tinMoons, folder.name);

  // slskd itself refuses a second enqueue of an unfinished file, so one
  // transfer per file alone would not show the app joined the repeats: it
  // must never have asked slskd twice.
  const { rejections, transfers } = await stubState();
  expect(rejections).toEqual({ alreadyInProgress: 0, throttled: 0 });
  for (const share of sharesOf(tinMoons)) {
    expect(
      transfers.filter((t) => t.filename === share.remotePath),
      share.remotePath,
    ).toHaveLength(1);
  }
  const tickCopies = listFilesRecursive(downloadsDirOnHost).filter((path) =>
    /_\d{18}\./.test(basename(path)),
  );
  expect(tickCopies).toEqual([]);
});

test('a downloaded track imports onto the recording selected in search', async ({
  context,
  page,
}) => {
  const track = vesperBells.tracks[0]!;
  const { folder, row, asked } = await pinnedRow(context, page, vesperBells, 'imports-pinned');
  // Pinned onto the selected recording, so the import neither skipped nor
  // kept anything to retry.
  await expectNothingKept(row);

  const title = weakTitle(track.title);
  await expectFileAppears(
    importedTrackPath(folder.name, vesperBells.artist, vesperBells.title, title, 'flac'),
    30_000,
  );
  expect(libraryItems(folder.name)).toEqual([{ title, mbTrackId: track.mbid }]);
  // beets looked up the picked recording and nothing else: it never searched
  // the weak matches it would have skipped.
  expect(await mbPathsSince(asked)).toEqual([`/ws/2/recording/${track.mbid}`]);
});

test('a folder missing tracks of the release is skipped for a decision', async ({
  context,
  page,
}) => {
  const { folder } = await freshSession(context, page, uniqueFolder('imports-partial'));
  // App-wide, and read when the batch folder is imported: set before the click.
  await setAlbumMode(context, true);
  try {
    const shared = findPeer('partial_hoarder')!.shares;
    expect(shared.length).toBeLessThan(tornMap.tracks.length);

    await performSearch(page, tornMap.title, 'ALBUM');
    await downloadButton(page, tornMap.title).click();
    await openDownloadsPanel(page);

    const row = await waitForTransferRow(page, shared[0]!.remotePath, 180_000);
    // The folder holds two of the release's three tracks, so the album import
    // is not applied unseen: it is skipped with the matches kept, and nothing
    // reaches the library. The card offers the decisions a skipped import does.
    await expect(row.getByText('SKIP', { exact: true })).toBeVisible({ timeout: 180_000 });
    await expect(drawer(page).getByRole('button', { name: 'Retry import' })).toBeVisible();
    await expect(drawer(page).getByRole('button', { name: 'Import as-is' })).toBeVisible();
    await expect(drawer(page).getByRole('button', { name: 'Force import' })).toBeVisible();
    await expect(
      drawer(page).getByRole('button', { name: /^(Show|Hide) matches/ }),
    ).toBeVisible();
    expect(
      listFilesRecursive(join(musicDirOnHost, folder.name)).filter((path) =>
        path.endsWith('.flac'),
      ),
    ).toEqual([]);
  } finally {
    await setAlbumMode(context, false);
  }
});

test('a fully tagged card clears with a close icon, not a trash can', async ({ context, page }) => {
  const { folder } = await freshSession(context, page, uniqueFolder('imports-tagged-x'));

  await performSearch(page, ashLullaby.title, 'ALBUM');
  await downloadButton(page, ashLullaby.title).click();
  await openDownloadsPanel(page);
  await expectAllInLibrary(page, ashLullaby, folder.name);

  // Every file is in the library, so the card's action only drops the record:
  // it is the clear (X) the rows use, and the trash can is gone.
  await expect(drawer(page).getByRole('button', { name: 'Remove request' })).toHaveCount(0);
  const clear = drawer(page)
    .getByRole('button', { name: 'Clear from Active Transfers' })
    .first();
  await expect(clear).toBeVisible({ timeout: 60_000 });
  await clear.click();
  await expect(
    drawer(page).getByRole('button', { name: 'Clear from Active Transfers' }),
  ).toHaveCount(0);
});

test('a lookup outage is reported as a failure, not a skip', async ({ context, page }) => {
  // beets retries a 503 for about 30 s before it gives up on the lookup.
  await setMbMode('unavailable');
  await freshSession(context, page, 'imports-outage');

  await performSearch(page, quietRelay.title, 'TRACK');
  await downloadButton(page, quietRelay.title).click();
  await openDownloadsPanel(page);

  const [share] = sharesOf(quietRelay);
  const row = await waitForTransferRow(page, share!.remotePath);
  await expect(row.getByText('ERR', { exact: true })).toBeVisible({ timeout: 150_000 });
  await expect(row.getByText(/^Lookup failed in MusicBrainz/)).toBeVisible();
  // A kept row names the folder its file sits in and shows the beets output.
  await expect(row.getByText(/^Kept in .*Quiet Relay \[[0-9a-z]{8}\]/)).toBeVisible();
  await row.getByRole('button', { name: 'Show beets output' }).click();
  await expect(row.locator('pre')).not.toBeEmpty();
});
