<script lang="ts">
  // Which rule applies how: per family and per gate, how many rules have a check that runs by itself, a question for an
  // agent, or are worked by a person. The engine says it for each rule; the page adds up. A row opens the list narrowed
  // to its family (a gate narrows to its families). Nothing here runs a check: that is for docmap-desktop.
  import type { App } from '../lib/app.svelte';
  import { byFamily, byGate, percent, type CoverageRow } from '../lib/coverage';
  import { translate, type Key } from '../lib/i18n';
  import { shownText } from '../lib/ruletext';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  const families = $derived(byFamily(app.rules));
  const gates = $derived(byGate(app.snapshot?.gates ?? [], app.rules));
  const gateFamilies = $derived(new Map((app.snapshot?.gates ?? []).map((g) => [g.name ?? '', g.families ?? []])));

  function share(row: CoverageRow, part: 'check' | 'agent' | 'manual'): string {
    return `${row[part]} (${percent(row[part], row.total)} %)`;
  }
</script>

<section aria-labelledby="cov-h">
  <h2 id="cov-h">{t('nav.coverage')}</h2>
  <p class="muted">{t('coverage.hint')}</p>

  <h3>{t('coverage.families')}</h3>
  <table aria-label={t('coverage.families')}>
    <thead>
      <tr>
        <th scope="col">{t('coverage.name')}</th>
        <th scope="col">{t('coverage.total')}</th>
        <th scope="col">{t('coverage.check')}</th>
        <th scope="col">{t('coverage.agent')}</th>
        <th scope="col">{t('coverage.manual')}</th>
      </tr>
    </thead>
    <tbody>
      {#each families as row (row.name)}
        <tr>
          <th scope="row">
            <button type="button" translate="no" onclick={() => app.openFamily(row.name)}>{shownText(row.name)}</button>
          </th>
          <td>{row.total}</td>
          <td>{share(row, 'check')}</td>
          <td>{share(row, 'agent')}</td>
          <td>{share(row, 'manual')}</td>
        </tr>
      {/each}
    </tbody>
  </table>

  <h3>{t('coverage.gates')}</h3>
  {#if gates.length === 0}
    <p class="muted">{t('coverage.no_gates')}</p>
  {:else}
    <table aria-label={t('coverage.gates')}>
      <thead>
        <tr>
          <th scope="col">{t('coverage.name')}</th>
          <th scope="col">{t('coverage.total')}</th>
          <th scope="col">{t('coverage.check')}</th>
          <th scope="col">{t('coverage.agent')}</th>
          <th scope="col">{t('coverage.manual')}</th>
          <th scope="col">{t('coverage.unknown')}</th>
        </tr>
      </thead>
      <tbody>
        {#each gates as row (row.name)}
          <tr>
            <th scope="row">
              <button
                type="button"
                onclick={() => {
                  app.setFilter({ ...app.filter, families: [...(gateFamilies.get(row.name) ?? [])] });
                  app.setView('rules');
                }}>{shownText(row.name)}</button
              >
            </th>
            <td>{row.total}</td>
            <td>{share(row, 'check')}</td>
            <td>{share(row, 'agent')}</td>
            <td>{share(row, 'manual')}</td>
            <td>{row.unknown.length > 0 ? row.unknown.map(shownText).join(', ') : '—'}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  .muted {
    color: var(--muted);
  }
  table {
    border-collapse: collapse;
    width: 100%;
    background: var(--surface);
    border: 1px solid var(--line);
    margin-bottom: 1rem;
  }
  th,
  td {
    padding: 0.3rem 0.6rem;
    border-bottom: 1px solid var(--line);
    text-align: left;
  }
  thead th {
    color: var(--muted);
    font-size: 0.8rem;
  }
  tbody th button {
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
    color: var(--text);
    font: inherit;
    text-decoration: underline;
    text-decoration-color: var(--line);
  }
</style>
