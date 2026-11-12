<script lang="ts">
  // The rule list: filters (family, severity, kind of check), a search over titles and texts, and the rows. j and k
  // move the cursor, Enter or a click opens the rule. The class of a rule is the hub's word for its file.
  import MultiSelect from 'appkit-ui/MultiSelect.svelte';
  import { TIP_ID } from 'appkit-ui/tooltip';
  import type { App } from '../lib/app.svelte';
  import { contextTarget } from 'appkit-ui/contextmenu';
  import { ruleTarget } from '../lib/context';
  import { checkKindOf, checkKindsOf, familiesOf, isActive, NO_CHECK, SEVERITIES } from '../lib/filter';
  import { labelOf, translate, type Key } from '../lib/i18n';
  import { shownText, titleText } from '../lib/ruletext';
  import { classLabel } from '../lib/sources';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  let search: HTMLInputElement | undefined = $state();
  // The words go, the focus stays in the field (the button it was on is gone).
  function clearQuery(): void {
    void app.setQuery('');
    search?.focus();
  }
  // Escape empties the words (Firefox would not); an empty field leaves Escape to the page. An open hint of the field
  // takes this Escape alone (the kit closes it, on the document), and so does a composition.
  function onSearchKey(e: KeyboardEvent & { currentTarget: HTMLInputElement }): void {
    const hintOpen = (e.currentTarget.getAttribute('aria-describedby') ?? '').split(' ').includes(TIP_ID);
    if (e.key === 'Escape' && !e.isComposing && !hintOpen && app.filter.query !== '') {
      e.preventDefault();
      clearQuery();
    }
  }
  // `/` takes the focus to the search box.
  $effect(() => {
    if (app.searchFocus > 0) search?.focus();
  });
  // The cursor stays in sight as j and k move it.
  $effect(() => {
    const id = app.selectedId;
    if (id !== null) document.getElementById(`rule-${id}`)?.scrollIntoView({ block: 'nearest' });
  });

  const familyOptions = $derived(familiesOf(app.rules).map((f) => ({ value: f, label: f })));
  const severityOptions = $derived(SEVERITIES.map((s) => ({ value: s, label: labelOf(app.lang, 'severity', s) })));
  const checkOptions = $derived(
    checkKindsOf(app.rules).map((k) => ({ value: k, label: k === NO_CHECK ? t('check.none') : k })),
  );
  const shown = $derived(app.shown);
  const hits = $derived(app.searchHits);
  const searching = $derived(hits !== null && hits.query === app.filter.query.trim());
  let bulkSeverity = $state<string>('critical');
  const writable = $derived(shown.filter((r) => app.canWrite(r)));
  const allMarked = $derived(writable.length > 0 && writable.every((r) => app.marked.has(r.id)));
</script>

