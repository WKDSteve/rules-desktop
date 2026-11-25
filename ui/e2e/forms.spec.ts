// A form that is not complete can be sent: it says what is missing at the field and puts the focus there, and the button
// is not locked. A dialog that holds typed text asks before Escape drops it, and the page asks before the tab is left
// while there is a change that is not saved. Checkboxes are big enough to hit.

import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { expectAccessible } from './a11y';
import { STATE } from './global';

test.use({ locale: 'en-US' });

const state = (): { token: string } => JSON.parse(readFileSync(STATE, 'utf8')) as { token: string };

async function pair(page: Page): Promise<void> {
  await page.addInitScript((token) => sessionStorage.setItem('rules-desktop.session', token), state().token);
  await page.goto('/');
  await expect(page.getByRole('menubar')).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'live' })).toBeVisible();
}

const nav = (page: Page, name: string) => page.getByRole('navigation').getByRole('link', { name, exact: true }).click();

/** Whether the page asks before it is left: what a `beforeunload` listener does with the event. */
const asksBeforeLeaving = (page: Page): Promise<boolean> =>
  page.evaluate(() => {
    const event = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(event);
    return event.defaultPrevented;
  });

test('New rule sent empty says what is missing at the fields, with the focus on the first', async ({ page }) => {
  await pair(page);
  await nav(page, 'Rules');
  await page.getByRole('button', { name: 'New rule …' }).click();
  const dialog = page.getByRole('dialog', { name: 'New rule' });
  const family = dialog.getByLabel('Family (letters, e.g. DEP)');
  const title = dialog.getByLabel('Title (one line)');
  const create = dialog.getByRole('button', { name: 'Create', exact: true });
  await expect(create).toBeEnabled();

  await create.click();
  await expect(dialog).toBeVisible();
  await expect(family).toHaveAttribute('aria-invalid', 'true');
  await expect(family).toBeFocused();
  await expect(dialog.locator('#new-family-error')).toHaveText('Fill in this field.');
  await expect(family).toHaveAttribute('aria-describedby', /new-family-error/);
  await expect(title).toHaveAttribute('aria-invalid', 'true');
  await expect(dialog.locator('#new-title-error')).toHaveText('The title is one line and not empty.');
  await expect(create).toBeEnabled();
  await expectAccessible(page, 'new rule with errors');

  // Typing a field takes its error away, the other stays; the preview says the same as the send.
  await family.fill('WR');
  await expect(family).not.toHaveAttribute('aria-invalid', 'true');
  await expect(dialog.locator('#new-family-error')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Preview' }).click();
  await expect(title).toBeFocused();
});

test('Waivers sent empty say what is missing at the fields, with the focus on the first', async ({ page }) => {
  await pair(page);
  await nav(page, 'Waivers');
  const waive = page.getByRole('button', { name: 'Waive', exact: true });
  await expect(waive).toBeEnabled();
  await waive.click();
  const rule = page.getByLabel('Rule', { exact: true });
  await expect(rule).toHaveAttribute('aria-invalid', 'true');
  await expect(rule).toBeFocused();
  await expect(page.locator('#waiver-id-error')).toHaveText('Fill in this field.');
  await expect(page.getByLabel('Reason')).toHaveAttribute('aria-describedby', /waiver-reason-error/);
  await expect(waive).toBeEnabled();
});

test('Sources: a rule set and a class sent empty say what is missing at the field', async ({ page }) => {
  await pair(page);
  await nav(page, 'Sources');
  const make = page.getByRole('button', { name: 'Make a rule set' });
  await make.click();
  const name = page.getByLabel('Name of the new rule set (the folder)');
  await expect(name).toHaveAttribute('aria-invalid', 'true');
  await expect(name).toBeFocused();
  await expect(page.locator('#ruleset-name-error')).toHaveText('Fill in this field.');
  await expect(make).toBeEnabled();
  await name.fill('x');
  await expect(page.locator('#ruleset-name-error')).toHaveCount(0);
  await page.getByRole('button', { name: 'Set a class' }).click();
  await expect(page.getByLabel('Rule set (name as it starts the source, e.g. gamma)')).toBeFocused();
});

test('Escape in New rule asks while something is typed, keeps it on "Keep editing", closes on "Discard"', async ({ page }) => {
  await pair(page);
  await nav(page, 'Rules');
  const open = page.getByRole('button', { name: 'New rule …' });
  await open.click();
  const dialog = page.getByRole('dialog', { name: 'New rule' });
  await expect(dialog).toHaveAttribute('closedby', 'none');
  expect(await asksBeforeLeaving(page)).toBe(false);

  // Nothing typed: Escape closes at once (the first Escape may belong to the hint of the focused field).
  await dialog.getByRole('heading', { name: 'New rule' }).click();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);

  await open.click();
  const title = dialog.getByLabel('Title (one line)');
  await title.fill('Something I would not like to lose');
  expect(await asksBeforeLeaving(page)).toBe(true);
  await page.keyboard.press('Escape');
  const ask = dialog.getByRole('alert').filter({ hasText: 'Discard what you entered?' });
  await expect(dialog).toBeVisible();
  await expect(ask).toBeVisible();
  await ask.getByRole('button', { name: 'Keep editing' }).click();
  await expect(ask).toBeHidden();
  await expect(title).toHaveValue('Something I would not like to lose');

  await page.keyboard.press('Escape');
  await dialog.getByRole('button', { name: 'Discard' }).click();
  await expect(dialog).toHaveCount(0);
  expect(await asksBeforeLeaving(page)).toBe(false);
});

