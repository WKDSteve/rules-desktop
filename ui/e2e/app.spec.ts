import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { expectAccessible, violations, watchViolations } from './a11y';
import { STATE } from './global';
import { changedFiles, changedLines, HOSTILE, ORIGIN, pairingLink } from './hub';

test.use({ locale: 'en-US' });

const state = (): { config: string; vault: string; token: string } =>
  JSON.parse(readFileSync(STATE, 'utf8')) as { config: string; vault: string; token: string };

/** What the page wrote to the console: an error or a warning is a finding. */
const consoleProblems = new WeakMap<Page, string[]>();

async function prepare(page: Page): Promise<void> {
  const problems: string[] = [];
  consoleProblems.set(page, problems);
  page.on('console', (m) => {
    if (m.type() === 'error' || m.type() === 'warning') problems.push(`${m.type()}: ${m.text()}`);
  });
  page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
  await watchViolations(page);
}

/** The tab holds the session, as it does after the link was followed. */
async function pair(page: Page): Promise<void> {
  await prepare(page);
  await page.addInitScript((token) => {
    sessionStorage.setItem('rules-desktop.session', token);
  }, state().token);
  await page.goto('/');
  await expect(page.getByRole('menubar')).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'live' })).toBeVisible();
}

async function view(page: Page, name: 'Overview' | 'Families' | 'Rules' | 'Coverage' | 'Waivers'): Promise<void> {
  await page.getByRole('navigation').getByRole('link', { name, exact: true }).click();
}

async function noProblems(page: Page): Promise<void> {
  // A test that never prepared the page (it only asks the hub) has nothing to report.
  if (!consoleProblems.has(page)) return;
  expect(await violations(page), 'a policy was broken').toEqual([]);
  expect(consoleProblems.get(page) ?? [], 'the console had findings').toEqual([]);
}

test.afterEach(async ({ page }) => {
  await noProblems(page);
});

test('the menu switches the start at login, and no test touches the registry', async ({ page }) => {
  let on = false;
  const posted: unknown[] = [];
  await page.route('**/api/rules/autostart', async (route) => {
    if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON() as { on: boolean };
      posted.push(body);
      on = body.on;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ on }) });
  });
  await pair(page);
  await page.getByRole('menuitem', { name: 'Help', exact: true }).click();
  const entry = page.getByRole('menuitemcheckbox', { name: 'Start the hub at login' });
  await expect(entry).toHaveAttribute('aria-checked', 'false');
  await entry.click();
  await expect.poll(() => posted).toEqual([{ on: true }]);
  await expect(page.getByText('The hub now starts at login.')).toBeVisible();
});

test('the link pairs the tab, leaves no code in the address and sets no cookie', async ({ page, context }) => {
  await prepare(page);
  await page.goto(pairingLink(state().config));
  await expect(page.getByRole('menubar')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible();
  // The icon in the bar is really drawn: the policy of the page lets it through.
  await expect.poll(() => page.locator('.kit-brand img').evaluate((i: HTMLImageElement) => i.naturalWidth)).toBeGreaterThan(0);
  // No code is left in the address; what is there is the place of the page.
  await expect.poll(() => page.url()).toBe(`${ORIGIN}/#view=overview`);
  expect(await page.evaluate(() => sessionStorage.getItem('rules-desktop.session'))).toMatch(/^[0-9a-f]{64}$/);
  expect(await context.cookies()).toEqual([]);
  // Nothing of a rule set stays in the browser: only the session, per tab.
  expect(await page.evaluate(() => Object.keys(localStorage))).toEqual([]);
});

test('a tab without a session is told how to pair and is shown no rule', async ({ page }) => {
  await prepare(page);
  await page.goto('/');
  await expect(page.getByText('This tab is not paired with the hub.')).toBeVisible();
  await expect(page.getByRole('navigation')).toHaveCount(0);
  await expectAccessible(page, 'not paired');
});

test('the data routes need a session and the page carries a strict policy', async ({ request }) => {
  for (const path of ['/api/rules/snapshot', '/api/rules/sources', '/api/rules/search?q=text', '/api/rules/events']) {
    expect((await request.get(path)).status(), path).toBe(401);
  }
  const page = await request.get('/');
  const csp = page.headers()['content-security-policy'] ?? '';
  expect(csp).toContain("default-src 'none'");
  expect(csp).toContain("script-src 'self'");
  expect(csp).not.toContain('unsafe');
  expect(page.headers()['x-content-type-options']).toBe('nosniff');
  // A foreign origin is refused whatever the credential.
  const foreign = await request.get('/api/rules/snapshot', {
    headers: { Origin: 'https://evil.example', Authorization: `Bearer ${state().token}` },
  });
  expect(foreign.status()).toBe(403);
});

test('the three views and their counters', async ({ page }) => {
  await pair(page);
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible();
  // secret has no class on this device: that is a finding, and the page says so.
  await expect(page.getByText('Files without a class on this device (not released): 1')).toBeVisible();
  await view(page, 'Families');
  const table = page.getByRole('table', { name: 'Families' });
  await expect(table.getByRole('row', { name: /^AA / })).toBeVisible();
  await expect(table.getByRole('row', { name: /^BB / })).toBeVisible();
  await view(page, 'Rules');
  await expect(page.locator('tr#rule-AA-01')).toBeVisible();
  // A row of a family opens the list narrowed to it.
  await view(page, 'Families');
  await page.getByRole('button', { name: 'BB', exact: true }).click();
  await expect(page.locator('tr#rule-BB-01')).toBeVisible();
  await expect(page.locator('tr#rule-AA-01')).toHaveCount(0);
});

test('the class of a rule is the one of its file on this device', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  await expect(page.locator('tr#rule-AA-01')).toContainText('public');
  await expect(page.locator('tr#rule-BB-01')).toContainText('internal');
  // `secret` has no entry in sources.json: shown, and marked as not classified.
  await expect(page.locator('tr#rule-SS-01')).toContainText('not classified');
});

