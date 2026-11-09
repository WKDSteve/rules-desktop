// The keyboard of the page: the keys every app shares are the kit's (`keymap.json`: Ctrl or Cmd and K for the palette,
// j/k, Enter, Escape, `/`, and the chord that starts with `g`); this file adds the targets of the chord, the views of this
// page (`g h` overview, `g f` families, `g l` rules, `g v` views, `g c` coverage, `g w` waivers, `g s` sources).

import { defaultKeymap, withChord } from 'appkit-ui/keymap';
import type { View } from './app.svelte';

export const keymap = withChord(defaultKeymap, {
  h: 'overview',
  f: 'families',
  l: 'rules',
  v: 'views',
  c: 'coverage',
  w: 'waivers',
  s: 'sources',
} satisfies Record<string, View>);

export { type ChordState, type Command, type KeyPress } from 'appkit-ui/keymap';
