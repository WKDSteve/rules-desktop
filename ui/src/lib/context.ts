// What the context menu offers (the kit's `ContextRegistry`): a family or a rule set stands for a selection of rules,
// and "open as a view" shows those rules grouped by a field of the engine in the tab Views, without saving; "save as a
// view" there makes a file of it. The page declares the targets (`use:contextTarget`); what an entry does goes
// through the methods of `App` the buttons use.
import { ContextRegistry, type ContextTarget } from 'appkit-ui/contextmenu';
import type { MenuItem } from 'appkit-ui/menu';
import type { App } from './app.svelte';
import { translate, type Key } from './i18n';
import { EVERY, type SavedView } from './views';

/** The fields a selection is offered grouped by (a family grouped by family would be one group). */
const FIELDS = ['severity', 'status', 'waiver', 'family'] as const;

/** A family or a rule set as a selection of rules: its title and what it selects. */
export function selectionTarget(title: string, select: Partial<SavedView['select']>): ContextTarget {
  return { kind: 'selection', id: title, data: { title, select: { ...EVERY, ...select } } };
}

export function createContextRegistry(app: App): ContextRegistry {
  const registry = new ContextRegistry();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  registry.provide('selection', (target) => {
    const data = target.data as { title: string; select: SavedView['select'] };
    const items: MenuItem[] = [{ kind: 'section', label: t('ctx.open_as') }];
    for (const field of FIELDS) {
      // A selection of one family is not grouped by family.
      if (field === 'family' && data.select.families.length === 1) continue;
      items.push({
        kind: 'action',
        label: t('ctx.open_as.by', { field: t(`views.field.${field}`) }),
        onselect: () => app.openTransientView({ title: data.title, display: 'columns', group_by: field, collapsed: [], select: data.select }),
      });
    }
    return items;
  });

  return registry;
}
