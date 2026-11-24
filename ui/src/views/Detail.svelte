<script lang="ts">
  // One rule: its words as text (never markup, never run), where it lives, the version of its file, and its check
  // as data. The class is the hub's word for the file. The rule text is the engine's; hidden characters in it are
  // shown as ⟦U+…⟧.
  import type { App } from '../lib/app.svelte';
  import { labelOf, translate, type Key } from '../lib/i18n';
  import { asJson, holdsHidden, shownText, sourceLine, titleText } from '../lib/ruletext';
  import { classLabel } from '../lib/sources';
  import RuleEditor from './RuleEditor.svelte';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);
  const detail = $derived(app.detail);

  let copied = $state<'idle' | 'ok' | 'failed'>('idle');
  async function copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      copied = 'ok';
    } catch {
      copied = 'failed';
    }
  }
  // The editor is open for one rule; another rule starts without it.
  let editing = $state(false);
  const openId = $derived(detail.state === 'none' ? null : detail.id);
  $effect(() => {
    void openId;
    editing = false;
  });
  // A new rule starts without the word of the last copy.
  $effect(() => {
    void detail;
    copied = 'idle';
  });
</script>

<aside aria-label={t('detail.label')}>
  <div class="head">
    <h2>{detail.state === 'ok' ? shownText(detail.doc.rule.id) : t('detail.label')}</h2>
    <button type="button" onclick={() => app.closeDetail()}>{t('detail.close')}</button>
  </div>

  {#if detail.state === 'loading'}
    <p role="status">{t('detail.loading')}</p>
  {:else if detail.state === 'failed'}
    <p role="alert">{t('detail.failed')}</p>
  {:else if detail.state === 'ok'}
    {@const rule = detail.doc.rule}
    {@const line = sourceLine(rule.source, rule.line)}
    <h3 class="title">{shownText(titleText(rule.title, rule.id))}</h3>
    <p class="chips">
      <span class={`sev ${rule.severity}`}>{labelOf(app.lang, 'severity', rule.severity)}</span>
      <span class="chip">{shownText(rule.family)}</span>
      <span class={`chip cls ${app.classOfRule(rule)}`}>{t(classLabel(app.classOfRule(rule)))}</span>
    </p>

    <dl>
      <dt>{t('detail.source')}</dt>
      <dd>
        <code translate="no">{shownText(line)}</code>
        <button type="button" onclick={() => void copy(line)}>{t('detail.copy')}</button>
        <button type="button" onclick={() => void app.openInEditor(rule.id)}>{t('detail.open')}</button>
        {#if copied !== 'idle'}<span role="status" class="muted">{copied === 'ok' ? t('detail.copied') : t('detail.copy_failed')}</span>{/if}
      </dd>
      {#if rule.section !== ''}
        <dt>{t('detail.section')}</dt>
        <dd>{shownText(rule.section)}</dd>
      {/if}
      <dt>{t('detail.version')}</dt>
      <dd><code translate="no">{shownText(rule.etag)}</code> ({t(`detail.scope.${rule.etag_scope}`)})</dd>
    </dl>

    {#if editing}
      <RuleEditor {app} {rule} onclose={() => (editing = false)} />
    {:else if rule.etag_scope !== 'rule'}
      <p class="note">{t('edit.file_scope')}</p>
    {:else if app.classOfRule(rule) === 'unclassified'}
      <p class="note">{t('edit.no_class')}</p>
    {:else}
      <button type="button" onclick={() => (editing = true)}>{t('edit.open')}</button>
      {#if !rule.editable}<p class="note">{t('detail.not_editable')}</p>{/if}
    {/if}
    <!-- A waiver is written into the project, not into the rule set: it needs no class and no file of its own. -->
    {#if !editing}<p><button type="button" onclick={() => app.startWaiver(rule.id)}>{t('waiver.start')}</button></p>{/if}
    {#if holdsHidden(rule.text) || holdsHidden(rule.title)}<p class="note">{t('detail.hidden')}</p>{/if}

    <h3>{t('detail.text')}</h3>
    <pre class="text">{shownText(rule.text)}</pre>

    {#if rule.check}
      <h3>{t('detail.check')}</h3>
      <pre translate="no">{asJson(rule.check)}</pre>
    {/if}
    {#if rule.agent}
      <h3>{t('detail.agent')}</h3>
      <pre translate="no">{asJson(rule.agent)}</pre>
    {/if}
  {/if}
</aside>

<style>
  aside {
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    padding: 0.8rem 1rem;
    min-width: 0;
    align-self: start;
    position: sticky;
    top: 0.5rem;
    max-height: calc(100vh - 1rem);
    overflow: auto;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
  }
  .head h2 {
    margin: 0;
    font-family: var(--mono);
  }
  button {
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.15rem 0.6rem;
    cursor: pointer;
  }
  .title {
    text-transform: none;
    letter-spacing: 0;
    color: var(--text);
    font-size: 1rem;
    margin-top: 0.6rem;
  }
  .chips {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
    margin: 0.2rem 0 0.6rem;
  }
  .chip,
  .sev {
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    padding: 0.02rem 0.55rem;
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
  .cls.unclassified,
  .cls.private {
    border-color: var(--warn);
    color: var(--warn);
  }
  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 0.25rem 0.8rem;
    margin: 0 0 0.6rem;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .note {
    border-left: 4px solid var(--warn);
    padding: 0.2rem 0.6rem;
  }
  .muted {
    color: var(--muted);
  }
  pre {
    background: var(--surface-2);
    border-radius: var(--radius);
    padding: 0.6rem;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin: 0 0 0.6rem;
  }
  .text {
    font-family: var(--font);
  }
</style>
