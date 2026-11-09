// The state of the page and what keeps it true: pairing, the held snapshot and the classes of its files, the event
// stream, the detail of the open rule and the search of the texts. Nothing of a rule set is kept in the browser:
// `prefs` holds the language, the theme, the single-key switch and the frecency of actions, `session` the token.

import { takePairingCode, type HistoryLike, type LocationLike } from 'appkit-ui/pair';
import type { Route } from 'appkit-ui/route';
import { Preferences, StreamSession } from 'appkit-ui/session';
import type { SseMessage } from 'appkit-ui/sse';
import { ApiError, type ClassChange, type Hub, type HubSources } from './api';
import type { Groupings, Rule, RuleSummary, Rulesets, Snapshot, Views, Waivers } from './contract';
import { isViewOp } from './views';
import { familyRows, type FamilyRow } from './families';
import { applyFilter, emptyFilter, moveSelection, type Filter } from './filter';
import { LANGS, type Key, type Lang } from './i18n';
import { DEFAULT_CONFLICT_MODE, parseConflictMode, type ConflictMode } from 'appkit-ui/conflicts';
import { bumped, parseFrecency, type Frecency, type PaletteItem } from 'appkit-ui/palette';
import type { Command } from './keys';
import { classOf, sourceIndex } from './sources';
import { retryOp } from './conflicts';
import type { Body } from './edit';
import {
  archiveRulesetOp,
  clientOpId,
  createRulesetOp,
  isRulesetOp,
  isWaiverOp,
  nextOf,
  restoreRulesetOp,
  retireOp,
  setOp,
  summarize,
  type Conflict,
  type Op,
  type Outcome,
} from './write';

export type Phase = 'starting' | 'needs_pairing' | 'offline' | 'unavailable' | 'ready';
export type Connection = 'live' | 'reconnecting' | 'offline';
export type View = 'overview' | 'families' | 'rules' | 'views' | 'coverage' | 'waivers' | 'sources';
export type Theme = 'auto' | 'light' | 'dark';

export const VIEWS: readonly View[] = ['overview', 'families', 'rules', 'views', 'coverage', 'waivers', 'sources'];

/**
 * A place the person can go back to: the view and the rule that is open in it. Filters, the search and the cursor are
 * not places (typing a letter must not make an entry). The place is also in the fragment of the address
 * (`#view=rules&rule=<id>`, appkit-ui/nav): a reload, a copied address and a link opened in a new tab show it.
 */
export interface NavState {
  view: View;
  id: string | null;
}

/** The longest rule id a place takes. */
const MAX_RULE_ID = 100;

export function isNavState(value: unknown): value is NavState {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as { view?: unknown; id?: unknown };
  return VIEWS.includes(v.view as View) && (v.id === null || (typeof v.id === 'string' && v.id.length <= MAX_RULE_ID));
}

/** The route of a place: its view, and the rule open in it. */
export function navRoute(place: NavState): Route {
  return place.id === null ? { view: place.view } : { view: place.view, rule: place.id };
}

/** The place a route names, or null (no view, a name that is no view). A rule id that cannot be one is left out. */
export function routeNav(route: Record<string, string>): NavState | null {
  const view = route.view;
  if (view === undefined || !VIEWS.includes(view as View)) return null;
  const id = route.rule;
  return { view: view as View, id: id !== undefined && id.length <= MAX_RULE_ID ? id : null };
}

export type Detail =
  | { state: 'none' }
  | { state: 'loading'; id: string }
  | { state: 'ok'; id: string; doc: Rule }
  | { state: 'failed'; id: string };

/** The key of a conflict: the place of its operation in the request and the rule, unique within one answer. */
export function conflictKey(conflict: Conflict): string {
  return `${conflict.n}:${conflict.id}`;
}

/** One line for the toast: a key of the dictionary and its values. */
export interface Notice {
  key: Key;
  params?: Record<string, string>;
  /** The operations that take back what the notice reports: the toast offers them as Undo. */
  undo?: Op[];
}

/** What the page needs from its surroundings; each part is replaced in the tests. */
export interface Env {
  hub: Hub;
  /** Per-tab storage: the session token only. */
  session: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
  /** Per-browser storage: language, theme, the single-key switch, the frecency of actions. Never a rule. */
  prefs: Pick<Storage, 'getItem' | 'setItem'>;
  preferredLanguages: readonly string[];
  fetchFn: typeof fetch;
  sleep(ms: number): Promise<void>;
  now(): number;
  /**
   * Ask the open tabs of the page for their session (appkit-ui/handoff): a tab the browser opened from a link has none
   * of its own. The token, or null when no tab answered.
   */
  askSession?(onLate: (token: string) => void): Promise<string | null>;
}

export const TOKEN_KEY = 'rules-desktop.session';
export const PREF_KEYS = {
  lang: 'rules-desktop.lang',
  theme: 'rules-desktop.theme',
  shortcuts: 'rules-desktop.shortcuts',
  frecency: 'rules-desktop.frecency',
  conflictMode: 'rules-desktop.conflict-mode',
} as const;

/** The pause before the text of the rules is searched for the words typed, and the fewest letters searched for. */
export const SEARCH_DELAY_MS = 200;
export const MIN_QUERY_CHARS = 2;
/** A window that gets the focus back asks for a scan at most this often. */
export const RESCAN_EVERY_MS = 5000;
/** Waits before the hub is asked again: when it did not answer, and when its engine was not ready. */
export const OFFLINE_RETRY_MS = 3000;
export const UNAVAILABLE_RETRY_MS = 5000;
/** A search the hub answers with `building` (it is still reading the texts) is asked again, as in tasks. */
export const SEARCH_RETRY_MS = 1500;
export const SEARCH_RETRIES = 8;
/** A pairing code the hub turned away for too many attempts (429) is still good: it is traded again a few times. */
export const EXCHANGE_RETRIES = 3;
/**
 * The open rule left the snapshot: the page waits this long and reads again before it asks for it. The hub answers
 * 404 for an id it does not hold (K8), and a retire or a file saved half way leaves a rule out for one state.
 */
