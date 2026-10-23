<script>
  // Outside: the sky now and to come, the sun, the garden and the terrace
  // as the plan draws them, the temperature since yesterday, the outdoor
  // cameras and what they saw, who is out. Each card only with its data:
  // the air, the pool and the garden lights stay for the houses that
  // measure them.
  import { onMount } from 'svelte';
  import { home, hub, value, num, isOn, reachable, hidden, nameOf, roomOf, protectedRoom, relative } from '../lib/home.svelte.js';
  import { sky, hours, outlook } from '../lib/sky.svelte.js';
  import { placedIndoors } from '../lib/outside.js';
  import { people, loadPeople, whereabouts, colorOf } from '../lib/people.svelte.js';
  import { weatherOf } from '../lib/icons.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import LightTile from '../ui/LightTile.svelte';
  import SunArc from '../ui/SunArc.svelte';
  import Forecast from '../ui/Forecast.svelte';
  import OutdoorPlan from '../ui/OutdoorPlan.svelte';
  import OutdoorDay from '../ui/OutdoorDay.svelte';
  import OutdoorCameras from '../ui/OutdoorCameras.svelte';

  const out = $derived(home.config?.outdoor ?? {});
  const w = $derived(out.weather);
  const air = $derived(out.air);
  const pool = $derived(out.pool);
  const v = (id, key) => value(id, key);

  const day = $derived(v(w, 'daylight') !== false);
  const skyNow = $derived(weatherOf(v(w, 'weather_code'), day));
  const line = $derived(outlook(hours(sky.forecast)));

  const aqi = $derived(v(air, 'aqi'));
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
  const aqiTone = $derived(aqi == null ? '' : aqi <= 40 ? 'good' : aqi <= 60 ? 'warm' : 'alert');
  const uv = $derived(v(air, 'uv_index'));
  const uvWord = $derived(
    uv == null ? '' : uv < 3 ? t('maison.dehors.uv_faible') : uv < 6 ? t('maison.dehors.uv_modere') : uv < 8 ? t('maison.dehors.uv_eleve') : t('maison.dehors.uv_tres_eleve'),
  );

  // Pool chemistry: in range or not, in words.
  const ph = $derived(v(pool, 'ph'));
  const orp = $derived(v(pool, 'orp'));
  const phWord = $derived(
    ph == null ? null : ph < 7.0 ? t('maison.dehors.ph_acide') : ph > 7.6 ? t('maison.dehors.ph_basique') : t('maison.dehors.ph_equilibre'),
  );
  const orpWord = $derived(
    orp == null ? null : orp < 650 ? t('maison.dehors.orp_faible') : orp > 800 ? t('maison.dehors.orp_forte') : t('maison.dehors.orp_saine'),
  );
  const pumpId = $derived(out.pool_pump);
  const pumpKnown = $derived(pumpId && reachable(pumpId) && v(pumpId, 'switch_1') != null);

  // The outdoor cameras: the layout's, then any other camera, minus those
  // in a protected room and those the plan places inside the house.
  const cameras = $derived.by(() => {
    const listed = home.config?.favorites?.cameras ?? [];
    const known = new Set(listed.map((c) => c.id));
    const others = Object.values(hub.devices)
      .filter((d) => d.camera && !known.has(d.id) && !hidden(d.id))
      .map((d) => ({ id: d.id, name: nameOf(d.id), refresh: 15, battery: d.points.some((p) => p.key === 'battery') }));
    return [...listed, ...others].filter((c) => {
      const d = hub.devices[c.id];
      return d && !protectedRoom(roomOf(d) ?? '') && !placedIndoors(home.plan, c.id);
    });
  });

  // Who is out, from the phones.
  onMount(() => {
    if (!people.loaded) loadPeople();
  });
  const folks = $derived(people.list.filter((p) => p.role !== 'guest'));
  const away = $derived(folks.filter((p) => whereabouts(p.id).home === false));
  const sorted = $derived([...away, ...folks.filter((p) => !away.includes(p))]);
  function place(p) {
    const here = whereabouts(p.id);
    if (here.home === false) return here.zone ?? t('maison.dehors.sorti');
    if (here.home === true) return t('personnes.carte.a_la_maison');
    return t('personnes.carte.inconnu');
  }
</script>

