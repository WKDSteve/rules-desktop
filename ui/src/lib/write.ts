// The write side of the page as plain functions: what to ask (the operations of rules-ops/1 as the page builds them),
// and what to make of the answer (what was saved, what clashed, what undoes it). The rules are the engine's: nothing
// here formats or parses a rule, it only passes on what the person typed and reads the engine's document.

import type { NextId, Result } from './contract';

/** One operation of a request: its name and its fields. The engine checks the rest, strictly. */
export type Op = { op: string } & Record<string, unknown>;

/** The request of `rules-ops/1` as the hub takes it. */
export interface OpsRequest {
  schema: 1;
  client_op_id: string;
  dry_run: boolean;
  ops: Op[];
}

/** An entry of the engine's answer, one per operation. */
export type Entry = Result['results'][number];

/** An operation that met another version of the file than the person saw: nothing was written. */
export interface Conflict {
  /** The rule's id. */
  id: string;
  /** The place of the operation in the request, from 1. */
  n: number;
  /** The tag the person saw, and the tag the file has now. */
  expected: string | null;
  actual: string | null;
  /** What was asked, so that it can be asked again once the person has looked at the other version. */
  op: Op | null;
}

/** An operation that failed for another reason. */
export interface Failure {
  n: number;
  id: string | null;
  code: string;
  message: string;
}

/** What an answer of `ops` comes to. */
export interface Outcome {
  /** Entries that changed a rule (a dry run saves nothing). */
  saved: number;
  changedIds: string[];
  /** The ids of the entries that went through, a dry run's too (what a new rule would be called). */
  ids: string[];
  conflicts: Conflict[];
  failures: Failure[];
  warnings: string[];
  /** The operations that take it back, last first; empty for a dry run or when nothing changed. */
  undo: Op[];
}

/** A client id for a request: 1 to 64 of the characters the hub takes. */
export function clientOpId(now: number, n: number): string {
  return `c-${now.toString(36)}-${n.toString(36)}`;
}

/** The operations whose inverse is not complete without the tag the file has after the write. */
const NEEDS_IF_MATCH: readonly string[] = ['set', 'retire'];