test('the keyboard: chord, search, palette and escape', async ({ page }) => {
  await pair(page);
  await page.keyboard.press('g');
  await page.keyboard.press('f');
  await expect(page.getByRole('table', { name: 'Families' })).toBeVisible();
  await page.keyboard.press('g');
  await page.keyboard.press('l');
  await expect(page.locator('tr#rule-AA-01')).toBeVisible();
  await page.keyboard.press('/');
  await expect(page.getByRole('searchbox')).toBeFocused();
  // A letter typed into the box is text, not a shortcut.
  await page.keyboard.type('gl');
  await expect(page.getByRole('searchbox')).toHaveValue('gl');
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('the search reads the texts of the rules', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  await page.getByRole('searchbox').fill('never dropped');
  // The hub reads the texts a moment after it starts: the list narrows when they are in.
  await expect(page.locator('tr#rule-AA-01')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('tr#rule-AA-02')).toHaveCount(0);
  await expect(page.getByText(/1 of \d+/)).toBeVisible();
});

test('the search is emptied in one click on its named button, or with Escape in the field', async ({ page }) => {
  await watchViolations(page);
  await pair(page);
  await view(page, 'Rules');
  const box = page.getByRole('searchbox');
  const clear = page.getByRole('button', { name: 'Clear the search', exact: true });
  // An empty field has no button.
  await expect(clear).toHaveCount(0);
  await box.fill('never dropped');
  await expect(clear).toBeVisible();
  expect(await violations(page)).toEqual([]);
  const top = await page.evaluate(() => window.scrollY);
  await clear.click();
  await expect(box).toHaveValue('');
  await expect(box).toBeFocused();
  await expect(clear).toHaveCount(0);
  expect(await page.evaluate(() => window.scrollY)).toBe(top);

  // Escape: a synthetic key press, which the browser itself does nothing with, so only the page's handler can empty the
  // field (Chrome empties a search field on a real Escape by itself).
  await box.fill('never dropped');
  const prevented = await box.evaluate((el) => {
    const press = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    el.dispatchEvent(press);
    return press.defaultPrevented;
  });
  expect(prevented).toBe(true);
  await expect(box).toHaveValue('');
  // The button is reached with Tab and works with Enter.
  await box.fill('gl');
  await box.press('Tab');
  await expect(clear).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(box).toHaveValue('');
  await expect(box).toBeFocused();
});

test('a rule is shown as text, with its file and its check as data', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  await page.locator('tr#rule-AA-01').getByRole('button').click();
  const detail = page.getByRole('complementary', { name: 'Rule' });
  await expect(detail.getByRole('heading', { name: 'AA-01' })).toBeVisible();
  // The markup in the text is what is read, character for character.
  await expect(detail).toContainText('<b>Not markup</b>');
  expect(await detail.locator('b').count()).toBe(0);
  await expect(detail).toContainText('"type": "grep"');
  await expect(detail).toContainText('alpha/regeln/alpha.md:');
  await page.keyboard.press('Escape');
  await expect(detail).toHaveCount(0);
});

test('a hostile rule set is text, never markup, never a script', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  await expect(page.locator('tr#rule-HX-01')).toContainText('<img src=x onerror=window.__xss=1>');
  await expect(page.locator('tr#rule-HX-02')).toContainText('<svg onload=window.__xss=2>');
  for (const h of HOSTILE) {
    await page.locator(`tr#rule-${h.id}`).getByRole('button').click();
    const detail = page.getByRole('complementary', { name: 'Rule' });
    await expect(detail.getByRole('heading', { name: h.id })).toBeVisible();
    // No element came out of the text.
    expect(await detail.locator('script, img, iframe, svg, details, form, style').count(), h.id).toBe(0);
    expect(await detail.locator('a[href]').count(), h.id).toBe(0);
  }
  expect(await page.evaluate(() => (window as unknown as { __xss?: unknown }).__xss)).toBeUndefined();
  // The hidden characters of a title are made visible, in the list and in the detail.
  await page.locator('tr#rule-HX-06').getByRole('button').click();
  await expect(page.getByRole('complementary', { name: 'Rule' })).toContainText('⟦U+202E⟧');
  await expect(page.getByRole('complementary', { name: 'Rule' })).toContainText('The text holds hidden characters');
  await expectAccessible(page, 'a hostile rule open');
});

