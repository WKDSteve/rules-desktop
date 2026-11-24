// A browser's page translation rewrites text it takes for prose. Identifiers, file names, commands and the JSON of a
// check must stay as they are, or a rule id turns into a word of another language. Every `<code>`, `<kbd>` and `<pre>`
// of the page therefore says `translate="no"`, except the one that shows the text of a rule (prose, which a person may
// want translated). This test reads every component and names the element that forgot it.

import { describe, expect, it } from 'vitest';

const COMPONENTS = import.meta.glob('./**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

/** The opening tags of code, kbd and pre elements in a component's markup. */
function tagsOf(source: string): string[] {
  return [...source.matchAll(/<(?:code|kbd|pre)\b[^>]*>/g)].map((m) => m[0]);
}

/** The tag that shows the text of a rule: prose, so it stays translatable. */
const PROSE = /class="text"/;

describe('identifiers and code are not translated', () => {
  it('finds the components', () => {
    expect(Object.keys(COMPONENTS).length).toBeGreaterThan(5);
  });

  for (const [file, source] of Object.entries(COMPONENTS)) {
    const tags = tagsOf(source).filter((tag) => !PROSE.test(tag));
    if (tags.length === 0) continue;
    it(`${file}: every code, kbd and pre says translate="no"`, () => {
      expect(tags.filter((tag) => !/\btranslate="no"/.test(tag))).toEqual([]);
    });
  }
});
