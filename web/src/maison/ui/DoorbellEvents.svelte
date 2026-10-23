<script>
  import { onMount, untrack } from 'svelte';
  import { hub, home, value, nameOf, hidden, ringing, lastSeen, dayAndTime, num } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** A doorbell that tells its events without an image: the last press
   *  and the last motion, read in the history (the live value falls back
   *  to « no » seconds later), and the batteries. A Ring doorbell gives
   *  its image now (`CameraTile`): this card is for one that does not. */
  const devices = $derived(Object.values(hub.devices).filter((d) => !hidden(d.id) && ringing(d)));
  const bell = $derived(devices.find((d) => d.points.some((p) => p.key === 'doorbell')) ?? devices[0]);
  const watchers = $derived(devices.filter((d) => d.points.some((p) => p.key === 'motion')));

  let pressed = $state(null);
  let moved = $state(null);

  async function refresh() {
    if (!bell) return;
    const week = 24 * 7;
    pressed = await lastSeen(bell.id, ['doorbell'], week);
    const seen = await Promise.all(watchers.map(async (d) => ({ id: d.id, ts: await lastSeen(d.id, ['motion'], week) })));
    moved = seen.filter((s) => s.ts).sort((a, b) => b.ts - a.ts)[0] ?? null;
  }

  onMount(() => {
    const timer = setInterval(refresh, 120_000);
    return () => clearInterval(timer);
  });
  // Read again when the devices change (not each of their values) and at
  // a press or a motion, as it happens.
  const ids = $derived([bell?.id, ...watchers.map((d) => d.id)].join());
  const ringingNow = $derived(!!bell && (value(bell.id, 'doorbell') === true || watchers.some((d) => value(d.id, 'motion') === true)));
  $effect(() => {
    void ids;
    void ringingNow;
    untrack(refresh);
  });

  const batteries = $derived(devices.map((d) => value(d.id, 'battery')).filter((b) => typeof b === 'number'));
  const lowest = $derived(batteries.length ? Math.min(...batteries) : null);
</script>

{#if bell}
  <a class="card bell" href="#/cameras">
    <div class="card-head">
      <h2><Icon name="bell" size={18} />{t('maison.accueil.sonnette_titre')}</h2>
      <span class="more" aria-hidden="true"><Icon name="arrow-top-right" size={18} /></span>
    </div>
    <div class="picture">{t('maison.accueil.sans_image')}</div>
    <div class="facts">
      <b>{pressed ? t('maison.accueil.dernier_appui', { when: dayAndTime(pressed, home.now) }) : t('maison.accueil.aucun_appui')}</b>
      <span class="muted">
        {#if moved}{t('maison.accueil.dernier_mouvement', { name: nameOf(moved.id), when: dayAndTime(moved.ts, home.now) })}{:else}{t('maison.accueil.aucun_mouvement')}{/if}
        {#if lowest != null}
          · {batteries.length > 1 && new Set(batteries).size > 1 ? t('maison.accueil.batteries', { pct: num(lowest) }) : t(batteries.length > 1 ? 'maison.accueil.batteries_egales' : 'maison.accueil.batterie', { pct: num(lowest) })}
        {/if}
      </span>
    </div>
  </a>
{/if}

<style>
  .bell {
    display: grid;
    gap: 14px;
    color: inherit;
    text-decoration: none;
    transition: transform 0.2s var(--ease);
  }

  .bell:hover {
    transform: translateY(-2px);
  }

  .picture {
    aspect-ratio: 16 / 10;
    border-radius: 18px;
    background: var(--surface-2);
    display: grid;
    place-items: center;
    padding: var(--sp-3);
    color: var(--ink-3);
    font-size: var(--fs-md);
    text-align: center;
  }

  .facts {
    display: grid;
    gap: var(--sp-1);
  }

  .facts .muted {
    font-size: var(--fs-sm);
  }

  @media (max-width: 767.98px) {
    .picture {
      display: none;
    }
  }
</style>