test('a rule file that appears on disk appears on the page by itself', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  await expect(page.locator('tr#rule-AA-01')).toBeVisible();
  // The engine does not trust a file younger than two seconds.
  await page.waitForTimeout(2300);
  writeFileSync(
    join(state().vault, 'rulesets', 'alpha', 'regeln', 'live.md'),
    '# Live\n\n#### `LV-01` - Written while the page was open\n\n```rule\nid = "LV-01",\nseverity = "critical",\n```\n\nA live rule.\n',
  );
  await expect(page.locator('tr#rule-LV-01')).toBeVisible({ timeout: 20_000 });
  // The search follows the change, too.
  await page.getByRole('searchbox').fill('live rule');
  await expect(page.locator('tr#rule-LV-01')).toBeVisible({ timeout: 20_000 });
});

test('German, the dark theme and the palette', async ({ page }) => {
  await pair(page);
  await page.getByRole('menuitem', { name: 'Language', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'Deutsch', exact: true }).click();
  await expect(page.getByRole('navigation').getByRole('link', { name: 'Regeln', exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.lang)).toBe('de');
  await page.getByRole('menuitem', { name: 'Farben', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'dunkel', exact: true }).click();
  expect(await page.evaluate(() => document.documentElement.dataset['theme'])).toBe('dark');
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('dialog', { name: 'Befehlspalette' })).toBeVisible();
  await expectAccessible(page, 'palette, dark, German');
});

for (const scheme of ['light', 'dark'] as const) {
  test(`no accessibility violation in ${scheme}: every view and the detail`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await pair(page);
    await expectAccessible(page, `overview ${scheme}`);
    await view(page, 'Families');
    await expectAccessible(page, `families ${scheme}`);
    await view(page, 'Coverage');
    await expect(page.getByRole('table', { name: 'By family' })).toBeVisible();
    await expectAccessible(page, `coverage ${scheme}`);
    await view(page, 'Waivers');
    await expect(page.getByText('No waiver in this project.')).toBeVisible();
    await expectAccessible(page, `waivers ${scheme}`);
    await page.keyboard.press('g');
    await page.keyboard.press('s');
    await expect(page.getByRole('table', { name: 'Classes' })).toBeVisible();
    await expectAccessible(page, `sources ${scheme}`);
    await view(page, 'Rules');
    await expect(page.locator('tr#rule-AA-01')).toBeVisible();
    await expectAccessible(page, `rules ${scheme}`);
    // Hover and focus show the colours of the app's palette (the accent, the focus ring): no violation either.
    await page.getByRole('menubar').getByRole('menuitem').first().hover();
    await expectAccessible(page, `rules, the pointer on the menu bar ${scheme}`);
    await page.locator('tr#rule-AA-01').getByRole('button').focus();
    await expect(page.locator('tr#rule-AA-01').getByRole('button')).toBeFocused();
    await expectAccessible(page, `rules, a rule in focus ${scheme}`);
    await page.locator('tr#rule-AA-01').getByRole('button').click();
    await expect(page.getByRole('complementary', { name: 'Rule' })).toBeVisible();
    await expectAccessible(page, `detail ${scheme}`);
    // The editor of a rule that may be written, and the form to retire it.
    await page.locator('tr#rule-WR-03').getByRole('button').click();
    const detail = page.getByRole('complementary', { name: 'Rule' });
    await detail.getByRole('button', { name: 'Edit', exact: true }).click();
    await expectAccessible(page, `editor ${scheme}`);
    await detail.getByRole('button', { name: 'Retire' }).click();
    await expectAccessible(page, `retire form ${scheme}`);
    await detail.getByRole('button', { name: 'Cancel' }).click();
    await page.getByRole('button', { name: 'New rule …' }).click();
    await expect(page.getByRole('dialog', { name: 'New rule' })).toBeVisible();
    await expectAccessible(page, `new rule ${scheme}`);
    // The kit's float: the keys on its handle move it, and Home puts it back.
    const newRule = page.getByRole('dialog', { name: 'New rule' });
    const before = await newRule.boundingBox();
    await newRule.getByRole('button', { name: 'Move or resize the window' }).focus();
    await page.keyboard.press('ArrowRight');
    await expect.poll(async () => (await newRule.boundingBox())?.x).toBeGreaterThan(before?.x ?? 0);
    await page.keyboard.press('Home');
    await expect.poll(async () => (await newRule.boundingBox())?.x).toBe(before?.x);
    // The first Escape is for the hint the focus opened on the handle, the second one for the dialog.
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog', { name: 'New rule' })).toHaveCount(0);
    // Several rules marked: the bar for the marked.
    await page.getByRole('checkbox', { name: 'Mark WR-01' }).check();
    await expectAccessible(page, `marked ${scheme}`);
    await page.getByRole('button', { name: 'Clear marks' }).click();
    await page.keyboard.press('Control+k');
    await expect(page.getByRole('dialog')).toBeVisible();
    await expectAccessible(page, `palette ${scheme}`);
  });
}

