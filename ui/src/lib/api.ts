// The hub's routes, as typed calls. Only the shapes the page needs. Every request carries the session as a bearer
// token and nothing else: no cookie, no cache, no referrer.

import type { NextId, Result, Rule, Rulesets, Snapshot, Waivers } from './contract';
import type { OpsRequest } from './write';
import { ApiError, failure, HubClient, textOf, type Hello, type SnapshotResult as KitSnapshotResult } from 'appkit-ui/client';

/** Where the routes of this app live (`IDENTITY.route` of rules-hub). */
export const API = '/api/rules';

// The transport (the bearer token, the refusal, the pairing code, hello, the snapshot with its tag, rescan) is the
// kit's; this file is the routes of the rules.
export { ApiError, failure, type Hello };

/** `GET /api/rules/about`: the build and the engine that answers (only the three words of its `hello`). */
export interface About {
  app: string;
  version: string;
  engine: { version: string | null; git: string | null; nvim: string | null } | null;
  /** A short word for why the engine did not answer; `null` when it did or there is none. */
  engineProblem: string | null;
}

/** The class a file is shown with on this device (`sources.json`); `unclassified` is not released. */
export type DataClass = 'public' | 'internal' | 'private' | 'unclassified';

/** `GET /api/rules/sources`: every file of the held snapshot with its class, worked out by the hub. */
export interface HubSources {
  schema: 1;
  kind: 'hub.sources';
  files: { path: string; data_class: DataClass; rules: number; key: string | null }[];
  keys: { key: string; class: Exclude<DataClass, 'unclassified'>; files: number }[];
  /** The class of a file no key names; none: such a file is not classified. */
  default: Exclude<DataClass, 'unclassified'> | null;
  unclassified: string[];
  private: string[];
}

/** A change of a data class: one target (a key, or the default) and the class; `class` null takes it away. */
export type ClassChange = (
  | { key: string }
  | { default: true }
) & {
  class: Exclude<DataClass, 'unclassified'> | null;
  confirm_loosen?: boolean;
};

/** A piece of a rule's text around a match; `marks` are `[start, end)` in UTF-16 units of `text`. */
export interface SearchSnippet {
  text: string;
  marks: [number, number][];
  more_before: boolean;
  more_after: boolean;
}

export interface SearchHit {
  id: string;
  score: number;
  snippet: SearchSnippet;
}

/** `GET /api/rules/search`: the answer of the kit's search index, as in tasks. */
export interface SearchResult {
  /** `building`: the hub has not read the texts yet; `failed`: it could not. */
  status: 'ready' | 'building' | 'failed';
  indexed: number;
  partial?: boolean;
  total: number;
  hits: SearchHit[];
}

export type SnapshotResult = KitSnapshotResult<Snapshot>;

/** Hits asked for at most. */
export const SEARCH_LIMIT = 50;

export class Hub extends HubClient<Snapshot> {
  constructor(fetchFn: typeof fetch, base: string = '') {
    super(fetchFn, API, base);
  }

  /** One rule with its text. A `signal` that aborts drops the request. */
  rule(id: string, signal?: AbortSignal): Promise<Rule> {
    return this.json<Rule>(`${API}/rule?id=${encodeURIComponent(id)}`, signal ? { signal } : undefined);
  }

  sources(): Promise<HubSources> {
    return this.json<HubSources>(`${API}/sources`);
  }

  /** The rules whose text holds the words (the hub searches the texts it keeps). */
  search(query: string, signal?: AbortSignal): Promise<SearchResult> {
    const path = `${API}/search?q=${encodeURIComponent(query)}&limit=${SEARCH_LIMIT}`;
    return this.json<SearchResult>(path, signal ? { signal } : undefined);
  }

  /** Show the file of a rule in Neovim, at its line (the hub starts the editor on its own machine). */
  open(id: string): Promise<void> {
    return this.post(`${API}/open`, { id });
  }

  /** Whether the hub starts at login; null where there is nothing to switch. */
  async autostart(): Promise<boolean | null> {
    try {
      const doc = await this.json<{ on: boolean | null }>(`${API}/autostart`);
      return typeof doc.on === 'boolean' ? doc.on : null;
    } catch {
      return null;
    }
  }

  /** Register the hub at login or take it away; what is true afterwards. */
  async setAutostart(on: boolean): Promise<boolean | null> {
    const doc = await this.postJson<{ on: boolean | null }>(`${API}/autostart`, { on });
    return typeof doc.on === 'boolean' ? doc.on : null;
  }

  /**
   * Ask the engine to change rules (`rules-ops/1`) through the hub. A refusal of the whole request is an
   * `ApiError` (`write_refused` for a rule set without a class, `bad_request`, `rate_limited` ...); a part that
   * failed is told per operation in the document, which the page reads (`write.ts`).
   */
  async ops(request: OpsRequest): Promise<Result> {
    return this.postJson<Result>(`${API}/ops`, request);
  }

  /**
   * Set the class of a key of `sources.json`, take the key away (`class` null), or do the same for the default. The
   * hub refuses a change that releases more than before (`confirm_needed`, 409) until `confirm_loosen` is true.
   */
  setClass(change: ClassChange): Promise<void> {
    return this.post(`${API}/sources/class`, change);
  }

  /** The waivers of the project of this device; a 409 `no_project` when the person set none. */
  waivers(): Promise<Waivers> {
    return this.json<Waivers>(`${API}/waivers`);
  }

  /** The rule sets that are loaded (an empty one too) and the ones in the archive of the vault. */
  rulesets(): Promise<Rulesets> {
    return this.json<Rulesets>(`${API}/rulesets`);
  }

  /** The next free id of a family, or of every family: the engine says it, nothing is reserved. */
  nextId(family?: string): Promise<NextId> {
    const query = family ? `?family=${encodeURIComponent(family)}` : '';
    return this.json<NextId>(`${API}/next_id${query}`);
  }

  /** What this build is and which engine answers: only the words the page shows are taken. */
  async about(): Promise<About> {
    const doc = await this.json<Record<string, unknown>>(`${API}/about`);
    const text = textOf;
    const engine = doc['engine'];
    const part = (key: string): string | null => text((engine as Record<string, unknown>)[key]);
    return {
      app: text(doc['app']) ?? 'rules-desktop',
      version: text(doc['version']) ?? '',
      engine:
        typeof engine === 'object' && engine !== null && !Array.isArray(engine)
          ? { version: part('version'), git: part('git'), nvim: part('nvim') }
          : null,
      engineProblem: text(doc['engine_problem']),
    };
  }
}
