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
    cellDropOp,
    defOf,
    dropOp,
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
    type Refusal,
    type ViewDef,
  } from '../lib/views';
  import { neighbourCell, neighbourGroup } from 'appkit-ui/groups';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  $effect(() => {
    void app.loadViews();
  });

  const views = $derived(app.savedViews?.views ?? []);
  const problems = $derived(app.savedViews?.problems ?? []);
  const saved = $derived<SavedView | null>(views.find((v) => v.id === app.viewId) ?? views[0] ?? null);
  // A view opened from the context menu is shown as a saved one is, without a file (nothing to rename or delete).
  const current = $derived<SavedView | null>(
    app.transientView ? { ...app.transientView, id: '', etag: '' } : saved,
  );
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

  /** The view opened from the context menu becomes a file of the vault, and is shown as that. */
  async function saveTransient(): Promise<void> {
    const def = app.transientView;
    if (!def) return;
    const outcome = await app.write([viewSaveOp(def)]);
    if (outcome && outcome.saved > 0) {
      app.transientView = null;
      app.viewId = outcome.ids[0] ?? null;
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

  // -- a drop writes the field --

  const itemOf = (id: string) => items.find((i) => i.id === id);
  /** Whether the field of the view can be written by a drag at all (the engine names how). */
  const movable = $derived(!!first?.writable && first.mode !== undefined && !app.writing);

  function refuse(reason: Refusal): void {
    if (reason !== 'same') app.notice = { key: `views.refused.${reason}` as Key };
  }

  async function drop(id: string, from: string, to: string): Promise<void> {
    const item = itemOf(id);
    if (!item || !first) return;
    const d = dropOp(item, first, from, to);
    if (d.kind === 'refused') return refuse(d.reason);
    await app.write([d.op]);
    focusItem(id);
  }

  function canDrop(id: string, from: string, to: string): boolean {
    const item = itemOf(id);
    return !!item && !!first && dropOp(item, first, from, to).kind === 'op';
  }

  /** The keys stay on the rule that moved, in its new group. */
  function focusItem(id: string): void {
    requestAnimationFrame(() => document.querySelector<HTMLElement>(`.views [data-view-item="${CSS.escape(id)}"]`)?.focus());
  }

  /** Shift with h or l on a rule: into the group before or after, as the keys of tasks-desktop. */
  function onItemKey(event: KeyboardEvent, id: string, from: string): void {
    if (!event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
    const key = event.key.toLowerCase();
    if (key !== 'h' && key !== 'l') return;
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat || app.writing) return;
    // A field the engine does not write: said, not silently ignored.
    if (!movable) return refuse('read_only');
    const to = neighbourGroup(groups, from, key === 'l' ? 1 : -1);
    if (to !== null) void drop(id, from, to);
  }

  // A matrix: a rule of the opened cell is dragged onto another cell, or moved with Shift and h, l, j, k.
  let draggingCell = $state<{ id: string; from: Cell } | null>(null);

  async function dropCell(id: string, from: Cell, to: Cell): Promise<void> {
    const item = itemOf(id);
    if (!item || !first || !second) return;
    const d = cellDropOp(item, first, second, from, to);
    if (d.kind === 'refused') return refuse(d.reason);
    await app.write([d.op]);
    opened = to;
    focusItem(id);
  }

  function onCellItemKey(event: KeyboardEvent, id: string, at: Cell): void {
    if (!event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
    const moves: Record<string, 'up' | 'down' | 'left' | 'right'> = { h: 'left', l: 'right', k: 'up', j: 'down' };
    const move = moves[event.key.toLowerCase()];
    if (!move) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return;
    const to = neighbourCell(rows, columns, at, move);
    if (to !== null) void dropCell(id, at, to);
  }

  const label = (value: string) => valueLabel(first, value);
  const rowLabel = (value: string) => valueLabel(first, value);
  const columnLabel = (value: string) => valueLabel(second, value);
  const countOfCell = (cell: Cell) => cells.find((c) => c.row === cell.row && c.column === cell.column)?.items.length ?? 0;
</script>

{#snippet rule(item: Item, group: Group<Item>)}
  {#if item.rule}
    <button
      type="button"
      class="rule"
      data-view-item={item.id}
      onclick={() => void app.openDetail(item.id)}
      onkeydown={(e) => onItemKey(e, item.id, group.value)}
    >
      <span class="id">{item.id}</span>
      <span class="title">{item.title}</span>
    </button>
  {:else}
    <!-- A retired rule has no detail: a key moves it all the same (back to active, by status). -->
    <button type="button" class="rule retired" data-view-item={item.id} onkeydown={(e) => onItemKey(e, item.id, group.value)}>
      <span class="id">{item.id}</span> <span class="title">{t('views.value.status.retired')}</span>
    </button>
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
          <select
            aria-labelledby="views-which"
            value={app.transientView ? '' : (saved?.id ?? '')}
            onchange={(e) => {
              app.transientView = null;
              app.viewId = e.currentTarget.value;
            }}
          >
            {#if app.transientView}<option value="">{t('views.transient')}</option>{/if}
            {#each views as v (v.id)}<option value={v.id}>{v.title}</option>{/each}
          </select>
        </label>
      {/if}
      {#if app.transientView}
        <p class="transient" role="status">
          <span>{t('views.transient.note', { title: app.transientView.title })}</span>
          <button type="button" disabled={app.writing} onclick={() => void saveTransient()}>{t('views.transient.save')}</button>
        </p>
      {:else if views.length > 0}
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
          dragging={draggingCell !== null}
          ondropcell={(cell) => {
            const from = draggingCell;
            draggingCell = null;
            if (from) void dropCell(from.id, from.from, cell);
          }}
        />
        {#if opened}
          <ul class="cell-items" aria-label={t('views.cell', { row: rowLabel(opened.row), column: columnLabel(opened.column), n: openedItems.length })}>
            {#each openedItems as item (item.id)}
              {@const at = opened}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <!-- The keys come from the button of the rule inside (they rise to the row); the row is what is dragged. -->
              <li
                draggable={!app.writing}
                ondragstart={(e) => {
                  draggingCell = { id: item.id, from: at };
                  e.dataTransfer?.setData('text/plain', item.id);
                }}
                ondragend={() => (draggingCell = null)}
                onkeydown={(e) => onCellItemKey(e, item.id, at)}
              >
                {@render rule(item, { value: '', items: [] })}
              </li>
            {/each}
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
          busy={app.writing}
          ondrop={movable ? (id, from, to) => void drop(id, from, to) : undefined}
          {canDrop}
          gripHint={t('views.grip')}
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
  .transient {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    margin: 0;
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
