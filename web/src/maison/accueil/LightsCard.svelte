<script>
  import { isOn, roomList, lampsOf } from '../lib/home.svelte.js';
  import { lightTiles } from './cards.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import LightTile from '../ui/LightTile.svelte';
  import AmbiancePicker from '../ui/AmbiancePicker.svelte';

  /** The favourite lights (or one tile per room with lights) and, under
   *  them, the ambiances room by room. `tiles`: the ones to show, in that
   *  order (null: all); `ambiance`: the band or not. */
  let { tiles = null, ambiance = true, title = null, compact = false } = $props();

  const all = $derived(lightTiles());
  const lights = $derived(tiles ? tiles.map((id) => all.find((l) => l.id === id)).filter(Boolean) : all);
  const litCount = $derived(lights.filter((l) => [l.id, ...(l.also ?? [])].some((i) => isOn(i))).length);

  // Ambiances, room by room (the last room chosen is remembered here).
  const moodRooms = $derived(roomList().map((r) => ({ name: r.name, lamps: lampsOf(r) })).filter((r) => r.lamps.length));
  let moodPick = $state((() => {
    try {
      return localStorage.getItem('moli.mood-room');
    } catch {
      return null;
    }
  })());
  const mood = $derived(ambiance ? (moodRooms.find((r) => r.name === moodPick) ?? moodRooms.find((r) => r.name === 'Salon') ?? moodRooms[0]) : null);
  function pickRoom(name) {
    moodPick = name;
    try {
      localStorage.setItem('moli.mood-room', name);
    } catch {
      /* remembered for this visit only */
    }
  }
</script>

{#if lights.length || mood}
  <section class="card lights" class:compact>
    <div class="card-head">
      <h2><Icon name="light" size={18} />{title ?? t('maison.accueil.lumieres')} <span class="count">{litCount ? t('maison.accueil.allumees', { count: litCount }) : t('maison.accueil.tout_eteint')}</span></h2>
      <a class="more" href="#/pieces" aria-label={t('maison.accueil.toutes_pieces')}><Icon name="arrow-top-right" size={18} /></a>
    </div>
    <div class="tiles">
      {#each lights as l (l.id)}
        <LightTile id={l.id} name={l.name} icon={l.icon} also={l.also ?? []} />
      {/each}
    </div>
    {#if mood}
      <div class="moods">
        <div class="moods-head">
          <span class="moods-title"><Icon name="palette" size={16} />{t('maison.accueil.ambiance')}</span>
          <div class="rooms" role="group" aria-label={t('maison.accueil.piece')}>
            {#each moodRooms as r (r.name)}
              <button class:on={r.name === mood.name} onclick={() => pickRoom(r.name)} aria-pressed={r.name === mood.name}>{r.name}</button>
            {/each}
          </div>
        </div>
        <AmbiancePicker lights={mood.lamps} label={mood.name} />
      </div>
    {/if}
  </section>
{/if}

<style>
  .count {
    font-weight: 550;
    color: var(--ink-3);
    margin-left: 4px;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--sp-3);
  }

  .moods {
    grid-template-columns: minmax(0, 1fr);
    margin-top: 18px;
    padding-top: 16px;
    border-top: 1px solid var(--line);
    display: grid;
    gap: var(--sp-3);
  }

  .moods-head {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px 14px;
    flex-wrap: wrap;
  }

  .moods-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 700;
    font-size: var(--fs-md);
  }

  .rooms {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: none;
    min-width: 0;
    flex: 1;
  }

  .rooms button {
    flex: none;
    border: 0;
    border-radius: 999px;
    padding: 6px 12px;
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 650;
    background: var(--surface-2);
    color: var(--ink-3);
    cursor: pointer;
  }

  .rooms button.on {
    background: var(--warm-soft);
    color: var(--warm-ink);
  }

  @media (max-width: 767.98px) {
    .tiles {
      grid-template-columns: 1fr 1fr;
    }

    .compact .tiles {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
</style>
