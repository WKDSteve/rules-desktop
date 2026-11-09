// The views of the rules: the rules laid in groups by a field of the engine (`rules.groupings`), as columns, a list of
// sections or a matrix, and the saved views of the vault (`rules.views`). Nothing here judges a rule: the fields, their
// values and how a drag is written are the engine's; this reads the value a rule has for a field and narrows the rules
// to a view's selection. The groups themselves are the kit's (`appkit-ui/groups`).

import type { Groupings, Retired, RuleSummary, Snapshot, Views, Waivers } from './contract';
import type { Op } from './write';

/** A field of `rules.groupings`. */
export type Grouping = Groupings['fields'][number];
/** A saved view as the engine lists it. */
export type SavedView = Views['views'][number];
/** What a `view_save` carries. */
export type ViewDef = Pick<SavedView, 'title' | 'display' | 'group_by' | 'collapsed' | 'select'> & { group_by2?: string };

/** One rule of a view: a loaded one (`active`) or a retired one, with what a field reads of it. */
export interface Item {
  id: string;
  title: string;
  family: string;
  severity: string | null;
  status: 'active' | 'retired';
  /** The rule set it comes from: the first part of its `source`. */
  ruleset: string;
  rule: RuleSummary | null;
}

/** The rule set a `source` names (`lua/regeln/LUA.md` is `lua`). */
export function rulesetOf(source: string): string {
  const at = source.indexOf('/');
  return at < 0 ? source : source.slice(0, at);
}

/** The rules of a view: the loaded ones in id order, then the retired ones. */
export function itemsOf(snapshot: Snapshot | null): Item[] {
  if (!snapshot) return [];
  const live: Item[] = snapshot.rules.map((r) => ({
    id: r.id,
    title: r.title,
    family: r.family,
    severity: r.severity,
    status: 'active',
    ruleset: rulesetOf(r.source),
    rule: r,
  }));
  const gone: Item[] = (snapshot.retired as Retired[]).map((r) => ({
    id: r.id,
    title: '',
    family: r.family,
    severity: null,
    status: 'retired',
    ruleset: rulesetOf(r.source),
    rule: null,
  }));
  return [...live, ...gone];
}

/** The key of a field as a view names it: the field, or `layout:<ruleset>:<output>` for the section of a layout. */
export function keyOf(grouping: Grouping): string {
  return grouping.field === 'layout' ? `layout:${grouping.ruleset ?? ''}:${grouping.layout ?? ''}` : grouping.field;
}

/** The field a view groups by, or null when the engine names it no more (a layout that is gone). */
export function fieldOf(groupings: Groupings | null, key: string | undefined): Grouping | null {
  if (!groupings || key === undefined) return null;
  return groupings.fields.find((g) => keyOf(g) === key) ?? null;
}

/** What the reading of a field needs beside the rule: the waivers of the project. */
export interface Context {
  /** The waivers by rule id: `active` or `expired`; a rule without one has none. */
  waivers: Map<string, 'active' | 'expired'>;
}

/** The waivers of a project as `Context.waivers`. */
export function waiversOf(doc: Waivers | null): Map<string, 'active' | 'expired'> {
  const out = new Map<string, 'active' | 'expired'>();
  for (const w of doc?.items ?? []) out.set(w.id, w.expired ? 'expired' : 'active');
  return out;
}

// The sections of a layout by rule id, kept per field object (the document is read once per change).
const sections = new WeakMap<Grouping, Map<string, string>>();
function sectionOf(grouping: Grouping, id: string): string | undefined {
  let map = sections.get(grouping);
  if (!map) {
    map = new Map((grouping.members ?? []).map((m) => [m.id, m.value] as const));
    sections.set(grouping, map);
  }
  return map.get(id);
}

/**
 * The values a rule has for a field: its family, its severity (a retired one has none), its status, the state of its
 * waiver, or the section it stands in in a layout (none: it is not in that layout).
 */
export function valuesOf(item: Item, grouping: Grouping, ctx: Context): string[] {
  switch (grouping.field) {
    case 'family':
      return [item.family];
    case 'severity':
      return item.severity === null ? [] : [item.severity];
    case 'status':
      return [item.status];
    case 'waiver': {
      const w = ctx.waivers.get(item.id);
      return w ? [w] : [];
    }
    case 'layout': {
      const at = sectionOf(grouping, item.id);
      return at === undefined ? [] : [at];
    }
  }
}

/** The rules a view selects: the lists narrow when they name something, the query matches the id or the title. */
export function selected(items: readonly Item[], select: SavedView['select']): Item[] {
  const has = (list: readonly string[], v: string | null) => list.length === 0 || (v !== null && list.includes(v));
  const query = select.query?.trim().toLowerCase() ?? '';
  return items.filter(
    (i) =>
      has(select.families, i.family) &&
      has(select.severities, i.severity) &&
      has(select.statuses, i.status) &&
      has(select.rulesets, i.ruleset) &&
      (query === '' || i.id.toLowerCase().includes(query) || i.title.toLowerCase().includes(query)),
  );
}

/** An empty selection: every rule. */
export const EVERY: SavedView['select'] = { families: [], severities: [], statuses: [], rulesets: [] };

/** The definition of a saved view, as `view_save` takes it back. */
export function defOf(view: SavedView): ViewDef {
  return {
    title: view.title,
    display: view.display,
    group_by: view.group_by,
    ...(view.group_by2 !== undefined ? { group_by2: view.group_by2 } : {}),
    collapsed: view.collapsed,
    select: view.select,
  };
}

/** Make a view (`id` absent) or replace one on the version read. */
export function viewSaveOp(def: ViewDef, id?: string, ifMatch?: string): Op {
  return { op: 'view_save', view: def, ...(id !== undefined ? { id } : {}), ...(ifMatch !== undefined ? { if_match: ifMatch } : {}) };
}

/** Delete a view on the version read. */
export function viewDeleteOp(id: string, ifMatch: string): Op {
  return { op: 'view_delete', id, if_match: ifMatch };
}

/** Whether an operation writes a view (the views are read again after it: no event of the stream tells). */
export { isViewOp } from './write';
