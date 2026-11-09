import { describe, expect, it } from 'vitest';
import type { Result } from './contract';
import {
  clientOpId,
  isWaiverOp,
  newOp,
  nextOf,
  reinstateOp,
  removeOp,
  renewOp,
  retireOp,
  setOp,
  summarize,
  waiveOp,
  type Op,
} from './write';
import type { NextId } from './contract';

/** An answer of the engine with the entries a test names. */
function result(entries: unknown[], dryRun = false): Result {
  return { kind: 'rules.result', dry_run: dryRun, ok: true, results: entries } as unknown as Result;
}

const set: Op = { op: 'set', id: 'DEP-01', patch: { severity: 'critical' }, if_match: 'sha256:aaaa' };

describe('summarize', () => {
  it('counts what changed, keeps the warnings and builds the way back with the tag the file has now', () => {
    const out = summarize(
      [set],
      result([
        {
          n: 1,
          ok: true,
          op: 'set',
          id: 'DEP-01',
          changed_ids: ['DEP-01'],
          etag_after: 'sha256:bbbb',
          warnings: ['pull first'],
          inverse: { op: 'set', id: 'DEP-01', patch: { severity: 'recommended' } },
        },
      ]),
    );
    expect(out.saved).toBe(1);
    expect(out.changedIds).toEqual(['DEP-01']);
    expect(out.warnings).toEqual(['pull first']);
    expect(out.undo).toEqual([
      { op: 'set', id: 'DEP-01', patch: { severity: 'recommended' }, if_match: 'sha256:bbbb' },
    ]);
  });

  it('puts the way back of several operations in the opposite order', () => {
    const entries = [1, 2, 3].map((n) => ({
      n,
      ok: true,
      op: 'set',
      id: `DEP-0${n}`,
      changed_ids: [`DEP-0${n}`],
      etag_after: `sha256:${n}`,
      inverse: { op: 'set', id: `DEP-0${n}`, patch: {} },
    }));
    const out = summarize([], result(entries));
    expect(out.undo.map((o) => o['id'])).toEqual(['DEP-03', 'DEP-02', 'DEP-01']);
  });

  it('does not add a tag where the inverse takes none, nor replace one it brings', () => {
    const out = summarize(
      [],
      result([
        { n: 1, ok: true, op: 'retire', id: 'A-1', changed_ids: ['A-1'], etag_after: 'sha256:1', inverse: { op: 'reinstate', id: 'A-1', places: {} } },
        { n: 2, ok: true, op: 'set', id: 'A-2', changed_ids: ['A-2'], etag_after: 'sha256:2', inverse: { op: 'set', id: 'A-2', patch: {}, if_match: 'sha256:own' } },
      ]),
    );
    expect(out.undo[1]).toEqual({ op: 'reinstate', id: 'A-1', places: {} });
    expect(out.undo[0]?.['if_match']).toBe('sha256:own');
  });

  it('saves nothing and offers no way back for a dry run or an entry that changed nothing', () => {
    const entry = { n: 1, ok: true, op: 'set', id: 'DEP-01', changed_ids: ['DEP-01'], inverse: { op: 'set', id: 'DEP-01', patch: {} } };
    expect(summarize([set], result([entry], true))).toMatchObject({ saved: 0, undo: [] });
    expect(summarize([set], result([{ ...entry, changed_ids: [] }]))).toMatchObject({ saved: 0, undo: [] });
  });

  it('counts a view made, changed or deleted although it changes no rule, with its way back, and not one left as it was', () => {
    const save: Op = { op: 'view_save', view: { title: 'T' } };
    const made = { n: 1, ok: true, op: 'view_save', id: 't', outcome: 'created', changed_ids: [], inverse: { op: 'view_delete', id: 't', if_match: 'sha256:1' } };
    expect(summarize([save], result([made]))).toMatchObject({ saved: 1, ids: ['t'], undo: [{ op: 'view_delete', id: 't', if_match: 'sha256:1' }] });
    expect(summarize([save], result([{ ...made, outcome: 'unchanged', inverse: undefined }]))).toMatchObject({ saved: 0, undo: [] });
    expect(summarize([save], result([made], true))).toMatchObject({ saved: 0, undo: [] });
  });

  it('takes the id of a failed entry from the operation when the entry does not say it', () => {
    const out = summarize(
      [set],
      result([{ n: 1, ok: false, op: 'set', changed_ids: [], error: { code: 'conflict', details: { expected: 'a', actual: 'b' } } }]),
    );
    expect(out.conflicts[0]?.id).toBe('DEP-01');
  });

  it('tells a conflict from another failure and keeps what was asked', () => {
    const out = summarize(
      [set],
      result([
        { n: 1, ok: false, op: 'set', id: 'DEP-01', changed_ids: [], error: { code: 'conflict', details: { expected: 'sha256:aaaa', actual: 'sha256:cccc' } } },
        { n: 2, ok: false, op: 'set', id: 'DEP-02', changed_ids: [], error: { code: 'invalid_argument', message: 'no' } },
        { n: 3, ok: false, op: 'set', id: 'DEP-03', changed_ids: [] },
      ]),
    );
    expect(out.conflicts).toEqual([{ id: 'DEP-01', n: 1, expected: 'sha256:aaaa', actual: 'sha256:cccc', op: set }]);
    expect(out.failures).toEqual([
      { n: 2, id: 'DEP-02', code: 'invalid_argument', message: 'no' },
      { n: 3, id: 'DEP-03', code: 'failed', message: '' },
    ]);
    expect(out.saved).toBe(0);
  });
});