test('Back and Forward walk the views and the open rule, and the address names the place', async ({ page }) => {
  await pair(page);
  const detail = page.getByRole('complementary', { name: 'Rule' });
  await view(page, 'Families');
  await view(page, 'Rules');
  await page.locator('tr#rule-AA-01').getByRole('button').click();
  await expect(detail.getByRole('heading', { name: 'AA-01' })).toBeVisible();
  // The place is in the fragment of the address too: a reload and a link in a new tab open it.
  await expect.poll(() => page.url()).toBe(`${ORIGIN}/#view=rules&rule=AA-01`);
  await page.goBack();
  await expect(detail).toHaveCount(0);
  await expect(page.locator('tr#rule-AA-01')).toBeVisible();
  await page.goBack();
  await expect(page.getByRole('table', { name: 'Families' })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible();
  await page.goForward();
  await page.goForward();
  await page.goForward();
  await expect(detail.getByRole('heading', { name: 'AA-01' })).toBeVisible();
  await expect.poll(() => page.url()).toBe(`${ORIGIN}/#view=rules&rule=AA-01`);
  // A reload opens the same place.
  await page.reload();
  await expect(detail.getByRole('heading', { name: 'AA-01' })).toBeVisible();
});

test('a view is a link: a new tab at its address takes the session of the open tab and shows that view', async ({ page, context }) => {
  await pair(page);
  const link = page.getByRole('navigation').getByRole('link', { name: 'Families', exact: true });
  const href = await link.getAttribute('href');
  expect(href).toBe('/#view=families');
  // The menu's item of the view is the same link.
  await page.getByRole('menuitem', { name: 'View', exact: true }).click();
  expect(await page.getByRole('menuitemradio', { name: 'Families', exact: true }).getAttribute('href')).toBe('/#view=families');
  await page.keyboard.press('Escape');
  // A tab the browser opens itself (its context menu, a middle click): it gets no copy of this tab's session storage.
  const tab = await context.newPage();
  await prepare(tab);
  await tab.goto(href!);
  await expect(tab.getByRole('table', { name: 'Families' })).toBeVisible();
  await expect(tab.getByRole('status').filter({ hasText: 'live' })).toBeVisible();
  expect(await tab.evaluate(() => sessionStorage.getItem('rules-desktop.session'))).toBe(state().token);
  await noProblems(tab);
  await tab.close();
  // A plain click on the link stays in the page.
  await link.click();
  await expect(page.getByRole('table', { name: 'Families' })).toBeVisible();
  expect(context.pages()).toHaveLength(1);
});

test('a new tab takes the session of an open tab that answers late', async ({ page, context }) => {
  await pair(page);
  const tab = await context.newPage();
  await prepare(tab);
  // A busy machine: the open tab answers long after the new tab stopped waiting and offered to pair.
  const busy = page.evaluate(() => {
    const end = performance.now() + 1500;
    while (performance.now() < end);
  });
  await tab.goto('/#view=families');
  await busy;
  await expect(tab.getByRole('table', { name: 'Families' })).toBeVisible();
  await expect(tab.getByRole('status').filter({ hasText: 'live' })).toBeVisible();
  expect(await tab.evaluate(() => sessionStorage.getItem('rules-desktop.session'))).toBe(state().token);
  await noProblems(tab);
  await tab.close();
});

test('a window that gets the focus back asks the hub for a scan, at most every few seconds', async ({ page }) => {
  const rescans: string[] = [];
  page.on('request', (r) => {
    if (r.method() === 'POST' && r.url().endsWith('/api/rules/rescan')) rescans.push(r.url());
  });
  await pair(page);
  // Whatever the load itself asked for is over once the pause of the page has passed.
  await page.waitForTimeout(5200);
  const before = rescans.length;
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect.poll(() => rescans.length).toBe(before + 1);
  // A second focus right after is not asked again.
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await page.waitForTimeout(500);
  expect(rescans.length).toBe(before + 1);
});

test('Back is never trapped by a rule that is gone: the entry of an open rule that vanished is replaced', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  await page.waitForTimeout(2300);
  const file = join(state().vault, 'rulesets', 'alpha', 'regeln', 'gone.md');
  writeFileSync(
    file,
    '# Gone\n\n#### `GN-01` - Written to be removed\n\n```rule\nid = "GN-01",\nseverity = "critical",\n```\n\nA short-lived rule.\n',
  );
  await expect(page.locator('tr#rule-GN-01')).toBeVisible({ timeout: 20_000 });
  await page.locator('tr#rule-GN-01').getByRole('button').click();
  const detail = page.getByRole('complementary', { name: 'Rule' });
  await expect(detail.getByRole('heading', { name: 'GN-01' })).toBeVisible();
  // The file goes while its rule is open: the page closes it by itself, with a word.
  rmSync(file);
  await expect(detail).toHaveCount(0, { timeout: 20_000 });
  await expect(page.locator('tr#rule-GN-01')).toHaveCount(0);
  // Back walks on to the place before, however often the page is asked: it is not held at the rule that is
  // gone (before, each visit of its entry made a new entry after it, and Back never got past it).
  const overview = page.getByRole('heading', { name: 'Overview' });
  for (let i = 0; i < 4 && !(await overview.isVisible()); i++) {
    await page.goBack();
    await page.waitForTimeout(1500);
  }
  await expect(overview).toBeVisible();
  // The hub answers 404 for the rule that is gone (that is what closes it), and Chrome reports a 404 as a finding
  // of the console: that one is expected here, nothing else is.
  const seen = consoleProblems.get(page) ?? [];
  const rest = seen.filter((m) => !m.includes('404'));
  seen.splice(0, seen.length, ...rest);
});

