// Test helper: a hub in a fetch function. It answers the routes of rules-hub with the golden documents, counts what
// it was asked, and holds the event stream open until the test writes to it or closes it.

import { Hub } from './api';
import type { Env } from './app.svelte';
import { errorDoc, goldenText, groupingsDoc, hubSourcesDoc, rulesetsDoc, snapshotDoc, viewsDoc, waiversDoc } from './fixtures';

export const CODE = 'ab'.repeat(16);
export const TOKEN = 'token-1';

export class MemoryStorage {
  readonly items = new Map<string, string>();
  getItem(key: string): string | null {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.items.set(key, value);
  }
  removeItem(key: string): void {
    this.items.delete(key);
  }
}

export interface Asked {
  method: string;
  path: string;
  query: URLSearchParams;
  headers: Headers;
}

export class FakeHub {
  asked: Asked[] = [];
  engineReady = true;
  /** The next hellos fail as a hub that is not there does. */
  downFor = 0;
  /** Codes the hub takes. */
  codes = new Set([CODE]);
  /** Tokens the hub takes; a request with another one is a 401. */
  tokens = new Set([TOKEN]);
  snapshotText = goldenText('snapshot');
  etag = 'sha256:9609c01610189534';
  /** Ids `rule` answers 404 for although they are in the snapshot. */
  goneIds = new Set<string>();
  /** open answers with this status (200: the editor is started). */
  openStatus = 200;
  /** What `waivers` answers: a status and a document (default: the golden waivers of a project). */
  waiversAnswer: () => { status: number; body: unknown } = () => ({ status: 200, body: waiversDoc() });
  /** What `rulesets` answers: a status and a document (default: the golden one). */
  rulesetsAnswer: () => { status: number; body: unknown } = () => ({ status: 200, body: rulesetsDoc() });
  /** What `groupings` answers (default: the golden one). */
  groupingsAnswer: () => { status: number; body: unknown } = () => ({ status: 200, body: groupingsDoc() });
  /** What `views` answers (default: the golden one, one view). */
  viewsAnswer: () => { status: number; body: unknown } = () => ({ status: 200, body: viewsDoc() });
  /** The bodies `sources/class` was sent, as parsed. */
  classRequests: Record<string, unknown>[] = [];
  /** What `sources/class` answers for a body: a status and a document (default: written). */
  classAnswer: (body: Record<string, unknown>) => { status: number; body: unknown } = () => ({
    status: 200,
    body: { ok: true },
  });
  /** The requests of `ops` the hub was sent, as parsed. */
  opsRequests: { dry_run: boolean; ops: Record<string, unknown>[] }[] = [];
  /** What `ops` answers for a request: a status and a document (default: every operation changed its rule). */
  opsAnswer: (request: { dry_run: boolean; ops: Record<string, unknown>[] }) => { status: number; body: unknown } = (
    request,
  ) => ({
    status: 200,
    body: {
      kind: 'rules.result',
      dry_run: request.dry_run,
      ok: true,
      results: request.ops.map((op, i) => ({
        n: i + 1,
        ok: true,
        op: op['op'],
        id: op['id'],
        changed_ids: [op['id']],
        etag_after: `sha256:after${i + 1}`,
        inverse: { op: 'set', id: op['id'], patch: { severity: 'recommended' } },
      })),
    },
  });
  searchIds: string[] = [];
  /** The `total` the search answers with; `null`: the number of `searchIds`. */
  searchTotal: number | null = null;
  /** The states of the search index for the next searches, one each; then `ready`. */
  searchStates: ('ready' | 'building' | 'failed')[] = [];
  /** The stream is refused with this status (0: it is opened). */
  streamStatus = 0;
  /** The next answers of a path, one status each, before the path answers as usual. */
  private failures = new Map<string, number[]>();
  private streams: ReadableStreamDefaultController<Uint8Array>[] = [];