describe('the operations the page builds', () => {
  it('always carry the tag the person saw for set and retire', () => {
    expect(setOp('DEP-01', { severity: 'critical' }, 'sha256:1')).toEqual({
      op: 'set',
      id: 'DEP-01',
      patch: { severity: 'critical' },
      if_match: 'sha256:1',
    });
    expect(retireOp('DEP-01', 'sha256:1')).toEqual({ op: 'retire', id: 'DEP-01', if_match: 'sha256:1' });
    expect(retireOp('DEP-01', 'sha256:1', { replaced_by: 'DEP-02', reason: 'merged' })).toEqual({
      op: 'retire',
      id: 'DEP-01',
      if_match: 'sha256:1',
      replaced_by: 'DEP-02',
      reason: 'merged',
    });
    expect(reinstateOp('DEP-01')).toEqual({ op: 'reinstate', id: 'DEP-01' });
  });

  it('give a client id the hub takes', () => {
    for (const [now, n] of [[0, 0], [1_790_000_000_000, 12345]] as const) {
      expect(clientOpId(now, n)).toMatch(/^[A-Za-z0-9._:-]{1,64}$/);
    }
    expect(clientOpId(5, 1)).not.toBe(clientOpId(5, 2));
  });
});

describe('a new rule', () => {
  it('leaves out what is empty and keeps the rest as it was typed', () => {
    expect(newOp({ family: 'DEP', title: 't', severity: 'critical' })).toEqual({
      op: 'new',
      family: 'DEP',
      title: 't',
      severity: 'critical',
    });
    expect(
      newOp({ family: 'DEP', title: 't', severity: 'critical', text: 'x', check: { type: 'grep' }, agent: { q: 1 }, ruleset: 'one/Checklists' }),
    ).toEqual({
      op: 'new',
      family: 'DEP',
      title: 't',
      severity: 'critical',
      text: 'x',
      check: { type: 'grep' },
      agent: { q: 1 },
      ruleset: 'one/Checklists',
    });
    expect(newOp({ family: 'A', title: 't', severity: 's', text: '', check: null, ruleset: '' })).not.toHaveProperty('check');
  });

  it('reads the next id of a family from the answer of the engine', () => {
    const doc = { items: [{ family: 'DEP', next: 'DEP-10' }, { family: 'ERR' }] } as unknown as NextId;
    expect(nextOf(doc, 'DEP')).toBe('DEP-10');
    expect(nextOf(doc, 'ERR')).toBeNull();
    expect(nextOf(doc, 'NEW')).toBeNull();
  });

  it('names the id of a dry run, which saves nothing', () => {
    const out = summarize(
      [],
      { kind: 'rules.result', dry_run: true, ok: true, results: [{ n: 1, ok: true, op: 'new', id: 'DEP-10', changed_ids: [] }] } as unknown as Result,
    );
    expect(out.ids).toEqual(['DEP-10']);
    expect(out.saved).toBe(0);
  });
});

describe('the operations on the waivers', () => {
  it('always carry the tag of the waivers file the person read, and leave out what is empty', () => {
    expect(waiveOp({ id: 'DEP-01', reason: 'legacy' }, 'sha256:1')).toEqual({
      op: 'waive',
      id: 'DEP-01',
      reason: 'legacy',
      if_match: 'sha256:1',
    });
    expect(waiveOp({ id: 'DEP-01', reason: 'r', expires: '2027-01-31', paths: ['src', 'docs/**'] }, 'sha256:1')).toEqual({
      op: 'waive',
      id: 'DEP-01',
      reason: 'r',
      expires: '2027-01-31',
      paths: ['src', 'docs/**'],
      if_match: 'sha256:1',
    });
    expect(waiveOp({ id: 'A-1', reason: 'r', expires: '', paths: [] }, 't')).not.toHaveProperty('expires');
    expect(renewOp('DEP-01', { expires: '2028-01-01' }, 'sha256:1')).toEqual({
      op: 'renew',
      id: 'DEP-01',
      expires: '2028-01-01',
      if_match: 'sha256:1',
    });
    expect(renewOp('DEP-01', { no_expiry: true }, 'sha256:1', 'still needed')).toEqual({
      op: 'renew',
      id: 'DEP-01',
      no_expiry: true,
      reason: 'still needed',
      if_match: 'sha256:1',
    });
    expect(removeOp('DEP-01', 'sha256:1')).toEqual({ op: 'remove', id: 'DEP-01', if_match: 'sha256:1' });
  });

  it('are told from the operations on rules', () => {
    expect(isWaiverOp(waiveOp({ id: 'A-1', reason: 'r' }, 't'))).toBe(true);
    expect(isWaiverOp(removeOp('A-1', 't'))).toBe(true);
    expect(isWaiverOp(setOp('A-1', {}, 't'))).toBe(false);
    expect(isWaiverOp(null)).toBe(false);
  });
});