test('a rule is edited, taken back, retired and reinstated in the browser, and only the wanted lines of the files move', async ({
  page,
}) => {
  await pair(page);
  await view(page, 'Rules');
  const detail = page.getByRole('complementary', { name: 'Rule' });
  const toast = page.getByRole('status').filter({ hasText: /Saved\.|Taken back\./ });

  // Edit: the severity and the title of WR-01.
  await page.locator('tr#rule-WR-01').getByRole('button').click();
  await detail.getByRole('button', { name: 'Edit', exact: true }).click();
  await detail.getByLabel('Severity').selectOption('critical');
  await detail.getByLabel('Title (one line)').fill('Edited title');
  await detail.getByRole('button', { name: 'Save' }).click();
  await expect(toast).toContainText('Saved.');
  await expect(detail.getByRole('heading', { name: 'Edited title' })).toBeVisible();
  expect(changedLines(state().vault)).toEqual([
    '-severity: recommended',
    '+severity: critical',
    '-#### `WR-01` — first writable',
    '+#### `WR-01` — Edited title',
  ]);

  // Undo: the file is what it was, byte for byte (nothing differs from the start any more).
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(toast).toContainText('Taken back.');
  await expect.poll(() => changedLines(state().vault)).toEqual([]);
  expect(changedFiles(state().vault)).toEqual([]);

  // Retire WR-02 with a reason: it leaves the list, its file moves to _retired, the overview lists it.
  await page.locator('tr#rule-WR-02').getByRole('button').click();
  await detail.getByRole('button', { name: 'Edit', exact: true }).click();
  await detail.getByRole('button', { name: 'Retire' }).click();
  await detail.getByLabel('Reason (optional)').fill('merged elsewhere');
  await detail.getByRole('button', { name: 'Retire this rule' }).click();
  await expect(page.locator('tr#rule-WR-02')).toHaveCount(0);
  expect(changedFiles(state().vault).some((l) => l.includes('_retired'))).toBe(true);
  // Only WR-02 moved: the other two rules are untouched.
  expect(changedFiles(state().vault).every((l) => l.includes('WR-02'))).toBe(true);

  // Reinstate from the overview: the file is where it was.
  await view(page, 'Overview');
  await page.getByRole('button', { name: 'Reinstate' }).click();
  await view(page, 'Rules');
  await expect(page.locator('tr#rule-WR-02')).toBeVisible();
  await expect.poll(() => changedFiles(state().vault)).toEqual([]);
});

test('a new rule is previewed, made with the next id and taken back, and only its own file appears', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  await page.getByRole('button', { name: 'New rule …' }).click();
  const dialog = page.getByRole('dialog', { name: 'New rule' });
  await dialog.getByLabel('Family (letters, e.g. DEP)').fill('WR');
  // The engine names the next id of the family (WR-01 to WR-03 exist, one of them was retired and came back).
  await expect(dialog.getByText('Next id: WR-04')).toBeVisible();
  await dialog.getByLabel('Title (one line)').fill('A brand new rule');
  await dialog.getByLabel('Severity').selectOption('critical');
  await dialog.getByLabel('Text').fill('What the new rule says.');
  // The first rule set is alpha: the rule is made in gamma, where the page may write one file per rule.
  await dialog.getByLabel('Rule set (name; empty: the first)').fill('gamma');

  // Preview: the engine says what would happen, and nothing is written.
  await dialog.getByRole('button', { name: 'Preview' }).click();
  await expect(dialog.getByText('This would create WR-04.')).toBeVisible();
  expect(changedFiles(state().vault)).toEqual([]);

  // Create: the dialog closes, the new rule is open, and one file appeared.
  await dialog.getByRole('button', { name: 'Create' }).click();
  await expect(dialog).toHaveCount(0);
  const detail = page.getByRole('complementary', { name: 'Rule' });
  await expect(detail.getByRole('heading', { name: 'WR-04' })).toBeVisible();
  await expect(page.locator('tr#rule-WR-04')).toBeVisible();
  const files = () => changedFiles(state().vault);
  expect(files()).toHaveLength(1);
  expect(files()[0]).toContain('_rules/WR/WR-04.md');
  const written = readFileSync(join(state().vault, 'rulesets', 'gamma', '_rules', 'WR', 'WR-04.md'), 'utf8');
  expect(written).toContain('id: WR-04');
  expect(written).toContain('severity: critical');
  expect(written).toContain('What the new rule says.');

  // Undo: the engine's way back retires the new rule (its id stays taken), so it leaves the list.
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(page.locator('tr#rule-WR-04')).toHaveCount(0);
  expect(files().every((l) => l.includes('WR-04'))).toBe(true);
  expect(files().some((l) => l.includes('_retired'))).toBe(true);
});

