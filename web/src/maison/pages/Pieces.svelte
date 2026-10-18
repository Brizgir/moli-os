<script module>
  /** The plan, kept between visits: the floors show at once the next time. */
  let cachedPlan = null;
</script>

<script>
  import { onMount } from 'svelte';
  import { roomList, roomNamed, savePref, roamingSpeakers } from '../lib/home.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import RoomCard from '../ui/RoomCard.svelte';
  import RoomForm from '../ui/RoomForm.svelte';
  import FilterChips from '../ui/FilterChips.svelte';
  import { categoryOf, loadFilter } from '../lib/filters.js';
  import { floorsOf } from '../lib/floors.js';
  import { t } from '../../lib/i18n.svelte.js';

  let plan = $state(cachedPlan);
  onMount(async () => {
    try {
      const res = await fetch('/api/plan');
      if (res.ok) plan = cachedPlan = await res.json();
    } catch {
      /* without the plan: one list, no floors */
    }
  });

  /** Which kinds of devices show (none chosen: all). */
  let filter = $state(loadFilter('pieces-filter'));
  /** Pads with their names or icons only: this device's choice. */
  let look = $state((() => {
    try {
      return localStorage.getItem('pieces-look') === 'icons' ? 'icons' : 'text';
    } catch {
      return 'text';
    }
  })());
  function choose(v) {
    look = v;
    savePref('pieces-look', v);
  }

  /** The room's sheet: `null` closed, `'new'` a new room, else the room. */
  let sheet = $state(null);

  // Speakers in no room (an Alexa group, « Partout »): one card of their own.
  const everywhere = $derived.by(() => {
    const media = roamingSpeakers();
    if (!media.length) return null;
    return {
      name: t('maison.pieces.toute_la_maison'),
      note: t('maison.pieces.toute_la_maison_note'),
      icon: 'speaker-multiple',
      group: null,
      lights: [],
      plugs: [],
      covers: [],
      climates: [],
      media,
      sensors: [],
      lit: 0,
    };
  });
  const all = $derived(roomList());
  const devices = $derived(
    [...all, everywhere].filter(Boolean).flatMap((r) => [r.group, ...r.lights, ...r.plugs, ...r.covers, ...r.climates, ...r.media, ...r.sensors].filter(Boolean)),
  );

  /** A room with only what the filter keeps; null when nothing is left. */
  function only(r) {
    if (!filter.length) return r;
    const on = (id) => filter.includes(id);
    const f = {
      ...r,
      group: on('lumieres') ? r.group : null,
      lights: on('lumieres') ? r.lights : [],
      plugs: on('prises') ? r.plugs : [],
      covers: on('climat') ? r.covers : [],
      climates: on('climat') ? r.climates : [],
      media: on('medias') ? r.media : [],
      sensors: r.sensors.filter((d) => on(categoryOf(d))),
    };
    const any = f.group || f.lights.length || f.plugs.length || f.covers.length || f.climates.length || f.media.length || f.sensors.length;
    return any ? f : null;
  }

  const rooms = $derived(all.map(only).filter(Boolean));

  // Floor by floor (from all the rooms: a filter does not move one).
  const placed = $derived(floorsOf(all, plan, roomNamed));
  const floors = $derived(
    placed.sections
      .map((s) => ({ ...s, rooms: rooms.filter((r) => placed.of.get(r.name) === s.id) }))
      .filter((s) => s.rooms.length),
  );
  const litIn = (list) => list.reduce((n, r) => n + r.lit, 0);
  const litAll = $derived(litIn(all));
</script>

