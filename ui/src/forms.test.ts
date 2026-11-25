// A form of this page can be sent while it is incomplete: it says what is missing at the field and puts the focus there.
// A submit button that is locked for an empty field leaves the person guessing why. A dialog that holds typed text asks
// before Escape drops it, and a page with an unsaved change asks before the tab is left. This test reads every
// component, so a form added later does not forget it.

import { describe, expect, it } from 'vitest';

const COMPONENTS = import.meta.glob('./**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

/** The `<button type="submit" …>` tags of a component (the attributes up to the first `>` that closes the tag). */
function submitButtons(source: string): string[] {
  return [...source.matchAll(/<button\b[^>]*type="submit"[^>]*>/g)].map((m) => m[0]);
}

describe('forms', () => {
  it('has components to read', () => {
    expect(Object.keys(COMPONENTS).length).toBeGreaterThan(10);
  });

  it('no submit button is locked for an empty field or for a request on its way', () => {
    const locked = Object.entries(COMPONENTS).flatMap(([file, source]) =>
      submitButtons(source)
        .filter((tag) => /\bdisabled=\{[^}]*(trim\(\)|writing|=== ''|\.length)/.test(tag))
        .map((tag) => `${file}: ${tag}`),
    );
    expect(locked).toEqual([]);
  });

  it('every form that is sent names its checks itself (novalidate), not through the browser bubble', () => {
    const open = Object.entries(COMPONENTS).flatMap(([file, source]) =>
      [...source.matchAll(/<form\b[^>]*>/g)].filter((m) => !/novalidate|class="editor"/.test(m[0])).map((m) => `${file}: ${m[0].slice(0, 60)}`),
    );
    expect(open).toEqual([]);
  });

  it('a dialog with fields asks before Escape drops them', () => {
    const plain = Object.entries(COMPONENTS).flatMap(([file, source]) =>
      [...source.matchAll(/<dialog\b[^>]*>[^]*?<\/dialog>/g)]
        .filter((m) => /<input\b|<textarea\b/.test(m[0]) && !/use:leaveGuard=/.test(m[0]))
        .map(() => file),
    );
    expect(plain).toEqual([]);
  });

  it('a component with typed text that is not saved asks before the tab is left', () => {
    for (const file of ['./views/NewRule.svelte', './views/RuleEditor.svelte', './views/Waivers.svelte']) {
      expect(COMPONENTS[file], file).toMatch(/warnBeforeUnload\(\)/);
    }
  });
});