test('several rules are changed in one request and taken back, and only their severity lines move', async ({ page }) => {
  await pair(page);
  await view(page, 'Rules');
  // A rule in a document cannot be marked: the page does not write it.
  await expect(page.getByRole('checkbox', { name: 'Mark AA-01' })).toBeDisabled();
  await page.getByRole('checkbox', { name: 'Mark WR-01' }).check();
  await page.getByRole('checkbox', { name: 'Mark WR-03' }).check();
  await expect(page.getByText('2 marked')).toBeVisible();
  await page.getByLabel('Set severity of the marked').selectOption('critical');
  await page.getByRole('button', { name: 'Apply' }).click();
  await expect(page.getByRole('status').filter({ hasText: '2 rules saved.' })).toBeVisible();
  expect(changedLines(state().vault).sort()).toEqual([
    '+severity: critical',
    '+severity: critical',
    '-severity: nice-to-have',
    '-severity: recommended',
  ]);
  // The marks are gone, and one Undo takes both back.
  await expect(page.getByText('2 marked')).toHaveCount(0);
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect.poll(() => changedLines(state().vault)).toEqual([]);
});

test('a change that met another version is not written until the person chooses, and then lands on the new version', async ({
  page,
}) => {
  await pair(page);
  await view(page, 'Rules');
  const detail = page.getByRole('complementary', { name: 'Rule' });
  await page.locator('tr#rule-WR-01').getByRole('button').click();
  await detail.getByRole('button', { name: 'Edit', exact: true }).click();
  await detail.getByLabel('Severity').selectOption('nice-to-have');

  // Somebody else changes the file while the editor is open.
  const file = join(state().vault, 'rulesets', 'gamma', '_rules', 'WR', 'WR-01.md');
  writeFileSync(file, readFileSync(file, 'utf8').replace('The text of WR-01.', 'The text of WR-01, changed elsewhere.'));
  await detail.getByRole('button', { name: 'Save' }).click();

  // Nothing is written: the center shows the field and both versions.
  const center = page.getByRole('dialog', { name: 'Changed in the meantime' });
  await expect(center).toBeVisible();
  await expect(center.getByRole('row', { name: /severity/ })).toContainText('nice-to-have');
  await expectAccessible(page, 'conflict center');
  expect(readFileSync(file, 'utf8')).toContain('severity: recommended');
  expect(readFileSync(file, 'utf8')).toContain('changed elsewhere');

  // Escape in the list does not lose the changes; the person decides with a button.
  await page.keyboard.press('Escape');
  await expect(center).toBeVisible();

  // The person writes the change onto the new version: their line changes, the other change stays.
  await center.getByRole('button', { name: 'Write my change onto the new version' }).click();
  await expect(center).toHaveCount(0);
  await expect.poll(() => readFileSync(file, 'utf8')).toContain('severity: nice-to-have');
  expect(readFileSync(file, 'utf8')).toContain('changed elsewhere');
});

test('the coverage shows how the rules apply per family, and a row opens the list of its family', async ({ page }) => {
  await pair(page);
  await page.keyboard.press('g');
  await page.keyboard.press('c');
  await expect(page.getByRole('heading', { name: 'Coverage' })).toBeVisible();
  const table = page.getByRole('table', { name: 'By family' });
  // The sums of the row are the rules of the family: a check, only an agent, or manual.
  const row = table.getByRole('row', { name: /^BB / });
  await expect(row).toBeVisible();
  const cells = (await row.getByRole('cell').allInnerTexts()).map((c) => parseInt(c, 10));
  const [total, check, agent, manual] = cells as [number, number, number, number];
  expect(check + agent + manual).toBe(total);
  expect(total).toBeGreaterThan(0);
  await table.getByRole('button', { name: 'BB', exact: true }).click();
  await expect(page.locator('tr#rule-BB-01')).toBeVisible();
  await expect(page.locator('tr#rule-AA-01')).toHaveCount(0);
});

