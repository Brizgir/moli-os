<script>
  import { CARDS, houseFacts } from './cards.js';
  import { catalogue } from '../lib/layout.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';

  /** What the house can add to the page: its rooms, its devices with a
   *  card of their own, the house's cards; what is already there says so. */
  let { cards, onadd, onclose } = $props();

  let query = $state('');
  const all = $derived(catalogue(houseFacts(), cards));
  const norm = (s) =>
    String(s ?? '')
      .normalize('NFD')
      .replace(/\p{M}/gu, '')
      .toLowerCase();
  const label = (entry) => entry.label ?? t(CARDS[entry.type]?.label ?? entry.type);
  const matches = (entry) => !query.trim() || norm(`${label(entry)} ${t(CARDS[entry.type]?.label ?? '')}`).includes(norm(query));
  const rooms = $derived(all.rooms.filter(matches));
  const devices = $derived(all.devices.filter(matches));
  const house = $derived(all.house.filter(matches));

  function onkey(e) {
    if (e.key === 'Escape') onclose?.();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="scrim" role="presentation" onclick={onclose}></div>
<div class="sheet" role="dialog" aria-modal="true" aria-label={t('maison.modifier.catalogue')}>
  <header>
    <h2>{t('maison.modifier.catalogue')}</h2>
    <button class="ghost" onclick={onclose}>{t('maison.modifier.fermer')}</button>
  </header>
  <label class="search">
    <Icon name="zoom-in" size={18} />
    <input type="search" bind:value={query} placeholder={t('maison.modifier.chercher')} />
  </label>

  {#snippet row(entry)}
    <li>
      <span class="what">
        <Icon name={CARDS[entry.type]?.icon ?? 'alert'} size={18} />
        <span class="words"><b>{label(entry)}</b>{#if entry.label}<small>{t(CARDS[entry.type].label)}</small>{/if}</span>
      </span>
      {#if entry.present === 'shown'}
        <span class="state on">{t('maison.modifier.deja_la')}</span>
      {:else if entry.present === 'hidden'}
        <span class="state">{t('maison.modifier.masquee')}</span>
      {:else}
        <button class="add" aria-label={t('maison.modifier.ajouter_carte', { name: label(entry) })} onclick={() => onadd?.(entry)}><Icon name="plus" size={18} /></button>
      {/if}
    </li>
  {/snippet}

  {#if rooms.length}
    <section>
      <h3>{t('maison.modifier.cat_pieces')}</h3>
      <ul>{#each rooms as entry (entry.label)}{@render row(entry)}{/each}</ul>
    </section>
  {/if}
  {#if devices.length}
    <section>
      <h3>{t('maison.modifier.cat_appareils')}</h3>
      <ul>{#each devices as entry (`${entry.type}:${entry.options.device}`)}{@render row(entry)}{/each}</ul>
    </section>
  {/if}
  {#if house.length}
    <section>
      <h3>{t('maison.modifier.cat_maison')}</h3>
      <ul>{#each house as entry (entry.type)}{@render row(entry)}{/each}</ul>
    </section>
  {/if}
  {#if !rooms.length && !devices.length && !house.length}
    <p class="muted">{t('maison.modifier.rien_trouve')}</p>
  {/if}
  <p class="muted hint">{t('maison.modifier.catalogue_aide')}</p>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgb(10 14 22 / 45%);
    backdrop-filter: blur(4px);
    z-index: 50;
  }

  .sheet {
    position: fixed;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    z-index: 51;
    width: min(480px, calc(100vw - 32px));
    max-height: calc(100dvh - 32px);
    overflow-y: auto;
    background: var(--surface);
    border-radius: var(--r-xl);
    box-shadow: var(--shadow-lift);
    padding: 24px;
    display: grid;
    gap: 16px;
    align-content: start;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 750;
  }

  h3 {
    margin: 0 0 8px;
    font-size: 13px;
    font-weight: 700;
    color: var(--ink-3);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 48px;
    padding: 0 14px;
    border-radius: var(--r-sm);
    background: var(--surface-2);
    color: var(--ink-3);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 16px;
    outline: none;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 4px;
  }

  li {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 52px;
    padding: 4px 6px 4px 12px;
    border-radius: 14px;
    background: var(--surface-2);
  }

  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--ink-2);
  }

  .words {
    display: grid;
    min-width: 0;
  }

  .words b {
    color: var(--ink);
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .words small {
    font-size: 12px;
    color: var(--ink-3);
  }

  .state {
    font-size: 13px;
    font-weight: 650;
    color: var(--ink-3);
    padding: 0 10px;
  }

  .state.on {
    color: var(--good);
  }

  .add {
    flex: none;
    width: 44px;
    height: 44px;
    border: 0;
    border-radius: 12px;
    background: var(--warm-soft);
    color: var(--warm-ink);
    cursor: pointer;
    display: grid;
    place-items: center;
  }

  .ghost {
    min-height: 40px;
    padding: 0 14px;
    border: 0;
    border-radius: 12px;
    background: var(--surface-2);
    color: var(--ink-2);
    font: inherit;
    font-size: 14px;
    font-weight: 650;
    cursor: pointer;
  }

  .muted {
    margin: 0;
    font-size: 13px;
  }
</style>
