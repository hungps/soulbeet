// Global app configuration, done once before every other project: points the
// app at the slskd stub and an alternative MusicBrainz server through the real
// settings UI. Both are app-wide (app_config table), not per-user.

import { expect, test } from '@playwright/test';
import { loginViaUi, openSettings } from '../helpers/app.js';
import {
  downloadsDirForApp,
  metadataServerHost,
  metadataServerName,
  slskdApiKey,
  slskdUrlForApp,
} from '../helpers/env.js';

test('configure slskd connection', async ({ page }) => {
  // The migration seeds a local admin/admin user.
  await loginViaUi(page, { username: 'admin', password: 'admin' });

  await openSettings(page);
  await page.getByRole('button', { name: 'Config' }).click();

  await page.getByPlaceholder('http://localhost:5030').fill(slskdUrlForApp);
  await page.getByPlaceholder('Enter slskd API key').fill(slskdApiKey);
  await page.getByLabel('Soulbeet downloads path').fill(downloadsDirForApp);

  await page.getByRole('button', { name: '+ Add server' }).click();
  await page.getByPlaceholder('Name (e.g. BrainzMash)').last().fill(metadataServerName);
  await page
    .getByPlaceholder('musicbrainz.example.com:8080')
    .last()
    .fill(metadataServerHost);

  await page.getByRole('button', { name: 'Save Configuration' }).click();

  await expect(page.getByText('Configuration saved')).toBeVisible();
});
