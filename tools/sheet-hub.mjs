// The app "hub" of the test sheets (.testing.lua): a rules-hub of the run on a copy of the fixture vault, started the
// way ui/e2e/hub.ts starts one (the real binary, the real engine), with nothing of the person's settings or rule sets.
// Prints the pairing link on its stdout once the engine answers, and ends the hub when its stdin closes.
//   node tools/sheet-hub.mjs <folder of the run> <port>
import { spawn, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const [folder, port] = process.argv.slice(2);
if (!folder || !/^\d+$/.test(port ?? '')) {
  console.error('usage: node tools/sheet-hub.mjs <folder of the run> <port>');
  process.exit(2);
}
if (!process.env.RULES_ENGINE_DIR || !process.env.LIB_NVIM_DIR) {
  console.error('set RULES_ENGINE_DIR (rules.nvim) and LIB_NVIM_DIR (lib.nvim)');
  process.exit(2);
}

const exe = resolve('target', 'debug', process.platform === 'win32' ? 'rules-hub.exe' : 'rules-hub');
const vault = join(folder, 'vault');
cpSync(resolve('crates', 'rules-hub', 'tests', 'fixtures', 'vault'), vault, { recursive: true });
const config = join(folder, 'config');
const project = join(folder, 'project');
for (const dir of [config, project]) mkdirSync(dir, { recursive: true });
writeFileSync(join(config, 'config.json'), JSON.stringify({ vault, project }));
writeFileSync(
  join(config, 'sources.json'),
  JSON.stringify({ schema: 1, sources: { alpha: { class: 'public' }, beta: { class: 'internal' }, gamma: { class: 'internal' } } }),
);

const env = {
  ...process.env,
  RULES_DESKTOP_CONFIG_DIR: config,
  RULES_DESKTOP_EDITOR_LOG: join(folder, 'editor.log'),
  RULES_DESKTOP_PORT: port,
};
const hub = spawn(exe, ['serve'], { env, stdio: 'ignore' });
process.stdin.on('end', () => {
  hub.kill();
  process.exit(0);
});
process.stdin.resume();
hub.on('exit', (code) => process.exit(code ?? 1));

const deadline = Date.now() + 30_000;
for (;;) {
  try {
    const answer = await fetch(`http://127.0.0.1:${port}/api/rules/hello`);
    if (answer.ok && (await answer.json()).engine.ready) break;
  } catch {
    // not up yet
  }
  if (Date.now() > deadline) {
    console.error('the hub did not get ready');
    hub.kill();
    process.exit(1);
  }
  await new Promise((resolveWait) => setTimeout(resolveWait, 200));
}
console.log(spawnSync(exe, ['link'], { env, encoding: 'utf8' }).stdout.trim());
