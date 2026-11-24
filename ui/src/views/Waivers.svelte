<script lang="ts">
  // The waivers of the project of this device: which findings are known and accepted, why, until when and for which
  // paths. The file lives in the project, not in a rule set; the hub names the project (the person set it in
  // config.json), this page never names a folder. Every change goes with the tag of the file that was read, so that
  // nothing is written over a change made since; the engine writes the file as a whole, keys sorted.
  import type { App } from '../lib/app.svelte';
  import { translate, type Key } from '../lib/i18n';
  import { shownText } from '../lib/ruletext';
  import { removeOp, renewOp, waiveOp } from '../lib/write';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  const doc = $derived(app.waivers);
  const counts = $derived(doc?.counts ?? {});

  // The form for a new waiver; a "Waive …" button of a rule fills its id.
  let id = $state('');
  let reason = $state('');
  let expires = $state('');
  let paths = $state('');
  let renewing = $state<string | null>(null);
  let renewTo = $state('');
  let renewNone = $state(false);

  $effect(() => {
    void app.loadWaivers();
  });
  $effect(() => {
    const draft = app.waiveDraft;
    if (draft !== null) {
      id = draft;
      app.waiveDraft = null;
    }
  });

  async function add(): Promise<void> {
    const etag = app.waiversEtag();
    if (etag === null || id.trim() === '' || reason.trim() === '') return;
    const list = paths
      .split('\n')
      .map((p) => p.trim())
      .filter((p) => p !== '');
    const outcome = await app.write([
      waiveOp({ id: id.trim(), reason: reason.trim(), expires: expires.trim(), paths: list }, etag),
    ]);
    if (outcome && outcome.saved > 0 && outcome.failures.length === 0 && outcome.conflicts.length === 0) {
      id = reason = expires = paths = '';
    }
  }

  async function renew(waiverId: string): Promise<void> {
    const etag = app.waiversEtag();
    if (etag === null || (!renewNone && renewTo.trim() === '')) return;
    const end = renewNone ? ({ no_expiry: true } as const) : { expires: renewTo.trim() };
    const outcome = await app.write([renewOp(waiverId, end, etag)]);
    if (outcome && outcome.saved > 0) renewing = null;
  }

  async function remove(waiverId: string): Promise<void> {
    const etag = app.waiversEtag();
    if (etag !== null) await app.write([removeOp(waiverId, etag)]);
  }

  function startRenew(waiverId: string, current?: string): void {
    renewing = renewing === waiverId ? null : waiverId;
    renewTo = current ?? '';
    renewNone = false;
  }
</script>

