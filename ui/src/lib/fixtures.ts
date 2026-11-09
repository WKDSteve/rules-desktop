// Test data: the golden documents of the pinned engine. They are imported from the copies the Rust tests answer with
// (crates/rules-hub/tests/fixtures/golden), not copied a second time: the one `cmp` in CI against the pinned engine
// covers the hub and the page alike.

import type { Error as EngineError, Groupings, Rule, Rulesets, Snapshot, Views, Waivers } from './contract';
import type { HubSources } from './api';
import goldenError from '../../../crates/rules-hub/tests/fixtures/golden/error.json';
import goldenGroupings from '../../../crates/rules-hub/tests/fixtures/golden/groupings.json';
import goldenViews from '../../../crates/rules-hub/tests/fixtures/golden/views.json';
import goldenRulesets from '../../../crates/rules-hub/tests/fixtures/golden/rulesets.json';
import goldenRule from '../../../crates/rules-hub/tests/fixtures/golden/rule.json';
import goldenSnapshot from '../../../crates/rules-hub/tests/fixtures/golden/snapshot.json';
import goldenWaivers from '../../../crates/rules-hub/tests/fixtures/golden/waivers.json';
import hubSources from '../../../crates/rules-hub/tests/fixtures/hub/sources.json';

/** A JSON import is typed by its values (`kind: string`); the kind is checked, then it is the contract's type. */
function ofKind<T>(doc: { kind: string }, kind: string): T {
  if (doc.kind !== kind) throw new Error(`golden document of kind ${doc.kind}, expected ${kind}`);
  return doc as unknown as T;
}

/** A fresh copy each time: a test that changes it does not change it for the next one. */
function copy<T>(doc: T): T {
  return structuredClone(doc);
}

/** `rules.snapshot`: four rules in three files (DEP-01, DEP-09, ERR-100, LUA-01). */
export function snapshotDoc(): Snapshot {
  return copy(ofKind<Snapshot>(goldenSnapshot, 'rules.snapshot'));
}

/** `rules.rule` of DEP-01 with its text and its check. */
export function ruleDoc(): Rule {
  return copy(ofKind<Rule>(goldenRule, 'rules.rule'));
}

/** `rules.rulesets`: the one rule set of the golden snapshot, nothing in the archive. */
export function rulesetsDoc(): Rulesets {
  return copy(ofKind<Rulesets>(goldenRulesets, 'rules.rulesets'));
}

/** `rules.waivers`: three waivers, one expired, one for a rule that is not loaded. */
export function waiversDoc(): Waivers {
  return copy(ofKind<Waivers>(goldenWaivers, 'rules.waivers'));
}

/** `rules.groupings` of the golden snapshot: family, severity, status, waiver (no layout in that rule set). */
export function groupingsDoc(): Groupings {
  return copy(ofKind<Groupings>(goldenGroupings, 'rules.groupings'));
}

/** `rules.views`: one view, "By severity" (columns, by severity, the family DEP). */
export function viewsDoc(): Views {
  return copy(ofKind<Views>(goldenViews, 'rules.views'));
}

/** `rules.error`: not_found. */
export function errorDoc(): EngineError {
  return copy(ofKind<EngineError>(goldenError, 'rules.error'));
}

/** The text of a golden document as the hub passes it on. */
export function goldenText(name: 'snapshot' | 'rule' | 'error'): string {
  const doc = name === 'snapshot' ? goldenSnapshot : name === 'rule' ? goldenRule : goldenError;
  return JSON.stringify(doc);
}

/**
 * What `GET /api/rules/sources` answers for the golden snapshot with a `sources.json` that holds
 * `{"Checklists": {"class": "internal"}, "Checklists/_rules": {"class": "private"}}` and no default. The document is
 * the one a Rust test of the hub compares its answer with (`tests/routes.rs`), not a copy written out here. The page
 * reads the classes from it; it never works one out.
 */
export function hubSourcesDoc(): HubSources {
  return copy(ofKind<HubSources>(hubSources, 'hub.sources'));
}
