<script lang="ts">
  // The first screen: how much there is, how it splits by severity, and what does not add up (the findings).
  // A finding here is something the engine reported while loading; the checks of the rules are not run by this app.
  import type { App } from '../lib/app.svelte';
  import { familyTotals } from '../lib/families';
  import { translate, type Key } from '../lib/i18n';
  import { shownText } from '../lib/ruletext';
  import { unclassifiedCount } from '../lib/sources';
  import { reinstateOp } from '../lib/write';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  const snap = $derived(app.snapshot);
  const totals = $derived(familyTotals(app.families));
  const loadErrors = $derived(snap?.load_errors ?? []);
  const retired = $derived(snap?.retired ?? []);
  const gates = $derived(snap?.gates ?? []);
  const unknownGates = $derived(gates.filter((g) => (g.unknown?.length ?? 0) > 0));
  const unclassified = $derived(unclassifiedCount(app.sources));
  const clean = $derived(loadErrors.length === 0 && unknownGates.length === 0 && unclassified === 0);
</script>

<section aria-labelledby="ov-h">
  <h2 id="ov-h">{t('nav.overview')}</h2>
  <dl class="tiles">
    <div>
      <dt>{t('overview.rules')}</dt>
      <dd>{snap?.counts.rules ?? app.rules.length}</dd>
    </div>
    <div>
      <dt>{t('overview.families')}</dt>
      <dd>{snap?.counts.families ?? app.families.length}</dd>
    </div>
    <div>
      <dt>{t('overview.files')}</dt>
      <dd>{snap?.counts.files ?? snap?.sources.length ?? 0}</dd>
    </div>
    <div>
      <dt>{t('overview.retired')}</dt>
      <dd>{retired.length}</dd>
    </div>
  </dl>

  {#if retired.length > 0}
    <h3>{t('retired.list')}</h3>
    <ul class="plain retired">
      {#each retired as gone (gone.id)}
        <li>
          <code translate="no">{shownText(gone.id)}</code>
          <!-- The hub knows the class of a retired rule; the files of the snapshot do not list it, so the hub refuses. -->
          <button type="button" disabled={app.writing} onclick={() => void app.write([reinstateOp(gone.id)])}>
            {t('retired.reinstate')}
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <h3>{t('overview.severity')}</h3>
  <ul class="plain severities">
    <li><span class="sev critical">{t('severity.critical')}</span> {totals.critical}</li>
    <li><span class="sev recommended">{t('severity.recommended')}</span> {totals.recommended}</li>
    <li><span class="sev nice">{t('severity.nice-to-have')}</span> {totals.nice_to_have}</li>
  </ul>

  <h3>{t('overview.findings')}</h3>
  <p class="muted">{t('overview.findings.hint')}</p>
  {#if clean}
    <p>{t('overview.findings.none')}</p>
  {/if}
  {#if loadErrors.length > 0}
    <h4>{t('overview.load_errors')}</h4>
    <ul class="findings">
      {#each loadErrors as error, i (i)}
        <li>{shownText(error)}</li>
      {/each}
    </ul>
  {/if}
  {#if unknownGates.length > 0}
    <h4>{t('overview.gates')}</h4>
    <ul class="findings">
      {#each unknownGates as gate (gate.name)}
        <li>
          <strong>{shownText(gate.name ?? '?')}</strong>:
          {t('overview.gate.unknown', { families: (gate.unknown ?? []).map(shownText).join(', ') })}
        </li>
      {/each}
    </ul>
  {/if}
  {#if unclassified > 0}
    <p class="finding">{t('overview.unclassified', { n: unclassified })}</p>
  {/if}
  {#if gates.length > 0}
    <h4>{t('overview.gates')}</h4>
    <ul class="plain">
      {#each gates as gate (gate.name)}
        <li><strong>{shownText(gate.name ?? '?')}</strong> · {t('overview.gate.rules', { n: gate.rules ?? 0 })}</li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: 0.75rem;
    margin: 0 0 1rem;
  }
  .tiles div {
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    padding: 0.6rem 0.8rem;
  }
  dt {
    color: var(--muted);
    font-size: 0.8rem;
  }
  dd {
    margin: 0;
    font-size: 1.6rem;
    font-weight: 600;
  }
  h4 {
    margin: 0.8rem 0 0.3rem;
    font-size: 0.9rem;
  }
  .plain {
    list-style: none;
    padding: 0;
    margin: 0 0 0.5rem;
  }
  .severities {
    display: flex;
    gap: 1.2rem;
    flex-wrap: wrap;
  }
  .findings li,
  .finding {
    border-left: 4px solid var(--warn);
    padding: 0.2rem 0.6rem;
    margin: 0.25rem 0;
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--muted);
  }
  .sev {
    border-radius: var(--radius-pill);
    padding: 0.05rem 0.55rem;
    border: 1px solid var(--line);
    font-size: 0.8rem;
  }
  .sev.critical {
    border-color: var(--bad);
    color: var(--bad);
  }
  .sev.recommended {
    border-color: var(--warn);
    color: var(--warn);
  }
</style>
