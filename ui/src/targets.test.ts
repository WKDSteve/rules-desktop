// A small button is hit with a finger or a shaky hand: its box must be at least 24 CSS px high (and 44 px on a touch
// screen). The buttons of this page get their height from a padding of 0.15rem, which alone is about 21 px, so every
// style rule that sets that padding also sets `min-height`, and the touch rule exists. This test reads every component.

import { describe, expect, it } from 'vitest';

const COMPONENTS = import.meta.glob('./**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

/** The css rules of a component that set the thin button padding, as `selector { body }`. */
function thinRules(source: string): { selector: string; body: string }[] {
  return [...source.matchAll(/([^{}]+)\{([^{}]*padding:\s*0\.15rem 0\.6rem;[^{}]*)\}/g)].map((m) => ({ selector: (m[1] ?? '').trim(), body: m[2] ?? '' }));
}

describe('buttons have a hit area', () => {
  for (const [file, source] of Object.entries(COMPONENTS)) {
    const rules = thinRules(source);
    if (rules.length === 0) continue;
    it(`${file}: a thin button has min-height 1.5rem and a touch rule of 2.75rem`, () => {
      expect(rules.filter((rule) => !/min-height:\s*1\.5rem/.test(rule.body)).map((rule) => rule.selector)).toEqual([]);
      expect(source).toMatch(/@media \(pointer: coarse\)[^]*min-height:\s*2\.75rem/);
    });
  }
});
