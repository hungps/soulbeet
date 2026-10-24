// Navidrome integration, only when the stack runs one (compose does, the
// native dev stack does not). Media-server URLs are configured per user in
// Settings > Media Server, so the app can no longer be pointed at Navidrome
// through the environment.

import { expect, test } from '@playwright/test';
import { loginViaApi, openSettings, registerUser, uniqueUser } from '../helpers/app.js';
import {
  navidromeAdminPassword,
  navidromeAdminUser,
  navidromeEnabled,
  navidromeUrlForApp,
} from '../helpers/env.js';

test.skip(!navidromeEnabled, 'stack has no Navidrome (set E2E_NAVIDROME=1)');

test('connects to Navidrome from the media server settings', async ({ page, context }) => {
  const user = uniqueUser('navidrome');
  await registerUser(context.request, user);
  await loginViaApi(context, user);
  await page.goto('/');

  await openSettings(page);
  await page.getByRole('button', { name: 'Media Server' }).click();

  await page.getByPlaceholder('http://localhost:4533').fill(navidromeUrlForApp);
  await page.getByPlaceholder('Navidrome username').fill(navidromeAdminUser);
  await page.getByPlaceholder('Navidrome password').fill(navidromeAdminPassword);
  await page.getByRole('button', { name: 'Connect' }).click();

  await expect(page.getByText('Connected', { exact: true })).toBeVisible({ timeout: 30_000 });
});

test('accepts local accounts without a media server', async ({ page, context }) => {
  // A locally registered user has no saved Navidrome URL, so login falls back
  // to the local Argon2 password.
  const user = uniqueUser('local-only');
  await registerUser(context.request, user);

  await page.goto('/login');
  await page.getByPlaceholder('Enter username').fill(user.username);
  await page.getByPlaceholder('Enter password').fill(user.password);
  await page.getByRole('button', { name: 'AUTHENTICATE' }).click();

  await expect(page.getByPlaceholder('Search artist, album or track...')).toBeVisible();
});
