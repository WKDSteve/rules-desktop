import { describe, expect, it } from 'vitest';
import { detectLang, dictionaries, isLang, labelOf, numberText, percentText, translate, type Key } from './i18n';

describe('i18n', () => {
  it('has the same keys and the same placeholders in both languages', () => {
    const keys = Object.keys(dictionaries.en).sort();
    expect(Object.keys(dictionaries.de).sort()).toEqual(keys);
    const holes = (s: string) => (s.match(/\{\w+\}/g) ?? []).sort();
    for (const key of keys as Key[]) {
      expect(holes(dictionaries.de[key]), key).toEqual(holes(dictionaries.en[key]));
      expect(dictionaries.de[key].trim().length, key).toBeGreaterThan(0);
      expect(dictionaries.en[key].trim().length, key).toBeGreaterThan(0);
    }
  });

  it('fills placeholders and leaves an unknown one standing', () => {
    expect(translate('en', 'list.count', { shown: 2, total: 9 })).toBe('2 of 9');
    expect(translate('de', 'list.count', { shown: 2 })).toBe('2 von {total}');
  });

  it('takes the language of the browser when the page has it, else English', () => {
    expect(detectLang(['de-AT', 'en'])).toBe('de');
    expect(detectLang(['fr', 'en-GB'])).toBe('en');
    expect(detectLang(['fr'])).toBe('en');
    expect(isLang('de')).toBe(true);
    expect(isLang('fr')).toBe(false);
  });

  it('names a known word of the engine and shows an unknown one as it is', () => {
    expect(labelOf('de', 'severity', 'nice-to-have')).toBe('wünschenswert');
    expect(labelOf('en', 'class', 'private')).toBe('private');
    expect(labelOf('en', 'severity', 'a-new-word')).toBe('a-new-word');
    expect(labelOf('en', 'severity', undefined)).toBe('?');
    // A word that is a property of every object is not a key.
    expect(labelOf('en', 'severity', 'constructor')).toBe('constructor');
  });

  it('has a word for every severity of the contract and every class of the hub', () => {
    for (const word of ['critical', 'recommended', 'nice-to-have']) {
      expect(Object.hasOwn(dictionaries.en, `severity.${word}`), word).toBe(true);
    }
    for (const word of ['public', 'internal', 'private', 'unclassified']) {
      expect(Object.hasOwn(dictionaries.en, `class.${word}`), word).toBe(true);
    }
  });
});

describe('numbers in the writing of the language', () => {
  it('numberText groups the thousands as the language does', () => {
    expect(numberText('en', 1200)).toBe('1,200');
    expect(numberText('de', 1200)).toBe('1.200');
  });

  it('percentText rounds to whole percents and tolerates an empty whole', () => {
    expect(percentText('en', 1, 3)).toBe('33%');
    expect(percentText('de', 1, 2).replace(/\s/g, ' ')).toBe('50 %');
    expect(percentText('en', 5, 0)).toBe('0%');
  });

  it('a count in a sentence is written with the language grouping', () => {
    expect(translate('en', 'list.count', { shown: 2, total: 1200 })).toBe('2 of 1,200');
    expect(translate('de', 'list.count', { shown: 2, total: 1200 })).toBe('2 von 1.200');
  });
});
