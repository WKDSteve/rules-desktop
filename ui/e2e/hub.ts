// A real hub for the browser tests: the binary of this repository, the real engine (a real Neovim and rules.nvim),
// a copy of the synthetic fixture vault with hostile rules added. Nothing of a real rule set is ever used.

import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  pairingLink as pairing,
  removeTree,
  sessionToken as token,
  stopChild,
  type HubTarget,
} from 'appkit-web/e2e-node';
import { failAfterCleanup, tempRoot } from './cleanup';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..');
export const PORT = 17697;
export const ORIGIN = `http://127.0.0.1:${PORT}`;

const exe = join(repo, 'target', 'debug', process.platform === 'win32' ? 'rules-hub.exe' : 'rules-hub');

/** Titles and texts an attacker could write into a rule set (a rule set is somebody else's text). */
export const HOSTILE: { id: string; title: string; body: string; severity?: string; extra?: string }[] = [
  { id: 'HX-01', title: '"><img src=x onerror=window.__xss=1>', body: 'plain text' },
  { id: 'HX-02', title: '<svg onload=window.__xss=2>', body: 'plain text' },
  {
    id: 'HX-03',
    title: 'Text with raw html',
    body: [
      '<script>window.__xss=3</script>',
      '',
      '<img src=x onerror="window.__xss=4">',
      '',
      '<iframe src="javascript:window.__xss=5"></iframe>',
      '',
      '<a href="javascript:window.__xss=6">raw anchor</a>',
      '',
      '<details open ontoggle="window.__xss=7">d</details>',
    ].join('\n'),
  },
  {
    id: 'HX-04',
    title: 'Links in the text',
    body: ['[js](javascript:window.__xss=8)', '', '![img](https://tracker.invalid/pixel.png)', '', '[ok](https://example.org/page)'].join('\n'),
  },
  {
    id: 'HX-05',
    title: 'A check and a question with markup',
    body: 'fields',
    extra: 'check = { type = "grep", include = "%.lua$", pattern = "<img src=x onerror=window.__xss=9>" },',
  },
  { id: 'HX-06', title: 'Invoice \u202Etxt.exe\u202C \u200Bready', body: 'a\u202Ereversed' },
  { id: 'HX-07', title: 'Long ' + 'x'.repeat(300), body: 'y'.repeat(600) },
];

function hostileFile(): string {
  const blocks = HOSTILE.map((h) =>
    [
      `#### \`${h.id}\` - ${h.title}`,
      '',
      '```rule',
      `id = "${h.id}",`,
      `severity = "${h.severity ?? 'recommended'}",`,
      ...(h.extra ? [h.extra] : []),
      '```',
      '',
      h.body,
      '',
    ].join('\n'),
  );
  return ['# Hostile rules', '', ...blocks].join('\n');
}

/** Is the binary the one of the current build of the interface? */
export async function assertFreshBinary(): Promise<void> {
  const dist = join(repo, 'ui', 'dist', 'index.html');
  if (!existsSync(dist)) throw new Error('ui/dist is not built: run `pnpm build` and `cargo build`');
  const served = await (await fetch(`${ORIGIN}/`)).text();
  if (served.trim() !== readFileSync(dist, 'utf8').trim()) {
    throw new Error('the hub binary holds an older interface: run `cargo build` after `pnpm build`');
  }
}

export interface Running {
  child: ChildProcess;
  config: string;
  vault: string;
  /** Ends the hub, waits for it, and removes its folder in the temp dir (an error if that fails). */
  stop(): Promise<void>;
}

/** A rule of the rule set the page may write: one file per rule, the only form `rules-ops/1` writes. */
function writableRule(id: string, severity: string, title: string): string {
  return [
    '---',
    `id: ${id}`,
    `severity: ${severity}`,
    'family: WR',
    '---',
    '',
    `#### \`${id}\` — ${title}`,
    '',
    '```rule',
    `id = "${id}",`,
    '```',
    '',
    `The text of ${id}.`,
    '',
    '---',
    '',
  ].join('\n');
}

const GIT = ['-c', 'user.name=e2e', '-c', 'user.email=e2e@example.invalid', '-c', 'core.autocrlf=false'];

/** The lines the writes changed in the files of the vault since it was made (`git diff`, no context). */
export function changedLines(vault: string): string[] {
  const diff = execFileSync('git', [...GIT, 'diff', '--no-color', '-U0'], { cwd: vault, encoding: 'utf8' });
  return diff.split('\n').filter((l) => /^[-+]/.test(l) && !l.startsWith('---') && !l.startsWith('+++'));
}

/** The files the vault differs in from when it was made, tracked or not (a moved file shows as two). */
export function changedFiles(vault: string): string[] {
  const out = execFileSync('git', [...GIT, 'status', '--porcelain', '-uall', '--', 'rulesets/gamma'], { cwd: vault, encoding: 'utf8' });
  return out.split('\n').filter((l) => l.trim() !== '');
}