<!-- One more room: the sheet already there (a name, an icon, other names). -->
{#snippet addRoom()}
  <button class="add-room" onclick={() => (sheet = 'new')}>
    <Icon name="plus" size={26} />
    <span>{t('maison.pieces.nouvelle')}</span>
    <small>{t('maison.pieces.nouvelle_indice')}</small>
  </button>
{/snippet}

<div class="pieces">
  <header>
    <h1 class="page-title">{t('maison.pieces.titre')}</h1>
    <p class="page-sub">{all.length ? `${t('maison.pieces.compte', { count: all.length })} · ${litAll ? t('maison.pieces.allumees', { count: litAll }) : t('maison.pieces.tout_eteint')}` : t('maison.pieces.sous_titre')}</p>
  </header>

  <div class="bar">
    <FilterChips {devices} bind:filter key="pieces-filter" />
    <button class="new-room" onclick={() => (sheet = 'new')}>
      <Icon name="plus" size={18} />
      <span>{t('maison.pieces.nouvelle')}</span>
    </button>
    <div class="look" role="radiogroup" aria-label={t('maison.pieces.affichage')}>
      <button role="radio" aria-checked={look === 'text'} class:active={look === 'text'} onclick={() => choose('text')} title={t('maison.pieces.avec_noms')}>Aa</button>
      <button role="radio" aria-checked={look === 'icons'} class:active={look === 'icons'} onclick={() => choose('icons')} title={t('maison.pieces.icones')}><Icon name="light" size={15} /></button>
    </div>
  </div>

  {#each floors as floor (floor.id)}
    <section class="floor" aria-label={floor.name ?? t('maison.pieces.titre')}>
      {#if floors.length > 1 && floor.name}
        {@const lit = litIn(floor.rooms)}
        <h2 class="floor-title">
          <span>{floor.name}</span>
          {#if lit}<small>{t('maison.pieces.allumees', { count: lit })}</small>{/if}
        </h2>
      {/if}
      <div class="rooms">
        {#each floor.rooms as room (room.name)}
          <div class="slot"><RoomCard {room} every={filter.includes('capteurs')} icons={look === 'icons'} onedit={(r) => (sheet = r)} /></div>
        {/each}
        {#if floor === floors.at(-1)}
          <div class="slot">{@render addRoom()}</div>
        {/if}
      </div>
    </section>
  {/each}

  {#if !all.length}
    <p class="none">{t('maison.pieces.aucune')}</p>
  {/if}

  {#if !floors.length}
    <div class="rooms">
      <div class="slot">{@render addRoom()}</div>
    </div>
  {/if}

  {#if sheet}
    <RoomForm room={sheet === 'new' ? null : sheet} onclose={() => (sheet = null)} />
  {/if}

  {#if everywhere && only(everywhere)}
    <section class="floor" aria-label={everywhere.name}>
      <div class="rooms">
        <div class="slot"><RoomCard room={only(everywhere)} icons={look === 'icons'} /></div>
      </div>
    </section>
  {/if}
</div>

<style>
  .pieces {
    display: grid;
    gap: 22px;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .bar > :global(:first-child) {
    flex: 1;
    min-width: 0;
  }

  .new-room {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 40px;
    padding: 8px 16px 8px 12px;
    border: none;
    border-radius: 999px;
    background: var(--ink);
    color: var(--surface);
    font: inherit;
    font-weight: 650;
    cursor: pointer;
  }

  .none {
    margin: 0;
    color: var(--ink-3);
  }

  .look {
    display: inline-flex;
    padding: 3px;
    border-radius: 999px;
    background: var(--surface-2);
  }

  .look button {
    display: grid;
    place-items: center;
    min-width: 34px;
    height: 28px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--ink-3);
    font: inherit;
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
  }

  .look button.active {
    background: var(--surface);
    color: var(--ink);
    box-shadow: 0 1px 3px rgb(0 0 0 / 15%);
  }

  .floor {
    display: grid;
    gap: 12px;
  }

  .floor-title {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 0;
    font-size: 13px;
    font-weight: 750;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink-2);
  }

  /* A thin rule after the name, to the edge. */
  .floor-title::after {
    content: '';
    flex: 1;
    align-self: center;
    height: 1px;
    background: color-mix(in srgb, var(--ink-3) 25%, transparent);
  }

  .floor-title small {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0;
    text-transform: none;
    color: var(--warm-ink);
  }

  .rooms {
    columns: 3 340px;
    column-gap: 20px;
  }

  .slot {
    break-inside: avoid;
    margin-bottom: 20px;
  }

  .add-room {
    box-sizing: border-box;
    width: 100%;
    display: grid;
    justify-items: center;
    align-content: center;
    gap: 6px;
    min-height: 140px;
    padding: 16px;
    border: 2px dashed color-mix(in srgb, var(--ink-3) 40%, transparent);
    border-radius: var(--r-lg);
    background: transparent;
    color: var(--ink-2);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .add-room:hover {
    border-color: var(--ink-3);
    color: var(--ink);
  }

  .add-room small {
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-3);
  }
</style>