<section aria-labelledby="wv-h">
  <h2 id="wv-h">{t('nav.waivers')}</h2>
  <p class="muted">{t('waiver.hint')}</p>

  {#if app.waiversState === 'no_project'}
    <p role="status" class="note">{t('waiver.no_project')}</p>
  {:else if app.waiversState === 'failed'}
    <p role="alert" class="bad">{t('waiver.failed')}</p>
  {:else if doc}
    <p class="count" role="status">
      {t('waiver.counts', { total: counts.total ?? 0, expired: counts.expired ?? 0, unknown: counts.unknown ?? 0 })}
    </p>

    {#if doc.items.length === 0}
      <p class="muted">{t('waiver.empty')}</p>
    {:else}
      <table aria-label={t('nav.waivers')}>
        <thead>
          <tr>
            <th scope="col">{t('waiver.id')}</th>
            <th scope="col">{t('waiver.reason')}</th>
            <th scope="col">{t('waiver.expires')}</th>
            <th scope="col">{t('waiver.state')}</th>
            <th scope="col"><span class="sr">{t('waiver.actions')}</span></th>
          </tr>
        </thead>
        <tbody>
          {#each doc.items as item (item.id)}
            <tr id={`waiver-${item.id}`}>
              <th scope="row"><code translate="no">{shownText(item.id)}</code></th>
              <td>
                {shownText(item.reason)}
                {#if item.paths && item.paths.length > 0}
                  <br /><span class="muted">{item.paths.map(shownText).join(', ')}</span>
                {/if}
              </td>
              <td>{item.expires ?? t('waiver.no_end')}</td>
              <td>
                {#if item.expired}<span class="flag bad">{t('waiver.expired')}</span>{/if}
                {#if !item.known}<span class="flag warn">{t('waiver.unknown')}</span>{/if}
                {#if !item.expired && item.known}<span class="flag">{t('waiver.ok')}</span>{/if}
              </td>
              <td class="actions">
                <button type="button" disabled={app.writing} onclick={() => startRenew(item.id, item.expires)}>{t('waiver.renew')}</button>
                <button type="button" disabled={app.writing} onclick={() => void remove(item.id)}>{t('waiver.remove')}</button>
              </td>
            </tr>
            {#if renewing === item.id}
              <tr>
                <td colspan="5">
                  <form
                    class="inline"
                    onsubmit={(event) => {
                      event.preventDefault();
                      void renew(item.id);
                    }}
                  >
                    <label data-hint={t('hint.form.waiver.renew_to')}>
                      {t('waiver.renew_to')}
                      <input type="text" spellcheck="false" bind:value={renewTo} disabled={renewNone} placeholder="2027-01-31" />
                    </label>
                    <label class="check" data-hint={t('hint.form.waiver.renew_none')}>
                      <input type="checkbox" bind:checked={renewNone} />
                      {t('waiver.renew_none')}
                    </label>
                    <button type="submit" disabled={app.writing}>{t('waiver.renew_apply')}</button>
                  </form>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}

    <h3>{t('waiver.new')}</h3>
    <form
      class="new"
      onsubmit={(event) => {
        event.preventDefault();
        void add();
      }}
    >
      <label data-hint={t('hint.form.waiver.id')}>
        {t('waiver.id')}
        <input type="text" list="waiver-rules" bind:value={id} autocomplete="off" spellcheck="false" />
        <datalist id="waiver-rules">
          {#each app.rules as rule (rule.id)}<option value={rule.id}></option>{/each}
        </datalist>
      </label>
      <label data-hint={t('hint.form.waiver.reason')}>
        {t('waiver.reason')}
        <input type="text" bind:value={reason} />
      </label>
      <label data-hint={t('hint.form.waiver.expires')}>
        {t('waiver.expires')}
        <input type="text" spellcheck="false" bind:value={expires} placeholder="2027-01-31" />
      </label>
      <label data-hint={t('hint.form.waiver.paths')}>
        {t('waiver.paths')}
        <textarea rows="3" spellcheck="false" bind:value={paths}></textarea>
      </label>
      <div>
        <button type="submit" disabled={app.writing || id.trim() === '' || reason.trim() === ''}>{t('waiver.add')}</button>
      </div>
    </form>
  {/if}
</section>

<style>
  .muted {
    color: var(--muted);
  }
  .note {
    color: var(--warn);
  }
  .bad {
    color: var(--bad);
  }
  .count {
    margin: 0.3rem 0;
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
  .flag {
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    padding: 0.02rem 0.5rem;
    font-size: 0.78rem;
    white-space: nowrap;
  }
  .flag.bad {
    border-color: var(--bad);
  }
  .flag.warn {
    border-color: var(--warn);
    color: var(--warn);
  }
  .actions {
    white-space: nowrap;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  form.new,
  form.inline {
    display: grid;
    gap: 0.5rem;
    max-width: 36rem;
  }
  form.inline {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    max-width: none;
  }
  label {
    display: grid;
    gap: 0.15rem;
    font-size: 0.85rem;
    color: var(--muted);
  }
  label.check {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }
  input[type='text'],
  textarea {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.25rem 0.4rem;
  }
  button {
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.15rem 0.6rem;
    cursor: pointer;
    color: var(--text);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