test('a waiver is made, renewed, taken back and removed in the project of the device, and only its file moves', async ({ page }) => {
  await pair(page);
  const file = join(state().config, '..', 'project', '.rules-waivers.json');
  await page.keyboard.press('g');
  await page.keyboard.press('w');
  await expect(page.getByRole('heading', { name: 'Waivers' })).toBeVisible();
  await expect(page.getByText('No waiver in this project.')).toBeVisible();

  // Waive: the file of the project appears, written whole with the keys sorted.
  await page.getByLabel('Rule', { exact: true }).fill('WR-01');
  await page.getByLabel('Reason', { exact: true }).fill('legacy until the rewrite');
  await page.getByLabel('Last day (YYYY-MM-DD)', { exact: true }).fill('2099-12-31');
  await page.getByRole('button', { name: 'Waive', exact: true }).click();
  const row = page.locator('tr#waiver-WR-01');
  await expect(row).toBeVisible();
  await expect(row).toContainText('2099-12-31');
  await expect(row).toContainText('valid');
  const waived = '{\n  "WR-01": {\n    "expires": "2099-12-31",\n    "reason": "legacy until the rewrite"\n  }\n}\n';
  expect(readFileSync(file, 'utf8')).toBe(waived);

  // Renew without an end: the short form of the file.
  await row.getByRole('button', { name: 'Renew …' }).click();
  await page.getByLabel('No last day').check();
  await page.getByRole('button', { name: 'Renew', exact: true }).click();
  await expect(row).toContainText('no end');
  expect(readFileSync(file, 'utf8')).toBe('{\n  "WR-01": "legacy until the rewrite"\n}\n');

  // Undo brings the last day back, byte for byte.
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(row).toContainText('2099-12-31');
  expect(readFileSync(file, 'utf8')).toBe(waived);

  // Remove: the file keeps an empty object.
  await row.getByRole('button', { name: 'Remove' }).click();
  await expect(page.getByText('No waiver in this project.')).toBeVisible();
  expect(readFileSync(file, 'utf8')).toBe('{}\n');
});

test('a class is set, a change that releases more is asked first, and only sources.json moves', async ({ page }) => {
  await pair(page);
  const file = join(state().config, 'sources.json');
  const keys = (): Record<string, { class: string }> =>
    (JSON.parse(readFileSync(file, 'utf8')) as { sources: Record<string, { class: string }> }).sources;
  await page.keyboard.press('g');
  await page.keyboard.press('s');
  await expect(page.getByRole('table', { name: 'Classes' })).toBeVisible();

  // A key the hub refuses is not written, and what the person typed stays.
  const keyInput = page.getByLabel('Rule set (name as it starts the source, e.g. gamma)', { exact: true });
  await keyInput.fill('bad key');
  await page.getByRole('button', { name: 'Set a class', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'could not be saved' })).toBeVisible();
  await expect(keyInput).toHaveValue('bad key');
  expect(keys()['bad key']).toBeUndefined();

  // Adding a key that holds something back needs no question.
  await keyInput.fill('delta');
  await page.locator('form.add select').selectOption('private');
  await page.getByRole('button', { name: 'Set a class', exact: true }).click();
  await expect(page.getByRole('row', { name: /^delta / })).toBeVisible();
  expect(keys()['delta']).toEqual({ class: 'private' });
  expect(keys()['alpha']).toEqual({ class: 'public' });

  // Releasing more is asked first; "No" writes nothing.
  const select = page.getByLabel('delta: Class', { exact: true });
  await select.selectOption('public');
  const ask = page.getByRole('alertdialog');
  await expect(ask).toBeVisible();
  expect(keys()['delta']).toEqual({ class: 'private' });
  await ask.getByRole('button', { name: 'No', exact: true }).click();
  await expect(ask).toHaveCount(0);
  expect(keys()['delta']).toEqual({ class: 'private' });
  // The select shows what is in force, not the choice that was not written.
  await expect(select).toHaveValue('private');

  // "Yes" sends it again with the word of the person.
  await select.selectOption('public');
  await ask.getByRole('button', { name: 'Yes, release more', exact: true }).click();
  await expect(ask).toHaveCount(0);
  await expect.poll(() => keys()['delta']).toEqual({ class: 'public' });

  // Take the key away again (asked first if that releases more).
  await page.getByRole('row', { name: /^delta / }).getByRole('button', { name: 'Take away' }).click();
  if (await ask.isVisible()) await ask.getByRole('button', { name: 'Yes, release more', exact: true }).click();
  await expect.poll(() => keys()['delta']).toBeUndefined();
  // The hub answers 409 to a change that releases more (that is what makes the page ask) and 400 to a bad key, and Chrome reports them as a
  // finding of the console: those are expected here, nothing else is.
  const seen = consoleProblems.get(page) ?? [];
  seen.splice(0, seen.length, ...seen.filter((m) => !m.includes('409') && !m.includes('400')));
});

