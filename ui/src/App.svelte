<script lang="ts">
  // The shell: the menu bar, the three views, the rule that is open, the palette and the toast. What is shown
  // depends on the phase of the page (pairing, offline, engine not ready, ready); everything else is the state of
  // `app` (lib/app.svelte.ts).
  import Brand from 'appkit-ui/Brand.svelte';
  // The mark of the app, from assets/icons of this repository (rule APP-11).
  import logo from '../../assets/icons/rules-desktop/rules-desktop-small.svg?url';
  import DocsDialog from 'appkit-ui/DocsDialog.svelte';
  import { helpItems } from 'appkit-ui/docs';
  import { kitTexts, lookup } from 'appkit-ui/i18n';
  import MenuBar, { type Menu } from 'appkit-ui/MenuBar.svelte';
  import { installNav } from 'appkit-ui/nav';
  import { routeHref } from 'appkit-ui/route';
  import { isNavState, navRoute, routeNav, VIEWS, type App, type NavState, type Theme } from './lib/app.svelte';
  import { LANGS, translate, type Key, type Lang } from './lib/i18n';
  import { resolveKey } from 'appkit-ui/keymap';
  import { keymap, type ChordState } from './lib/keys';
  import { DOCS, LICENSE, REPO_URL } from './lib/docs';
  import Detail from './views/Detail.svelte';
  import Families from './views/Families.svelte';
  import Overview from './views/Overview.svelte';
  import ConflictCenter from './views/ConflictCenter.svelte';
  import Coverage from './views/Coverage.svelte';
  import NewRule from './views/NewRule.svelte';
  import Palette from './views/Palette.svelte';
  import Rules from './views/Rules.svelte';
  import Sources from './views/Sources.svelte';
  import Toast from './views/Toast.svelte';
  import Waivers from './views/Waivers.svelte';

  let { app, code = null }: { app: App; code?: string | null } = $props();

  const t = (key: Key, params: Record<string, string | number> = {}) => translate(app.lang, key, params);
  /** A text of the kit's own parts (the Help menu, the documentation window), in the language of the page. */
  const kit = (key: keyof (typeof kitTexts)['en'], params: Record<string, string | number> = {}) =>
    lookup(kitTexts[app.lang], key, params);
  // The documentation window: opened at a document from the Help menu, or at its search box.
  let docsOpen = $state(false);
  let docsAt = $state<string | null>(null);
  function openDocs(id: string | null): void {
    docsAt = id;
    docsOpen = true;
  }
  const LANG_NAMES: Record<Lang, string> = { de: 'Deutsch', en: 'English' };
  const THEMES: readonly Theme[] = ['auto', 'light', 'dark'];

  // The page is started once, from its address.
  $effect(() => {
    void app.start(code);
    return () => app.stop();
  });

  // Back and Forward: the view and the open rule are places of the tab's history. The state is in `history.state`, and
  // the place is in the fragment of the address too (appkit-ui/nav): a reload and a link in a new tab open it.
  const nav = installNav<NavState>({
    history: window.history,
    window,
    isState: isNavState,
    restore: (place) => app.restoreNav(place),
    address: { location: window.location, toRoute: navRoute, fromRoute: routeNav },
  });
  $effect(() => () => nav.stop());
  $effect(() => {
    // Only a page that shows something has a place: a pairing page or an offline page has none to go back to.
    if (app.phase === 'ready') nav.record(app.navState(), { replace: app.takeNavReplace() });
  });

  // The language and the theme of the document follow the state.
  $effect(() => {
    document.documentElement.lang = app.lang;
    if (app.theme === 'auto') delete document.documentElement.dataset['theme'];
    else document.documentElement.dataset['theme'] = app.theme;
    document.title = t('app.title');
  });

  const menus = $derived<Menu[]>([
    {
      id: 'view',
      label: t('menu.view'),
      items: [
        ...VIEWS.map((view) => ({
          kind: 'radio' as const,
          label: t(`nav.${view}`),
          checked: app.view === view,
          onselect: () => app.setView(view),
          // A link to the view: the browser opens it in a new tab (middle click, Ctrl+click, its context menu), which
          // takes the session of an open tab.
          href: routeHref(location, { view }),
        })),
        { kind: 'separator' as const },
        { kind: 'action' as const, label: t('menu.rescan'), onselect: () => void app.reload() },
      ],
    },
    {
      id: 'language',
      label: t('menu.language'),
      items: LANGS.map((lang) => ({
        kind: 'radio' as const,
        label: LANG_NAMES[lang],
        checked: app.lang === lang,
        onselect: () => app.setLang(lang),
      })),
    },
    {
      id: 'theme',
      label: t('menu.theme'),
      items: THEMES.map((theme) => ({
        kind: 'radio' as const,
        label: t(`theme.${theme}`),
        checked: app.theme === theme,
        onselect: () => app.setTheme(theme),
      })),
    },
    {
      id: 'help',
      label: t('menu.help'),
      items: [
        {
          kind: 'check' as const,
          label: t('menu.shortcuts'),
          checked: app.shortcuts,
          onselect: () => app.setShortcuts(!app.shortcuts),
        },
        { kind: 'info' as const, label: t('help.keys') },
        ...helpItems({
          text: kit,
          docs: DOCS,
          opendoc: (id) => openDocs(id),
          opensearch: () => openDocs(null),
          repo: { url: REPO_URL },
          license: LICENSE,
        }),
      ],
    },
  ]);

  // The keyboard. A letter in a field, a menu or a dialog is typed, not a shortcut.
  let chord: ChordState = null;
  function onKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    const target = event.target instanceof HTMLElement ? event.target : null;
    const typing =
      target !== null && target.closest('input, textarea, select, [contenteditable="true"], [role="menu"], dialog') !== null;
    const result = resolveKey(
      keymap,
      {
        key: event.key,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        altKey: event.altKey,
        shiftKey: event.shiftKey,
        typing,
      },
      chord,
      Date.now(),
      app.shortcuts,
    );
    chord = result.chord;
    if (result.command === null) return;
    // Enter on a button or a link is that control's own; only the list and the table take it as "open".
    if (result.command.name === 'open' && target?.closest('button, a, summary') != null) return;
    event.preventDefault();
    app.command(result.command);
  }