  readonly fetchFn: typeof fetch = async (input, init = {}) => {
    const url = new URL(String(input), 'http://hub.test');
    const headers = new Headers(init.headers);
    const method = init.method ?? 'GET';
    this.asked.push({ method, path: url.pathname, query: url.searchParams, headers });
    const failure = this.failures.get(url.pathname)?.shift();
    if (failure !== undefined) return json({ error: { code: `status_${failure}`, message: 'failed' } }, failure);
    const token = (headers.get('Authorization') ?? '').replace(/^Bearer /, '');
    if (url.pathname === '/api/rules/hello') {
      if (this.downFor > 0) {
        this.downFor -= 1;
        throw new TypeError('network');
      }
      return json({
        schema: 1,
        kind: 'hub.hello',
        app: 'rules-desktop',
        version: '0.1.0',
        instance: 'i1',
        engine: { ready: this.engineReady, problem: this.engineReady ? null : 'no_engine', capabilities: null },
      });
    }
    if (url.pathname === '/auth/exchange') {
      const { code } = JSON.parse(String(init.body)) as { code: string };
      if (!this.codes.delete(code)) return json({ error: { code: 'bad_code', message: 'no' } }, 401);
      return json({ token: TOKEN });
    }
    if (!this.tokens.has(token)) return json({ error: { code: 'unauthorized', message: 'no session' } }, 401);
    switch (url.pathname) {
      case '/api/rules/snapshot': {
        if (headers.get('If-None-Match') === `"${this.etag}"`) return new Response(null, { status: 304 });
        return json(this.snapshotText, 200, { ETag: `"${this.etag}"` });
      }
      case '/api/rules/sources':
        return json(hubSourcesDoc());
      case '/api/rules/sources/class': {
        const body = JSON.parse(String(init.body)) as Record<string, unknown>;
        this.classRequests.push(body);
        const answer = this.classAnswer(body);
        return json(answer.body, answer.status);
      }
      case '/api/rules/rule': {
        const id = url.searchParams.get('id') ?? '';
        const held = (JSON.parse(this.snapshotText) as { rules: { id: string }[] }).rules.some((r) => r.id === id);
        if (!held || this.goneIds.has(id)) return json({ error: { code: 'not_found', message: errorDoc().message } }, 404);
        const doc = JSON.parse(goldenText('rule')) as { rule: { id: string } };
        doc.rule.id = id;
        return json(doc);
      }
      case '/api/rules/search':
        return json({
          status: this.searchStates.shift() ?? 'ready',
          indexed: 4,
          total: this.searchTotal ?? this.searchIds.length,
          hits: this.searchIds.map((id) => ({
            id,
            score: 1,
            snippet: { text: '', marks: [], more_before: false, more_after: false },
          })),
        });
      case '/api/rules/rescan':
        return new Response(null, { status: 202 });
      case '/api/rules/rulesets': {
        const answer = this.rulesetsAnswer();
        return json(answer.body, answer.status);
      }
      case '/api/rules/waivers': {
        const answer = this.waiversAnswer();
        return json(answer.body, answer.status);
      }
      case '/api/rules/groupings': {
        const answer = this.groupingsAnswer();
        return json(answer.body, answer.status);
      }
      case '/api/rules/views': {
        const answer = this.viewsAnswer();
        return json(answer.body, answer.status);
      }
      case '/api/rules/ops': {
        const request = JSON.parse(String(init.body)) as (typeof this.opsRequests)[number];
        this.opsRequests.push(request);
        const answer = this.opsAnswer(request);
        return json(answer.body, answer.status);
      }
      case '/api/rules/open':
        return this.openStatus === 200
          ? json({ ok: true })
          : json({ error: { code: this.openStatus === 404 ? 'open_unavailable' : 'open_failed', message: 'no' } }, this.openStatus);
      case '/api/rules/events': {
        if (this.streamStatus !== 0) return new Response(null, { status: this.streamStatus });
        let controller: ReadableStreamDefaultController<Uint8Array> | null = null;
        const body = new ReadableStream<Uint8Array>({ start: (c) => void (controller = c) });
        const opened = controller as ReadableStreamDefaultController<Uint8Array> | null;
        if (opened) this.streams.push(opened);
        init.signal?.addEventListener('abort', () => {
          this.streams = this.streams.filter((c) => c !== opened);
          try {
            opened?.error(new DOMException('aborted', 'AbortError'));
          } catch {
            // closed already
          }
        });
        return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
      }
    }
    return json({ error: { code: 'not_found', message: 'no such route' } }, 404);
  };

  /** Write an event to every open stream. */
  send(event: string, data: unknown): void {
    const bytes = new TextEncoder().encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
    for (const c of this.streams) c.enqueue(bytes);
  }

  get openStreams(): number {
    return this.streams.length;
  }

  /** The hub closes every stream (as when it stops). */
  closeStreams(): void {
    for (const c of this.streams) c.close();
    this.streams = [];
  }

  count(path: string, method = 'GET'): number {
    return this.asked.filter((a) => a.path === path && a.method === method).length;
  }

  /** The next `times` requests of `path` are answered with `status`. */
  failWith(path: string, status: number, times = 1): void {
    this.failures.set(path, [...(this.failures.get(path) ?? []), ...Array<number>(times).fill(status)]);
  }

  /** The golden snapshot again, all four rules, under a tag of its own. */
  restore(): void {
    this.snapshotText = goldenText('snapshot');
    this.etag = `${this.etag}-restored`;
  }

  /** The snapshot without one rule, under a new tag. */
  dropRule(id: string): void {
    const doc = snapshotDoc();
    doc.rules = doc.rules.filter((r) => r.id !== id);
    this.snapshotText = JSON.stringify(doc);
    this.etag = `${this.etag}-${id}`;
  }
}

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(typeof body === 'string' ? body : JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  });
}

/** An environment around a fake hub; `sleep` returns at once and keeps what it was asked to wait. */
export function testEnv(fake: FakeHub, now: { t: number } = { t: 1_800_000_000_000 }) {
  const session = new MemoryStorage();
  const prefs = new MemoryStorage();
  const slept: number[] = [];
  const env: Env = {
    hub: new Hub(fake.fetchFn),
    session,
    prefs,
    preferredLanguages: ['en'],
    fetchFn: fake.fetchFn,
    sleep: async (ms) => {
      slept.push(ms);
    },
    now: () => now.t,
  };
  return { env, session, prefs, slept, now };
}
