// The red wavy line under a rule id, a path or a line of JSON is noise and teaches people to ignore it. The fields
// that take an identifier, a path, a date or code say `spellcheck="false"`; the fields of prose (title, text, reason)
// keep the browser's spell check. This test reads every component and names an identifier field that forgot it.

import { describe, expect, it } from 'vitest';

const COMPONENTS = import.meta.glob('./**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

/** What a field is bound to when it takes an identifier, a path, a date or code (not prose). */
const IDENTIFIER_BINDS = [
  'family',
  'ruleset',
  'moveTo',
  'replacedBy',
  'newRuleset',
  'newKey',
  'id',
  'expires',
  'renewTo',
  'paths',
  'check',
  'agent',
  'draft.check',
  'draft.agent',
];

/** The opening tags of inputs and textareas. */
function fieldsOf(source: string): string[] {
  return [...source.matchAll(/<(?:input|textarea)\b[^>]*>/g)].map((m) => m[0]);
}

describe('identifier fields are not spell checked', () => {
  for (const [file, source] of Object.entries(COMPONENTS)) {
    const named = fieldsOf(source).filter((tag) => IDENTIFIER_BINDS.some((bind) => tag.includes(`bind:value={${bind}}`)));
    if (named.length === 0) continue;
    it(`${file}: its identifier fields say spellcheck="false"`, () => {
      expect(named.filter((tag) => !/\bspellcheck="false"/.test(tag))).toEqual([]);
    });
  }
});
