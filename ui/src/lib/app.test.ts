import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  App,
  conflictKey,
  EXCHANGE_RETRIES,
  GONE_SETTLE_MS,
  isNavState,
  navRoute,
  PREF_KEYS,
  RESCAN_EVERY_MS,
  routeNav,
  SEARCH_DELAY_MS,
  SEARCH_RETRIES,
  SEARCH_RETRY_MS,
  TOKEN_KEY,
} from './app.svelte';
import { snapshotDoc } from './fixtures';
import type { Conflict } from './write';
import { CODE, FakeHub, TOKEN, testEnv } from './testhub';

const apps: App[] = [];

/** An app on a fake hub, its tab paired already unless `paired` is false. */
function setup(paired = true) {
  const fake = new FakeHub();
  const t = testEnv(fake);
  if (paired) t.session.setItem(TOKEN_KEY, TOKEN);
  const app = new App(t.env);
  apps.push(app);
  return { fake, app, ...t };
}

/** Started, with the stream open. */
async function ready(paired = true) {
  const s = setup(paired);
  await s.app.start(null);
  await vi.waitFor(() => expect(s.fake.openStreams).toBe(1));
  await vi.waitFor(() => expect(s.app.connection).toBe('live'));
  return s;
}

afterEach(() => {
  for (const app of apps.splice(0)) app.stop();
});

describe('the place in the address', () => {
  it('a route names a view and the rule open in it, and reads back', () => {
    const place = { view: 'rules' as const, id: 'DEP-01' };
    expect(navRoute(place)).toEqual({ view: 'rules', rule: 'DEP-01' });
    expect(routeNav(navRoute(place))).toEqual(place);
    expect(routeNav({ view: 'coverage' })).toEqual({ view: 'coverage', id: null });
    expect(routeNav({ view: 'nope', rule: 'DEP-01' })).toBeNull();
    expect(routeNav({ rule: 'DEP-01' })).toBeNull();
    // A rule id that no place takes is left out, the view stays.
    expect(routeNav({ view: 'rules', rule: 'x'.repeat(101) })).toEqual({ view: 'rules', id: null });
  });

  it('the page opens on the place the address named, its rule read at once', async () => {
    const s = setup();
    s.app.openPlace({ view: 'rules', id: 'DEP-01' });
    expect(s.app.view).toBe('rules');
    await s.app.start(null);
    await vi.waitFor(() => expect(s.app.detail.state).toBe('ok'));
    expect(s.app.navState()).toEqual({ view: 'rules', id: 'DEP-01' });
  });

  it('keeps the place in the address when it takes the pairing code out', async () => {
    const s = setup(false);
    const replaced: string[] = [];
    const history = { replaceState: (_d: unknown, _u: string, url?: string | URL | null) => void replaced.push(String(url)) };
    await s.app.startFrom({ hash: `#pair=${CODE}&view=rules`, pathname: '/', search: '' }, history);
    expect(replaced).toEqual(['/#view=rules']);
  });
});

describe('the session for a new tab', () => {
  it('a tab without a session and without a code takes the session of an open tab', async () => {
    const s = setup(false);
    const ask = vi.fn(async () => TOKEN);
    const app = new App({ ...s.env, askSession: ask });
    apps.push(app);
    await app.start(null);
    expect(ask).toHaveBeenCalledTimes(1);
    expect(app.phase).toBe('ready');
    expect(s.session.getItem(TOKEN_KEY)).toBe(TOKEN);
  });

  it('asks no other tab with a session of its own or with a code, and pairs when none answers', async () => {
    const ask = vi.fn(async () => null);
    const own = setup();
    const a = new App({ ...own.env, askSession: ask });
    apps.push(a);
    await a.start(null);
    const coded = setup(false);
    const b = new App({ ...coded.env, askSession: ask });
    apps.push(b);
    await b.start(CODE);
    expect(ask).not.toHaveBeenCalled();
    const lone = setup(false);
    const c = new App({ ...lone.env, askSession: ask });
    apps.push(c);
    await c.start(null);
    expect(ask).toHaveBeenCalledTimes(1);
    expect(c.phase).toBe('needs_pairing');
  });

  it('takes an answer that comes after the wait, and keeps it', async () => {
    const s = setup(false);
    let late: (token: string) => void = () => {};
    const app = new App({
      ...s.env,
      askSession: async (onLate) => {
        late = onLate;
        return null;
      },
    });
    apps.push(app);
    await app.start(null);
    expect(app.phase).toBe('needs_pairing');
    late(TOKEN);
    expect(s.session.getItem(TOKEN_KEY)).toBe(TOKEN);
    await vi.waitFor(() => expect(app.phase).toBe('ready'));
  });

  it('a late answer changes nothing in a tab that has paired since, or was stopped', async () => {
    let late: (token: string) => void = () => {};
    const askSession = async (onLate: (token: string) => void): Promise<null> => {
      late = onLate;
      return null;
    };
    const paired = setup(false);
    const a = new App({ ...paired.env, askSession });
    apps.push(a);
    await a.start(null);
    const answer = late;
    await a.start(CODE);
    expect(a.phase).toBe('ready');
    const own = paired.session.getItem(TOKEN_KEY);
    answer('late-token-of-another-tab');
    expect(paired.session.getItem(TOKEN_KEY)).toBe(own);

    const gone = setup(false);
    const b = new App({ ...gone.env, askSession });
    apps.push(b);
    await b.start(null);
    b.stop();
    late(TOKEN);
    expect(gone.session.getItem(TOKEN_KEY)).toBeNull();
  });
});

