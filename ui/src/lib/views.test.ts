import { groupsOf } from 'appkit-ui/groups';
import { describe, expect, it } from 'vitest';
import type { Grouping } from './views';
import {
  defOf,
  EVERY,
  fieldOf,
  isViewOp,
  itemsOf,
  keyOf,
  rulesetOf,
  selected,
  valuesOf,
  viewDeleteOp,
  viewSaveOp,
  waiversOf,
} from './views';
import { groupingsDoc, snapshotDoc, viewsDoc, waiversDoc } from './fixtures';

const ctx = { waivers: waiversOf(waiversDoc()) };
const field = (name: string): Grouping => fieldOf(groupingsDoc(), name)!;

describe('the rules of a view', () => {
  it('are the loaded rules and the retired ones, each with its rule set', () => {
    const snap = snapshotDoc();
    snap.retired = [{ id: 'OLD-01', family: 'OLD', source: 'Checklists/_retired/OLD/OLD-01.md' }];
    const items = itemsOf(snap);
    expect(items.map((i) => [i.id, i.status])).toEqual([
      ['DEP-01', 'active'],
      ['DEP-09', 'active'],
      ['ERR-100', 'active'],
      ['LUA-01', 'active'],
      ['OLD-01', 'retired'],
    ]);
    expect(items.at(-1)?.severity).toBeNull();
    expect(rulesetOf('lua/regeln/LUA.md')).toBe('lua');
    expect(rulesetOf('lone')).toBe('lone');
    expect(itemsOf(null)).toEqual([]);
  });

  it('read each field: family, severity, status, waiver, and the section of a layout', () => {
    const [dep01, dep09, err] = itemsOf(snapshotDoc());
    expect(valuesOf(dep01!, field('family'), ctx)).toEqual(['DEP']);
    expect(valuesOf(dep01!, field('severity'), ctx)).toEqual([dep01!.severity]);
    expect(valuesOf(dep01!, field('status'), ctx)).toEqual(['active']);
    expect(valuesOf(dep01!, field('waiver'), ctx)).toEqual(['active']);
    expect(valuesOf(dep09!, field('waiver'), ctx)).toEqual(['expired']);
    expect(valuesOf(err!, field('waiver'), ctx)).toEqual([]);
    const layout: Grouping = {
      field: 'layout',
      ruleset: 'Checklists',
      layout: 'regeln/DEP.md',
      values: ['A', 'B'],
      truncated: false,
      writable: true,
      mode: 'move',
      members: [{ id: 'DEP-01', value: 'B' }],
    };
    expect(keyOf(layout)).toBe('layout:Checklists:regeln/DEP.md');
    expect(valuesOf(dep01!, layout, ctx)).toEqual(['B']);
    expect(valuesOf(dep09!, layout, ctx)).toEqual([]);
  });

  it('lay in the groups of the kit: every value of the engine, the empty ones too', () => {
    const groups = groupsOf(itemsOf(snapshotDoc()), field('family'), (i, g) => valuesOf(i, g, ctx));
    expect(groups.map((g) => [g.value, g.items.map((i) => i.id)])).toEqual([
      ['DEP', ['DEP-01', 'DEP-09']],
      ['ERR', ['ERR-100']],
      ['LUA', ['LUA-01']],
    ]);
  });

  it('are narrowed by the selection of a view: each list that names something, and the query', () => {
    const items = itemsOf(snapshotDoc());
    expect(selected(items, EVERY)).toHaveLength(4);
    expect(selected(items, { ...EVERY, families: ['DEP'] }).map((i) => i.id)).toEqual(['DEP-01', 'DEP-09']);
    expect(selected(items, { ...EVERY, statuses: ['retired'] })).toEqual([]);
    expect(selected(items, { ...EVERY, query: 'err-1' }).map((i) => i.id)).toEqual(['ERR-100']);
  });
});

describe('the saved views', () => {
  it('know their field, and a field the engine names no more is none', () => {
    expect(fieldOf(groupingsDoc(), 'severity')?.mode).toBe('set');
    expect(fieldOf(groupingsDoc(), 'layout:Checklists:gone.md')).toBeNull();
    expect(fieldOf(null, 'severity')).toBeNull();
  });

  it('are written with view_save and view_delete, on the version read', () => {
    const view = viewsDoc().views[0]!;
    const def = defOf(view);
    expect(def).toEqual({ title: 'By severity', display: 'columns', group_by: 'severity', collapsed: [], select: view.select });
    expect(viewSaveOp(def)).toEqual({ op: 'view_save', view: def });
    expect(viewSaveOp(def, view.id, view.etag)).toEqual({ op: 'view_save', view: def, id: view.id, if_match: view.etag });
    expect(viewDeleteOp(view.id, view.etag)).toEqual({ op: 'view_delete', id: view.id, if_match: view.etag });
    expect(isViewOp(viewDeleteOp('a', 'b'))).toBe(true);
    expect(isViewOp({ op: 'set' })).toBe(false);
  });
});
