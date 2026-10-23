<script>
  // The outdoor temperature since yesterday, read in the history: a line
  // over the last 24 hours, the coldest and warmest moments, and how it
  // compares with the same time yesterday.
  import { onMount } from 'svelte';
  import { clock, num } from '../lib/home.svelte.js';
  import { temperatureDay, sparkPath } from '../lib/outside.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';

  /** `id`: the weather device (its `temperature` point). */
  let { id } = $props();

  const W = 360;
  const H = 120;
  let series = $state(null);

  async function load() {
    if (!id) return;
    try {
      const res = await fetch(`/api/history?point=${encodeURIComponent(`${id}/temperature`)}&hours=24&points=300`);
      series = res.ok ? await res.json() : null;
    } catch {
      series = null;
    }
  }

  onMount(() => {
    load();
    const timer = setInterval(load, 15 * 60_000);
    return () => clearInterval(timer);
  });

  const day = $derived(temperatureDay(series));
  const path = $derived(day ? sparkPath(day.points, W, H - 20, day.from, day.to) : null);
  const deltaWord = $derived.by(() => {
    if (!day) return '';
    const d = Math.round(day.delta);
    if (d === 0) return t('maison.dehors.pareil');
    return `${d > 0 ? '+' : '−'} ${num(Math.abs(d), 0)}°`;
  });
</script>

{#if day && path}
  <section class="card">
    <div class="card-head">
      <h2><Icon name="thermometer" size={18} />{t('maison.dehors.depuis_hier')}</h2>
      <span class="muted">{t('maison.dehors.heure_par_heure')}</span>
    </div>
    <svg viewBox="0 0 {W} {H}" role="img" aria-label={t('maison.dehors.courbe_alt')}>
      <line x1="0" y1={H - 20} x2={W} y2={H - 20} class="ground" />
      <path d={path.fill} class="area" />
      <path d={path.line} class="line" />
      <circle cx={path.x(day.max.ts)} cy={path.y(day.max.v)} r="3.5" class="mark" />
      <circle cx={path.x(day.min.ts)} cy={path.y(day.min.v)} r="3.5" class="mark" />
      <circle cx={path.x(day.to)} cy={path.y(day.last)} r="5" class="now" />
      <text x="0" y={H - 4} class="axis">{t('maison.dehors.hier_a', { time: clock(day.from) })}</text>
      <text x={W} y={H - 4} text-anchor="end" class="axis">{t('maison.dehors.maintenant')}</text>
    </svg>
    <div class="facts">
      <div><small>{t('maison.dehors.plus_froid')}</small><b class="num">{num(day.min.v, 0)}°</b><em class="num">{t('maison.dehors.a_heure', { time: clock(day.min.ts) })}</em></div>
      <div><small>{t('maison.dehors.plus_chaud')}</small><b class="num">{num(day.max.v, 0)}°</b><em class="num">{t('maison.dehors.a_heure', { time: clock(day.max.ts) })}</em></div>
      <div><small>{t('maison.dehors.vs_hier')}</small><b class="num">{deltaWord}</b></div>
    </div>
  </section>
{/if}

<style>
  svg {
    width: 100%;
    height: auto;
    display: block;
  }

  .ground {
    stroke: var(--line);
  }

  .area {
    fill: var(--cool);
    opacity: 0.1;
  }

  .line {
    fill: none;
    stroke: var(--cool);
    stroke-width: 3;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .mark {
    fill: var(--surface);
    stroke: var(--cool);
    stroke-width: 2;
  }

  .now {
    fill: var(--cool);
  }

  .axis {
    font-size: 11px;
    font-weight: 600;
    fill: var(--ink-3);
  }

  .facts {
    display: flex;
    gap: 22px;
    margin-top: 10px;
  }

  .facts > div {
    display: grid;
    gap: 2px;
  }

  small {
    font-size: var(--fs-xs);
    font-weight: 650;
    color: var(--ink-3);
  }

  b {
    font-size: 17px;
    font-weight: 650;
  }

  em {
    font-style: normal;
    font-size: var(--fs-sm);
    color: var(--ink-3);
  }
</style>