<section aria-labelledby="rules-h">
  <h2 id="rules-h">{t('nav.rules')}</h2>
  <button type="button" class="new" onclick={() => app.openNewRule()}>{t('new.open')}</button>

  <div class="bar" role="group" aria-label={t('filter.label')}>
    <MultiSelect
      label={t('filter.family')}
      options={familyOptions}
      selected={app.filter.families}
      onchange={(families) => app.setFilter({ ...app.filter, families })}
      clearLabel={t('filter.clear_one', { label: t('filter.family') })}
      resetLabel={t('filter.reset_one')}
      searchLabel={t('filter.search_label')}
    />
    <MultiSelect
      label={t('filter.severity')}
      options={severityOptions}
      selected={app.filter.severities}
      onchange={(severities) => app.setFilter({ ...app.filter, severities })}
      clearLabel={t('filter.clear_one', { label: t('filter.severity') })}
      resetLabel={t('filter.reset_one')}
      searchLabel={t('filter.search_label')}
    />
    <MultiSelect
      label={t('filter.check')}
      options={checkOptions}
      selected={app.filter.checkKinds}
      onchange={(checkKinds) => app.setFilter({ ...app.filter, checkKinds })}
      clearLabel={t('filter.clear_one', { label: t('filter.check') })}
      resetLabel={t('filter.reset_one')}
      searchLabel={t('filter.search_label')}
    />
    <span class="search">
      <input
        bind:this={search}
        type="search"
        aria-label={t('filter.search')}
        placeholder={t('filter.search')}
        value={app.filter.query}
        oninput={(e) => void app.setQuery(e.currentTarget.value)}
        onkeydown={onSearchKey}
        autocomplete="off"
        spellcheck="false"
      />
      {#if app.filter.query !== ''}
        <button
          type="button"
          class="search-clear"
          aria-label={t('filter.search_clear')}
          title={t('filter.search_clear')}
          onmousedown={(e) => e.preventDefault()}
          onclick={clearQuery}>×</button
        >
      {/if}
    </span>
    {#if isActive(app.filter)}
      <button type="button" onclick={() => app.resetFilter()}>{t('filter.clear')}</button>
    {/if}
  </div>

  <p class="count" role="status">
    {t('list.count', { shown: shown.length, total: app.rules.length })}
    {#if searching && hits?.status === 'building'}<span class="note">{t('search.building')}</span>{/if}
    {#if searching && hits?.status === 'failed'}<span class="note">{t('search.failed')}</span>{/if}
    {#if searching && hits?.cut}<span class="note">{t('search.cut', { total: hits.total, shown: hits.ids.size })}</span>{/if}
  </p>

  {#if app.marked.size > 0}
    <div class="bulk" role="group" aria-label={t('bulk.marked', { n: app.marked.size })}>
      <span>{t('bulk.marked', { n: app.marked.size })}</span>
      <label>
        <span class="sr">{t('bulk.severity')}</span>
        <select bind:value={bulkSeverity}>
          {#each SEVERITIES as value (value)}<option {value}>{t(`severity.${value}`)}</option>{/each}
        </select>
      </label>
      <button type="button" disabled={app.writing} onclick={() => void app.writeMarked({ severity: bulkSeverity })}>{t('bulk.apply')}</button>
      <button type="button" disabled={app.writing} onclick={() => void app.writeMarked({ retire: true })}>{t('bulk.retire')}</button>
      <button type="button" onclick={() => app.clearMarks()}>{t('bulk.clear')}</button>
    </div>
  {/if}

  {#if shown.length === 0}
    <p class="muted">{t('list.empty')}</p>
  {:else}
    <table aria-label={t('list.label')}>
      <thead>
        <tr>
          <th scope="col">
            <input
              type="checkbox"
              aria-label={t('bulk.mark_all')}
              checked={allMarked}
              disabled={writable.length === 0}
              onchange={(e) => app.markShown(e.currentTarget.checked)}
            />
          </th>
          <th scope="col">{t('list.id')}</th>
          <th scope="col">{t('list.title')}</th>
          <th scope="col">{t('list.severity')}</th>
          <th scope="col">{t('list.check')}</th>
          <th scope="col">{t('list.class')}</th>
        </tr>
      </thead>
      <tbody>
        {#each shown as rule (rule.id)}
          {@const cls = app.classOfRule(rule)}
          <tr id={`rule-${rule.id}`} use:contextTarget={ruleTarget(rule.id)} class:cursor={app.selectedId === rule.id} aria-selected={app.selectedId === rule.id}>
            <td>
              <input
                type="checkbox"
                aria-label={t('bulk.mark', { id: rule.id })}
                checked={app.marked.has(rule.id)}
                disabled={!app.canWrite(rule)}
                onchange={() => app.toggleMark(rule.id)}
              />
            </td>
            <th scope="row"><code>{shownText(rule.id)}</code></th>
            <td class="title">
              <button type="button" onclick={() => void app.openDetail(rule.id)}>{shownText(titleText(rule.title, rule.id))}</button>
            </td>
            <td><span class={`sev ${rule.severity}`}>{labelOf(app.lang, 'severity', rule.severity)}</span></td>
            <td>{checkKindOf(rule) === NO_CHECK ? t('check.none') : shownText(checkKindOf(rule))}</td>
            <td><span class={`cls ${cls}`}>{t(classLabel(cls))}</span></td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    margin-bottom: 0.5rem;
  }
  .search {
    position: relative;
    display: flex;
    flex: 1 1 14rem;
    min-width: 10rem;
  }
  input[type='search'] {
    flex: 1;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.35rem 1.9rem 0.35rem 0.6rem;
  }
  /* Our own button clears, with a name; the browser's unnamed one would stand beside it. */
  input[type='search']::-webkit-search-cancel-button {
    appearance: none;
  }
  .search-clear {
    position: absolute;
    right: 0.2rem;
    top: 50%;
    transform: translateY(-50%);
    min-width: 1.5rem;
    min-height: 1.5rem;
    padding: 0 0.3rem;
    border: none;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    line-height: 1.4;
  }
  .search-clear:hover,
  .search-clear:focus-visible {
    color: var(--text);
  }
  .new,
  .bar > button {
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.3rem 0.7rem;
    cursor: pointer;
  }
  .bulk {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: center;
    margin: 0.4rem 0;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .bulk button {
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.15rem 0.6rem;
    cursor: pointer;
    color: var(--text);
  }
  .bulk select {
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .count {
    margin: 0.3rem 0;
    color: var(--muted);
  }
  .note {
    margin-left: 0.6rem;
    color: var(--warn);
  }
  .muted {
    color: var(--muted);
  }
  table {
    border-collapse: collapse;
    width: 100%;
    background: var(--surface);
    border: 1px solid var(--line);
  }
  th,
  td {
    padding: 0.3rem 0.6rem;
    border-bottom: 1px solid var(--line);
    text-align: left;
    vertical-align: baseline;
  }
  thead th {
    color: var(--muted);
    font-size: 0.8rem;
  }
  tr.cursor {
    background: var(--accent-soft);
  }
  .title button {
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
    text-align: left;
    color: var(--text);
    text-decoration: underline;
    text-decoration-color: var(--line);
  }
  .title button:hover {
    text-decoration-color: var(--accent);
  }
  .sev,
  .cls {
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    padding: 0.02rem 0.5rem;
    font-size: 0.78rem;
    white-space: nowrap;
  }
  .sev.critical {
    border-color: var(--bad);
    color: var(--bad);
  }
  .sev.recommended {
    border-color: var(--warn);
    color: var(--warn);
  }
  .cls.unclassified,
  .cls.private {
    border-color: var(--warn);
    color: var(--warn);
  }
</style>
