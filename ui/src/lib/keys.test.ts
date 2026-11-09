import { resolveKey, type ChordState, type KeyPress } from 'appkit-ui/keymap';
import { describe, expect, it } from 'vitest';
import { keymap } from './keys';

function press(key: string, extra: Partial<KeyPress> = {}): KeyPress {
  return { key, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, typing: false, ...extra };
}

/** Feed keys at the given times; the commands that came out. */
function run(keys: [string, number, Partial<KeyPress>?][], shortcuts = true): (string | null)[] {
  let chord: ChordState = null;
  return keys.map(([key, at, extra]) => {
    const result = resolveKey(keymap, press(key, extra), chord, at, shortcuts);
    chord = result.chord;
    const c = result.command;
    return c === null ? null : c.arg === undefined ? c.name : `${c.name}:${c.arg}`;
  });
}

// The keys themselves are tested in the kit (keymap.test.ts); here are the views of this page.
describe('the keys of the page', () => {
  it('go to every view with g and a letter', () => {
    const letters: [string, string][] = [['h', 'overview'], ['f', 'families'], ['l', 'rules'], ['v', 'views'], ['c', 'coverage'], ['w', 'waivers'], ['s', 'sources']];
    for (const [letter, view] of letters) {
      expect(run([['g', 0], [letter, 10]])).toEqual([null, `go:${view}`]);
    }
    expect(Object.keys(keymap.chord.targets).sort()).toEqual(letters.map(([l]) => l).sort());
  });

  it('forget a g that nothing follows in time, as in tasks-desktop', () => {
    expect(keymap.chord.timeoutMs).toBe(1500);
    expect(run([['g', 0], ['h', 1500]])).toEqual([null, null]);
  });

  it('move, open, search and close, and open the palette with Ctrl or Cmd and K', () => {
    expect(run([['j', 0], ['k', 1], ['Enter', 2], ['/', 3], ['Escape', 4]])).toEqual(['next', 'previous', 'open', 'search', 'escape']);
    expect(run([['k', 0, { ctrlKey: true }], ['K', 1, { metaKey: true, typing: true }]], false)).toEqual(['palette', 'palette']);
  });

  it('with the single keys off take no letter, but Enter and Escape', () => {
    expect(run([['g', 0], ['h', 1], ['j', 2], ['/', 3], ['Enter', 4], ['Escape', 5]], false)).toEqual([null, null, null, null, 'open', 'escape']);
  });
});