export function prepareVault(): { root: string; vault: string; config: string } {
  const root = tempRoot('e2e');
  try {
    return fillRoot(root);
  } catch (error) {
    // No process holds a file of it yet: one removal, here and now, before the error goes up.
    rmSync(root, { recursive: true, force: true });
    throw error;
  }
}

function fillRoot(root: string): { root: string; vault: string; config: string } {
  const vault = join(root, 'vault');
  cpSync(join(repo, 'crates', 'rules-hub', 'tests', 'fixtures', 'vault'), vault, { recursive: true });
  writeFileSync(join(vault, 'rulesets', 'alpha', 'regeln', 'hostile.md'), hostileFile());
  const gamma = join(vault, 'rulesets', 'gamma', '_rules', 'WR');
  mkdirSync(gamma, { recursive: true });
  writeFileSync(join(gamma, 'WR-01.md'), writableRule('WR-01', 'recommended', 'first writable'));
  writeFileSync(join(gamma, 'WR-02.md'), writableRule('WR-02', 'recommended', 'second writable'));
  writeFileSync(join(gamma, 'WR-03.md'), writableRule('WR-03', 'nice-to-have', 'third writable'));
  // The state the vault starts from: what a write changed is read from it afterwards.
  for (const args of [['init', '-q'], ['add', '-A'], ['commit', '-q', '-m', 'baseline']]) {
    execFileSync('git', [...GIT, ...args], { cwd: vault, stdio: 'ignore' });
  }
  const config = join(root, 'config');
  mkdirSync(config);
  // The project of the waivers: a folder of its own, outside the vault (the person's word about this device).
  const project = join(root, 'project');
  mkdirSync(project);
  writeFileSync(join(config, 'config.json'), JSON.stringify({ vault, project }));
  writeFileSync(
    join(config, 'sources.json'),
    JSON.stringify({
      schema: 1,
      sources: { alpha: { class: 'public' }, beta: { class: 'internal' }, gamma: { class: 'internal' } },
    }),
  );
  return { root, vault, config };
}

export function env(config: string): NodeJS.ProcessEnv {
  return {
    ...process.env,
    RULES_DESKTOP_CONFIG_DIR: config,
    // The editor is not started in a test: the file it would open is written here.
    RULES_DESKTOP_EDITOR_LOG: join(config, '..', 'editor.log'),
    RULES_DESKTOP_PORT: String(PORT),
  };
}

export async function startHub(): Promise<Running & { root: string }> {
  // Something already answers on the port: those tests would run against another hub and another vault.
  const taken = await fetch(`${ORIGIN}/api/rules/hello`).then(
    () => true,
    () => false,
  );
  if (taken) throw new Error(`port ${PORT} is in use: stop the hub that runs there`);
  const { root, vault, config } = prepareVault();
  const child = spawn(exe, ['serve'], { env: env(config), stdio: 'ignore' });
  const stop = async (): Promise<void> => {
    await stopChild(child);
    await removeTree(root);
  };
  try {
    await waitUntilReady(config);
  } catch (error) {
    // The hub that did not come up goes, and so does its folder: a failed start leaves nothing behind, or says so.
    return failAfterCleanup(error, stop);
  }
  return { child, config, vault, root, stop };
}

/** Waits until the hub answers with a ready engine, has written `hub.json` and serves the current build. */
async function waitUntilReady(config: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  for (;;) {
    try {
      const r = await fetch(`${ORIGIN}/api/rules/hello`);
      if (r.ok) {
        const doc = (await r.json()) as { engine: { ready: boolean } };
        if (doc.engine.ready) break;
        throw new Error('the engine is not ready: set RULES_ENGINE_DIR and LIB_NVIM_DIR');
      }
    } catch (error) {
      if (Date.now() > deadline || (error instanceof Error && error.message.includes('engine'))) throw error;
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  // The port answers before `hub.json` is written, and `rules-desktop link` (the pairing of the tests) needs that file.
  while (!existsSync(join(config, 'hub.json'))) {
    if (Date.now() > deadline) throw new Error('the hub answers but wrote no hub.json');
    await new Promise((r) => setTimeout(r, 100));
  }
  await assertFreshBinary();
}

/** What `appkit-web/e2e-node` needs of this app's hub for the pairing. */
const target = (): HubTarget => ({ exe, env, origin: ORIGIN });

/** A pairing link, as `rules-desktop link` prints it. */
export const pairingLink = (config: string): string => pairing(config, target());

/**
 * A session token, got the way the page gets one. The hub allows ten exchanges a minute, so the tests share one
 * session and only the pairing tests go through a link.
 */
export const sessionToken = (config: string): Promise<string> => token(config, target());
