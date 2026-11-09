<script lang="ts">
  // The saved views of the rules: a view groups the rules by a field of the engine (`rules.groupings`) as columns, a
  // list of sections that fold, or a matrix of two fields, and narrows them to its selection. The views are files of
  // the vault (`rules.views`), the same on every machine: made, renamed, folded and deleted through `view_save` and
  // `view_delete`, on the version read. The groups are the kit's; nothing here names a field.
  import GroupMatrix from 'appkit-ui/GroupMatrix.svelte';
  import Groups from 'appkit-ui/Groups.svelte';
  import { cellsOf, groupsOf, NONE, type Cell, type Group } from 'appkit-ui/groups';
  import type { App } from '../lib/app.svelte';
  import { translate, type Key } from '../lib/i18n';
  import {
    defOf,
    EVERY,
    fieldOf,
    itemsOf,
    keyOf,
    selected,
    valuesOf,
    viewDeleteOp,
    viewSaveOp,
    waiversOf,
    type Grouping,
    type Item,
    type SavedView,
    type ViewDef,
  } from '../lib/views';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  $effect(() => {
    void app.loadViews();
  });

  const views = $derived(app.savedViews?.views ?? []);
  const problems = $derived(app.savedViews?.problems ?? []);
  const current = $derived<SavedView | null>(views.find((v) => v.id === app.viewId) ?? views[0] ?? null);
  const ctx = $derived({ waivers: waiversOf(app.waivers) });
  const read = (item: Item, grouping: Grouping) => valuesOf(item, grouping, ctx);
  const items = $derived(current ? selected(itemsOf(app.snapshot), current.select) : []);
  const first = $derived(fieldOf(app.groupings, current?.group_by));
  const second = $derived(fieldOf(app.groupings, current?.group_by2));
  const groups = $derived(first ? groupsOf(items, first, read) : []);
  const rows = $derived(first && second ? groupsOf(items, first, read) : []);
  const columns = $derived(first && second ? groupsOf(items, second, read) : []);
  const cells = $derived(first && second ? cellsOf(items, first, second, read) : []);
  let opened = $state<Cell | null>(null);
  const openedItems = $derived(opened ? (cells.find((c) => c.row === opened!.row && c.column === opened!.column)?.items ?? []) : []);

  /** The words of a field, as the select and the corner of a matrix show it. */
  function fieldName(grouping: Grouping): string {
    if (grouping.field === 'layout') return t('views.field.layout', { layout: `${grouping.ruleset ?? ''}/${grouping.layout ?? ''}` });
    return t(`views.field.${grouping.field}` as Key);
  }

  /** The words of a value of a field: a severity and a status in the words of the page, the rest as it is. */
  function valueLabel(grouping: Grouping | null, value: string): string {
    if (value === NONE) return t('views.none');
    if (grouping?.field === 'severity') return t(`severity.${value}` as Key);
    if (grouping?.field === 'status' || grouping?.field === 'waiver') return t(`views.value.${grouping.field}.${value}` as Key);
    return value;
  }

  // -- making, renaming, deleting --

  let title = $state('');
  let display = $state<SavedView['display']>('columns');
  let groupBy = $state('family');
  let groupBy2 = $state('severity');
  let renaming = $state('');
  let confirmDelete = $state(false);
  // The form for a new view: open by itself while there is none, else as the person leaves it (a reading of the views
  // again must not fold it under the person's typing).
  let newOpen = $state(false);
  $effect(() => {
    if (app.viewsState === 'ready' && views.length === 0) newOpen = true;
  });

  async function make(): Promise<void> {
    if (title.trim() === '') return;
    const def: ViewDef = {
      title: title.trim(),
      display,
      group_by: groupBy,
      ...(display === 'matrix' ? { group_by2: groupBy2 } : {}),
      collapsed: [],
      select: EVERY,
    };
    const outcome = await app.write([viewSaveOp(def)]);
    if (outcome && outcome.saved > 0) {
      // The engine named the file (a title that is taken gets -2, -3): the view it made is the one shown.
      app.viewId = outcome.ids[0] ?? null;
      title = '';
    }
  }

  async function rename(): Promise<void> {
    if (!current || renaming.trim() === '' || renaming.trim() === current.title) return;
    const outcome = await app.write([viewSaveOp({ ...defOf(current), title: renaming.trim() }, current.id, current.etag)]);
    if (outcome && outcome.saved > 0) renaming = '';
  }

  async function remove(): Promise<void> {
    if (!current) return;
    confirmDelete = false;
    const outcome = await app.write([viewDeleteOp(current.id, current.etag)]);
    if (outcome && outcome.saved > 0) app.viewId = null;
  }

  /** A group of the list folded or opened: kept in the file of the view, like every other part of it. */
  function toggle(value: string): void {
    if (!current) return;
    const collapsed = current.collapsed.includes(value) ? current.collapsed.filter((v) => v !== value) : [...current.collapsed, value];
    void app.write([viewSaveOp({ ...defOf(current), collapsed }, current.id, current.etag)], { undoable: false });
  }

  const label = (value: string) => valueLabel(first, value);
  const rowLabel = (value: string) => valueLabel(first, value);
  const columnLabel = (value: string) => valueLabel(second, value);
  const countOfCell = (cell: Cell) => cells.find((c) => c.row === cell.row && c.column === cell.column)?.items.length ?? 0;
</script>

