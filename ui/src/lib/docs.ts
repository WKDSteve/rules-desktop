// The documents the Help menu opens: the readme and the files of `docs/`, bundled with the page at build time (so
// they read without a network, and are the text of this commit).

import type { DocEntry } from 'appkit-ui/docs';

export const REPO_URL = 'https://github.com/WKDSteve/rules-desktop';
export const LICENSE = { name: 'MIT', url: `${REPO_URL}/blob/main/LICENSE` } as const;

/** The first level-one heading, else the name of the file: the title of a document in the list. */
export function titleOf(text: string, fallback: string): string {
  const m = /^#[ \t]+(.+?)[ \t]*#*[ \t]*$/m.exec(text);
  return m?.[1] ?? fallback;
}

/** The documents in the order the menu shows them: the readme first, the rest by name. */
export function entriesOf(files: Readonly<Record<string, string>>): DocEntry[] {
  const entries = Object.entries(files).map(([path, text]) => {
    const id = (path.split('/').pop() ?? path).replace(/\.md$/i, '');
    return { id, title: titleOf(text, id), text };
  });
  return entries.sort((a, b) => (a.id === 'README' ? -1 : b.id === 'README' ? 1 : a.id.localeCompare(b.id)));
}

const files = import.meta.glob(['../../../README.md', '../../../docs/*.md'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

export const DOCS: DocEntry[] = entriesOf(files);
