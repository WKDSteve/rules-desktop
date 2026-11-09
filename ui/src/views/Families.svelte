<script lang="ts">
  // The families with their counters. A row opens the list narrowed to that family (click or Enter); j and k move
  // the cursor. The counts are the engine's: the page only adds them up.
  import { contextTarget } from 'appkit-ui/contextmenu';
  import type { App } from '../lib/app.svelte';
  import { selectionTarget } from '../lib/context';
  import { familyTotals } from '../lib/families';
  import { translate, type Key } from '../lib/i18n';
  import { shownText } from '../lib/ruletext';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);
  const totals = $derived(familyTotals(app.families));
</script>

<section aria-labelledby="fam-h">
  <h2 id="fam-h">{t('nav.families')}</h2>
  <table aria-label={t('families.label')}>
    <thead>
      <tr>
        <th scope="col">{t('families.family')}</th>
        <th scope="col" class="num">{t('families.count')}</th>
        <th scope="col" class="num">{t('severity.critical')}</th>
        <th scope="col" class="num">{t('severity.recommended')}</th>
        <th scope="col" class="num">{t('severity.nice-to-have')}</th>
        <th scope="col" class="num">{t('families.with_check')}</th>
      </tr>
    </thead>
    <tbody>
      {#each app.families as row (row.family)}
        <tr
          class:cursor={app.selectedFamily === row.family}
          aria-selected={app.selectedFamily === row.family}
          use:contextTarget={selectionTarget(row.family, { families: [row.family] })}
        >
          <th scope="row">
            <button
              type="button"
              onclick={() => {
                app.selectedFamily = row.family;
                app.openFamily(row.family);
              }}>{shownText(row.family)}</button
            >
          </th>
          <td class="num">{row.count}</td>
          <td class="num">{row.critical}</td>
          <td class="num">{row.recommended}</td>
          <td class="num">{row.nice_to_have}</td>
          <td class="num">{row.with_check}</td>
        </tr>
      {/each}
    </tbody>
    <tfoot>
      <tr>
        <th scope="row">{t('families.total')}</th>
        <td class="num">{totals.count}</td>
        <td class="num">{totals.critical}</td>
        <td class="num">{totals.recommended}</td>
        <td class="num">{totals.nice_to_have}</td>
        <td class="num">{totals.with_check}</td>
      </tr>
    </tfoot>
  </table>
</section>

<style>
  table {
    border-collapse: collapse;
    width: 100%;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }
  th,
  td {
    padding: 0.35rem 0.7rem;
    border-bottom: 1px solid var(--line);
    text-align: left;
  }
  thead th {
    color: var(--muted);
    font-weight: 600;
    font-size: 0.8rem;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  tr.cursor {
    background: var(--accent-soft);
  }
  tfoot th,
  tfoot td {
    font-weight: 600;
    border-bottom: 0;
  }
  button {
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-weight: 600;
    text-decoration: underline;
  }
</style>
