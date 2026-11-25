// Every field a person types into or picks from explains itself: hovering it (or its label), focusing it or tapping
// its label shows the hint popup of `appkit-ui/tooltip`. The popup looks for the closest `[data-hint]` around the
// element that has the focus, so the hint must sit on the field or on an element around it (its `<label>`): a hint on
// a sibling `<span>` shows on hover but never on focus. This test reads every component of the page and names each
// field that has none, so a field added later does not forget it.
// The check itself (`fieldsWithoutHint`) and its own cases are the kit's (`appkit-ui/checks/formHints`).

import { fieldsWithoutHint } from 'appkit-ui/checks/formHints';
import { describe, expect, it } from 'vitest';

/** Every component of the page, read as text: a component added later is in it without a change here. */
const COMPONENTS = import.meta.glob('./**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

describe('every field has a hint', () => {
  it('in every component of the page', () => {
    expect(Object.keys(COMPONENTS).length).toBeGreaterThan(10);
    const missing = Object.entries(COMPONENTS).flatMap(([file, source]) => fieldsWithoutHint(source, file.slice(2)));
    expect(missing.map((m) => `${m.file}:${m.line} ${m.field}`)).toEqual([]);
  });
});
