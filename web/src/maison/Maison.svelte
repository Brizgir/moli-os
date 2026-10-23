<script>
  import '@fontsource-variable/figtree';
  import './maison.css';
  import { onMount, untrack } from 'svelte';
  import { home, hub, start, value, savePref, visiblePages, pageChoices, camerasAsDoorbell, dismissHeld } from './lib/home.svelte.js';
  import { loadPeople } from './lib/people.svelte.js';
  import { menuFacts } from './lib/menu.svelte.js';
  import { t } from '../lib/i18n.svelte.js';
  import Icon from './ui/Icon.svelte';
  import PinSheet from './ui/PinSheet.svelte';
  import Setup from './ui/Setup.svelte';
  import Login from './ui/Login.svelte';
  import Accueil from './pages/Accueil.svelte';
  import Salon from './pages/Salon.svelte';
  import Pieces from './pages/Pieces.svelte';
  import Plan from './pages/Plan.svelte';
  import Cameras from './pages/Cameras.svelte';
  import Energie from './pages/Energie.svelte';
  import Impression from './pages/Impression.svelte';
  import Dehors from './pages/Dehors.svelte';
  import Moli from './pages/Moli.svelte';
  import Automatismes from './pages/Automatismes.svelte';
  import Systeme from './pages/Systeme.svelte';
  import Parler from './pages/Parler.svelte';
  import Orb from './moli/Orb.svelte';
  import { moli, loadStatus } from './lib/moli.svelte.js';

  let { route } = $props();

  // Shown only if the link stays down a few seconds (a blink is not news).
  let offline = $state(false);
  $effect(() => {
    if (hub.connected) {
      offline = false;
      return;
    }
    const timer = setTimeout(() => (offline = true), 3000);
    return () => clearTimeout(timer);
  });

  // `label` is a catalogue key. `id`: a page the menu shows only when the
  // house justifies it (`lib/pages.js`) or a person asks (Système, « Pages
  // affichées »); its route answers either way.
  const NAV = [
    { path: '', label: 'commun.nav.accueil', icon: 'home' },
    { path: 'pieces', id: 'pieces', label: 'commun.nav.pieces', icon: 'light-group' },
    { path: 'chauffage', id: 'chauffage', label: 'commun.nav.chauffage', icon: 'radiator' },
    { path: 'salon', id: 'salon', label: 'commun.nav.salon', icon: 'remote' },
    { path: 'plan', id: 'plan', label: 'commun.nav.plan', icon: 'home-roof' },
    { path: 'carte', id: 'carte', label: 'commun.nav.carte', icon: 'map' },
    { path: 'cameras', id: 'cameras', label: 'commun.nav.cameras', icon: 'cctv' },
    { path: 'parler', id: 'parler', label: 'commun.nav.parler', icon: 'talk' },
    { path: 'energie', id: 'energie', label: 'commun.nav.energie', icon: 'bolt' },
    { path: 'impression', id: 'impression', label: 'commun.nav.impression', icon: 'printer3d' },
    { path: 'dehors', id: 'dehors', label: 'commun.nav.dehors', icon: 'flower-outline' },
    { path: 'automatismes', id: 'automatismes', label: 'commun.nav.automatismes', icon: 'robot' },
    { path: 'infra', id: 'infra', label: 'commun.nav.infra', icon: 'server' },
    { path: 'moli', id: 'moli', label: 'commun.demander_moli', orb: true },
  ];

  const visible = $derived(visiblePages(menuFacts(), pageChoices()));
  // Until the layout is read, the pages every house has (no menu that jumps).
  const items = $derived(
    NAV.filter((i) => !i.id || (home.config ? visible[i.id] : ['pieces', 'automatismes', 'moli'].includes(i.id))).map((i) =>
      i.id === 'cameras' && camerasAsDoorbell() ? { ...i, label: 'commun.nav.sonnette_cameras', icon: 'bell' } : i,
    ),
  );

  // The phone's tab bar: home, rooms, Moli in the middle, the heating (or
  // the next page the house has), and « Plus » for the rest of the menu.
  const fourth = $derived(items.find((i) => i.path === 'chauffage') ?? items.find((i) => !['', 'pieces', 'moli'].includes(i.path)) ?? null);
  const tabs = $derived(['', 'pieces'].map((p) => items.find((i) => i.path === p)).filter(Boolean));
  const rest = $derived(items.filter((i) => !['', 'pieces', 'moli'].includes(i.path) && i !== fourth));
  let moreOpen = $state(false);
  const inMore = $derived(page === 'systeme' || rest.some((i) => i.path === page));

  let prefersDark = $state(false);

  onMount(() => {
    start();
    loadPeople();
    loadStatus();
    const media = matchMedia('(prefers-color-scheme: dark)');
    prefersDark = media.matches;
    const update = (e) => (prefersDark = e.matches);
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  });

  // Auto: night after sunset (the house's own daylight), else the system's.
  const daylight = $derived(value(home.config?.outdoor?.weather, 'daylight'));
  const theme = $derived(
    home.theme === 'light'
      ? 'day'
      : home.theme === 'dark'
        ? 'night'
        : daylight === false
          ? 'night'
          : daylight === true
            ? 'day'
            : prefersDark
              ? 'night'
              : 'day',
  );

  $effect(() => {
    const root = document.documentElement.style;
    root.background = theme === 'night' ? '#0d1016' : '#edf0f5';
    document.body.style.background = 'transparent';
    return () => {
      root.background = '';
      document.body.style.background = '';
    };
  });

  // The theme button says the preference; a forced one says its own side.
  const themeName = $derived(home.theme === 'auto' ? t('commun.theme.auto') : theme === 'night' ? t('commun.theme.nuit') : t('commun.theme.jour'));
  const themeTitle = $derived(
    home.theme === 'auto' ? t('commun.theme.titre_auto') : home.theme === 'dark' ? t('commun.theme.titre_nuit') : t('commun.theme.titre_jour'),
  );

  function cycleTheme() {
    home.theme = home.theme === 'auto' ? (theme === 'night' ? 'light' : 'dark') : 'auto';
    savePref('maison-theme', home.theme);
  }

  const page = $derived(route.split('/')[0] ?? '');
  // A tap in « Plus » goes somewhere: the sheet closes behind it. So does
  // the code sheet: an order held on another page is not what the person
  // is doing any more.
  $effect(() => {
    void route;
    moreOpen = false;
    if (untrack(() => home.held)) dismissHeld();
  });
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (moreOpen = false)} />

