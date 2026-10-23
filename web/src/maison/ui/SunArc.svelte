<script>
  // The sun's day, from the weather's sunrise and sunset: its arc with the
  // sun where it is now, the two times, how long the day is and how long
  // before the next change.
  import { home, clock } from '../lib/home.svelte.js';
  import { sunFacts, hoursMinutes } from '../lib/outside.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  let { sunrise, sunset } = $props();

  const sun = $derived(sunFacts(sunrise, sunset, home.now));
  // The arc: a half circle of radius 120 around (160, 128), the ground under it.
  const point = (pos) => {
    const a = Math.PI * (1 - pos);
    return [160 + 120 * Math.cos(a), 128 - 120 * Math.sin(a)];
  };
  const dot = $derived(sun ? point(sun.pos) : null);
  const lit = $derived(dot ? `M40 128 A120 120 0 0 1 ${dot[0].toFixed(1)} ${dot[1].toFixed(1)}` : '');

  function duration(ms) {
    const { h, m } = hoursMinutes(ms);
    if (!h) return t('maison.dehors.duree_min', { m });
    return m ? t('maison.dehors.duree_hm', { h, m: String(m).padStart(2, '0') }) : t('maison.dehors.duree_h', { h });
  }
</script>

{#if sun}
  <div class="sun">
    <div class="head">
      <b><Icon name="sun-angle" size={18} />{t('maison.dehors.soleil_titre')}</b>
      {#if sun.phase === 'day'}
        <span class="chip warm">{t('maison.dehors.jour_encore', { left: duration(sun.until) })}</span>
      {:else}
        <span class="chip cool">{t('maison.dehors.nuit_lever', { left: duration(sun.until) })}</span>
      {/if}
    </div>
    <svg class="arc" viewBox="0 0 320 140" aria-hidden="true">
      <path d="M40 128 A120 120 0 0 1 280 128" class="rail" />
      {#if sun.phase !== 'before'}<path d={lit} class="lit" />{/if}
      <line x1="16" y1="128" x2="304" y2="128" class="ground" />
      {#if sun.phase === 'day'}
        <circle cx={dot[0]} cy={dot[1]} r="16" class="halo" />
        <circle cx={dot[0]} cy={dot[1]} r="9" class="disc" />
      {/if}
      <text x="160" y="48" text-anchor="middle" class="noon num">{t('maison.dehors.plus_haut', { time: clock(sun.noon) })}</text>
    </svg>
    <div class="times num">
      <span><b>{clock(sun.rise)}</b> {t('maison.dehors.lever')}</span>
      <span>{t('maison.dehors.coucher')} <b>{clock(sun.set)}</b></span>
    </div>
    <p class="note">
      {t('maison.dehors.jour_de', { length: duration(sun.length) })}
      {sun.phase === 'day' ? t('maison.dehors.coucher_dans', { left: duration(sun.until) }) : t('maison.dehors.lever_dans', { left: duration(sun.until) })}
    </p>
  </div>
{/if}

<style>
  .sun {
    display: grid;
    align-content: space-between;
    gap: var(--sp-2);
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 10px;
  }

  .head > b {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: 17px;
    font-weight: 700;
  }

  .head :global(svg) {
    color: var(--ink-3);
  }

  .arc {
    width: 100%;
    max-width: 360px;
    margin: 0 auto;
    height: auto;
    display: block;
  }

  .rail {
    fill: none;
    stroke: var(--surface-3);
    stroke-width: 4;
    stroke-linecap: round;
  }

  .lit {
    fill: none;
    stroke: var(--sun);
    stroke-width: 4;
    stroke-linecap: round;
  }

  .ground {
    stroke: var(--line);
    stroke-width: 1.5;
  }

  .halo {
    fill: var(--sun);
    opacity: 0.25;
  }

  .disc {
    fill: var(--sun);
  }

  .noon {
    font-size: var(--fs-xs);
    font-weight: 650;
    fill: var(--ink-3);
  }

  .times {
    display: flex;
    justify-content: space-between;
    font-size: var(--fs-md);
  }

  .times b {
    font-weight: 700;
  }

  .note {
    font-size: 13.5px;
    color: var(--ink-3);
  }
</style>
