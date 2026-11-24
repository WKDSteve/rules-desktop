// What the browser tests leave in the temp dir: nothing. The one place they get a folder or a file there from, and the
// one way they take it away again (the Rust tests have `Scratch`, crates/rules-hub/tests/common/scratch.rs).
//
// A folder a running process still holds cannot be removed on Windows: the hub and the Neovim of its engine keep files
// of the vault open until they have ended. So the hub is stopped and waited for first, and the removal is retried for a
// few seconds (a virus scanner may hold a file just as long); then it fails loudly instead of leaving the folder
// silently. CI counts what is left after the tests (tools/check-temp-leftovers.py).

import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** What every folder and file of the tests is called first; the leftover check counts it. */
export const PREFIX = 'rules-desktop-';

/** A new, empty folder `rules-desktop-<label>-XXXXXX` in the temp dir. Take it away with `removeTree` of `appkit-web/e2e-node`. */
export function tempRoot(label: string): string {
  return mkdtempSync(join(tmpdir(), `${PREFIX}${label}-`));
}

/** A path `rules-desktop-<label>` in the temp dir for a file of the caller's (nothing is made). */
export function tempFile(label: string): string {
  return join(tmpdir(), `${PREFIX}${label}`);
}

/**
 * After `error` stopped a start: runs `cleanup`, then throws. `error` itself when the cleanup went well; otherwise an
 * error that names both, so that a folder the cleanup could not remove is said and not left behind silently.
 */
export async function failAfterCleanup(error: unknown, cleanup: () => Promise<void>): Promise<never> {
  try {
    await cleanup();
  } catch (cleanupError) {
    throw new Error(`${messageOf(error)}; the cleanup failed too: ${messageOf(cleanupError)}`, { cause: error });
  }
  throw error;
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
