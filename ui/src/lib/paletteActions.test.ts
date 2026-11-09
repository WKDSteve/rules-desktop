import { describe, expect, it } from 'vitest';
import { App } from './app.svelte';
import { paletteActions } from './paletteActions';
import { FakeHub, testEnv } from './testhub';

function app(): App {
  return new App(testEnv(new FakeHub()).env);
}

describe('the actions of the palette', () => {
  it('offer the other views, the other language and themes, the key switch and reading again', () => {
    const a = app();
    expect(paletteActions(a).map((x) => x.key)).toEqual([
      'go.families',
      'go.rules',
      'go.views',
      'go.coverage',
      'go.waivers',
      'go.sources',
      'rescan',
      'rule.new',
      'shortcuts.off',
      'lang.de',
      'theme.light',
      'theme.dark',
    ]);
  });

  it('follow the state of the page', () => {
    const a = app();
    a.setView('rules');
    a.setLang('de');
    a.setTheme('dark');
    a.setShortcuts(false);
    a.setFilter({ families: ['DEP'], severities: [], checkKinds: [], query: '' });
    const keys = paletteActions(a).map((x) => x.key);
    expect(keys).toEqual(['go.overview', 'go.families', 'go.views', 'go.coverage', 'go.waivers', 'go.sources', 'rescan', 'rule.new', 'filter.reset', 'shortcuts.on', 'lang.en', 'theme.auto', 'theme.light']);
    expect(paletteActions(a).find((x) => x.key === 'go.overview')?.label).toBe('Gehe zu: Übersicht');
  });

  it('do what they say', async () => {
    const a = app();
    const byKey = (k: string) => paletteActions(a).find((x) => x.key === k)!;
    await byKey('go.rules').run();
    expect(a.view).toBe('rules');
    await byKey('lang.de').run();
    expect(a.lang).toBe('de');
    await byKey('theme.dark').run();
    expect(a.theme).toBe('dark');
    await byKey('shortcuts.off').run();
    expect(a.shortcuts).toBe(false);
    a.setFilter({ families: ['DEP'], severities: [], checkKinds: [], query: '' });
    await byKey('filter.reset').run();
    expect(a.filter.families).toEqual([]);
  });

  it('have keys that are unique', () => {
    const keys = paletteActions(app()).map((x) => x.key);
    expect(new Set(keys).size).toBe(keys.length);
  });
});