describe('the start', () => {
  it('takes the pairing code out of the address, trades it and keeps the token for the tab only', async () => {
    const s = setup(false);
    const replaced: string[] = [];
    const history = { replaceState: (_d: unknown, _u: string, url?: string | URL | null) => void replaced.push(String(url)) };
    await s.app.startFrom({ hash: `#pair=${CODE}`, pathname: '/', search: '' }, history);
    expect(replaced).toEqual(['/']);
    expect(s.fake.count('/auth/exchange', 'POST')).toBe(1);
    expect(s.session.getItem(TOKEN_KEY)).toBe(TOKEN);
    expect(s.app.phase).toBe('ready');
    expect(s.app.snapshot?.rules).toHaveLength(4);
    expect(s.app.etag).toBe('sha256:9609c01610189534');
    expect(s.app.sources?.kind).toBe('hub.sources');
    expect(s.app.classOfRule({ source: 'Checklists/_rules/ERR/ERR-100.md' })).toBe('private');
    await vi.waitFor(() => expect(s.app.connection).toBe('live'));
    // The stream carries the token as a bearer, not in its address.
    const events = s.fake.asked.find((a) => a.path === '/api/rules/events');
    expect(events?.headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
    expect(events?.query.toString()).toBe('');
  });

  it('asks to be paired without a code and without a token, and asks the hub nothing', async () => {
    const s = setup(false);
    await s.app.start(null);
    expect(s.app.phase).toBe('needs_pairing');
    expect(s.app.pairRefused).toBe(false);
    expect(s.fake.asked).toEqual([]);
  });

  it('says so when the code was refused and the tab has no session', async () => {
    const s = setup(false);
    s.fake.codes.clear();
    await s.app.start(CODE);
    expect(s.app.phase).toBe('needs_pairing');
    expect(s.app.pairRefused).toBe(true);
  });

  it('trades a code again that the hub turned away for too many attempts, and does not call it used up', async () => {
    const s = setup(false);
    s.fake.failWith('/auth/exchange', 429);
    await s.app.start(CODE);
    expect(s.fake.count('/auth/exchange', 'POST')).toBe(2);
    expect(s.app.pairRefused).toBe(false);
    expect(s.app.phase).toBe('ready');
  });

  it('says to wait when the hub keeps turning the code away, not that it was used up', async () => {
    const s = setup(false);
    s.fake.failWith('/auth/exchange', 429, EXCHANGE_RETRIES + 1);
    await s.app.start(CODE);
    expect(s.fake.count('/auth/exchange', 'POST')).toBe(EXCHANGE_RETRIES + 1);
    expect(s.app.phase).toBe('needs_pairing');
    expect(s.app.pairRefused).toBe(false);
    expect(s.app.notice).toEqual({ key: 'notice.pair_rate_limited' });
  });

  it('does not call a code used up for a body the hub could not read', async () => {
    const s = setup(false);
    s.fake.failWith('/auth/exchange', 400);
    await s.app.start(CODE);
    expect(s.app.pairRefused).toBe(false);
  });

  it('keeps the session of the tab when a code is refused', async () => {
    const s = setup(true);
    s.fake.codes.clear();
    await s.app.start(CODE);
    expect(s.app.phase).toBe('ready');
    expect(s.app.pairRefused).toBe(false);
  });

  it('waits for an engine that is not ready, with its problem, and for a hub that is not there', async () => {
    const s = setup();
    s.fake.downFor = 1;
    s.fake.engineReady = false;
    let hellos = 0;
    const sleep = s.env.sleep;
    s.env.sleep = async (ms) => {
      hellos += 1;
      if (hellos === 1) expect(s.app.phase).toBe('offline');
      if (hellos === 2) {
        expect(s.app.phase).toBe('unavailable');
        expect(s.app.problem).toBe('no_engine');
        s.fake.engineReady = true;
      }
      await sleep(ms);
    };
    await s.app.start(null);
    expect(s.slept.slice(0, 2)).toEqual([3000, 5000]);
    expect(s.app.phase).toBe('ready');
    expect(s.app.problem).toBeNull();
  });
});

describe('a session that is gone', () => {
  it('turns a 401 into the pairing page and forgets the token and what was read', async () => {
    const s = await ready();
    s.fake.tokens.clear();
    await s.app.refresh();
    expect(s.app.phase).toBe('needs_pairing');
    expect(s.app.snapshot).toBeNull();
    expect(s.app.sources).toBeNull();
    expect(s.session.getItem(TOKEN_KEY)).toBeNull();
  });

  it('is told by a refused stream too', async () => {
    const s = setup();
    s.fake.streamStatus = 401;
    await s.app.start(null);
    await vi.waitFor(() => expect(s.app.phase).toBe('needs_pairing'));
    expect(s.session.getItem(TOKEN_KEY)).toBeNull();
  });
});

describe('the stream', () => {
  it('reads the snapshot with its tag on every rules event, also one with empty lists', async () => {
    const s = await ready();
    const before = s.fake.count('/api/rules/snapshot');
    s.fake.send('rules', { rev: 'rev:d62208', seq: 2, cause: 'fs', changed_ids: [], added: [], removed: [], truncated: false });
    await vi.waitFor(() => expect(s.fake.count('/api/rules/snapshot')).toBe(before + 1));
    const last = s.fake.asked.filter((a) => a.path === '/api/rules/snapshot').at(-1);
    expect(last?.headers.get('If-None-Match')).toBe('"sha256:9609c01610189534"');
  });

  it('reads the open rule again when its id changed', async () => {
    const s = await ready();
    await s.app.openDetail('DEP-01');
    expect(s.app.detail.state).toBe('ok');
    const rules = s.fake.count('/api/rules/rule');
    await s.app.onMessage({ event: 'rules', data: JSON.stringify({ changed_ids: ['DEP-09'], removed: [] }) });
    expect(s.fake.count('/api/rules/rule')).toBe(rules);
    await s.app.onMessage({ event: 'rules', data: JSON.stringify({ changed_ids: ['DEP-01', 'DEP-09'], removed: [] }) });
    expect(s.fake.count('/api/rules/rule')).toBe(rules + 1);
    expect(s.app.detail.state).toBe('ok');
  });

  it('keeps the open rule that is left out for one state, as a real hub sends it', async () => {
    const s = await ready();
    await s.app.openDetail('DEP-01');
    // As the hub sees it: the rule is out of its snapshot when it says `removed`, so it answers 404 for it (K8).
    s.fake.dropRule('DEP-01');
    const sleep = s.env.sleep;
    s.env.sleep = async (ms) => {
      // The next state brings it back (a retire or a save that was not done yet).
      if (ms === GONE_SETTLE_MS) s.fake.restore();
      await sleep(ms);
    };
    await s.app.onMessage({ event: 'rules', data: JSON.stringify({ changed_ids: [], removed: ['DEP-01'] }) });
    expect(s.slept).toContain(GONE_SETTLE_MS);
    expect(s.app.detail).toMatchObject({ state: 'ok', id: 'DEP-01' });
    expect(s.app.notice).toBeNull();
  });

  it('closes the open rule that is still gone after the pause, on a 404 of the hub, with a word', async () => {
    const s = await ready();
    await s.app.openDetail('DEP-01');
    s.fake.dropRule('DEP-01');
    const rules = s.fake.count('/api/rules/rule');
    await s.app.onMessage({ event: 'rules', data: JSON.stringify({ changed_ids: [], removed: ['DEP-01'] }) });
    expect(s.fake.count('/api/rules/rule')).toBe(rules + 1);
    expect(s.app.detail.state).toBe('none');
    expect(s.app.selectedId).toBeNull();
    expect(s.app.notice).toEqual({ key: 'detail.gone', params: { id: 'DEP-01' } });
  });

  it('closes an open rule that is gone on a resync or a truncated event as on a removal', async () => {
    for (const message of [
      { event: 'resync', data: '{}' },
      { event: 'rules', data: JSON.stringify({ changed_ids: [], removed: [], truncated: true }) },
    ]) {
      const s = await ready();
      await s.app.openDetail('LUA-01');
      s.fake.dropRule('LUA-01');
      await s.app.onMessage(message);
      expect(s.app.detail.state, message.event).toBe('none');
      expect(s.app.notice, message.event).toEqual({ key: 'detail.gone', params: { id: 'LUA-01' } });
    }
  });

  it('says live while the stream is open, also after a read of the snapshot failed', async () => {
    const s = await ready();
    s.fake.failWith('/api/rules/snapshot', 502);
    await s.app.refresh();
    expect(s.app.connection).toBe('live');
    await s.app.refresh();
    expect(s.app.connection).toBe('live');
    expect(s.fake.openStreams).toBe(1);
  });

  it('shows the rules when their classes cannot be read, all unclassified, and asks for the classes again', async () => {
    const s = setup();
    s.fake.failWith('/api/rules/sources', 502, 2);
    await s.app.start(null);
    expect(s.app.phase).toBe('ready');
    expect(s.app.rules).toHaveLength(4);
    expect(s.app.sources).toBeNull();
    for (const rule of s.app.rules) expect(s.app.classOfRule(rule), rule.id).toBe('unclassified');
    expect(s.app.notice).toEqual({ key: 'notice.sources_failed' });
    await vi.waitFor(() => expect(s.app.connection).toBe('live'));
    // The read on the open stream failed too; the next one, with the snapshot unchanged, gets them.
    await s.app.refresh();
    expect(s.app.sources?.kind).toBe('hub.sources');
    expect(s.app.classOfRule({ source: 'Checklists/_rules/ERR/ERR-100.md' })).toBe('private');
  });

  it('reads everything again on resync', async () => {
    const s = await ready();
    await s.app.openDetail('LUA-01');
    const [snap, src, rule] = ['/api/rules/snapshot', '/api/rules/sources', '/api/rules/rule'].map((p) => s.fake.count(p));
    await s.app.onMessage({ event: 'resync', data: '{}' });
    expect(s.fake.count('/api/rules/snapshot')).toBe(snap! + 1);
    expect(s.fake.count('/api/rules/sources')).toBe(src! + 1);
    expect(s.fake.count('/api/rules/rule')).toBe(rule! + 1);
  });

  it('reads again when the stream stands at another revision', async () => {
    const s = await ready();
    const before = s.fake.count('/api/rules/snapshot');
    await s.app.onMessage({ event: 'hello', data: JSON.stringify({ rev: 'rev:d62208', seq: 1 }) });
    expect(s.fake.count('/api/rules/snapshot')).toBe(before);
    await s.app.onMessage({ event: 'hello', data: JSON.stringify({ rev: 'rev:other', seq: 1 }) });
    expect(s.fake.count('/api/rules/snapshot')).toBe(before + 1);
  });

  it('reads the classes again only with a new snapshot', async () => {
    const s = await ready();
    const before = s.fake.count('/api/rules/sources');
    await s.app.refresh();
    expect(s.fake.count('/api/rules/sources')).toBe(before);
    s.fake.dropRule('LUA-01');
    await s.app.refresh();
    expect(s.fake.count('/api/rules/sources')).toBe(before + 1);
    expect(s.app.rules.map((r) => r.id)).not.toContain('LUA-01');
  });

  it('connects again after the hub closed it', async () => {
    const s = await ready();
    s.fake.closeStreams();
    await vi.waitFor(() => expect(s.fake.openStreams).toBe(1));
    expect(s.slept).toContain(1000);
  });
});

describe('the focus', () => {
  it('asks for a scan when the window comes back, at most every few seconds', async () => {
    const s = await ready();
    await s.app.focused();
    expect(s.fake.count('/api/rules/rescan', 'POST')).toBe(1);
    s.now.t += RESCAN_EVERY_MS - 1;
    await s.app.focused();
    expect(s.fake.count('/api/rules/rescan', 'POST')).toBe(1);
    s.now.t += 1;
    const snaps = s.fake.count('/api/rules/snapshot');
    await s.app.focused();
    expect(s.fake.count('/api/rules/rescan', 'POST')).toBe(2);
    expect(s.fake.count('/api/rules/snapshot')).toBe(snaps + 1);
  });
});

describe('the search', () => {
  it('searches the texts from two letters on, after a pause, and keeps the title hits', async () => {
    const s = await ready();
    await s.app.setQuery('f');
    expect(s.fake.count('/api/rules/search')).toBe(0);
    s.fake.searchIds = ['DEP-01'];
    await s.app.setQuery('one');
    expect(s.slept).toContain(SEARCH_DELAY_MS);
    expect(s.fake.count('/api/rules/search')).toBe(1);
    expect(s.app.searchHits?.status).toBe('ready');
    // DEP-01 by its title and by its text; nothing else.
    expect(s.app.shown.map((r) => r.id)).toEqual(['DEP-01']);
    s.fake.searchIds = ['ERR-100'];
    await s.app.setQuery('of');
    expect(s.app.shown.map((r) => r.id)).toEqual(['ERR-100']);
  });

  it('asks again while the hub is still reading the texts', async () => {
    const s = await ready();
    s.fake.searchStates = ['building', 'ready'];
    s.fake.searchIds = ['DEP-01'];
    await s.app.setQuery('text');
    expect(s.fake.count('/api/rules/search')).toBe(2);
    expect(s.slept).toContain(SEARCH_RETRY_MS);
    expect(s.app.searchHits?.status).toBe('ready');
    expect(s.app.shown.map((r) => r.id)).toEqual(['DEP-01']);
  });

  it('stops asking a hub that keeps building after a few times', async () => {
    const s = await ready();
    s.fake.searchStates = Array<'building'>(SEARCH_RETRIES + 5).fill('building');
    await s.app.setQuery('text');
    expect(s.fake.count('/api/rules/search')).toBe(SEARCH_RETRIES + 1);
    expect(s.app.searchHits?.status).toBe('building');
  });

  it('says when the hub found the words in more rules than it named', async () => {
    const s = await ready();
    s.fake.searchIds = ['DEP-01'];
    await s.app.setQuery('text');
    expect(s.app.searchHits).toMatchObject({ total: 1, cut: false });
    s.fake.searchTotal = 120;
    await s.app.setQuery('texts');
    expect(s.app.searchHits).toMatchObject({ total: 120, cut: true });
  });

  it('asks only for the last words when they come quickly', async () => {
    const s = await ready();
    let release = (): void => undefined;
    const gate = new Promise<void>((r) => (release = r));
    s.env.sleep = () => gate;
    const first = s.app.setQuery('fu');
    const second = s.app.setQuery('fun');
    release();
    await Promise.all([first, second]);
    const asked = s.fake.asked.filter((a) => a.path === '/api/rules/search').map((a) => a.query.get('q'));
    expect(asked).toEqual(['fun']);
  });
});

describe('the keyboard', () => {
  it('moves the cursor through the shown rules, opens, and closes the detail before the selection', async () => {
    const s = await ready();
    s.app.command({ name: 'go', arg: 'rules' });
    s.app.command({ name: 'next' });
    s.app.command({ name: 'next' });
    expect(s.app.selectedId).toBe('DEP-09');
    // A move reads no rule: each read is a question to the engine.
    expect(s.fake.count('/api/rules/rule')).toBe(0);
    s.app.command({ name: 'open' });
    await vi.waitFor(() => expect(s.app.detail.state).toBe('ok'));
    s.app.command({ name: 'palette' });
    s.app.command({ name: 'escape' });
    expect(s.app.paletteOpen).toBe(false);
    expect(s.app.detail.state).toBe('ok');
    s.app.command({ name: 'escape' });
    expect(s.app.detail.state).toBe('none');
    expect(s.app.selectedId).toBe('DEP-09');
    s.app.command({ name: 'escape' });
    expect(s.app.selectedId).toBeNull();
  });

  it('opens the rules of a family from the family table', async () => {
    const s = await ready();
    s.app.command({ name: 'go', arg: 'families' });
    s.app.command({ name: 'next' });
    s.app.command({ name: 'next' });
    expect(s.app.selectedFamily).toBe('ERR');
    s.app.command({ name: 'open' });
    expect(s.app.view).toBe('rules');
    expect(s.app.shown.map((r) => r.id)).toEqual(['ERR-100']);
  });

  it('sends the search key to the search box of the rules', async () => {
    const s = await ready();
    s.app.command({ name: 'search' });
    expect(s.app.view).toBe('rules');
    expect(s.app.searchFocus).toBe(1);
  });
});

describe('what the browser keeps', () => {
  it('is the token in the tab and the settings, never a rule', async () => {
    const s = await ready();
    s.app.setLang('de');
    s.app.setTheme('dark');
    s.app.setShortcuts(false);
    s.app.setConflictMode('single');
    await s.app.choose({ kind: 'entry', key: 'rule:LUA-01', label: 'x', hint: 'LUA-01', id: 'LUA-01', score: 1 });
    expect(s.app.detail).toMatchObject({ state: 'ok', id: 'LUA-01' });
    let ran = 0;
    const action = { key: 'rescan', label: 'Read again', run: () => void (ran += 1) };
    await s.app.choose({ kind: 'action', key: 'action:rescan', label: action.label, hint: '', action, score: 1 });
    expect(ran).toBe(1);
    expect([...s.session.items.keys()]).toEqual([TOKEN_KEY]);
    expect([...s.prefs.items.keys()].sort()).toEqual(Object.values(PREF_KEYS).sort());
    const kept = [...s.prefs.items.values(), ...s.session.items.values()].join('\n');
    for (const rule of snapshotDoc().rules) {
      expect(kept).not.toContain(rule.id);
      expect(kept).not.toContain(rule.source);
    }
    // A new page reads the settings back.
    const again = new App(s.env);
    expect([again.lang, again.theme, again.shortcuts, again.conflictMode]).toEqual(['de', 'dark', false, 'single']);
    expect(Object.keys(again.frecency)).toEqual(['action:rescan']);
  });
});

describe('opening the file of a rule', () => {
  it('asks the hub for the id and says nothing when it worked', async () => {
    const s = await ready();
    await s.app.openInEditor('DEP-01');
    const asked = s.fake.asked.filter((a) => a.path === '/api/rules/open');
    expect(asked).toHaveLength(1);
    expect(asked[0]?.method).toBe('POST');
    expect(s.app.notice).toBeNull();
  });

  it('says a hub without an editor cannot, and any other failure in its own word', async () => {
    const s = await ready();
    s.fake.openStatus = 404;
    await s.app.openInEditor('DEP-01');
    expect(s.app.notice?.key).toBe('notice.open_unavailable');
    s.fake.openStatus = 502;
    await s.app.openInEditor('DEP-01');
    expect(s.app.notice?.key).toBe('notice.open_failed');
  });
});
describe('the places of the history', () => {
  it('is the view and the rule that is open, and nothing of the filter or the cursor', async () => {
    const s = await ready();
    expect(s.app.navState()).toEqual({ view: 'overview', id: null });
    s.app.setView('rules');
    s.app.select('DEP-01');
    s.app.setFilter({ ...s.app.filter, severities: ['required'] });
    expect(s.app.navState()).toEqual({ view: 'rules', id: null });
    await s.app.openDetail('DEP-01');
    expect(s.app.navState()).toEqual({ view: 'rules', id: 'DEP-01' });
  });

  it('brings the page back to a view and closes or opens the rule as the place says', async () => {
    const s = await ready();
    await s.app.openDetail('DEP-01');
    s.app.restoreNav({ view: 'families', id: null });
    expect(s.app.view).toBe('families');
    expect(s.app.detail.state).toBe('none');
    s.app.restoreNav({ view: 'rules', id: 'DEP-01' });
    await vi.waitFor(() => expect(s.app.detail.state).toBe('ok'));
    expect(s.app.view).toBe('rules');
  });

  it('says once that a rule which is gone was left by the page itself, so its entry is replaced', async () => {
    const s = await ready();
    expect(s.app.takeNavReplace()).toBe(false);
    await s.app.openDetail('DEP-01');
    s.fake.dropRule('DEP-01');
    await s.app.onMessage({ event: 'rules', data: JSON.stringify({ changed_ids: [], removed: ['DEP-01'] }) });
    expect(s.app.detail.state).toBe('none');
    expect(s.app.takeNavReplace()).toBe(true);
    expect(s.app.takeNavReplace()).toBe(false);
  });

  it('takes only what the app wrote: a known view and an id of a sane length', () => {
    expect(isNavState({ view: 'rules', id: 'DEP-01' })).toBe(true);
    expect(isNavState({ view: 'overview', id: null })).toBe(true);
    for (const bad of [null, 'rules', {}, { view: 'settings', id: null }, { view: 'rules', id: 7 }, { view: 'rules', id: 'x'.repeat(101) }]) {
      expect(isNavState(bad), JSON.stringify(bad)).toBe(false);
    }
  });
});

describe('writing', () => {
  const critical = { op: 'set', id: 'DEP-01', patch: { severity: 'critical' }, if_match: 'sha256:5e6671e2dabdab83' };

  it('sends the operations as they are with a client id, says what was saved and offers the way back', async () => {
    const s = await ready();
    const outcome = await s.app.write([critical]);
    expect(outcome?.saved).toBe(1);
    expect(s.fake.opsRequests).toHaveLength(1);
    expect(s.fake.opsRequests[0]).toMatchObject({ schema: 1, dry_run: false, ops: [critical] });
    expect(s.app.notice).toMatchObject({ key: 'write.saved' });
    expect(s.app.notice?.undo).toEqual([
      { op: 'set', id: 'DEP-01', patch: { severity: 'recommended' }, if_match: 'sha256:after1' },
    ]);
    expect(s.app.writing).toBe(false);
  });

  it('asks the hub for the state it has after a write, and for the open rule again', async () => {
    const s = await ready();
    await s.app.openDetail('DEP-01');
    const snapshots = s.fake.count('/api/rules/snapshot');
    const rules = s.fake.count('/api/rules/rule');
    await s.app.write([critical]);
    expect(s.fake.count('/api/rules/snapshot')).toBe(snapshots + 1);
    expect(s.fake.count('/api/rules/rule')).toBe(rules + 1);
  });

  it('writes nothing for a dry run and leaves the notice alone', async () => {
    const s = await ready();
    const snapshots = s.fake.count('/api/rules/snapshot');
    const outcome = await s.app.write([critical], { dryRun: true });
    expect(s.fake.opsRequests[0]?.dry_run).toBe(true);
    expect(outcome?.saved).toBe(0);
    expect(s.app.notice).toBeNull();
    expect(s.fake.count('/api/rules/snapshot')).toBe(snapshots);
  });

  it('takes the Undo as a request of its own, without a way back for the undo, and says it was taken back', async () => {
    const s = await ready();
    await s.app.write([critical]);
    await s.app.undo();
    expect(s.fake.opsRequests).toHaveLength(2);
    expect(s.fake.opsRequests[1]?.ops).toEqual([
      { op: 'set', id: 'DEP-01', patch: { severity: 'recommended' }, if_match: 'sha256:after1' },
    ]);
    expect(s.app.notice).toEqual({ key: 'write.undone' });
  });

  it('keeps a conflict for the conflict center and says nothing was written', async () => {
    const s = await ready();
    s.fake.opsAnswer = () => ({
      status: 200,
      body: {
        kind: 'rules.result',
        dry_run: false,
        ok: false,
        results: [
          { n: 1, ok: false, op: 'set', id: 'DEP-01', changed_ids: [], error: { code: 'conflict', details: { expected: 'sha256:a', actual: 'sha256:b' } } },
        ],
      },
    });
    const snapshots = s.fake.count('/api/rules/snapshot');
    const outcome = await s.app.write([critical]);
    expect(outcome?.conflicts).toHaveLength(1);
    expect(s.app.conflict).toMatchObject({ id: 'DEP-01', expected: 'sha256:a', actual: 'sha256:b', op: critical });
    expect(s.app.notice).toEqual({ key: 'write.conflict', params: { id: 'DEP-01' } });
    expect(s.fake.count('/api/rules/snapshot')).toBe(snapshots);
    s.app.clearConflict();
    expect(s.app.conflict).toBeNull();
  });

  it('tells a refusal of the whole request in its own words', async () => {
    const s = await ready();
    for (const [status, code, key] of [
      [403, 'write_refused', 'write.refused'],
      [429, 'rate_limited', 'write.rate'],
      [503, 'engine_unavailable', 'write.unavailable'],
      [404, 'not_found', 'write.not_held'],
    ] as const) {
      s.fake.opsAnswer = () => ({ status, body: { error: { code, message: 'no' } } });
      expect(await s.app.write([critical])).toBeNull();
      expect(s.app.notice?.key, code).toBe(key);
    }
    s.fake.opsAnswer = () => ({ status: 400, body: { error: { code: 'bad_request', message: 'ops[1] has no op' } } });
    await s.app.write([critical]);
    expect(s.app.notice).toEqual({ key: 'write.failed', params: { message: 'ops[1] has no op' } });
  });

  it('sends one request at a time and nothing when there is nothing to do or the page is not ready', async () => {
    const s = await ready();
    expect(await s.app.write([])).toBeNull();
    const first = s.app.write([critical]);
    expect(await s.app.write([critical])).toBeNull();
    await first;
    expect(s.fake.opsRequests).toHaveLength(1);
    const cold = setup(false);
    expect(await cold.app.write([critical])).toBeNull();
    expect(cold.fake.opsRequests).toHaveLength(0);
  });

  it('closes the detail of a rule that was retired without asking the hub for it again', async () => {
    const s = await ready();
    await s.app.openDetail('DEP-01');
    s.fake.opsAnswer = () => {
      const doc = JSON.parse(s.fake.snapshotText) as { rules: { id: string }[] };
      doc.rules = doc.rules.filter((r) => r.id !== 'DEP-01');
      s.fake.snapshotText = JSON.stringify(doc);
      s.fake.etag = 'sha256:next';
      return {
        status: 200,
        body: { kind: 'rules.result', dry_run: false, ok: true, results: [{ n: 1, ok: true, op: 'retire', id: 'DEP-01', changed_ids: ['DEP-01'] }] },
      };
    };
    const asked = s.fake.count('/api/rules/rule');
    await s.app.write([{ op: 'retire', id: 'DEP-01', if_match: 'sha256:5e6671e2dabdab83' }]);
    expect(s.app.detail.state).toBe('none');
    expect(s.fake.count('/api/rules/rule')).toBe(asked);
    expect(s.app.notice).toMatchObject({ key: 'write.saved' });
  });

  it('reads the tag of a rule from the snapshot', async () => {
    const s = await ready();
    expect(s.app.etagOf('DEP-01')).toBe('sha256:5e6671e2dabdab83');
    expect(s.app.etagOf('NOPE-01')).toBeNull();
  });
});

describe('several rules at once and the conflict center', () => {
  // ERR-100 is a file of its own in the golden snapshot (the form the page writes); the DEP and LUA rules are documents.
  it('marks only rules the page may write, and sends one request for all of them', async () => {
    const s = await ready();
    s.app.toggleMark('ERR-100');
    s.app.toggleMark('DEP-01');
    expect(s.app.canWrite({ source: 'Checklists/DEP.md', etag_scope: 'file' })).toBe(false);
    s.app.marked = new Set(['ERR-100', 'DEP-01']);
    const outcome = await s.app.writeMarked({ severity: 'critical' });
    expect(outcome?.saved).toBe(1);
    expect(s.fake.opsRequests).toHaveLength(1);
    expect(s.fake.opsRequests[0]?.ops).toEqual([
      { op: 'set', id: 'ERR-100', patch: { severity: 'critical' }, if_match: 'sha256:4102a4b0a072760a' },
    ]);
    expect(s.app.marked.size).toBe(0);
  });

  it('asks for nothing when every marked rule is already so or may not be written', async () => {
    const s = await ready();
    s.app.marked = new Set(['ERR-100', 'DEP-01']);
    expect(await s.app.writeMarked({ severity: 'nice-to-have' })).toBeNull();
    expect(s.app.notice).toEqual({ key: 'bulk.none' });
    expect(s.fake.opsRequests).toHaveLength(0);
  });

  it('retires the marked rules in one request, each with the tag it saw', async () => {
    const s = await ready();
    s.app.marked = new Set(['ERR-100']);
    await s.app.writeMarked({ retire: true });
    expect(s.fake.opsRequests[0]?.ops).toEqual([{ op: 'retire', id: 'ERR-100', if_match: 'sha256:4102a4b0a072760a' }]);
  });

  it('forgets a mark when the rule leaves the rule set', async () => {
    const s = await ready();
    s.app.marked = new Set(['ERR-100']);
    const doc = JSON.parse(s.fake.snapshotText) as { rules: { id: string }[] };
    doc.rules = doc.rules.filter((r) => r.id !== 'ERR-100');
    s.fake.snapshotText = JSON.stringify(doc);
    s.fake.etag = 'sha256:gone';
    await s.app.reload();
    expect(s.app.marked.size).toBe(0);
  });

  const conflictAnswer = () => ({
    status: 200,
    body: {
      kind: 'rules.result',
      dry_run: false,
      ok: false,
      results: [
        { n: 1, ok: false, op: 'set', id: 'ERR-100', changed_ids: [], error: { code: 'conflict', details: { expected: 'sha256:a', actual: 'sha256:b' } } },
      ],
    },
  });
  const change = { op: 'set', id: 'ERR-100', patch: { severity: 'critical' }, if_match: 'sha256:a' };

  it('reads the rule as the file has it when a change clashes, for the conflict center', async () => {
    const s = await ready();
    s.fake.opsAnswer = conflictAnswer;
    await s.app.write([change]);
    await vi.waitFor(() => expect(s.app.conflictRule).not.toBeNull());
    expect(s.app.conflict?.id).toBe('ERR-100');
  });

  it('writes the same change onto the new version when the person says so, and only then', async () => {
    const s = await ready();
    s.fake.opsAnswer = conflictAnswer;
    await s.app.write([change]);
    expect(s.fake.opsRequests).toHaveLength(1);
    s.fake.opsAnswer = (request) => ({
      status: 200,
      body: { kind: 'rules.result', dry_run: request.dry_run, ok: true, results: [{ n: 1, ok: true, op: 'set', id: 'ERR-100', changed_ids: ['ERR-100'] }] },
    });
    await s.app.resolveConflict('retry');
    expect(s.fake.opsRequests).toHaveLength(2);
    expect(s.fake.opsRequests[1]?.ops).toEqual([{ ...change, if_match: 'sha256:b' }]);
    expect(s.app.conflict).toBeNull();
    expect(s.app.notice?.key).toBe('write.saved');
  });

  it('writes nothing when the person looks at the rule or drops the change', async () => {
    const s = await ready();
    s.fake.opsAnswer = conflictAnswer;
    await s.app.write([change]);
    await s.app.resolveConflict('view');
    expect(s.app.conflict).toBeNull();
    expect(s.app.view).toBe('rules');
    expect(s.app.detail.state === 'none' ? null : s.app.detail.id).toBe('ERR-100');
    await s.app.write([change]);
    await s.app.resolveConflict('drop');
    expect(s.app.conflict).toBeNull();
    expect(s.fake.opsRequests).toHaveLength(2);
  });

  it('brings up the next conflict of a request after the first is dealt with', async () => {
    const s = await ready();
    s.fake.opsAnswer = () => ({
      status: 200,
      body: {
        kind: 'rules.result',
        dry_run: false,
        ok: false,
        results: ['ERR-100', 'DEP-01'].map((id, i) => ({ n: i + 1, ok: false, op: 'set', id, changed_ids: [], error: { code: 'conflict', details: { expected: 'a', actual: 'b' } } })),
      },
    });
    await s.app.write([change, { ...change, id: 'DEP-01' }]);
    expect(s.app.conflicts.map((c) => c.id)).toEqual(['ERR-100', 'DEP-01']);
    s.app.clearConflict();
    expect(s.app.conflict?.id).toBe('DEP-01');
  });
});

describe('the conflicts, all at once or one at a time', () => {
  const two = () => ({
    status: 200,
    body: {
      kind: 'rules.result',
      dry_run: false,
      ok: false,
      results: ['ERR-100', 'DEP-01'].map((id, i) => ({ n: i + 1, ok: false, op: 'set', id, changed_ids: [], error: { code: 'conflict', details: { expected: 'a', actual: 'b' } } })),
    },
  });
  const change = { op: 'set', id: 'ERR-100', patch: { severity: 'critical' }, if_match: 'a' };

  it('shows every conflict by default, and keeps the choice of the person', async () => {
    const s = await ready();
    expect(s.app.conflictMode).toBe('list');
    s.app.setConflictMode('single');
    expect(s.prefs.items.get(PREF_KEYS.conflictMode)).toBe('single');
    s.prefs.items.set(PREF_KEYS.conflictMode, '<img src=x>');
    expect(new App(s.env).conflictMode).toBe('list');
  });

  it('reads the rule of every conflict, so the list compares each one', async () => {
    const s = await ready();
    s.fake.opsAnswer = two;
    await s.app.write([change, { ...change, id: 'DEP-01' }]);
    await vi.waitFor(() => expect(Object.keys(s.app.conflictRules).sort()).toEqual(['DEP-01', 'ERR-100']));
  });

  it('deals with the conflict of its key and leaves the others, and the first stays first by default', async () => {
    const s = await ready();
    s.fake.opsAnswer = two;
    await s.app.write([change, { ...change, id: 'DEP-01' }]);
    const second = conflictKey(s.app.conflicts[1] as Conflict);
    await s.app.resolveConflict('drop', second);
    expect(s.app.conflicts.map((c) => c.id)).toEqual(['ERR-100']);
    // A key that is not there does nothing.
    await s.app.resolveConflict('drop', second);
    expect(s.app.conflicts).toHaveLength(1);
    s.app.clearConflict(conflictKey(s.app.conflicts[0] as Conflict));
    expect(s.app.conflicts).toHaveLength(0);
    expect(s.app.conflictRule).toBeNull();
  });

  it('keeps the other conflicts of the list when one is written again and goes through', async () => {
    const s = await ready();
    s.fake.opsAnswer = two;
    await s.app.write([change, { ...change, id: 'DEP-01' }]);
    s.fake.opsAnswer = (request) => ({
      status: 200,
      body: { kind: 'rules.result', dry_run: request.dry_run, ok: true, results: [{ n: 1, ok: true, op: 'set', id: 'DEP-01', changed_ids: ['DEP-01'] }] },
    });
    await s.app.resolveConflict('retry', conflictKey(s.app.conflicts[1] as Conflict));
    expect(s.app.conflicts.map((c) => c.id)).toEqual(['ERR-100']);
  });

  it('lets every conflict go at once', async () => {
    const s = await ready();
    s.fake.opsAnswer = two;
    await s.app.write([change, { ...change, id: 'DEP-01' }]);
    s.app.clearAllConflicts();
    expect(s.app.conflicts).toHaveLength(0);
    expect(s.app.conflictRules).toEqual({});
    expect(s.app.notice).toBeNull();
    // A rule that is read after its conflict went is not kept.
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 10));
    expect(s.app.conflictRules).toEqual({});
  });
});