<div class="maison" data-theme={theme}>
  <nav class="rail" aria-label={t('commun.nav.aria')}>
    <a class="brand" href="#/" aria-label={t('commun.nav.logo')}>
      <svg viewBox="0 0 32 32" aria-hidden="true">
        <circle cx="16" cy="16" r="15" fill="var(--sun)" />
        <circle cx="16" cy="16" r="8.5" fill="none" stroke="#161512" stroke-width="4.3" />
      </svg>
      <span class="brand-name">Moli</span>
    </a>
    <ul>
      {#each items as item (item.path)}
        <li>
          <a href="#/{item.path}" class:active={page === item.path} aria-current={page === item.path ? 'page' : undefined}>
            <span class="glyph">
              {#if item.orb}<Orb size={22} state={moli.busy ? 'thinking' : 'idle'} />{:else}<Icon name={item.icon} size={22} />{/if}
            </span>
            <span class="label">{t(item.label)}</span>
          </a>
        </li>
      {/each}
    </ul>
    <div class="rail-foot">
      <button class="theme" onclick={cycleTheme} title={themeTitle}>
        <span class="glyph"><Icon name={theme === 'night' ? 'moon' : 'sun'} size={20} /></span>
        <span class="label">{themeName}</span>
      </button>
      <!-- The Atelier is reached from Système (its first card). -->
      <a href="#/systeme" class:active={page === 'systeme'} aria-current={page === 'systeme' ? 'page' : undefined} title={t('commun.nav.systeme_titre')}>
        <span class="glyph"><Icon name="branch" size={20} /></span>
        <span class="label">{t('commun.nav.systeme')}</span>
      </a>
    </div>
  </nav>

  <!-- Phones: a tab bar at the bottom instead of the side menu. -->
  <nav class="tabbar" aria-label={t('commun.nav.aria')}>
    {#each tabs as item (item.path)}
      <a href="#/{item.path}" class:active={page === item.path} aria-current={page === item.path ? 'page' : undefined}>
        <Icon name={item.icon} size={22} /><span>{t(item.label)}</span>
      </a>
    {/each}
    <a class="tab-moli" href="#/moli" class:active={page === 'moli'} aria-current={page === 'moli' ? 'page' : undefined} aria-label={t('commun.demander_moli')}>
      <Orb size={44} state={moli.busy ? 'thinking' : 'idle'} />
    </a>
    {#if fourth}
      <a href="#/{fourth.path}" class:active={page === fourth.path} aria-current={page === fourth.path ? 'page' : undefined}>
        <Icon name={fourth.icon} size={22} /><span>{t(fourth.label)}</span>
      </a>
    {/if}
    <button class:active={inMore || moreOpen} aria-expanded={moreOpen} aria-haspopup="dialog" onclick={() => (moreOpen = !moreOpen)}>
      <Icon name="dots" size={22} /><span>{t('commun.nav.plus')}</span>
    </button>
  </nav>

  {#if moreOpen}
    <div class="more-sheet" role="presentation" onclick={(e) => e.target === e.currentTarget && (moreOpen = false)}>
      <div class="more-panel" role="dialog" aria-modal="true" aria-label={t('commun.nav.plus_titre')}>
        <ul>
          {#each rest as item (item.path)}
            <li>
              <a href="#/{item.path}" class:active={page === item.path} aria-current={page === item.path ? 'page' : undefined}>
                <Icon name={item.icon} size={22} /><span>{t(item.label)}</span>
              </a>
            </li>
          {/each}
          <li>
            <a href="#/systeme" class:active={page === 'systeme'} aria-current={page === 'systeme' ? 'page' : undefined}>
              <Icon name="branch" size={22} /><span>{t('commun.nav.systeme')}</span>
            </a>
          </li>
          <li>
            <a href="#/atelier"><Icon name="atelier" size={22} /><span>{t('commun.nav.atelier')}</span></a>
          </li>
          <li>
            <button onclick={cycleTheme} title={themeTitle}>
              <Icon name={theme === 'night' ? 'moon' : 'sun'} size={22} /><span>{home.theme === 'auto' ? t('commun.theme.auto_pied') : themeName}</span>
            </button>
          </li>
        </ul>
      </div>
    </div>
  {/if}

  <main class="content">
    {#if page === 'salon'}
      <Salon />
    {:else if page === 'pieces'}
      <Pieces />
    {:else if page === 'plan'}
      <Plan />
    {:else if page === 'carte'}
      <!-- The map (Leaflet): loaded only when visited. -->
      {#await import('./pages/Carte.svelte') then { default: Carte }}<Carte />{/await}
    {:else if page === 'chauffage'}
      {#await import('./pages/Chauffage.svelte') then { default: Chauffage }}<Chauffage {route} />{/await}
    {:else if page === 'cameras'}
      <Cameras {route} />
    {:else if page === 'parler'}
      <Parler />
    {:else if page === 'energie' && route.startsWith('energie/solaire')}
      <!-- The solar demo (a simulation): loaded only when visited. -->
      {#await import('./pages/Solaire.svelte') then { default: Solaire }}<Solaire />{/await}
    {:else if page === 'energie'}
      <Energie />
    {:else if page === 'impression'}
      <Impression />
    {:else if page === 'dehors'}
      <Dehors />
    {:else if page === 'moli'}
      <Moli />
    {:else if page === 'automatismes'}
      <Automatismes {route} />
    {:else if page === 'systeme'}
      <Systeme />
    {:else if page === 'infra'}
      <!-- The house's machines: loaded only when visited. -->
      {#await import('./pages/Infra.svelte') then { default: Infra }}<Infra />{/await}
    {:else}
      <Accueil />
    {/if}
  </main>

  {#if home.held}
    <PinSheet />
  {/if}

  {#if hub.session?.login_required}
    <!-- From the Internet, not signed in: nothing of the house, only this. -->
    <Login />
  {/if}

  {#if hub.session?.setup && !home.setupLater}
    <Setup onlater={() => (home.setupLater = true)} />
  {/if}

  {#if offline}
    <div class="offline" role="status"><Icon name="offline" size={16} />{t('commun.hors_ligne')}</div>
  {/if}

  <div class="notices" role="status" aria-live="polite">
    {#each hub.notices as n (n.id)}
      <div class="notice">
        <span class="bell"><Icon name="bell" size={18} /></span>
        <div>
          <b>{n.title ?? n.from ?? 'Moli'}</b>
          <p>{n.message}</p>
          {#if n.title && n.from}<small>{n.from}</small>{/if}
        </div>
        <button onclick={() => (hub.notices = hub.notices.filter((x) => x.id !== n.id))} aria-label={t('commun.fermer')}><Icon name="close" size={16} /></button>
      </div>
    {/each}
  </div>

  <div class="notes" role="status" aria-live="polite">
    {#each home.notes as n (n.id)}
      <p class="note {n.tone}">{n.text}</p>
    {/each}
  </div>
</div>

<style>
  /* Large screens: the menu at the side, each page by its name. */
  .maison {
    display: grid;
    grid-template-columns: 248px minmax(0, 1fr);
  }

  .rail {
    position: sticky;
    top: 0;
    height: 100dvh;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 26px 16px 18px;
    overflow-y: auto;
    scrollbar-width: none;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 12px 20px;
    color: var(--ink);
    text-decoration: none;
  }

  .brand svg {
    width: 34px;
    height: 34px;
    display: block;
    flex: none;
  }

  .brand-name {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .rail ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
  }

  .rail ul a,
  .rail-foot a,
  .rail-foot button {
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 46px;
    padding: 0 14px;
    border: 0;
    border-radius: 14px;
    background: none;
    color: var(--ink-2);
    font: inherit;
    text-align: left;
    text-decoration: none;
    cursor: pointer;
    transition: color 0.2s var(--ease), background 0.2s var(--ease);
  }

  .rail .glyph {
    flex: none;
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
  }

  .rail a.active {
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow);
  }

  .rail a:hover,
  .rail button:hover {
    color: var(--ink);
  }

  .label {
    font-size: 15px;
    font-weight: 550;
    line-height: 1.2;
  }

  .rail a.active .label {
    font-weight: 650;
  }

  .rail-foot {
    margin-top: auto;
    padding-top: 18px;
    display: grid;
    gap: 2px;
  }

  .rail-foot a,
  .rail-foot button {
    color: var(--ink-3);
  }

  .tabbar,
  .more-sheet {
    display: none;
  }

  .content {
    padding: 28px 36px 120px 8px;
    max-width: 1440px;
    width: 100%;
    min-width: 0;
  }

  .notes {
    position: fixed;
    left: 50%;
    bottom: 28px;
    transform: translateX(-50%);
    display: grid;
    gap: 8px;
    z-index: 40;
    pointer-events: none;
  }

  /* The link to Moli dropped (a restart, the Wi-Fi): say so instead of a
     dashboard that silently stops moving. */
  .offline {
    position: fixed;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 60;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-radius: 999px;
    background: var(--warm);
    color: #1c1407;
    font-size: 13.5px;
    font-weight: 600;
    box-shadow: var(--shadow);
  }

  .notices {
    position: fixed;
    top: 18px;
    right: 18px;
    z-index: 42;
    display: grid;
    gap: 10px;
    width: min(380px, calc(100vw - 32px));
  }

  .notice {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 14px 12px 14px 14px;
    border-radius: var(--r-md);
    background: var(--surface);
    box-shadow: var(--shadow-lift), inset 0 0 0 1px var(--line);
    animation: drop 0.4s var(--ease) both;
  }

  .notice .bell {
    width: 36px;
    height: 36px;
    flex: none;
    border-radius: 12px;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, #b05fd8 14%, var(--surface));
    color: #b05fd8;
  }

  .notice div {
    flex: 1;
    display: grid;
    gap: 2px;
  }

  .notice p {
    font-size: 14.5px;
    line-height: 1.4;
  }

  .notice small {
    color: var(--ink-3);
    font-size: 12px;
  }

  .notice button {
    border: 0;
    background: none;
    color: var(--ink-3);
    padding: 2px;
  }

  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-10px);
    }
  }

  .note {
    background: var(--ink);
    color: var(--bg);
    padding: 12px 18px;
    border-radius: 16px;
    font-weight: 550;
    box-shadow: var(--shadow-lift);
  }

  .note.error {
    background: var(--alert);
    color: #fff;
  }

  /* Tablets: a narrow rail, the name under each icon. */
  @media (max-width: 1099.98px) {
    .maison {
      grid-template-columns: 104px minmax(0, 1fr);
    }

    .rail {
      align-items: center;
      padding: 22px 0 18px;
    }

    .brand {
      padding: 0 0 14px;
    }

    .brand-name {
      display: none;
    }

    .rail ul a,
    .rail-foot a,
    .rail-foot button {
      flex-direction: column;
      justify-content: center;
      gap: 4px;
      width: 84px;
      min-height: 56px;
      padding: 8px 2px;
      border-radius: 18px;
      text-align: center;
    }

    .label {
      font-size: 11.5px;
      font-weight: 600;
      overflow-wrap: anywhere;
    }
  }

  /* Phones: a tab bar at the bottom (home, rooms, Moli, heating, more). */
  @media (max-width: 767.98px) {
    /* minmax(0, …): a `1fr` column grows to the widest page content and
       pushes every page sideways on a phone. */
    .maison {
      grid-template-columns: minmax(0, 1fr);
    }

    .rail {
      display: none;
    }

    .tabbar {
      position: fixed;
      left: 12px;
      right: 12px;
      bottom: calc(12px + env(safe-area-inset-bottom));
      z-index: 25;
      display: grid;
      grid-template-columns: repeat(5, minmax(0, 1fr));
      gap: 4px;
      padding: 8px;
      border-radius: 24px;
      background: var(--surface);
      box-shadow: var(--shadow-lift);
    }

    .tabbar a,
    .tabbar button {
      min-width: 0;
      min-height: 56px;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 3px;
      border: 0;
      border-radius: 16px;
      background: none;
      color: var(--ink-2);
      font: inherit;
      font-size: 11px;
      font-weight: 550;
      text-decoration: none;
      cursor: pointer;
    }

    .tabbar span {
      max-width: 100%;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .tabbar .active {
      background: var(--bg);
      color: var(--ink);
      font-weight: 650;
    }

    .tabbar .tab-moli,
    .tabbar .tab-moli.active {
      background: none;
    }

    .more-sheet {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 24;
      background: rgb(10 14 22 / 35%);
      animation: fade 0.2s var(--ease) both;
    }

    .more-panel {
      position: absolute;
      left: 12px;
      right: 12px;
      bottom: calc(96px + env(safe-area-inset-bottom));
      max-height: calc(100dvh - 140px);
      overflow-y: auto;
      padding: 10px;
      border-radius: 24px;
      background: var(--surface);
      box-shadow: var(--shadow-lift);
    }

    .more-panel ul {
      list-style: none;
      margin: 0;
      padding: 0;
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 6px;
    }

    .more-panel a,
    .more-panel button {
      box-sizing: border-box;
      width: 100%;
      min-height: 52px;
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 0 14px;
      border: 0;
      border-radius: 16px;
      background: var(--surface-2);
      color: var(--ink);
      font: inherit;
      font-size: 14.5px;
      font-weight: 600;
      text-align: left;
      text-decoration: none;
      cursor: pointer;
    }

    .more-panel .active {
      background: var(--warm-soft);
      color: var(--warm-ink);
    }

    .content {
      padding: 20px 16px 120px;
    }

    .notes {
      bottom: calc(100px + env(safe-area-inset-bottom));
      width: calc(100% - 32px);
    }
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }
</style>