function text(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

/** Read the engine's answer for a request. `request` is the list of operations that was sent. */
export function summarize(request: readonly Op[], result: Result): Outcome {
  const out: Outcome = { saved: 0, changedIds: [], ids: [], conflicts: [], failures: [], warnings: [], undo: [] };
  for (const entry of result.results) {
    // An entry that failed may not say which rule: the operation that was sent does.
    const asked = text(request[entry.n - 1]?.['id']);
    const id = entry.id ?? asked;
    if (entry.warnings) out.warnings.push(...entry.warnings);
    if (!entry.ok) {
      const error = entry.error ?? {};
      const code = error.code ?? 'failed';
      // A name that is taken is a refusal, not a clash of versions: there is no tag to compare.
      if (code === 'conflict' && !isRulesetOp(request[entry.n - 1])) {
        out.conflicts.push({
          id: id ?? '',
          n: entry.n,
          expected: text(error.details?.['expected']),
          actual: text(error.details?.['actual']),
          op: request[entry.n - 1] ?? null,
        });
      } else {
        out.failures.push({ n: entry.n, id, code, message: error.message ?? '' });
      }
      continue;
    }
    if (entry.id) out.ids.push(entry.id);
    // A rule set that was made, moved or brought back has no rule id: its entry names the rule set instead. A view
    // holds no rule either: an entry of a view that was made, changed or deleted is a change all the same.
    const view = isViewOp(request[entry.n - 1]) && entry.outcome !== 'unchanged';
    if (result.dry_run || (entry.changed_ids.length === 0 && !entry.ruleset && !view)) continue;
    out.saved += 1;
    out.changedIds.push(...entry.changed_ids);
    const inverse = entry.inverse;
    const name = text(inverse?.['op']);
    if (inverse && name !== null) {
      const undo: Op = { ...inverse, op: name };
      // The tag the file has now: the person's undo must not overwrite a change made since.
      if (NEEDS_IF_MATCH.includes(name) && undo['if_match'] === undefined && entry.etag_after) {
        undo['if_match'] = entry.etag_after;
      }
      out.undo.push(undo);
    }
  }
  out.undo.reverse();
  return out;
}

// ── the operations the page builds ───────────────────────────────────────────

/** The fields of a rule `set` can change; `null` for `check` or `agent` takes it away. */
export interface Patch {
  severity?: string;
  title?: string;
  text?: string;
  check?: unknown;
  agent?: unknown;
}

/** Change parts of a rule. `ifMatch` is the tag the person saw: without it the engine would write over anything. */
export function setOp(id: string, patch: Patch, ifMatch: string): Op {
  return { op: 'set', id, patch, if_match: ifMatch };
}

/** Retire a rule (its id stays taken), optionally with the rule that replaces it and a reason. */
export function retireOp(id: string, ifMatch: string, extra: { replaced_by?: string; reason?: string } = {}): Op {
  const op: Op = { op: 'retire', id, if_match: ifMatch };
  if (extra.replaced_by) op['replaced_by'] = extra.replaced_by;
  if (extra.reason) op['reason'] = extra.reason;
  return op;
}

/** What a new rule is made of. The optional parts are left out of the request when they are empty. */
export interface NewRule {
  family: string;
  title: string;
  severity: string;
  text?: string;
  check?: unknown;
  agent?: unknown;
  /** The name of the rule set; none: the first one. */
  ruleset?: string;
}

/** Make a rule with the next free id of its family (the engine picks it, under the lock of the rule set). */
export function newOp(rule: NewRule): Op {
  const op: Op = { op: 'new', family: rule.family, title: rule.title, severity: rule.severity };
  if (rule.text) op['text'] = rule.text;
  if (rule.check !== undefined && rule.check !== null) op['check'] = rule.check;
  if (rule.agent !== undefined && rule.agent !== null) op['agent'] = rule.agent;
  if (rule.ruleset) op['ruleset'] = rule.ruleset;
  return op;
}

/** The next id the engine named for a family, or `null` (a family it does not have yet, or none left). */
export function nextOf(doc: NextId, family: string): string | null {
  return doc.items.find((item) => item.family === family)?.next ?? null;
}

/** Where a rule goes in its document: after or before another rule of the same layout (the engine finds the layout). */
export function moveOp(id: string, place: { after: string } | { before: string }): Op {
  return { op: 'move', id, ...place };
}

/** The operations on the waivers of the project of the device. */
export const WAIVER_OPS: readonly string[] = ['waive', 'renew', 'remove'];

/** Whether an operation writes a saved view (`view_save`, `view_delete`): it holds no rule. */
export function isViewOp(op: Op | null | undefined): boolean {
  return op?.op === 'view_save' || op?.op === 'view_delete';
}

export function isWaiverOp(op: Op | null | undefined): boolean {
  return op !== null && op !== undefined && WAIVER_OPS.includes(op.op);
}

/**
 * Waive a rule in the project of the device (the hub names the project; the page never does). `ifMatch` is the tag of
 * the waivers file the person saw. `paths` limits the waiver to those paths.
 */
export function waiveOp(
  waiver: { id: string; reason: string; expires?: string; paths?: string[] },
  ifMatch: string,
): Op {
  const op: Op = { op: 'waive', id: waiver.id, reason: waiver.reason, if_match: ifMatch };
  if (waiver.expires) op['expires'] = waiver.expires;
  if (waiver.paths && waiver.paths.length > 0) op['paths'] = waiver.paths;
  return op;
}

/** Give a waiver a new last day (`expires`) or none (`no_expiry`), and maybe a new reason. */
export function renewOp(
  id: string,
  end: { expires: string } | { no_expiry: true },
  ifMatch: string,
  reason?: string,
): Op {
  const op: Op = { op: 'renew', id, ...end, if_match: ifMatch };
  if (reason) op['reason'] = reason;
  return op;
}

export function removeOp(id: string, ifMatch: string): Op {
  return { op: 'remove', id, if_match: ifMatch };
}

/** The operations on the rule sets of the vault. */
export const RULESET_OPS: readonly string[] = ['create_ruleset', 'archive_ruleset', 'restore_ruleset'];

export function isRulesetOp(op: Op | null | undefined): boolean {
  return op !== null && op !== undefined && RULESET_OPS.includes(op.op);
}

/** Make an empty rule set (a folder of the vault). `name` is the folder name. */
export function createRulesetOp(name: string): Op {
  return { op: 'create_ruleset', name };
}

/** Move a rule set to the archive of the vault. It is never deleted; `restore_ruleset` brings it back. */
export function archiveRulesetOp(name: string): Op {
  return { op: 'archive_ruleset', name };
}

export function restoreRulesetOp(name: string): Op {
  return { op: 'restore_ruleset', name };
}

/** Bring a retired rule back to the file it came from. */
export function reinstateOp(id: string): Op {
  return { op: 'reinstate', id };
}