describe('the waivers', () => {
  const waive = { op: 'waive', id: 'DEP-01', reason: 'r', if_match: 'sha256:c23dd7021c9336a8' };

  it('reads the waivers of the project and keeps the tag of the file', async () => {
    const s = await ready();
    await s.app.loadWaivers();
    expect(s.app.waiversState).toBe('ready');
    expect(s.app.waivers?.items.map((w) => w.id)).toEqual(['DEP-01', 'DEP-09', 'OLD-01']);
    expect(s.app.waiversEtag()).toBe('sha256:c23dd7021c9336a8');
  });

  it('says there is no project when the person set none, and when the file cannot be used', async () => {
    const s = await ready();
    s.fake.waiversAnswer = () => ({ status: 409, body: { error: { code: 'no_project', message: 'no' } } });
    await s.app.loadWaivers();
    expect(s.app.waiversState).toBe('no_project');
    expect(s.app.waivers).toBeNull();
    s.fake.waiversAnswer = () => ({ status: 400, body: { error: { code: 'invalid_argument', message: 'no' } } });
    await s.app.loadWaivers();
    expect(s.app.waiversState).toBe('failed');
  });

  it('reads the file again after a change of it, with the tag it has then', async () => {
    const s = await ready();
    await s.app.loadWaivers();
    const reads = s.fake.count('/api/rules/waivers');
    await s.app.write([waive]);
    expect(s.fake.opsRequests[0]?.ops).toEqual([waive]);
    expect(s.fake.count('/api/rules/waivers')).toBe(reads + 1);
  });

  it('says the waivers file clashed, not a rule, and reads it again when the person looks', async () => {
    const s = await ready();
    await s.app.loadWaivers();
    s.fake.opsAnswer = () => ({
      status: 200,
      body: {
        kind: 'rules.result',
        dry_run: false,
        ok: false,
        results: [{ n: 1, ok: false, op: 'waive', id: 'DEP-01', changed_ids: [], error: { code: 'conflict', details: { expected: 'a', actual: 'b' } } }],
      },
    });
    const rules = s.fake.count('/api/rules/rule');
    await s.app.write([waive]);
    expect(s.app.notice).toEqual({ key: 'write.conflict_waivers' });
    // No rule is read for the center: the clash is about the file of the project.
    expect(s.fake.count('/api/rules/rule')).toBe(rules);
    const reads = s.fake.count('/api/rules/waivers');
    await s.app.resolveConflict('view');
    expect(s.app.view).toBe('waivers');
    expect(s.fake.count('/api/rules/waivers')).toBe(reads + 1);
    // It is not offered to write it again onto the new version: that is for the rules.
    expect(s.app.conflict).toBeNull();
  });

  it('starts a waiver for a rule in the view of the waivers', async () => {
    const s = await ready();
    s.app.startWaiver('DEP-01');
    expect(s.app.view).toBe('waivers');
    expect(s.app.waiveDraft).toBe('DEP-01');
  });
});