{#snippet rule(item: Item, _group: Group<Item>)}
  {#if item.rule}
    <button type="button" class="rule" onclick={() => void app.openDetail(item.id)}>
      <span class="id">{item.id}</span>
      <span class="title">{item.title}</span>
    </button>
  {:else}
    <span class="rule retired"><span class="id">{item.id}</span> <span class="title">{t('views.value.status.retired')}</span></span>
  {/if}
{/snippet}

<section class="views" aria-label={t('nav.views')}>
  <h2>{t('nav.views')}</h2>
  {#if app.viewsState === 'failed'}
    <p class="muted" role="status">{t('views.failed')}</p>
  {:else}
    <div class="bar">
      {#if views.length > 0}
        <label>
          <span id="views-which">{t('views.which')}</span>
          <select aria-labelledby="views-which" value={current?.id ?? ''} onchange={(e) => (app.viewId = e.currentTarget.value)}>
            {#each views as v (v.id)}<option value={v.id}>{v.title}</option>{/each}
          </select>
        </label>
        <form class="inline" onsubmit={(e) => (e.preventDefault(), void rename())}>
          <label>
            <span>{t('views.rename')}</span>
            <input type="text" bind:value={renaming} placeholder={current?.title ?? ''} maxlength="300" />
          </label>
          <button type="submit" disabled={app.writing || renaming.trim() === ''}>{t('views.rename.save')}</button>
        </form>
        {#if confirmDelete}
          <span class="confirm" role="group" aria-label={t('views.delete.ask', { title: current?.title ?? '' })}>
            <span>{t('views.delete.ask', { title: current?.title ?? '' })}</span>
            <button type="button" onclick={() => void remove()}>{t('views.delete.yes')}</button>
            <button type="button" onclick={() => (confirmDelete = false)}>{t('views.delete.no')}</button>
          </span>
        {:else}
          <button type="button" disabled={app.writing} onclick={() => (confirmDelete = true)}>{t('views.delete')}</button>
        {/if}
      {/if}
    </div>

    <details class="new" bind:open={newOpen}>
      <summary>{t('views.new')}</summary>
      <form onsubmit={(e) => (e.preventDefault(), void make())}>
        <label><span>{t('views.new.title')}</span><input type="text" bind:value={title} maxlength="300" required /></label>
        <label>
          <span>{t('views.new.display')}</span>
          <select bind:value={display}>
            {#each ['columns', 'list', 'matrix'] as const as d (d)}<option value={d}>{t(`views.display.${d}`)}</option>{/each}
          </select>
        </label>
        <label>
          <span>{t('views.new.group_by')}</span>
          <select bind:value={groupBy}>
            {#each app.groupings?.fields ?? [] as g (keyOf(g))}<option value={keyOf(g)}>{fieldName(g)}</option>{/each}
          </select>
        </label>
        {#if display === 'matrix'}
          <label>
            <span>{t('views.new.group_by2')}</span>
            <select bind:value={groupBy2}>
              {#each (app.groupings?.fields ?? []).filter((g) => keyOf(g) !== groupBy) as g (keyOf(g))}<option value={keyOf(g)}>{fieldName(g)}</option>{/each}
            </select>
          </label>
        {/if}
        <button type="submit" disabled={app.writing || title.trim() === ''}>{t('views.new.save')}</button>
      </form>
    </details>

    {#if problems.length > 0}
      <ul class="problems" aria-label={t('views.problems')}>
        {#each problems as p (p.id)}<li><code>{p.file}</code>: {p.message}</li>{/each}
      </ul>
    {/if}

    {#if current}
      {#if !first || (current.display === 'matrix' && !second)}
        <p class="muted" role="status">{t('views.field_gone')}</p>
      {:else if current.display === 'matrix' && second}
        <GroupMatrix
          {rows}
          {columns}
          count={countOfCell}
          {rowLabel}
          {columnLabel}
          label={current.title}
          corner={`${fieldName(first)} \\ ${fieldName(second)}`}
          cellLabel={(cell, n) => t('views.cell', { row: rowLabel(cell.row), column: columnLabel(cell.column), n })}
          {opened}
          onopen={(cell) => (opened = opened && opened.row === cell.row && opened.column === cell.column ? null : cell)}
        />
        {#if opened}
          <ul class="cell-items" aria-label={t('views.cell', { row: rowLabel(opened.row), column: columnLabel(opened.column), n: openedItems.length })}>
            {#each openedItems as item (item.id)}<li>{@render rule(item, { value: '', items: [] })}</li>{/each}
          </ul>
        {/if}
      {:else}
        <Groups
          {groups}
          mode={current.display === 'list' ? 'list' : 'columns'}
          {label}
          item={rule}
          collapsed={current.collapsed}
          ontoggle={toggle}
          columnsLabel={current.title}
        />
      {/if}
    {:else if app.viewsState === 'ready'}
      <p class="muted">{t('views.empty')}</p>
    {/if}
  {/if}
</section>

<style>
  .views {
    display: grid;
    gap: 0.75rem;
  }
  h2 {
    margin: 0;
    font-size: 1.1rem;
  }
  .bar,
  .inline,
  .new form,
  .confirm {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: end;
  }
  label {
    display: grid;
    gap: 0.15rem;
    font-size: 0.85rem;
  }
  .muted {
    margin: 0;
    color: var(--muted);
  }
  .problems {
    margin: 0;
    padding-left: 1.2rem;
    color: var(--warn);
    font-size: 0.85rem;
  }
  .rule {
    display: grid;
    gap: 0.1rem;
    width: 100%;
    text-align: left;
    padding: 0.3rem 0.4rem;
    border: none;
    background: none;
    color: var(--text);
    cursor: pointer;
  }
  .rule .id {
    font-family: var(--mono, monospace);
    font-size: 0.8rem;
    color: var(--text);
  }
  .rule.retired {
    opacity: 0.75;
  }
  .cell-items {
    margin: 0;
    padding-left: 1.2rem;
  }
</style>