test('a waiver that is typed and not written makes the tab ask before it is left, an empty form does not', async ({ page }) => {
  await pair(page);
  await nav(page, 'Waivers');
  const reason = page.getByLabel('Reason');
  await expect(reason).toBeVisible();
  expect(await asksBeforeLeaving(page)).toBe(false);
  await reason.fill('Known and accepted');
  expect(await asksBeforeLeaving(page)).toBe(true);
  await reason.fill('');
  expect(await asksBeforeLeaving(page)).toBe(false);
});

test('the editor of a rule makes the tab ask before it is left once something changed', async ({ page }) => {
  await pair(page);
  await nav(page, 'Rules');
  await page.locator('tr#rule-WR-01').getByRole('button').click();
  const detail = page.getByRole('complementary', { name: 'Rule' });
  await detail.getByRole('button', { name: 'Edit', exact: true }).click();
  expect(await asksBeforeLeaving(page)).toBe(false);
  await detail.getByLabel('Title (one line)').fill('A title that is not saved yet');
  expect(await asksBeforeLeaving(page)).toBe(true);
  await detail.getByRole('button', { name: 'Cancel' }).click();
  expect(await asksBeforeLeaving(page)).toBe(false);
});

test('a checkbox is at least 24 px wide and high', async ({ page }) => {
  await pair(page);
  await nav(page, 'Rules');
  const box = await page.getByRole('checkbox', { name: 'Mark WR-01' }).boundingBox();
  expect(box?.width ?? 0).toBeGreaterThanOrEqual(24);
  expect(box?.height ?? 0).toBeGreaterThanOrEqual(24);
  await nav(page, 'Waivers');
});

test.describe('on a touch screen', () => {
  test.use({ hasTouch: true, isMobile: true });
  test('a checkbox is at least 44 px wide and high', async ({ page }) => {
    await pair(page);
    await nav(page, 'Rules');
    expect(await page.evaluate(() => matchMedia('(pointer: coarse)').matches)).toBe(true);
    const box = await page.getByRole('checkbox', { name: 'Mark WR-01' }).boundingBox();
    expect(box?.width ?? 0).toBeGreaterThanOrEqual(44);
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
  });
});