describe('the data classes', () => {
  const loosen = { key: 'checklists', class: 'public' as const };
  const needsWord = (body: Record<string, unknown>) =>
    body['confirm_loosen'] === true
      ? { status: 200, body: { ok: true } }
      : { status: 409, body: { error: { code: 'confirm_needed', message: 'no' } } };

  it('sends the change, reads the classes again and says it is saved', async () => {
    const s = await ready();
    const reads = s.fake.count('/api/rules/sources');
    expect(await s.app.setClass({ key: 'checklists', class: 'private' })).toBe(true);
    expect(s.fake.classRequests).toEqual([{ key: 'checklists', class: 'private' }]);
    expect(s.fake.count('/api/rules/sources')).toBe(reads + 1);
    expect(s.app.notice).toEqual({ key: 'classes.saved' });
    expect(s.app.classConfirm).toBeNull();
  });

  it('keeps a change that releases more and asks, without reading or saying saved', async () => {
    const s = await ready();
    s.fake.classAnswer = needsWord;
    const reads = s.fake.count('/api/rules/sources');
    expect(await s.app.setClass(loosen)).toBe(false);
    expect(s.app.classConfirm).toEqual(loosen);
    expect(s.fake.count('/api/rules/sources')).toBe(reads);
    expect(s.app.notice).not.toEqual({ key: 'classes.saved' });
  });

  it('sends it again with the word of the person when they say yes', async () => {
    const s = await ready();
    s.fake.classAnswer = needsWord;
    await s.app.setClass(loosen);
    expect(await s.app.confirmClass()).toBe(true);
    expect(s.fake.classRequests).toEqual([loosen, { ...loosen, confirm_loosen: true }]);
    expect(s.app.classConfirm).toBeNull();
    expect(s.app.notice).toEqual({ key: 'classes.saved' });
  });

  it('sends nothing more when the person says no', async () => {
    const s = await ready();
    s.fake.classAnswer = needsWord;
    await s.app.setClass(loosen);
    s.app.cancelClass();
    await s.app.confirmClass();
    expect(s.app.classConfirm).toBeNull();
    expect(s.fake.classRequests).toEqual([loosen]);
  });

  it('says it failed on any other refusal and keeps no question open', async () => {
    const s = await ready();
    s.fake.classAnswer = () => ({ status: 400, body: { error: { code: 'bad_request', message: 'no' } } });
    expect(await s.app.setClass(loosen)).toBe(false);
    expect(s.app.notice).toEqual({ key: 'classes.failed' });
    expect(s.app.classConfirm).toBeNull();
  });
});

