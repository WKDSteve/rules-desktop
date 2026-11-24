// The cleanup of the browser tests, without a hub: the folder and the state file go, also when something fails, and a
// folder a process still holds goes once that process has been stopped and waited for.

import { spawn } from 'node:child_process';
import { existsSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { removeTree, stopChild } from 'appkit-web/e2e-node';
import { PREFIX, failAfterCleanup, tempFile, tempRoot } from './cleanup';
import { teardown } from './global';
import type { Running } from './hub';

describe('the temp dir of the browser tests', () => {
  it('a root is named for the leftover check and goes with everything in it', async () => {
    const root = tempRoot('cleanup-test');
    expect(basename(root).startsWith(`${PREFIX}cleanup-test-`)).toBe(true);
    writeFileSync(join(root, 'file'), 'x');
    await removeTree(root);
    expect(existsSync(root)).toBe(false);
    // One that is not there any more is no error.
    await removeTree(root);
  });

  it('a failed cleanup after a failed start is named in the error, a good one leaves it as it was', async () => {
    const started = new Error('the engine is not ready');
    await expect(failAfterCleanup(started, () => Promise.resolve())).rejects.toBe(started);
    const both = failAfterCleanup(started, () => Promise.reject(new Error('cannot remove held: EBUSY')));
    await expect(both).rejects.toThrow('the engine is not ready; the cleanup failed too: cannot remove held: EBUSY');
    await expect(both).rejects.toHaveProperty('cause', started);
  });

  it('a folder a running process holds goes once the process is stopped and waited for', async () => {
    const root = tempRoot('cleanup-held');
    // The working directory of a process is held by it on Windows: the folder cannot go while it runs.
    const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { cwd: root, stdio: 'ignore' });
    await new Promise((r) => child.once('spawn', r));
    try {
      await stopChild(child);
      expect(child.exitCode !== null || child.signalCode !== null).toBe(true);
      await removeTree(root, { tries: 1 });
      expect(existsSync(root)).toBe(false);
    } finally {
      child.kill();
      await removeTree(root);
    }
  });

  it('the teardown removes the state file also when the hub cannot be removed, and says so', async () => {
    const state = tempFile(`e2e-state-test-${process.pid}.json`);
    writeFileSync(state, '{}');
    const hub = {
      stop: () => Promise.reject(new Error('cannot remove the root')),
    } as unknown as Running;
    await expect(teardown(hub, state)).rejects.toThrow(/cannot remove the root/);
    expect(existsSync(state)).toBe(false);

    writeFileSync(state, '{}');
    await teardown(null, state);
    expect(existsSync(state)).toBe(false);
  });
});