export const GONE_SETTLE_MS = 2000;

function safe<T>(work: () => T): T | null {
  try {
    return work();
  } catch {
    return null;
  }
}

/** The hits of a text search the list uses; `cut`: the hub found the words in more rules than it named. */
export interface SearchHits {
  query: string;
  ids: ReadonlySet<string>;
  status: 'ready' | 'building' | 'failed';
  /** How many rules hold the words, as the hub counted them. */
  total: number;
  cut: boolean;
}

/** The ids of an event of the stream; anything that is not a list of strings is no id. */
function ids(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : [];
}

export class App {
  phase = $state<Phase>('starting');
  /** The engine's problem while it is not ready (a word of the hub). */
  problem = $state<string | null>(null);
  /** The pairing code of the address was refused and the tab has no session. */
  pairRefused = $state(false);
  view = $state<View>('overview');
  /** What the person chose for the whole page (the kit's `Preferences`): the language, the theme, the single keys. */
  readonly prefs: Preferences<Lang>;
  /** The connection to the hub, and the event stream (the kit's `StreamSession`). */
  readonly stream = new StreamSession();
  get lang(): Lang {
    return this.prefs.lang;
  }
  get theme(): Theme {
    return this.prefs.theme;
  }
  get shortcuts(): boolean {
    return this.prefs.shortcuts;
  }
  get connection(): Connection {
    return this.stream.connection;
  }
  set connection(value: Connection) {
    this.stream.connection = value;
  }

  snapshot = $state.raw<Snapshot | null>(null);
  etag = $state<string | null>(null);
  sources = $state.raw<HubSources | null>(null);
  filter = $state.raw<Filter>(emptyFilter());
  selectedId = $state<string | null>(null);
  /** The family the cursor of the family table is on. */
  selectedFamily = $state<string | null>(null);
  detail = $state.raw<Detail>({ state: 'none' });
  /** The ids the hub found the words of the filter in, with the state of its index; `null`: no text search. */
  searchHits = $state.raw<SearchHits | null>(null);
  paletteOpen = $state(false);
  notice = $state.raw<Notice | null>(null);
  /** Whether the hub starts at login; null: nothing to switch here. */
  autostart = $state<boolean | null>(null);
  /** Goes up by one each time the search box should take the focus (`/`). */
  searchFocus = $state(0);
  frecency = $state.raw<Frecency>({});

  /** The page was stopped (the kit's `StreamSession` knows). */
  private get stopped(): boolean {
    return this.stream.stopped;
  }
  /** The event stream is open; only the stream says `live`. */
  private get streamOpen(): boolean {
    return this.stream.open;
  }
  private detailAbort: AbortController | null = null;
  private searchAbort: AbortController | null = null;
  private searchSeq = 0;
  private lastRescan = -Infinity;
  /** The read in progress; a change while it runs makes exactly one more. */
  private loop: Promise<void> | null = null;
  private dirty = false;

  constructor(private readonly env: Env) {
    this.prefs = new Preferences<Lang>(env.prefs, PREF_KEYS, LANGS, env.preferredLanguages, 'en');
    // Only actions are remembered: a key that is not one is not read back, so a rule id never comes back from the profile.
    this.frecency = parseFrecency(safe(() => env.prefs.getItem(PREF_KEYS.frecency)), ['action:']);
    this.conflictMode = parseConflictMode(safe(() => env.prefs.getItem(PREF_KEYS.conflictMode)));
  }

  // ── derived ───────────────────────────────────────────────────────────

  get rules(): readonly RuleSummary[] {
    return this.snapshot?.rules ?? [];
  }

  /** The rules the list shows: the filter, with the hits of the text search when they are for the words typed. */
  shown = $derived.by(() => {
    const hits = this.searchHits;
    const current = hits && hits.query === this.filter.query.trim() ? hits.ids : null;
    return applyFilter(this.snapshot?.rules ?? [], this.filter, current);
  });

  families = $derived.by<FamilyRow[]>(() => familyRows(this.snapshot?.families ?? []));

  private index = $derived.by(() => sourceIndex(this.sources));

  /** The class of a rule, by its file as the hub classified it. */
  classOfRule(rule: Pick<RuleSummary, 'source'>): ReturnType<typeof classOf> {
    return classOf(this.index, rule.source);
  }

  // ── settings ──────────────────────────────────────────────────────────

  setLang(lang: Lang): void {
    this.prefs.setLang(lang);
  }

  setTheme(theme: Theme): void {
    this.prefs.setTheme(theme);
  }

  setShortcuts(on: boolean): void {
    this.prefs.setShortcuts(on);
  }

  setView(view: View): void {
    this.view = view;
  }

  // ── start ─────────────────────────────────────────────────────────────

  /** Bring the page up from its address: a pairing code in the fragment is taken out of it at once and traded. */
  startFrom(location: LocationLike, history: HistoryLike): Promise<void> {
    return this.start(takePairingCode(location, history, undefined, { keepRoute: true }));
  }

  /** The rule the address named open in its view, waiting for the session (`openPlace`). */
  private placeId: string | null = null;