describe('the rule sets of the vault', () => {
  const made = {
    status: 200,
    body: {
      kind: 'rules.result',
      dry_run: false,
      ok: true,
      results: [
        {
          n: 1,
          ok: true,
          op: 'create_ruleset',
          ruleset: 'beta',
          changed_ids: [],
          outcome: 'created',
          inverse: { op: 'archive_ruleset', name: 'beta' },
        },
      ],
    },
  };

  it('reads the rule sets and the archive', async () => {
    const s = await ready();
    await s.app.loadRulesets();
    expect(s.app.rulesets?.items.map((i) => i.folder)).toEqual(['Checklists']);
    s.fake.rulesetsAnswer = () => ({ status: 500, body: { error: { code: 'internal', message: 'no' } } });
    await s.app.loadRulesets();
    expect(s.app.rulesets).toBeNull();
  });

  it('counts a made rule set as saved although it names no rule, reads the list again and offers Undo', async () => {
    const s = await ready();
    s.fake.opsAnswer = () => made;
    const reads = s.fake.count('/api/rules/rulesets');
    expect(await s.app.makeRuleset('beta')).toBe(true);
    expect(s.fake.opsRequests[0]?.ops).toEqual([{ op: 'create_ruleset', name: 'beta' }]);
    expect(s.fake.count('/api/rules/rulesets')).toBe(reads + 1);
    expect(s.app.notice?.key).toBe('write.saved');
    expect(s.app.notice?.undo).toEqual([{ op: 'archive_ruleset', name: 'beta' }]);
  });

  it('asks before a rule set goes to the archive, and sends nothing when the person says no', async () => {
    const s = await ready();
    s.app.askArchive('Checklists');
    expect(s.app.archiveAsk).toBe('Checklists');
    s.app.cancelArchive();
    expect(await s.app.archiveRuleset()).toBe(false);
    expect(s.fake.opsRequests).toEqual([]);
    s.app.askArchive('Checklists');
    expect(await s.app.archiveRuleset()).toBe(true);
    expect(s.fake.opsRequests[0]?.ops).toEqual([{ op: 'archive_ruleset', name: 'Checklists' }]);
    expect(s.app.archiveAsk).toBeNull();
  });

  it('says a name that is taken is a refusal, not a clash of versions', async () => {
    const s = await ready();
    s.fake.opsAnswer = () => ({
      status: 200,
      body: {
        kind: 'rules.result',
        dry_run: false,
        ok: false,
        results: [{ n: 1, ok: false, op: 'create_ruleset', changed_ids: [], error: { code: 'conflict', message: 'taken' } }],
      },
    });
    expect(await s.app.makeRuleset('beta')).toBe(false);
    expect(s.app.conflicts).toEqual([]);
    expect(s.app.notice?.key).toBe('write.failed');
  });
});

describe('the list of rule sets follows the snapshot', () => {
  it('is dropped when a new snapshot arrives, so that the view reads it again', async () => {
    const s = await ready();
    await s.app.loadRulesets();
    expect(s.app.rulesets).not.toBeNull();
    s.fake.dropRule('LUA-01');
    await s.app.refresh();
    expect(s.app.rulesets).toBeNull();
  });

  it('is kept when the snapshot did not change', async () => {
    const s = await ready();
    await s.app.loadRulesets();
    await s.app.refresh();
    expect(s.app.rulesets).not.toBeNull();
  });
});
