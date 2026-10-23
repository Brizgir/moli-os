<script>
  import { home, value, num } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';

  /** Outside in brief: the air, the UV, the humidity, the pool, each when
   *  it is measured. `facts`: the ones to show, in that order (null: all). */
  let { facts = null, title = null } = $props();

  const out = $derived(home.config?.outdoor ?? {});
  const aqi = $derived(value(out.air, 'aqi'));
  const aqiWord = $derived(
    aqi == null
      ? '—'
      : aqi <= 20
        ? t('maison.air.tres_bon')
        : aqi <= 40
          ? t('maison.air.bon')
          : aqi <= 60
            ? t('maison.air.moyen')
            : aqi <= 80
              ? t('maison.air.mediocre')
              : t('maison.air.mauvais'),
  );
  const humidity = $derived(value(out.weather, 'humidity'));
  const poolTemp = $derived(value(out.pool, 'temperature'));
  const uv = $derived(value(out.air, 'uv_index'));

  const all = $derived([
    { key: 'air', has: aqi != null, label: t('maison.accueil.air'), text: aqiWord, num: false },
    { key: 'uv', has: uv != null, label: 'UV', text: num(uv, 0), num: true },
    { key: 'humidity', has: humidity != null, label: t('maison.accueil.humidite'), text: `${num(humidity)} %`, num: true },
    { key: 'pool', has: poolTemp != null, label: t('maison.accueil.piscine'), text: `${num(poolTemp, 1)}°`, num: true },
  ]);
  const shown = $derived((facts ? facts.map((k) => all.find((f) => f.key === k)).filter(Boolean) : all).filter((f) => f.has));
  // Without the air or the pool, the sky above says the rest.
  const outside = $derived(shown.some((f) => f.key !== 'humidity'));
</script>

{#if outside}
  <a class="card outside" href="#/dehors">
    <div class="card-head"><h2><Icon name="flower-outline" size={18} />{title ?? t('maison.accueil.dehors')}</h2><span class="more" aria-hidden="true"><Icon name="arrow-top-right" size={18} /></span></div>
    <div class="facts">
      {#each shown as f (f.key)}
        <div><small>{f.label}</small><b class:num={f.num}>{f.text}</b></div>
      {/each}
    </div>
  </a>
{/if}

<style>
  .outside {
    color: inherit;
    text-decoration: none;
    transition: transform 0.2s var(--ease);
  }

  .outside:hover {
    transform: translateY(-2px);
  }

  .facts {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--sp-2);
  }

  .facts small {
    display: block;
    font-size: 11px;
    color: var(--ink-3);
    font-weight: 650;
  }

  .facts b {
    font-size: 17px;
    font-weight: 650;
  }
</style>