  /** The address named a place (a reload, a link opened in a new tab): the page opens on it once it is paired. */
  openPlace(place: NavState): void {
    this.view = place.view;
    this.placeId = place.id;
  }

  /** Bring the page up. `code` is a pairing code taken from the address. */
  async start(code: string | null): Promise<void> {
    this.stream.resume();
    this.phase = 'starting';
    this.pairRefused = false;
    let token = safe(() => this.env.session.getItem(TOKEN_KEY));
    if (code) {
      for (let attempt = 0; ; attempt++) {
        try {
          const traded = await this.env.hub.exchange(code);
          token = traded;
          safe(() => this.env.session.setItem(TOKEN_KEY, traded));
          break;
        } catch (error) {
          // A code that is spent or too old (401, the one answer of the hub for both): a session the tab has is still
          // good. Only that refusal says the code is bad.
          if (error instanceof ApiError && error.status === 401) {
            this.pairRefused = token === null;
            break;
          }
          // Too many attempts: the hub did not look at the code, which is still good but no longer in the address.
          const limited = error instanceof ApiError && error.status === 429;
          if (limited && attempt < EXCHANGE_RETRIES) {
            await this.env.sleep(OFFLINE_RETRY_MS);
            if (this.stopped) return;
            continue;
          }
          if (token !== null) break;
          if (limited) {
            this.phase = 'needs_pairing';
            this.notice = { key: 'notice.pair_rate_limited' };
          } else {
            this.phase = 'offline';
          }
          return;
        }
      }
    }
    // A tab without a session and without a code (opened from a link) takes the session of an open tab of the page.
    if (token === null && !code && this.env.askSession) {
      token = await this.env.askSession((late) => this.takeLateSession(late));
      if (this.stopped) return;
      if (token !== null) {
        const taken = token;
        safe(() => this.env.session.setItem(TOKEN_KEY, taken));
      }
    }
    if (token === null) {
      this.phase = 'needs_pairing';
      return;
    }
    this.env.hub.setToken(token);
    this.env.hub.onUnauthorized = () => this.lostSession();

    for (;;) {
      try {
        const hello = await this.env.hub.hello();
        if (hello.engine.ready) break;
        this.problem = hello.engine.problem ?? 'unknown';
        this.phase = 'unavailable';
        await this.env.sleep(UNAVAILABLE_RETRY_MS);
      } catch {
        this.connection = 'offline';
        this.phase = 'offline';
        await this.env.sleep(OFFLINE_RETRY_MS);
      }
      if (this.stopped) return;
    }
    this.problem = null;
    // The rule of the place the address named is read beside the list: the page is at that place from its first
    // picture on (the history records it as the entry it started on).
    const place = this.placeId;
    this.placeId = null;
    if (place !== null) void this.openDetail(place);
    await this.refresh();
    if (this.stopped || !this.env.hub.hasToken) return;
    void this.followStream();
  }

  /** An open tab answered after the wait, so this tab offers to pair: it takes the session, unless it has paired since. */
  private takeLateSession(token: string): void {
    if (this.stopped || this.phase !== 'needs_pairing') return;
    safe(() => this.env.session.setItem(TOKEN_KEY, token));
    void this.start(null);
  }

  stop(): void {
    this.stream.stop();
    this.detailAbort?.abort();
    this.searchAbort?.abort();
  }

  private lostSession(): void {
    safe(() => this.env.session.removeItem(TOKEN_KEY));
    this.env.hub.setToken(null);
    this.stream.cut();
    this.detailAbort?.abort();
    this.phase = 'needs_pairing';
    this.connection = 'offline';
    this.snapshot = null;
    this.etag = null;
    this.sources = null;
    this.searchHits = null;
    this.selectedId = null;
    this.detail = { state: 'none' };
    this.paletteOpen = false;
  }

  // ── reading ───────────────────────────────────────────────────────────

  /**
   * Read the snapshot again, with the tag of the one held. One at a time: a change that arrives while a read runs
   * makes exactly one more read after it, and a caller then waits for that one too.
   */
  refresh(): Promise<void> {
    if (this.loop) {
      this.dirty = true;
      return this.loop;
    }
    this.loop = (async () => {
      try {
        do {
          this.dirty = false;
          await this.readOnce();
        } while (this.dirty && !this.stopped);
      } finally {
        this.loop = null;
      }
    })();
    return this.loop;
  }

  private async readOnce(): Promise<void> {
    const hub = this.env.hub;
    try {
      const result = await hub.snapshot(this.snapshot ? this.etag : null);
      if (result.kind === 'fresh') {
        this.snapshot = result.doc;
        this.etag = result.etag;
        // The classes are of the files of a snapshot: a new snapshot may have other files. So may the vault: a rule set
        // moved to the archive or made elsewhere is read again by the view that shows them.
        this.sources = null;
        this.rulesets = null;
      }
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return;
      if (error instanceof ApiError && error.code === 'engine_unavailable') {
        this.phase = 'unavailable';
        this.notice = { key: 'notice.engine_gone' };
        return;
      }
      if (this.snapshot === null) this.phase = 'offline';
      // The hub did not answer this read; whether the stream is open is the stream's to say.
      if (!this.streamOpen) this.connection = 'offline';
      return;
    }
    // The rules do not wait for their classes: without them every rule is unclassified (fail-closed), and the next
    // read asks again.
    if (this.sources === null) await this.readSources();
    if (!hub.hasToken) return;
    this.phase = 'ready';
    this.connection = this.streamOpen ? 'live' : 'reconnecting';
    this.keepSelection();
  }

