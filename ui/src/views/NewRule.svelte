<script lang="ts">
  import { closeOnBackdrop } from 'appkit-ui/backdrop';
  import { tick } from 'svelte';
  // The dialog for a new rule: the family, the title, the severity and the text; the next id of the family is shown
  // as the engine names it (it is not reserved: the engine picks the id under the lock of the rule set when the rule
  // is made). Preview asks the engine for a dry run and shows what would happen; Create writes. What a good rule is,
  // is the engine's to say: its refusal is shown as it comes.
  import type { App } from '../lib/app.svelte';
  import { dataPatch } from '../lib/edit';
  import { floating } from '../lib/floating';
  import { focusFirstInvalid } from '../lib/formErrors';
  import { translate, type Key } from '../lib/i18n';
  import { leaveGuard } from '../lib/leaveGuard';
  import { warnBeforeUnload } from '../lib/unsaved';
  import { newOp, type Outcome } from '../lib/write';
  import FieldError from './FieldError.svelte';
  import LeaveAsk from './LeaveAsk.svelte';

  let { app }: { app: App } = $props();
  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);

  const SEVERITIES = ['critical', 'recommended', 'nice-to-have'] as const;
  let dialog: HTMLDialogElement | undefined = $state();
  let familyInput: HTMLInputElement | undefined = $state();
  let family = $state('');
  let title = $state('');
  let severity = $state<string>('recommended');
  let text = $state('');
  let check = $state('');
  let agent = $state('');
  let ruleset = $state('');
  let next = $state<string | null>(null);
  // What is wrong with a field, told beside it when the form is sent; the field leaves the list when it is typed in.
  type Field = 'family' | 'title' | 'check' | 'agent';
  let errors = $state<Partial<Record<Field, Key>>>({});
  let form: HTMLFormElement | undefined = $state();
  let sending = $state(false);
  let previewing = $state(false);
  let leaving = $state(false);
  let preview = $state<Outcome | null>(null);

  const open = $derived(app.newRuleOpen);
  const families = $derived(app.families.map((f) => f.family));

  function reset(): void {
    family = '';
    title = '';
    severity = 'recommended';
    text = '';
    check = '';
    agent = '';
    ruleset = '';
    next = null;
    errors = {};
    leaving = false;
    preview = null;
  }

  const clear = (field: Field): void => {
    if (errors[field]) errors = { ...errors, [field]: undefined };
  };
  // Something typed that closing would lose: Escape asks first, and the tab asks before it is closed.
  const dirty = $derived([family, title, text, check, agent, ruleset].some((v) => v.trim() !== ''));
  $effect(() => {
    if (open && dirty) return warnBeforeUnload();
  });

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) {
      reset();
      // An intent opened the form: its finding is the start of the title and the text, taken once
      if (app.newRulePrefill) {
        title = app.newRulePrefill.title;
        text = app.newRulePrefill.text;
        app.newRulePrefill = null;
      }
      dialog.showModal();
      familyInput?.focus();
    }
    if (!open && dialog.open) dialog.close();
  });

  // A preview says what the fields said when it was asked for: when one changes, it is no longer true.
  $effect(() => {
    void [family, title, severity, text, check, agent, ruleset];
    preview = null;
  });

  // The next id follows the family the person types, after a short pause; an older answer is dropped.
  let asked = 0;
  $effect(() => {
    const wanted = family.trim();
    const ticket = ++asked;
    if (!open || wanted === '') {
      next = null;
      return;
    }
    const timer = setTimeout(() => {
      void app.nextId(wanted).then((id) => {
        if (ticket === asked) next = id;
      });
    }, 300);
    return () => clearTimeout(timer);
  });

  /** The operation the fields stand for, or the reason they do not. */
  function build(): ReturnType<typeof newOp> | null {
    const data: { check?: unknown; agent?: unknown } = {};
    const found: Partial<Record<Field, Key>> = {};
    if (family.trim() === '') found.family = 'form.required';
    if (title.trim() === '' || /[\r\n]/.test(title)) found.title = 'edit.bad_title';
    for (const [field, value] of [
      ['check', check],
      ['agent', agent],
    ] as const) {
      const read = dataPatch(value);
      if (!read.ok) found[field] = 'edit.bad_json';
      else if (read.value !== null) data[field] = read.value;
    }
    errors = found;
    if (Object.keys(found).length > 0) {
      void tick().then(() => focusFirstInvalid(form));
      return null;
    }
    return newOp({ family: family.trim(), title: title.trim(), severity, text, ruleset: ruleset.trim(), ...data });
  }

  async function run(dryRun: boolean): Promise<void> {
    const op = build();
    if (!op) return;
    sending = true;
    previewing = dryRun;
    const outcome = await app.write([op], { dryRun }).finally(() => (sending = false));
    if (!outcome) return;
    if (dryRun) {
      preview = outcome;
      return;
    }
    if (outcome.saved > 0 && outcome.failures.length === 0 && outcome.conflicts.length === 0) {
      const id = outcome.changedIds[0];
      app.closeNewRule();
      app.setView('rules');
      if (id) void app.openDetail(id);
    } else {
      preview = outcome;
    }
  }