</script>

<svelte:window onkeydown={onKeydown} onfocus={() => void app.focused()} />

<a class="skip" href="#content">{t('app.skip')}</a>

<header>
  <h1>
    <Brand label={t('app.title')} hint={t('hint.brand')} onclick={() => app.setView('overview')}>
      <span class="brand-mark"><img src={logo} alt="" width="20" height="20" />{t('app.title')}</span>
    </Brand>
  </h1>
  <MenuBar {menus} label={t('menu.label')} />
  {#if app.phase === 'ready'}
    <nav aria-label={t('nav.label')}>
      <!-- Each view is a link to its place: a plain click goes there in the page; the browser's ways to open a link
           (middle click, Ctrl+click, Shift+click, its context menu) open it in a new tab, which takes the session of
           an open tab (appkit-ui/handoff). -->
      {#each VIEWS as view (view)}
        <a
          href={routeHref(location, { view })}
          aria-current={app.view === view ? 'page' : undefined}
          onclick={(e) => {
            if (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey) return;
            e.preventDefault();
            app.setView(view);
          }}>{t(`nav.${view}`)}</a
        >
      {/each}
    </nav>
  {/if}
  <span class="connection" class:live={app.connection === 'live'} role="status">{t(`connection.${app.connection}`)}</span>
</header>

<DocsDialog
  open={docsOpen}
  docs={DOCS}
  docId={docsAt}
  onclose={() => (docsOpen = false)}
  labels={{
    title: kit('kit.docs.title'),
    label: kit('kit.docs.label'),
    placeholder: kit('kit.docs.placeholder'),
    none: kit('kit.docs.none'),
    list: kit('kit.docs.list'),
    close: kit('kit.dismiss'),
  }}
/>

<main id="content">
  {#if app.phase === 'starting'}
    <p role="status">{t('phase.starting')}</p>
  {:else if app.phase === 'needs_pairing'}
    <section class="state">
      <p>{app.pairRefused ? t('phase.pair_refused') : t('phase.needs_pairing')}</p>
      <p class="muted"><code>{t('phase.needs_pairing.how')}</code></p>
    </section>
  {:else if app.phase === 'offline'}
    <p role="alert">{t('phase.offline')}</p>
  {:else if app.phase === 'unavailable'}
    <p role="alert">{t('phase.unavailable', { problem: app.problem ?? '?' })}</p>
  {:else}
    <div class="layout" class:open={app.detail.state !== 'none'}>
      <div class="view">
        {#if app.view === 'overview'}
          <Overview {app} />
        {:else if app.view === 'families'}
          <Families {app} />
        {:else if app.view === 'coverage'}
          <Coverage {app} />
        {:else if app.view === 'waivers'}
          <Waivers {app} />
        {:else if app.view === 'sources'}
          <Sources {app} />
        {:else}
          <Rules {app} />
        {/if}
      </div>
      {#if app.detail.state !== 'none'}
        <Detail {app} />
      {/if}
    </div>
  {/if}
</main>

{#if app.phase === 'ready'}
  <Palette {app} />
  <NewRule {app} />
  <ConflictCenter {app} />
{/if}
<Toast {app} />

<style>
  .brand-mark {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
  }
  .skip {
    position: absolute;
    left: -999px;
    top: 0;
    background: var(--surface);
    padding: 0.4rem 0.8rem;
    z-index: 50;
  }
  .skip:focus {
    left: 0.5rem;
    top: 0.5rem;
  }
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 1rem;
    padding: 0.4rem 1rem;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }
  header h1 {
    margin: 0;
  }
  nav {
    display: flex;
    gap: 0.25rem;
  }
  nav a {
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius);
    padding: 0.25rem 0.7rem;
    cursor: pointer;
    color: inherit;
    text-decoration: none;
  }
  nav a[aria-current='page'] {
    background: var(--accent-soft);
    border-color: var(--accent);
  }
  .connection {
    margin-left: auto;
    color: var(--muted);
    font-size: 0.85rem;
  }
  .connection.live {
    color: var(--ok);
  }
  main {
    padding: 1rem;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 1rem;
  }
  .layout.open {
    grid-template-columns: minmax(0, 1fr) minmax(18rem, 34rem);
  }
  @media (max-width: 52rem) {
    .layout.open {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .muted {
    color: var(--muted);
  }
</style>
