import { buildContextMenu } from 'appkit-ui/contextmenu';
import { describe, expect, it } from 'vitest';
import { App } from './app.svelte';
import { createContextRegistry, ruleTarget, selectionTarget } from './context';
import { FakeHub, testEnv } from './testhub';

function app(): App {
  return new App(testEnv(new FakeHub()).env);
}

describe('the context menu of a family or a rule set', () => {
  it('opens its rules as a view grouped by a field, without saving, and a family is not grouped by family', () => {
    const a = app();
    const items = buildContextMenu(createContextRegistry(a), [selectionTarget('DEP', { families: ['DEP'] })]);
    const actions = items.filter((i) => i.kind === 'action');
    expect(actions.map((i) => i.label)).toEqual(['Grouped by Severity', 'Grouped by Status', 'Grouped by Waiver']);
    (actions[0] as { onselect: () => void }).onselect();
    expect(a.view).toBe('views');
    expect(a.transientView).toEqual({
      title: 'DEP',
      display: 'columns',
      group_by: 'severity',
      collapsed: [],
      select: { families: ['DEP'], severities: [], statuses: [], rulesets: [] },
    });
  });

  it('offers the family as a field for a rule set', () => {
    const items = buildContextMenu(createContextRegistry(app()), [selectionTarget('alpha', { rulesets: ['alpha'] })]);
    expect(items.filter((i) => i.kind === 'action').map((i) => i.label)).toContain('Grouped by Family');
  });
});

describe('the context menu of a rule row', () => {
  it('offers details, the editor and the id, and the details entry opens the rule', () => {
    const a = app();
    const items = buildContextMenu(createContextRegistry(a), [ruleTarget('DEP-01')]);
    const actions = items.filter((i) => i.kind === 'action');
    expect(actions.map((i) => i.label)).toEqual(['Open details', 'Show in the editor', 'Copy the id']);
    (actions[0] as { onselect: () => void }).onselect();
    expect(a.selectedId).toBe('DEP-01');
  });

  it('is the innermost target first: a rule row inside a family offers its own entries first', () => {
    const items = buildContextMenu(createContextRegistry(app()), [ruleTarget('DEP-01'), selectionTarget('DEP', { families: ['DEP'] })]);
    expect(items.find((i) => i.kind === 'action')?.label).toBe('Open details');
  });
});
