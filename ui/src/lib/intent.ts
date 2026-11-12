// The target's check of a `wkd-intent/1` (appkit `schemas/wkd-intent/1/README.md`), ported from the reference
// `tools/check-wkd-intent.py` and tested against the same golden examples. An intent is untrusted input, wherever it
// came from: `intentProblems` never throws on one, it names every problem, and a person confirms before anything is
// written (the intent only opens a view or fills the form of this app).

export const MAX_BYTES = 64 * 1024;
const SCHEMA = 'wkd-intent/1';
const ACTIONS = ['tasks.new', 'tasks.show', 'rules.show', 'rules.new', 'ai.handoff'] as const;
const MODES = ['form', 'ai-draft', 'agent'] as const;
const SHOW = ['tasks.show', 'rules.show'];
const CLASSES = ['public', 'internal'];
const LIMITS = { title: 120, excerpt: 4096, repro: 1000 } as const;
const MAX_PATH = 300;
const RUN_URL = 'https://github.com/';

/** What this app takes, as it names it in its handshake (`intents` next to `call hello`). */
export const OFFERED = { schema: SCHEMA, actions: ['rules.show', 'rules.new'], modes: ['form'] } as const;

// Control characters other than tab, line break and carriage return, and the C1 block.
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/;
// One word that leaves the repository: an absolute path, a home, a `..` segment. Quotes and `=` start a word too.
const ESCAPE = /(?:^|[\s"'=])(?:[/\\]|[A-Za-z]:[/\\]|~[/\\]|\$\{?HOME\b|%[A-Za-z]+%)|(?:^|[\s"'=/\\])\.\.(?:[/\\]|\s|$)/;

const leavesRepo = (text: string): boolean => ESCAPE.test(text);

type Json = unknown;
const isObject = (value: Json): value is Record<string, Json> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

/** Every string in the document, keys included, found with a loop (what the parser nested must not overflow here). */
function* strings(value: Json): Generator<string> {
  const stack: Json[] = [value];
  while (stack.length > 0) {
    const item = stack.pop();
    if (typeof item === 'string') yield item;
    else if (isObject(item)) stack.push(...Object.keys(item), ...Object.values(item));
    else if (Array.isArray(item)) stack.push(...item);
  }
}

function checkFinding(finding: Record<string, Json>, out: string[]): void {
  if (finding.schema !== 'testing-finding/1') out.push('finding.schema is not testing-finding/1');
  if (!(typeof finding.fp === 'string' && /^[0-9a-f]{16}$/.test(finding.fp))) out.push('finding.fp is not 16 hex characters');
  if (typeof finding.class !== 'string' || !CLASSES.includes(finding.class)) out.push('finding.class is not public or internal');
  if (!(typeof finding.title === 'string' && finding.title !== '')) out.push('finding.title is missing');
  if (typeof finding.excerpt !== 'string') out.push('finding.excerpt is missing');
  for (const [key, limit] of Object.entries(LIMITS)) {
    const text = finding[key];
    if (typeof text === 'string' && [...text].length > limit) out.push(`finding.${key} is longer than ${limit}`);
  }
  const repro = finding.repro;
  if (repro !== undefined && repro !== null && typeof repro !== 'string') out.push('finding.repro is not a string');
  else if (typeof repro === 'string' && leavesRepo(repro)) out.push('finding.repro leaves the repository');
  const where = finding.where;
  if (where !== undefined && where !== null) {
    const file = isObject(where) ? where.file : undefined;
    if (typeof file !== 'string') out.push('finding.where.file is missing');
    else if ([...file].length > MAX_PATH) out.push(`finding.where.file is longer than ${MAX_PATH}`);
    else if (leavesRepo(file)) out.push('finding.where.file leaves the repository');
  }
  const run = finding.run;
  const url = isObject(run) ? run.url : undefined;
  if (url !== undefined && url !== null && !(typeof url === 'string' && url.startsWith(RUN_URL))) {
    out.push(`finding.run.url does not start with ${RUN_URL}`);
  }
}

/** Every reason to refuse `raw`, empty when this app may open its view or its form. `accepted` is what it offers. */
export function intentProblems(raw: string, accepted: readonly string[] = OFFERED.actions): string[] {
  // Measured in UTF-8 bytes, before parsing.
  if (new TextEncoder().encode(raw).length > MAX_BYTES) return [`larger than ${MAX_BYTES} bytes`];
  let doc: Json;
  try {
    doc = JSON.parse(raw);
  } catch {
    return ['not JSON in UTF-8'];
  }
  if (!isObject(doc)) return ['not a JSON object'];
  if (doc.schema !== SCHEMA) return [`schema ${JSON.stringify(doc.schema)} is not ${SCHEMA}`];

  const out: string[] = [];
  const { action, mode } = doc;
  const known = typeof action === 'string' && (ACTIONS as readonly string[]).includes(action);
  if (!known) out.push('unknown action');
  else if (!accepted.includes(action)) out.push(`this target does not take ${action}`);
  if (typeof mode !== 'string' || !(MODES as readonly string[]).includes(mode)) out.push('unknown mode');
  else if (known && SHOW.includes(action) && mode !== 'form') out.push(`${action} only knows mode form`);

  const source = doc.source;
  const app = isObject(source) ? source.app : undefined;
  if (!(typeof app === 'string' && /^[a-z][a-z0-9-]{0,63}$/.test(app))) out.push('source.app is missing or not a plain name');
  for (const key of ['finding', 'target']) if (!(key in doc)) out.push(`${key} is missing`);

  const { finding, target } = doc;
  if (known && SHOW.includes(action)) {
    if (finding !== null && finding !== undefined) out.push(`${action} carries no finding`);
    const id = isObject(target) ? target.id : undefined;
    if (!(typeof id === 'string' && id !== '')) out.push('target.id is missing');
    else if ([...id].length > MAX_PATH) out.push(`target.id is longer than ${MAX_PATH}`);
    else if (leavesRepo(id)) out.push('target.id leaves the repository');
  } else if (known) {
    if (target !== null && target !== undefined) out.push(`${action} has no target`);
    if (isObject(finding)) checkFinding(finding, out);
    else out.push(`${action} needs a finding`);
  }
  for (const text of strings(doc)) {
    if (CONTROL.test(text)) {
      out.push('a field holds a control character');
      break;
    }
  }
  return out;
}

/** What an accepted intent asks this app to do. */
export type Intent =
  | { action: 'rules.show'; id: string }
  | { action: 'rules.new'; title: string; text: string };

/**
 * The intent in `raw`, or the reasons it is refused. `rules.show` names a rule (looked up, never opened as a path);
 * `rules.new` fills the form of a new rule from the finding: the title, and the excerpt with where it was seen. The
 * mode (`ai-draft`, `agent`) is not taken up here: this app offers `form` only, and a person writes the rule.
 */
export function readIntent(raw: string): { ok: true; intent: Intent } | { ok: false; problems: string[] } {
  const problems = intentProblems(raw);
  if (problems.length > 0) return { ok: false, problems };
  const doc = JSON.parse(raw) as Record<string, Json>;
  if (doc.action === 'rules.show') {
    return { ok: true, intent: { action: 'rules.show', id: (doc.target as { id: string }).id } };
  }
  const finding = doc.finding as { title: string; excerpt: string; where?: { file: string; line?: number } | null };
  const where = finding.where ? `\n\n${finding.where.file}${typeof finding.where.line === 'number' ? `:${finding.where.line}` : ''}` : '';
  return { ok: true, intent: { action: 'rules.new', title: finding.title, text: `${finding.excerpt}${where}` } };
}