<div class="dehors">
  <header>
    <h1 class="page-title">{t('maison.dehors.titre')}</h1>
    <p class="page-sub">{t('maison.dehors.sous_titre')}</p>
  </header>

  {#if w}
    <section class="card sky" class:night={!day}>
      <div>
        <div class="now">
          <svg class="sky-icon" viewBox="0 0 24 24" aria-hidden="true"><path d={skyNow.icon} /></svg>
          <div>
            <strong class="temp num">{num(v(w, 'temperature'), 0)}°</strong>
            <span class="label">{skyNow.label}</span>
            <span class="muted">{t('maison.dehors.ressenti', { feels: num(v(w, 'feels_like'), 0), max: num(v(w, 'today_max'), 0), min: num(v(w, 'today_min'), 0) })}</span>
          </div>
        </div>
        {#if line}<span class="outlook"><Icon name="wind" size={16} />{line}</span>{/if}
        <div class="facts">
          <div><Icon name="humidity" size={20} /><small>{t('maison.dehors.humidite')}</small><b class="num">{num(v(w, 'humidity'))} %</b></div>
          <div><Icon name="wind" size={20} /><small>{t('maison.dehors.vent')}</small><b class="num">{num(v(w, 'wind_speed'))} km/h</b><em class="muted num">{t('maison.dehors.rafales', { speed: num(v(w, 'wind_gusts')) })}</em></div>
          <div><Icon name="water" size={20} /><small>{t('maison.dehors.pluie_jour')}</small><b class="num">{num(v(w, 'today_rain'), 1)} mm</b><em class="muted num">{t('maison.dehors.risque', { pct: num(v(w, 'today_rain_chance')) })}</em></div>
          {#if v(w, 'cloud_cover') != null}
            <div><Icon name="cloud" size={20} /><small>{t('maison.dehors.nuages')}</small><b class="num">{num(v(w, 'cloud_cover'))} %</b></div>
          {:else}
            <div><Icon name="sun-angle" size={20} /><small>{t('maison.dehors.soleil')}</small><b class="num">{String(v(w, 'sunrise') ?? '').slice(11, 16) || '—'} → {String(v(w, 'sunset') ?? '').slice(11, 16) || '—'}</b></div>
          {/if}
        </div>
      </div>
      <SunArc sunrise={v(w, 'sunrise')} sunset={v(w, 'sunset')} />
    </section>
  {:else}
    <p class="muted">{t('maison.dehors.sans_meteo')}</p>
  {/if}

  {#if sky.forecast}
    <div class="row wide-left">
      <Forecast />
    </div>
  {/if}

  <div class="row wide-left">
    <OutdoorPlan />
    {#if w}<OutdoorDay id={w} />{/if}
  </div>

  <OutdoorCameras {cameras} />

  <div class="grid">
    {#if folks.length}
      <section class="card">
        <div class="card-head">
          <h2><Icon name="people" size={18} />{t('maison.dehors.qui_sorti')}</h2>
          <span class="muted">{away.length ? t('maison.dehors.dehors_compte', { count: away.length }) : t('maison.dehors.tous_la')}</span>
        </div>
        <ul class="people">
          {#each sorted as p (p.id)}
            {@const here = whereabouts(p.id)}
            <li class:in={here.home === true}>
              <span class="dot" style:background={colorOf(p)}>{p.name.slice(0, 1).toUpperCase()}</span>
              <span class="who"><b>{p.name}</b><small>{place(p)}{#if here.since}{' · '}{relative(here.since, home.now)}{/if}</small></span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <!-- Each card only with its data (an air sensor, a pool): the code stays. -->
    {#if aqi != null || uv != null || v(air, 'pm2_5') != null}
      <section class="card">
        <div class="card-head"><h2><Icon name="leaf" size={18} />{t('maison.dehors.air')}</h2><span class="chip {aqiTone}">{aqiWord}</span></div>
        <div class="pairs">
          <div><small>{t('maison.dehors.indice')}</small><b class="num">{num(aqi)}</b></div>
          <div><small>UV</small><b class="num">{num(uv, 0)}</b><em class="muted">{uvWord}</em></div>
          <div><small>{t('maison.dehors.particules')}</small><b class="num">{num(v(air, 'pm2_5'), 1)} µg/m³</b></div>
          <div><small>{t('maison.dehors.ozone')}</small><b class="num">{num(v(air, 'ozone'))} µg/m³</b></div>
        </div>
      </section>
    {/if}

    {#if pool}
      <section class="card">
        <div class="card-head">
          <h2><Icon name="pool" size={18} />{t('maison.dehors.piscine')}</h2>
          {#if v(pool, 'action_required') === true}<span class="chip cool">{t('maison.dehors.coup_oeil')}</span>{:else}<span class="chip good">{t('maison.dehors.tout_va_bien')}</span>{/if}
        </div>
        <div class="pairs">
          <div><small>{t('maison.dehors.eau')}</small><b class="num">{v(pool, 'temperature') != null ? `${num(v(pool, 'temperature'), 1)}°` : '—'}</b></div>
          <div><small>pH</small><b class="num">{num(ph, 1)}</b>{#if phWord}<em class="muted">{phWord}</em>{/if}</div>
          <div><small>{t('maison.dehors.chlore')}</small><b class="num">{orp != null ? `${num(orp)} mV` : '—'}</b>{#if orpWord}<em class="muted">{orpWord}</em>{/if}</div>
          <div><small>{t('maison.dehors.filtration')}</small><b class="num">{v(pool, 'filtration') != null ? t('maison.dehors.heures_jour', { hours: num(v(pool, 'filtration')) }) : '—'}</b></div>
        </div>
        {#if v(pool, 'temperature') == null && ph == null}
          <p class="muted small">{t('maison.dehors.sans_mesure')}</p>
        {/if}
        {#if pumpId}
          <div class="pump">
            {#if pumpKnown}
              <LightTile id={pumpId} name={t('maison.dehors.pompe')} icon="pool" compact />
            {:else}
              <p class="muted small"><Icon name="info" size={15} />{t('maison.dehors.pompe_inconnue')}</p>
            {/if}
          </div>
        {/if}
      </section>
    {/if}

    {#if out.garden_lights?.length}
      <section class="card">
        <div class="card-head"><h2><Icon name="flower-outline" size={18} />{t('maison.dehors.jardin')}</h2>
          <span class="muted">{out.garden_lights.filter((l) => isOn(l.id)).length ? t('maison.dehors.eclaire') : t('maison.dehors.noir')}</span>
        </div>
        <div class="lights">
          {#each out.garden_lights as l (l.id)}
            <LightTile id={l.id} name={l.name} icon="light" compact />
          {/each}
        </div>
      </section>
    {/if}
  </div>

  {#if w || air}
    <!-- CC BY 4.0: the weather and air data credit their source. -->
    <p class="credit">{t('maison.dehors.credit')} <a href="https://open-meteo.com/" target="_blank" rel="noopener">Open-Meteo.com</a></p>
  {/if}
</div>

<style>
  .credit {
    color: var(--ink-3);
    font-size: var(--fs-xs);
    text-align: center;
  }

  .credit a {
    color: inherit;
  }

  .dehors {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--sp-5);
  }

  .sky {
    padding: 28px;
    overflow: hidden;
    display: grid;
    grid-template-columns: 1.25fr 1fr;
    gap: 28px;
    background:
      radial-gradient(120% 140% at 100% 0%, color-mix(in srgb, var(--sun) 22%, transparent), transparent 55%),
      var(--surface);
  }

  .sky.night {
    background:
      radial-gradient(120% 140% at 100% 0%, color-mix(in srgb, var(--cool) 22%, transparent), transparent 55%),
      var(--surface);
  }

  .now {
    display: flex;
    align-items: center;
    gap: 22px;
  }

  .sky-icon {
    width: 96px;
    height: 96px;
    fill: var(--sun);
    flex: none;
  }

  .night .sky-icon {
    fill: var(--cool);
  }

  .now > div {
    display: grid;
    gap: 2px;
  }

  .temp {
    font-size: 64px;
    font-weight: 250;
    letter-spacing: -0.04em;
    line-height: 0.95;
  }

  .label {
    font-size: var(--fs-lg);
    font-weight: 650;
  }

  .outlook {
    margin-top: 14px;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 8px 14px;
    border-radius: 999px;
    background: var(--cool-soft);
    color: var(--cool);
    font-weight: 650;
    font-size: var(--fs-md);
  }

  .facts {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--sp-4);
    margin-top: 26px;
  }

  .facts > div,
  .pairs > div {
    display: grid;
    gap: 2px;
    align-content: start;
  }

  .facts :global(svg) {
    color: var(--ink-3);
    margin-bottom: 4px;
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
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--sp-5);
    align-items: start;
  }

  .row.wide-left {
    grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: var(--sp-5);
    align-items: start;
  }

  .grid:empty {
    display: none;
  }

  .pairs {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 18px 16px;
  }

  .small {
    font-size: var(--fs-sm);
    margin-top: 12px;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .pump {
    margin-top: 16px;
  }

  .lights {
    display: grid;
    gap: 10px;
  }

  .people {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 10px;
  }

  .people li {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }

  .people li.in {
    opacity: 0.6;
  }

  .dot {
    flex: none;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: #fff;
    font-weight: 800;
  }

  .who b {
    display: block;
    font-size: 15px;
  }

  .who small {
    font-weight: 500;
    font-size: var(--fs-sm);
  }

  @media (max-width: 767.98px) {
    .sky {
      grid-template-columns: 1fr;
      padding: 22px;
      gap: 18px;
    }

    .facts {
      grid-template-columns: 1fr 1fr;
    }

    .sky-icon {
      width: 64px;
      height: 64px;
    }

    .temp {
      font-size: 52px;
    }

    .row,
    .row.wide-left {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