</script>

<dialog
  bind:this={dialog}
  use:floating={{ kind: 'new-rule', moveLabel: t('float.move'), moveHint: t('hint.float.move') }}
  use:closeOnBackdrop={{ closeable: () => !dirty }}
  use:leaveGuard={{ dirty: () => dirty, ask: () => (leaving = true) }}
  aria-label={t('new.label')}
  onclose={() => {
    if (!dialog?.open) app.closeNewRule();
  }}
>
  <form
    bind:this={form}
    novalidate
    aria-busy={sending}
    onsubmit={(event) => {
      event.preventDefault();
      void run(false);
    }}
  >
    <h2>{t('new.label')}</h2>
    {#if leaving}<LeaveAsk {app} ondiscard={() => { leaving = false; dialog?.close(); }} onkeep={() => (leaving = false)} />{/if}
    <label data-hint={t('hint.form.new.family')}>
      {t('new.family')}
      <input
        type="text"
        list="new-rule-families"
        bind:value={family}
        bind:this={familyInput}
        autocomplete="off"
        spellcheck="false"
        aria-invalid={errors.family ? 'true' : undefined}
        aria-describedby={errors.family ? 'new-family-error' : undefined}
        oninput={() => clear('family')}
      />
      <datalist id="new-rule-families">
        {#each families as name (name)}<option value={name}></option>{/each}
      </datalist>
    </label>
    <FieldError id="new-family-error" text={errors.family ? t(errors.family) : null} />
    <p class="muted" role="status">
      {#if family.trim() !== ''}{next ? t('new.next', { id: next }) : t('new.next_none')}{/if}
    </p>
    <label data-hint={t('hint.form.edit.title')}>
      {t('edit.title')}
      <input
        type="text"
        bind:value={title}
        aria-invalid={errors.title ? 'true' : undefined}
        aria-describedby={errors.title ? 'new-title-error' : undefined}
        oninput={() => clear('title')}
      />
    </label>
    <FieldError id="new-title-error" text={errors.title ? t(errors.title) : null} />
    <label data-hint={t('hint.form.edit.severity')}>
      {t('edit.severity')}
      <select bind:value={severity}>
        {#each SEVERITIES as value (value)}<option {value}>{t(`severity.${value}`)}</option>{/each}
      </select>
    </label>
    <label data-hint={t('hint.form.edit.text')}>
      {t('edit.text')}
      <textarea rows="6" bind:value={text}></textarea>
    </label>
    <label data-hint={t('hint.form.edit.check')}>
      {t('edit.check')}
      <textarea
        rows="3"
        class="mono"
        spellcheck="false"
        bind:value={check}
        aria-invalid={errors.check ? 'true' : undefined}
        aria-describedby={errors.check ? 'new-check-error' : undefined}
        oninput={() => clear('check')}
      ></textarea>
    </label>
    <FieldError id="new-check-error" text={errors.check ? t(errors.check) : null} />
    <label data-hint={t('hint.form.edit.agent')}>
      {t('edit.agent')}
      <textarea
        rows="2"
        class="mono"
        spellcheck="false"
        bind:value={agent}
        aria-invalid={errors.agent ? 'true' : undefined}
        aria-describedby={errors.agent ? 'new-agent-error' : undefined}
        oninput={() => clear('agent')}
      ></textarea>
    </label>
    <FieldError id="new-agent-error" text={errors.agent ? t(errors.agent) : null} />
    <label data-hint={t('hint.form.new.ruleset')}>
      {t('new.ruleset')}
      <input type="text" spellcheck="false" bind:value={ruleset} />
    </label>

    {#if preview}
      <div role="status" class="preview">
        {#each preview.ids as id (id)}<p>{t('new.would', { id })}</p>{/each}
        {#each preview.failures as failure (failure.n)}
          <p class="bad">{t('new.failed', { message: failure.message || failure.code })}</p>
        {/each}
        {#each preview.warnings as warning (warning)}<p class="note">{warning}</p>{/each}
      </div>
    {/if}

    <div class="row">
      <button type="submit">{sending && !previewing ? t('form.sending') : t('new.create')}</button>
      <button type="button" onclick={() => void run(true)}>
        {sending && previewing ? t('form.sending') : t('new.preview')}
      </button>
      <button type="button" onclick={() => app.closeNewRule()}>{t('edit.cancel')}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    padding: 1rem 1.2rem;
    width: min(36rem, calc(100vw - 2rem));
  }
  dialog::backdrop {
    background: var(--backdrop);
  }
  form {
    display: grid;
    gap: 0.5rem;
  }
  h2 {
    margin: 0;
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
  button {
    background: none;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 0.15rem 0.6rem;
    min-height: 1.5rem;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .bad {
    color: var(--bad);
  }
  p {
    margin: 0;
  }

  @media (pointer: coarse) {
    button {
      min-height: 2.75rem;
    }
  }
</style>
