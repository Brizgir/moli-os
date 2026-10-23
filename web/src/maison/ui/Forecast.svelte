<script>
  // The sky to come, laid flat on the Outside page: the next hours as a
  // strip, the next days as rows (the same forecast the home page opens
  // in its dialog, `SkyDetail`).
  import { onMount } from 'svelte';
  import { sky, watchSky, hours, days, dayName } from '../lib/sky.svelte.js';
  import { weatherOf } from '../lib/icons.js';
  import { num } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  onMount(watchSky);

  const rows = $derived(hours(sky.forecast));
  const week = $derived(days(sky.forecast));
  const lo = $derived(Math.min(...week.map((d) => d.min).filter(Number.isFinite)));
  const hi = $derived(Math.max(...week.map((d) => d.max).filter(Number.isFinite)));
</script>

{#if rows.length}
  <section class="card">
    <div class="card-head">
      <h2><Icon name="clock" size={18} />{t('maison.dehors.prochaines_heures')}</h2>
      <span class="muted num">{t('maison.dehors.heures_a_venir', { n: rows.length - 1 })}</span>
    </div>
    <ol class="strip">
      {#each rows as h, i (h.at)}
        {@const s = weatherOf(h.code, h.day)}
        <li class:first={i === 0} title={s.label}>
          <small>{i === 0 ? t('maison.meteo.maintenant') : t('maison.meteo.heure', { h: h.hour })}</small>
          <Icon path={s.icon} size={24} label={s.label} />
          <b class="num">{num(h.temp, 0)}°</b>
          <em class="num" class:dry={h.rain < 20}>{h.rain}%</em>
        </li>
      {/each}
    </ol>
  </section>
{/if}

{#if week.length}
  <section class="card">
    <div class="card-head">
      <h2><Icon name="calendar" size={18} />{t('maison.dehors.sept_jours')}</h2>
      <span class="muted num">{t('maison.dehors.plage', { min: num(lo, 0), max: num(hi, 0) })}</span>
    </div>
    <ol class="days">
      {#each week as d, i (d.date)}
        {@const s = weatherOf(d.code, true)}
        <li title={s.label}>
          <small>{dayName(d.date, i)}</small>
          <Icon path={s.icon} size={22} label={s.label} />
          <span class="range num">
            <b>{num(d.max, 0)}°</b>
            <i class="bar" aria-hidden="true"><i style="left:{((d.min - lo) / Math.max(1, hi - lo)) * 100}%; right:{((hi - d.max) / Math.max(1, hi - lo)) * 100}%"></i></i>
            <span>{num(d.min, 0)}°</span>
          </span>
          <em class="num" class:dry={d.rain < 20}>{d.rain}%</em>
        </li>
      {/each}
    </ol>
  </section>
{/if}

<style>
  .strip {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    display: flex;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: thin;
  }

  .strip li {
    flex: none;
    width: 72px;
    display: grid;
    justify-items: center;
    gap: 6px;
    padding: 12px 4px;
    border-radius: var(--r-sm);
    text-align: center;
  }

  .strip li.first {
    background: var(--surface-2);
  }

  .strip small {
    font-size: var(--fs-xs);
    font-weight: 650;
    color: var(--ink-3);
    white-space: nowrap;
  }

  .strip b {
    font-size: var(--fs-base);
    font-weight: 700;
  }

  .strip :global(svg) {
    color: var(--ink-2);
  }

  em {
    font-style: normal;
    font-size: var(--fs-xs);
    font-weight: 650;
    color: var(--cool);
  }

  em.dry {
    color: var(--ink-3);
    opacity: 0.55;
  }

  .days {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--sp-1);
  }

  .days li {
    display: grid;
    grid-template-columns: 52px 26px 1fr 48px;
    align-items: center;
    gap: var(--sp-3);
    padding: 7px 10px;
    border-radius: var(--r-sm);
  }

  .days li:first-child {
    background: var(--surface-2);
  }

  .days small {
    font-size: var(--fs-sm);
    font-weight: 700;
  }

  .days :global(svg) {
    color: var(--ink-2);
  }

  .days em {
    font-size: var(--fs-sm);
    text-align: right;
  }

  .range {
    display: grid;
    grid-template-columns: 36px 1fr 36px;
    align-items: center;
    gap: 10px;
    font-size: var(--fs-md);
  }

  .range b {
    font-weight: 700;
    text-align: right;
  }

  .bar {
    position: relative;
    height: 6px;
    border-radius: 6px;
    background: var(--surface-3);
    display: block;
  }

  .bar i {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 6px;
    background: linear-gradient(90deg, var(--cool), var(--sun));
  }

  @media (max-width: 767.98px) {
    .days li {
      grid-template-columns: 44px 24px 1fr 40px;
      gap: var(--sp-2);
    }
  }
</style>
