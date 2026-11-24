<script lang="ts">
  // The editor of one rule: severity, title, text and, for a rule whose check is data, check and agent as JSON. It
  // sends only what changed, with the tag of the file the person saw. What a good rule is, is the engine's to say:
  // its refusal is shown as it comes. Saving closes the editor; a conflict leaves it open with what was typed.
  import { untrack } from 'svelte';
  import type { App } from '../lib/app.svelte';
  import { draftOf, patchOf, unchanged, type Body, type Draft } from '../lib/edit';
  import { translate, type Key } from '../lib/i18n';
  import { moveOp, retireOp, setOp } from '../lib/write';

  let { app, rule, onclose }: { app: App; rule: Body; onclose: () => void } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  const SEVERITIES = ['critical', 'recommended', 'nice-to-have'] as const;
  // The draft starts as the rule is when the editor opens; a rule read again meanwhile does not wipe what was typed.
  let draft = $state<Draft>(untrack(() => draftOf(rule)));
  let problem = $state<Key | null>(null);
  let retiring = $state(false);
  let reason = $state('');
  let replacedBy = $state('');
  let moving = $state(false);
  let moveTo = $state('');
  let moveWhere = $state<'after' | 'before'>('after');
  const dirty = $derived(!unchanged(rule, draft));

  async function save(): Promise<void> {
    const result = patchOf(rule, draft);
    if (!result.ok) {
      problem = result.reason === 'json' ? 'edit.bad_json' : 'edit.bad_title';
      return;
    }
    problem = null;
    const outcome = await app.write([setOp(rule.id, result.patch, rule.etag)]);
    if (outcome && outcome.saved > 0 && outcome.conflicts.length === 0 && outcome.failures.length === 0) onclose();
  }

  async function move(): Promise<void> {
    const target = moveTo.trim();
    if (target === '') return;
    const outcome = await app.write([moveOp(rule.id, moveWhere === 'after' ? { after: target } : { before: target })]);
    if (outcome && outcome.saved > 0 && outcome.conflicts.length === 0 && outcome.failures.length === 0) onclose();
  }

  async function retire(): Promise<void> {
    const outcome = await app.write([
      retireOp(rule.id, rule.etag, { replaced_by: replacedBy.trim(), reason: reason.trim() }),
    ]);
    if (outcome && outcome.saved > 0 && outcome.conflicts.length === 0 && outcome.failures.length === 0) onclose();
  }
</script>

<form
  class="editor"
  aria-label={t('edit.label')}
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <label data-hint={t('hint.form.edit.severity')}>
    {t('edit.severity')}
    <select bind:value={draft.severity}>
      {#each SEVERITIES as severity (severity)}
        <option value={severity}>{t(`severity.${severity}`)}</option>
      {/each}
    </select>
  </label>
  <label data-hint={t('hint.form.edit.title')}>
    {t('edit.title')}
    <input type="text" bind:value={draft.title} />
  </label>
  <label data-hint={t('hint.form.edit.text')}>
    {t('edit.text')}
    <textarea rows="8" bind:value={draft.text}></textarea>
  </label>
  {#if rule.editable}
    <label data-hint={t('hint.form.edit.check')}>
      {t('edit.check')}
      <textarea rows="4" class="mono" spellcheck="false" bind:value={draft.check}></textarea>
    </label>
    <label data-hint={t('hint.form.edit.agent')}>
      {t('edit.agent')}
      <textarea rows="3" class="mono" spellcheck="false" bind:value={draft.agent}></textarea>
    </label>
    <p class="muted">{t('edit.data_hint')}</p>
  {:else}
    <p class="note">{t('detail.not_editable')}</p>
  {/if}
  {#if problem}<p role="alert" class="bad">{t(problem)}</p>{/if}
  <div class="row">
    <button type="submit" disabled={!dirty || app.writing}>{t('edit.save')}</button>
    <button type="button" onclick={onclose}>{t('edit.cancel')}</button>
    <button type="button" class="push" onclick={() => (moving = !moving)} aria-expanded={moving}>{t('move.open')}</button>
    <button type="button" onclick={() => (retiring = !retiring)} aria-expanded={retiring}>{t('retire.open')}</button>
  </div>
</form>

{#if moving}
  <div class="retire">
    <p class="muted">{t('move.hint')}</p>
    <label data-hint={t('hint.form.move.where')}>
      <select bind:value={moveWhere} aria-label={t('move.open')}>
        <option value="after">{t('move.after')}</option>
        <option value="before">{t('move.before')}</option>
      </select>
    </label>
    <label data-hint={t('hint.form.move.target')}>
      {t('move.target')}
      <input type="text" spellcheck="false" bind:value={moveTo} />
    </label>
    <button type="button" disabled={app.writing || moveTo.trim() === ''} onclick={() => void move()}>{t('move.confirm')}</button>
  </div>
{/if}

{#if retiring}
  <div class="retire">
    <p class="muted">{t('retire.hint')}</p>
    <label data-hint={t('hint.form.retire.reason')}>
      {t('retire.reason')}
      <input type="text" bind:value={reason} />
    </label>
    <label data-hint={t('hint.form.retire.replaced_by')}>
      {t('retire.replaced_by')}
      <input type="text" spellcheck="false" bind:value={replacedBy} />
    </label>
    <button type="button" disabled={app.writing} onclick={() => void retire()}>{t('retire.confirm')}</button>
  </div>
{/if}

<style>
  .editor,
  .retire {
    display: grid;
    gap: 0.5rem;
    margin: 0.5rem 0;
  }
  label {
    display: grid;
    gap: 0.15rem;
    font-size: 0.85rem;
    color: var(--muted);
  }
  input,
  select,
  textarea {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.25rem 0.4rem;
  }
  .mono {
    font-family: var(--mono);
  }
  .row {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .push {
    margin-left: auto;
  }
  button {
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.15rem 0.6rem;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .bad {
    color: var(--bad);
  }
</style>
