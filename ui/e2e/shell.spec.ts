import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { expect, test } from '@playwright/test';
import { collectShell, diffShell, paletteRecolor, SHELL_PROBES, type ShellSignature } from 'appkit-ui/shell-check';
import golden from 'appkit-ui/shell.golden.json' with { type: 'json' };
import { STATE } from './global';

// The shell of this page is the one of every app of appkit: the styles of the page, the menu bar, an open menu and the
// palette are those of the reference kept with the kit (`shell.golden.json`). A page that overrides a token or a rule of
// the kit shows here as the property it changed. The colours are those of this app's palette (`src/palette.css`, the
// colours of its icon): a colour of a token in the reference is met by the colour the palette gives that token.
const tokensCss = readFileSync(createRequire(import.meta.url).resolve('appkit-ui/tokens.css'), 'utf8');
const paletteCss = readFileSync(new URL('../src/palette.css', import.meta.url), 'utf8');
const recolor = paletteRecolor(tokensCss, paletteCss);

test.use({ locale: 'en-US' });

for (const scheme of ['light', 'dark'] as const) {
  test(`the shell has the styles of the shell of appkit, in ${scheme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    const token = (JSON.parse(readFileSync(STATE, 'utf8')) as { token: string }).token;
    await page.addInitScript((t) => sessionStorage.setItem('rules-desktop.session', t), token);
    await page.goto('/');
    await expect(page.getByRole('menubar')).toBeVisible();
    const seen: ShellSignature = {};
    // What is there in each state; what was read first is not read again (an open menu changes its own trigger).
    const read = async (): Promise<void> => {
      const now = (await page.evaluate(collectShell, SHELL_PROBES)) as ShellSignature;
      for (const [name, style] of Object.entries(now)) if (!(name in seen)) seen[name] = style;
    };
    await read();
    await page.getByRole('menubar').getByRole('menuitem').first().click();
    await expect(page.getByRole('menu')).toBeVisible();
    await read();
    await page.keyboard.press('Escape');
    await page.keyboard.press('Control+k');
    await expect(page.getByRole('combobox')).toBeVisible();
    await read();
    expect(diffShell(scheme, seen, golden[scheme] as ShellSignature, recolor[scheme])).toEqual([]);
  });
}