test('a rule set is made, moved to the archive and brought back, and nothing is deleted', async ({ page }) => {
  await pair(page);
  const vault = state().vault;
  await page.keyboard.press('g');
  await page.keyboard.press('s');
  await expect(page.getByRole('table', { name: 'Rule sets' })).toBeVisible();

  // Made: an empty folder pair, listed although no rule shows it.
  await page.getByLabel('Name of the new rule set (the folder)', { exact: true }).fill('delta');
  await page.getByRole('button', { name: 'Make a rule set', exact: true }).click();
  const row = page.locator('tr#ruleset-delta');
  await expect(row).toBeVisible();
  expect(existsSync(join(vault, 'rulesets', 'delta', '_rules', '.gitkeep'))).toBe(true);
  expect(existsSync(join(vault, 'rulesets', 'delta', '_layouts', '.gitkeep'))).toBe(true);

  // Archived after a question; "No" moves nothing.
  await row.getByRole('button', { name: 'Archive', exact: true }).click();
  const ask = page.getByRole('alertdialog', { name: 'Archive' });
  await expect(ask).toBeVisible();
  await ask.getByRole('button', { name: 'No', exact: true }).click();
  await expect(ask).toHaveCount(0);
  expect(existsSync(join(vault, 'rulesets', 'delta'))).toBe(true);
  await row.getByRole('button', { name: 'Archive', exact: true }).click();
  await ask.getByRole('button', { name: 'Yes, archive', exact: true }).click();
  await expect(row).toHaveCount(0);
  await expect(page.locator('li#archived-delta')).toBeVisible();
  expect(existsSync(join(vault, 'rulesets', 'delta'))).toBe(false);
  expect(existsSync(join(vault, 'archive', 'rulesets', 'delta', '_rules', '.gitkeep'))).toBe(true);

  // Undo brings it back; then it goes to the archive again and is restored from the list.
  await page.getByRole('button', { name: 'Undo' }).click();
  await expect(row).toBeVisible();
  expect(existsSync(join(vault, 'rulesets', 'delta'))).toBe(true);
  await row.getByRole('button', { name: 'Archive', exact: true }).click();
  await ask.getByRole('button', { name: 'Yes, archive', exact: true }).click();
  await page.locator('li#archived-delta').getByRole('button', { name: 'Restore' }).click();
  await expect(row).toBeVisible();
  expect(existsSync(join(vault, 'archive', 'rulesets', 'delta'))).toBe(false);

  // A change from outside (the editor, git): a rule set with a rule moves away, the snapshot changes, and the list
  // follows without the view being opened again.
  const rules = join(vault, 'rulesets', 'delta', '_rules', 'DX');
  mkdirSync(rules, { recursive: true });
  writeFileSync(
    join(rules, 'DX-01.md'),
    ['---', 'id: DX-01', 'severity: recommended', 'family: DX', '---', '', '#### `DX-01` — outside', '', '```rule', 'id = "DX-01",', '```', '', 'The text.', '', '---', ''].join(String.fromCharCode(10)),
  );
  await expect(page.locator('tr#ruleset-delta')).toContainText('1');
  mkdirSync(join(vault, 'archive', 'rulesets'), { recursive: true });
  renameSync(join(vault, 'rulesets', 'delta'), join(vault, 'archive', 'rulesets', 'delta'));
  await expect(row).toHaveCount(0);
  await expect(page.locator('li#archived-delta')).toBeVisible();
});

test('help: the documentation opens from the menu and is searched, the name leads to the overview, a click beside a dialog closes it', async ({
  page,
}) => {
  await pair(page);
  await view(page, 'Rules');

  // The Help menu has the section Documentation with one entry per document, and the project links.
  await page.getByRole('menuitem', { name: 'Help', exact: true }).click();
  await expect(page.getByRole('menuitem', { name: 'GitHub repository' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'License: MIT' })).toBeVisible();
  await page.getByRole('menuitem', { name: 'Releasing', exact: true }).click();
  const docs = page.getByRole('dialog', { name: 'Documentation' });
  await expect(docs).toBeVisible();
  await expect(docs.locator('article.doc')).toContainText(/release/i);
  await expectAccessible(page, 'the documentation window');
  const names = docs.locator('li button .name');
  expect(await names.count()).toBeGreaterThan(1);
  await docs.getByRole('searchbox').fill('loopback');
  await expect(names).toHaveCount(1);
  await docs.getByRole('searchbox').fill('zzzzzz-nothing');
  await expect(docs.getByText('Nothing found')).toBeVisible();
  // A click beside it closes it.
  await page.mouse.click(4, 4);
  await expect(docs).toHaveCount(0);

  // The name at the left of the bar goes back to the overview.
  await page.getByRole('button', { name: 'Rules', exact: true }).first().click();
  await expect(page.getByRole('navigation').getByRole('link', { name: 'Overview', exact: true })).toHaveAttribute('aria-current', 'page');

  // The dialog of a new rule: empty, a click beside it closes it; with a title typed in, it stays.
  await view(page, 'Rules');
  await page.getByRole('button', { name: 'New rule …' }).click();
  const dialog = page.getByRole('dialog', { name: 'New rule' });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel('Title').fill('A draft that stays');
  await page.mouse.click(4, 4);
  await expect(dialog).toBeVisible();
  await dialog.getByLabel('Title').fill('');
  await page.mouse.click(4, 4);
  await expect(dialog).toHaveCount(0);
  expect(consoleProblems.get(page)).toEqual([]);
});