  /** The switch of the menu: says what came of it, and shows what the system says afterwards. */
  async toggleAutostart(): Promise<void> {
    if (this.autostart === null) return;
    const wanted = !this.autostart;
    try {
      const now = await this.env.hub.setAutostart(wanted);
      this.autostart = now;
      this.notice = { key: now !== wanted ? 'notice.autostart_failed' : wanted ? 'notice.autostart_on' : 'notice.autostart_off' };
    } catch {
      this.notice = { key: 'notice.autostart_failed' };
    }
  }

  /** Once per page: each read starts a program on the machine of the hub, and the entry changes only through the switch. */
  #autostartRead = false;
  private async readAutostart(): Promise<void> {
    if (this.#autostartRead) return;
    this.#autostartRead = true;
    try {
      this.autostart = await this.env.hub.autostart();
    } catch {
      this.autostart = null;
    }
  }

  private async readSources(): Promise<void> {
    void this.readAutostart();
    try {
      this.sources = await this.env.hub.sources();
    } catch (error) {
      this.sources = null;
      if (error instanceof ApiError && error.status === 401) return;
      if (this.notice?.key !== 'notice.sources_failed') this.notice = { key: 'notice.sources_failed' };
    }
  }

  /** A selected rule that left the list is not selected any more; the open detail is the stream's business. */
  private keepSelection(): void {
    if (this.selectedId !== null && !this.rules.some((r) => r.id === this.selectedId)) this.selectedId = null;
    if (this.marked.size > 0 && [...this.marked].some((id) => !this.rules.some((r) => r.id === id))) {
      this.marked = new Set([...this.marked].filter((id) => this.rules.some((r) => r.id === id)));
    }
    if (this.selectedFamily !== null && !this.families.some((f) => f.family === this.selectedFamily)) {
      this.selectedFamily = null;
    }
  }

  // ── the stream ────────────────────────────────────────────────────────

  private followStream(): Promise<void> {
    return this.stream.follow({
      fetchFn: this.env.fetchFn,
      url: () => this.env.hub.eventsUrl,
      bearer: () => this.env.hub.bearer,
      hasToken: () => this.env.hub.hasToken,
      sleep: (ms) => this.env.sleep(ms),
      // What happened while the stream was down.
      onOpen: () => void this.refresh(),
      onMessage: (m) => void this.onMessage(m),
      onLost: () => this.lostSession(),
    });
  }

  /** One event of the stream. Returns when what it asked for is read (the tests wait for it). */
  async onMessage(message: SseMessage): Promise<void> {
    if (message.event === 'rules') {
      let doc: Record<string, unknown> = {};
      try {
        const parsed: unknown = JSON.parse(message.data);
        if (parsed !== null && typeof parsed === 'object') doc = parsed as Record<string, unknown>;
      } catch {
        doc = { truncated: true };
      }
      // Always the snapshot: the ids follow the etag of the file, and a change of the gates alone comes with empty
      // lists. The tag makes it cheap when nothing changed.
      await this.refresh();
      // A layout or a rule that moved changes what the views show.
      if (this.view === 'views') await this.loadViews();
      const open = this.detail.state === 'none' ? null : this.detail.id;
      if (open === null) return;
      if (ids(doc['removed']).includes(open)) {
        await this.settleGone(open);
      } else if (ids(doc['changed_ids']).includes(open) || doc['truncated'] === true) {
        await this.loadDetail(open);
      }
      return;
    }
    if (message.event === 'resync') {
      // The stream lost events: everything is read again.
      this.sources = null;
      await this.refresh();
      if (this.detail.state !== 'none') await this.loadDetail(this.detail.id);
      return;
    }
    if (message.event === 'hello') {
      // Where the stream stands: another revision than ours came between our read and the stream.
      try {
        const doc = JSON.parse(message.data) as { rev?: unknown };
        if (typeof doc.rev === 'string' && this.snapshot && doc.rev !== this.snapshot.rev) await this.refresh();
      } catch {
        await this.refresh();
      }
    }
  }

  /**
   * The open rule is not in the new snapshot. It is not closed at once: a retire or a file saved half way leaves it
   * out for one state, and the hub answers 404 for any id it does not hold (K8), so asking at once would close it
   * every time. The page waits a moment and reads the snapshot again, then asks for the rule once; only a 404 closes
   * the detail, with a word why (`loadDetail`).
   */
  private async settleGone(id: string): Promise<void> {
    await this.env.sleep(GONE_SETTLE_MS);
    if (this.stopped || !this.isOpen(id)) return;
    await this.refresh();
    if (this.isOpen(id)) await this.loadDetail(id);
  }

  /** The hub does not hold the open rule any more: the detail closes with a word why. */
  private closeGone(id: string): void {
    // The page left the place by itself: the history entry of that rule is replaced, not followed by a new one
    // (Back would lead to the entry of a rule that is gone and the page would leave it again at once).
    this.navReplace = true;
    this.detail = { state: 'none' };
    if (this.selectedId === id) this.selectedId = null;
    this.notice = { key: 'detail.gone', params: { id } };
  }

  /** The window got the focus back: ask for a scan, at most every few seconds, then read. */
  async focused(): Promise<void> {
    if (this.phase !== 'ready') return;
    const now = this.env.now();
    if (now - this.lastRescan < RESCAN_EVERY_MS) return;
    this.lastRescan = now;
    try {
      await this.env.hub.rescan();
    } catch {
      // A hub that is gone shows itself through the stream.
    }
    await this.refresh();
  }

  /** Read again on request (the menu, the palette): a scan of the hub, then the snapshot, without the pause. */
  async reload(): Promise<void> {
    if (this.phase !== 'ready') return;
    this.lastRescan = this.env.now();
    try {
      await this.env.hub.rescan();
    } catch {
      // A hub that is gone shows itself through the stream.
    }
    await this.refresh();
  }

  // ── filter and search ─────────────────────────────────────────────────

  setFilter(filter: Filter): void {
    const before = this.filter.query;
    this.filter = filter;
    if (filter.query !== before) void this.searchText(filter.query);
  }

  setQuery(text: string): Promise<void> {
    this.filter = { ...this.filter, query: text };
    return this.searchText(text);
  }

  resetFilter(): void {
    this.setFilter(emptyFilter());
  }

  /** Narrow the list to one family and show it (Enter in the family table). */
  openFamily(family: string): void {
    this.setFilter({ ...emptyFilter(), families: [family] });
    this.view = 'rules';
  }

  /** The words are searched in the texts by the hub a moment after the last key, from two letters on. */
  private async searchText(text: string): Promise<void> {
    const seq = ++this.searchSeq;
    this.searchAbort?.abort();
    const query = text.trim();
    if ([...query].length < MIN_QUERY_CHARS || this.phase !== 'ready') {
      this.searchHits = null;
      return;
    }
    for (let attempt = 0; ; attempt++) {
      // The hub reads the texts a moment after it starts: a `building` answer is asked again, a few times.
      await this.env.sleep(attempt === 0 ? SEARCH_DELAY_MS : SEARCH_RETRY_MS);
      if (seq !== this.searchSeq) return;
      const abort = new AbortController();
      this.searchAbort = abort;
      try {
        const result = await this.env.hub.search(query, abort.signal);
        if (seq !== this.searchSeq) return;
        const found = new Set(result.hits.map((h) => h.id));
        const total = Math.max(result.total, found.size);
        this.searchHits = {
          query,
          ids: found,
          status: result.status,
          total,
          // More rules hold the words than the hub named (it names SEARCH_LIMIT at most): the list must say so.
          cut: result.partial === true || total > found.size,
        };
        if (result.status !== 'building' || attempt >= SEARCH_RETRIES) return;
      } catch {
        if (seq === this.searchSeq) this.searchHits = { query, ids: new Set(), status: 'failed', total: 0, cut: false };
        return;
      }
    }
  }

  // ── selection and detail ──────────────────────────────────────────────

  /** Put the cursor on a rule. The rule is read only when it is opened: each read is a question to the engine. */
  select(id: string | null): void {
    this.selectedId = id;
  }

  async openDetail(id: string): Promise<void> {
    this.selectedId = id;
    await this.loadDetail(id);
  }

  closeDetail(): void {
    this.detailAbort?.abort();
    this.detail = { state: 'none' };
  }

  /** Set when the page left a place by itself; `takeNavReplace` reads it once. */
  private navReplace = false;

  /** Is the change of place that is recorded next one the page made by itself (a replaced entry)? Reads once. */
  takeNavReplace(): boolean {
    const replace = this.navReplace;
    this.navReplace = false;
    return replace;
  }

  /** The place the page is at now, for the history of the tab. */
  navState(): NavState {
    return { view: this.view, id: this.detail.state === 'none' ? null : this.detail.id };
  }

  /** Back or Forward brought the tab to this place. */
  restoreNav(place: NavState): void {
    this.view = place.view;
    if (place.id === null) this.closeDetail();
    else void this.openDetail(place.id);
  }

  /** Is this rule the one the detail shows or reads? */
  private isOpen(id: string): boolean {
    const detail = this.detail;
    return detail.state !== 'none' && detail.id === id;
  }

  private async loadDetail(id: string): Promise<void> {
    // What is shown stays while the same rule is read again.
    if (!(this.detail.state === 'ok' && this.detail.id === id)) this.detail = { state: 'loading', id };
    this.detailAbort?.abort();
    const abort = new AbortController();
    this.detailAbort = abort;
    try {
      const doc = await this.env.hub.rule(id, abort.signal);
      if (!abort.signal.aborted && this.isOpen(id)) {
        this.detail = { state: 'ok', id, doc };
      }
    } catch (error) {
      if (abort.signal.aborted || !this.isOpen(id)) return;
      // The hub holds no such rule: after a removal, a resync or a truncated event alike, the detail closes.
      if (error instanceof ApiError && error.status === 404) this.closeGone(id);
      else this.detail = { state: 'failed', id };
    }
  }

  // ── keys and palette ──────────────────────────────────────────────────

  /** Run a command of the keyboard (`keys.ts`). */
  command(command: Command): void {
    if (this.phase !== 'ready') return;
    switch (command.name) {
      case 'palette':
        this.paletteOpen = true;
        return;
      case 'go':
        if (VIEWS.includes(command.arg as View)) this.view = command.arg as View;
        return;
      case 'search':
        this.view = 'rules';
        this.searchFocus += 1;
        return;
      case 'next':
      case 'previous': {
        const delta = command.name === 'next' ? 1 : -1;
        if (this.view === 'rules') {
          this.selectedId = moveSelection(
            this.shown.map((r) => r.id),
            this.selectedId,
            delta,
          );
        } else if (this.view === 'families') {
          this.selectedFamily = moveSelection(
            this.families.map((f) => f.family),
            this.selectedFamily,
            delta,
          );
        }
        return;
      }
      case 'open':
        if (this.view === 'rules' && this.selectedId !== null) void this.openDetail(this.selectedId);
        else if (this.view === 'families' && this.selectedFamily !== null) this.openFamily(this.selectedFamily);
        return;
      case 'escape':
        // The palette first, then the detail, then the selection.
        if (this.paletteOpen) this.paletteOpen = false;
        else if (this.detail.state !== 'none') this.closeDetail();
        else if (this.view === 'families') this.selectedFamily = null;
        else this.selectedId = null;
        return;
    }
  }

  openPalette(): void {
    if (this.phase === 'ready') this.paletteOpen = true;
  }

  closePalette(): void {
    this.paletteOpen = false;
  }

  /** Run what was chosen in the palette. Only an action is remembered; a rule is opened and forgotten. */
  async choose(item: PaletteItem): Promise<void> {
    this.paletteOpen = false;
    if (item.kind === 'entry') {
      this.view = 'rules';
      await this.openDetail(item.id);
      return;
    }
    this.frecency = bumped(this.frecency, item.key, this.env.now());
    safe(() => this.env.prefs.setItem(PREF_KEYS.frecency, JSON.stringify(this.frecency)));
    await item.action.run();
  }

  /** Show the file of a rule in the editor of the machine the hub runs on; the answer is a word in the toast. */
  async openInEditor(id: string): Promise<void> {
    try {
      await this.env.hub.open(id);
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return;
      const unavailable = error instanceof ApiError && error.code === 'open_unavailable';
      this.notice = { key: unavailable ? 'notice.open_unavailable' : 'notice.open_failed' };
    }
  }

  dismissNotice(): void {
    this.notice = null;
  }

  // ── writing ─────────────────────────────────────────────────────────────

  /** A request is on its way: the page does not send a second one beside it. */
  writing = $state(false);
  /** The operations that met another version of the file, first first: the conflict center shows them; nothing was written. */
  conflicts = $state.raw<Conflict[]>([]);
  /** The rule as the file has it now, for the first conflict (read when the conflict arrived). */
  conflictRule = $state.raw<Body | null>(null);
  /** The same for every conflict, by the id of the rule: the list of the conflict center compares each one. */
  conflictRules = $state.raw<Record<string, Body>>({});
  /** How the conflict center shows the conflicts: every one in a list (the default), or one at a time. */
  conflictMode = $state<ConflictMode>(DEFAULT_CONFLICT_MODE);

  get conflict(): Conflict | null {
    return this.conflicts[0] ?? null;
  }
  private opCount = 0;

  // ── the data classes ────────────────────────────────────────────────────

  /** A change of a class that releases more than before: the hub wants the word of the person; shown until chosen. */
  classConfirm = $state.raw<ClassChange | null>(null);

  /**
   * Change a data class. The hub says whether the change releases more than before (it knows the policy in force): then
   * the person is asked and the change goes again with their word. Returns whether the change was written: a view that
   * shows the choice of the person (a select) puts it back to what is in force when it was not.
   */
  async setClass(change: ClassChange): Promise<boolean> {
    this.classConfirm = null;
    try {
      await this.env.hub.setClass(change);
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return false;
      if (error instanceof ApiError && error.code === 'confirm_needed') {
        this.classConfirm = change;
        return false;
      }
      this.notice = { key: 'classes.failed' };
      return false;
    }
    await this.readSources();
    this.notice = { key: 'classes.saved' };
    return true;
  }

  /** The person said yes to a change that releases more. */
  async confirmClass(): Promise<boolean> {
    const change = this.classConfirm;
    return change ? this.setClass({ ...change, confirm_loosen: true }) : false;
  }

  cancelClass(): void {
    this.classConfirm = null;
  }

  // ── the rule sets of the vault ───────────────────────────────────────────

  /** The rule sets that are loaded and the archived ones, as the engine says it; `null` until read. */
  rulesets = $state.raw<Rulesets | null>(null);
  /** The rule set the person is asked about before it goes to the archive (nothing is deleted). */
  archiveAsk = $state<string | null>(null);

  async loadRulesets(): Promise<void> {
    try {
      this.rulesets = await this.env.hub.rulesets();
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return;
      this.rulesets = null;
    }
  }

  /** Make an empty rule set. Returns whether it was made. */
  async makeRuleset(name: string): Promise<boolean> {
    const outcome = await this.write([createRulesetOp(name)]);
    return outcome !== null && outcome.saved > 0 && outcome.failures.length === 0;
  }

  /** Ask before a rule set goes to the archive. */
  askArchive(name: string): void {
    this.archiveAsk = name;
  }

  cancelArchive(): void {
    this.archiveAsk = null;
  }

  /** The person said yes: the rule set is moved to the archive; Undo brings it back. */
  async archiveRuleset(): Promise<boolean> {
    const name = this.archiveAsk;
    this.archiveAsk = null;
    if (name === null) return false;
    const outcome = await this.write([archiveRulesetOp(name)]);
    return outcome !== null && outcome.saved > 0 && outcome.failures.length === 0;
  }

  /** Bring a rule set back from the archive. */
  async restoreRuleset(name: string): Promise<boolean> {
    const outcome = await this.write([restoreRulesetOp(name)]);
    return outcome !== null && outcome.saved > 0 && outcome.failures.length === 0;
  }

  // ── the waivers of the project of the device ─────────────────────────────

  /** What the waivers file of the project says; `null` until it was read. */
  waivers = $state.raw<Waivers | null>(null);
  /** `no_project`: the person set none (config.json); `failed`: the engine could not read the file. */
  waiversState = $state<'idle' | 'ready' | 'no_project' | 'failed'>('idle');
  /** The rule a "Waive …" button was pressed for: the form of the waivers view starts with its id. */
  waiveDraft = $state<string | null>(null);

  async loadWaivers(): Promise<void> {
    try {
      this.waivers = await this.env.hub.waivers();
      this.waiversState = 'ready';
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return;
      this.waivers = null;
      this.waiversState = error instanceof ApiError && error.code === 'no_project' ? 'no_project' : 'failed';
    }
  }

  /** Waive a rule: go to the waivers with its id in the form. */
  startWaiver(id: string): void {
    this.waiveDraft = id;
    this.view = 'waivers';
  }

  /** The tag of the waivers file the person saw: what a change must be sent with. */
  waiversEtag(): string | null {
    return this.waivers?.etag ?? null;
  }

  // ── the views of the rules (`rules.groupings`, `rules.views`) ─────────────

  /** The fields a view groups by, as the engine names them; `null` until read. */
  groupings = $state.raw<Groupings | null>(null);
  /** The saved views of the vault; `null` until read. */
  savedViews = $state.raw<Views | null>(null);
  /** `failed`: the engine could not answer (no vault, say: the views live in the vault). */
  viewsState = $state<'idle' | 'ready' | 'failed'>('idle');
  /** The saved view shown in the tab, by id; `null`: the first one. */
  viewId = $state<string | null>(null);

  /**
   * Read the fields and the saved views again (and the waivers, which a view can group by). A change of a view sends
   * no event of the stream (the snapshot is the rules'), so they are read when the tab shows, after a write of a view
   * and with every change of the rules.
   */
  async loadViews(): Promise<void> {
    try {
      const [groupings, views] = await Promise.all([this.env.hub.groupings(), this.env.hub.views()]);
      this.groupings = groupings;
      this.savedViews = views;
      this.viewsState = 'ready';
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return;
      this.viewsState = 'failed';
    }
    if (this.waiversState === 'idle') await this.loadWaivers();
  }

  /** The dialog for a new rule is open. */
  newRuleOpen = $state(false);

  openNewRule(): void {
    if (this.phase === 'ready') this.newRuleOpen = true;
  }

  closeNewRule(): void {
    this.newRuleOpen = false;
  }

  /** The next free id of a family, as the engine says it; `null` when it names none or cannot be asked. */
  async nextId(family: string): Promise<string | null> {
    try {
      return nextOf(await this.env.hub.nextId(family), family);
    } catch {
      return null;
    }
  }

  /** The tag of the file a rule lives in, as the snapshot gave it: what a change must be sent with. */
  etagOf(id: string): string | null {
    return this.rules.find((r) => r.id === id)?.etag ?? null;
  }

  /**
   * Ask the engine to change rules. The page never decides what a change is: it sends the operations, reads the
   * document the engine answers with, and shows what came of each. Every change goes with the tag the person saw
   * (`if_match`), so that nothing is written over a change made since. Returns what came of it, or `null` when the
   * hub refused the whole request (told in a notice).
   */
  async write(ops: Op[], options: { dryRun?: boolean; undoable?: boolean } = {}): Promise<Outcome | null> {
    if (this.phase !== 'ready' || this.writing || ops.length === 0) return null;
    const dryRun = options.dryRun === true;
    this.writing = true;
    try {
      this.opCount += 1;
      const result = await this.env.hub.ops({
        schema: 1,
        client_op_id: clientOpId(this.env.now(), this.opCount),
        dry_run: dryRun,
        ops,
      });
      const outcome = summarize(ops, result);
      if (dryRun) return outcome;
      this.reportWrite(outcome, options.undoable !== false);
      // The waivers file is not watched: read it again after a change of it, or after a clash (it says what it is now).
      if (ops.some(isWaiverOp)) await this.loadWaivers();
      // The rule sets are not rules: no event of the stream tells they changed.
      if (ops.some(isRulesetOp)) await this.loadRulesets();
      if (ops.some(isViewOp)) await this.loadViews();
      if (outcome.saved > 0) await this.afterWrite(outcome.changedIds);
      return outcome;
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return null;
      const key = error instanceof ApiError ? WRITE_KEYS[error.code] : undefined;
      this.notice = key
        ? { key }
        : { key: 'write.failed', params: { message: error instanceof Error ? error.message : 'failed' } };
      return null;
    } finally {
      this.writing = false;
    }
  }

  /** Say what came of a write in one notice; a conflict is also kept for the conflict center. */
  private reportWrite(outcome: Outcome, undoable: boolean): void {
    const first = outcome.conflicts[0];
    // The conflicts that are still waiting stay (writing one of them again must not lose the others); a newer
    // refusal of the same rule replaces the older.
    const waiting = this.conflicts.filter((c) => !outcome.conflicts.some((n) => n.id === c.id));
    this.conflicts = [...outcome.conflicts, ...waiting];
    const kept: Record<string, Body> = {};
    for (const c of waiting) {
      const rule = this.conflictRules[c.id];
      if (rule) kept[c.id] = rule;
    }
    this.conflictRules = kept;
    this.conflictRule = this.conflict ? (this.conflictRules[this.conflict.id] ?? null) : null;
    if (first) {
      for (const conflict of outcome.conflicts) if (!isWaiverOp(conflict.op)) void this.readConflictRule(conflict.id);
      this.notice = isWaiverOp(first.op)
        ? { key: 'write.conflict_waivers' }
        : { key: 'write.conflict', params: { id: first.id } };
    } else if (outcome.failures[0]) {
      this.notice = { key: 'write.failed', params: { message: outcome.failures[0].message || outcome.failures[0].code } };
    } else if (outcome.saved > 1) {
      this.notice = { key: 'write.saved_many', params: { n: String(outcome.saved) }, ...(undoable && outcome.undo.length ? { undo: outcome.undo } : {}) };
    } else if (outcome.saved === 1) {
      this.notice = { key: 'write.saved', ...(undoable && outcome.undo.length ? { undo: outcome.undo } : {}) };
    }
  }

  /** The hub has read the new state before it answered: read it here, and the rule that is open again. */
  private async afterWrite(changed: readonly string[]): Promise<void> {
    await this.refresh();
    const open = this.detail.state === 'none' ? null : this.detail.id;
    if (open === null || !changed.includes(open)) return;
    // A rule that left the rule set (retired) is not asked for: the hub would answer 404. Its detail just closes.
    if (this.rules.some((r) => r.id === open)) await this.loadDetail(open);
    else this.closeDetail();
  }

  /** Take back what the notice reports: its operations go as a request of their own, with no Undo for the undo. */
  async undo(): Promise<void> {
    const ops = this.notice?.undo;
    if (!ops || ops.length === 0) return;
    this.notice = null;
    const outcome = await this.write(ops, { undoable: false });
    if (outcome && outcome.saved > 0 && outcome.conflicts.length === 0 && outcome.failures.length === 0) {
      this.notice = { key: 'write.undone' };
    }
  }

  private async readConflictRule(id: string): Promise<void> {
    try {
      const doc = await this.env.hub.rule(id);
      // A rule that was read after its conflict went is not kept.
      if (!this.conflicts.some((c) => c.id === id)) return;
      this.conflictRules = { ...this.conflictRules, [id]: doc.rule };
      if (this.conflict?.id === id) this.conflictRule = doc.rule;
    } catch {
      // The rule may be gone: the conflict center then shows only what was asked.
    }
  }

  /** Forget a conflict (the first by default: the person looked at it or went on); the next one, if any, comes up. */
  clearConflict(key?: string): void {
    const target = key === undefined ? this.conflict : this.conflicts.find((c) => conflictKey(c) === key);
    if (!target) return;
    this.conflicts = this.conflicts.filter((c) => c !== target);
    const next = this.conflict;
    this.conflictRule = next ? (this.conflictRules[next.id] ?? null) : null;
    if (next && !isWaiverOp(next.op) && !(next.id in this.conflictRules)) void this.readConflictRule(next.id);
  }

  /** Let every conflict go. */
  clearAllConflicts(): void {
    this.conflicts = [];
    this.conflictRule = null;
    this.conflictRules = {};
    if (this.notice?.key === 'write.conflict') this.notice = null;
  }

  /** The way the conflict center shows its conflicts, kept in this browser. */
  setConflictMode(mode: ConflictMode): void {
    this.conflictMode = mode;
    safe(() => this.env.prefs.setItem(PREF_KEYS.conflictMode, mode));
  }

  /**
   * What the person chose for the first conflict: write the same change onto the version the file has now (their
   * decision to write over the other change), look at the rule as it is, or drop their change.
   */
  async resolveConflict(choice: 'retry' | 'view' | 'drop', key?: string): Promise<void> {
    const conflict = key === undefined ? this.conflict : this.conflicts.find((c) => conflictKey(c) === key);
    if (!conflict) return;
    if (choice === 'retry') {
      const op = retryOp(conflict);
      this.clearConflict(key);
      if (op) await this.write([op]);
    } else if (choice === 'view') {
      this.clearConflict(key);
      if (isWaiverOp(conflict.op)) {
        this.view = 'waivers';
        await this.loadWaivers();
      } else {
        this.setView('rules');
        await this.openDetail(conflict.id);
      }
    } else {
      this.clearConflict(key);
    }
    if (this.notice?.key === 'write.conflict') this.notice = null;
  }

  // ── the marked rules (several at once) ──────────────────────────────────

  /** The ids the person marked in the list. */
  marked = $state.raw<ReadonlySet<string>>(new Set());

  /** May the page write this rule? A file of its own, in a rule set that has a class. */
  canWrite(rule: Pick<RuleSummary, 'source' | 'etag_scope'>): boolean {
    return rule.etag_scope === 'rule' && this.classOfRule(rule) !== 'unclassified';
  }

  toggleMark(id: string): void {
    const next = new Set(this.marked);
    if (!next.delete(id)) next.add(id);
    this.marked = next;
  }

  markShown(on: boolean): void {
    this.marked = on ? new Set(this.shown.filter((r) => this.canWrite(r)).map((r) => r.id)) : new Set();
  }

  clearMarks(): void {
    this.marked = new Set();
  }

  /** The marked rules the page may write, with the tag it saw for each. */
  private markedWritable(): { id: string; etag: string; severity: string }[] {
    return this.rules
      .filter((r) => this.marked.has(r.id) && this.canWrite(r))
      .map((r) => ({ id: r.id, etag: r.etag, severity: r.severity }));
  }

  /** One request for all marked rules: the same severity, or retired. A rule that is already so, or not writable, is left out. */
  async writeMarked(change: { severity: string } | { retire: true }): Promise<Outcome | null> {
    const rules = this.markedWritable();
    const ops =
      'retire' in change
        ? rules.map((r) => retireOp(r.id, r.etag))
        : rules.filter((r) => r.severity !== change.severity).map((r) => setOp(r.id, { severity: change.severity }, r.etag));
    if (ops.length === 0) {
      this.notice = { key: 'bulk.none' };
      return null;
    }
    const outcome = await this.write(ops);
    if (outcome && outcome.conflicts.length === 0 && outcome.failures.length === 0) this.clearMarks();
    return outcome;
  }
}

/** The words for the refusals of the hub that have their own (by the code of the error document). */
const WRITE_KEYS: Record<string, Key> = {
  write_refused: 'write.refused',
  rate_limited: 'write.rate',
  engine_unavailable: 'write.unavailable',
  not_found: 'write.not_held',
};

