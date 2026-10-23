<script>
  import { hub, home, value, heatedRooms, readPref, savePref, deg } from '../lib/home.svelte.js';
  import { t, locale } from '../../lib/i18n.svelte.js';
  import { RANGES, line, steps, heats, latest, valueAt, stretchAt, scale } from '../lib/heating-curves.js';
  import HeatingPlot from './HeatingPlot.svelte';

  /** Each heated room of a Netatmo home (`instance`) over 24 h, 7 or 30
   *  days, from the history: the temperature it read, the setpoint it
   *  aimed at, when it heated. One temperature axis for every room, so they
   *  compare; « Toutes les pièces » lays their temperatures over each other,
   *  « Extérieur » adds the weather's as a thin line. */
  let { instance = null } = $props();

  const HOUR = 3_600_000;
  const PALETTE = ['#e9a23b', '#5b8def', '#3fae7c', '#c46ad8', '#e0705a', '#2fb3b8'];
  const VIEWS = ['rooms', 'all'];

  let range = $state(RANGES[readPref('maison-courbes', 'day')] ? readPref('maison-courbes', 'day') : 'day');
  let view = $state(VIEWS.includes(readPref('maison-courbes-vue', 'rooms')) ? readPref('maison-courbes-vue', 'rooms') : 'rooms');
  let outside = $state(readPref('maison-courbes-dehors', '0') === '1');
  let hover = $state(null);

  const rooms = $derived(heatedRooms().filter((r) => !instance || hub.devices[r.id]?.instance === instance));
  const roomIds = $derived(rooms.map((r) => r.id).join('\n'));
  /** The weather's temperature, if the house has one (`home.json` `outdoor.weather`). */
  const weather = $derived.by(() => {
    const id = home.config?.outdoor?.weather;
    return id && hub.devices[id]?.points?.some((p) => p.key === 'temperature') ? `${id}/temperature` : null;
  });

  let data = $state(null);
  let loading = $state(false);
  let error = $state('');

  async function series(point, hours, points) {
    const res = await fetch(`/api/history?point=${encodeURIComponent(point)}&hours=${hours}&points=${points}`);
    if (res.status === 404) throw new Error('off');
    if (!res.ok) return null;
    return res.json();
  }

  let round = 0;
  async function load(ids, hours, outdoorPoint) {
    const mine = ++round;
    loading = true;
    try {
      const to = Date.now();
      const per = await Promise.all(
        ids.map(async (id) => {
          const [temperature, setpoint, heating] = await Promise.all([series(`${id}/temperature`, hours, 400), series(`${id}/setpoint`, hours, 2000), series(`${id}/heating`, hours, 2000)]);
          return [id, { temperature, setpoint, heating }];
        }),
      );
      const outdoor = outdoorPoint ? await series(outdoorPoint, hours, 400) : null;
      if (mine !== round) return;
      data = { hours, from: to - hours * HOUR, to, rooms: Object.fromEntries(per), outdoor };
      error = '';
    } catch (e) {
      if (mine === round) error = e.message;
    } finally {
      if (mine === round) loading = false;
    }
  }

  $effect(() => {
    const ids = roomIds ? roomIds.split('\n') : [];
    const hours = RANGES[range];
    const point = weather;
    if (!ids.length) return;
    load(ids, hours, point);
    const timer = setInterval(() => load(ids, hours, point), 5 * 60_000);
    return () => clearInterval(timer);
  });

  function choose(next) {
    range = next;
    hover = null;
    savePref('maison-courbes', next);
  }
  function show(next) {
    view = next;
    hover = null;
    savePref('maison-courbes-vue', next);
  }
  function toggleOutside() {
    outside = !outside;
    savePref('maison-courbes-dehors', outside ? '1' : '0');
  }

  const from = $derived(data?.from ?? 0);
  const to = $derived(data?.to ?? 0);
  const curves = $derived(
    data
      ? rooms.map((r, i) => {
          const s = data.rooms[r.id] ?? {};
          const live = value(r.id, 'temperature');
          return {
            ...r,
            color: PALETTE[i % PALETTE.length],
            temp: line(s.temperature, from, to),
            set: steps(s.setpoint, from, to),
            heat: heats(s.heating, from, to),
            last: latest(s.temperature) ?? (typeof live === 'number' ? [Date.now(), live] : null),
            aim: latest(s.setpoint)?.[1] ?? value(r.id, 'setpoint'),
            heating: value(r.id, 'heating') === true,
          };
        })
      : [],
  );
  const outdoor = $derived(data && outside && data.outdoor ? line(data.outdoor, from, to) : []);
  // One axis for every room (and outside, when shown), so they compare.
  const axis = $derived(scale([...curves.flatMap((c) => [...c.temp.map(([, v]) => v), ...c.set.map((s) => s.value)]), ...outdoor.map(([, v]) => v)]));
  const outdoorLine = $derived(outdoor.length ? [{ id: '__outdoor', points: outdoor, color: 'var(--ink-3)', faint: true }] : []);

  function when(ts) {
    const d = new Date(ts);
    const opts = range === 'day' ? { weekday: 'short', hour: '2-digit', minute: '2-digit' } : { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' };
    const s = d.toLocaleString(locale(), opts);
    return s.charAt(0).toUpperCase() + s.slice(1);
  }
  const ago = (ts) => (Date.now() - ts < 20 * HOUR ? new Date(ts).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' }) : when(ts));

  /** What a room said at the picked moment. */
  function at(c, ts) {
    const h = stretchAt(c.heat, ts);
    return { temp: valueAt(c.temp, ts), aim: stretchAt(c.set, ts)?.value ?? null, share: h?.share ?? 0 };
  }
  const heatText = (share) => (share >= 1 ? t('maison.chauffage.courbes.chauffe') : share > 0 ? t('maison.chauffage.courbes.chauffe_parfois') : t('maison.chauffage.courbes.repos'));
</script>

{#if rooms.length}
  <section class="card curves">
    <div class="head">
      <h2>{t('maison.chauffage.courbes.titre')}</h2>
      <div class="segmented" role="radiogroup" aria-label={t('maison.chauffage.courbes.periode')}>
        {#each Object.keys(RANGES) as r (r)}
          <button role="radio" aria-checked={range === r} class:on={range === r} onclick={() => choose(r)}>{t(`maison.chauffage.courbes.${r}`)}</button>
        {/each}
      </div>
    </div>
    <div class="tools">
      <div class="segmented" role="radiogroup" aria-label={t('maison.chauffage.courbes.vue')}>
        {#each VIEWS as v (v)}
          <button role="radio" aria-checked={view === v} class:on={view === v} onclick={() => show(v)}>{t(`maison.chauffage.courbes.vue_${v}`)}</button>
        {/each}
      </div>
      {#if weather}
        <button class="chip" class:on={outside} aria-pressed={outside} onclick={toggleOutside}><i class="key outdoor"></i>{t('maison.chauffage.courbes.exterieur')}</button>
      {/if}
    </div>

    {#if error === 'off'}
      <p class="muted">{t('maison.chauffage.courbes.desactive')}</p>
    {:else if error}
      <p class="muted">{t('maison.chauffage.courbes.erreur', { error })}</p>
    {:else if !data}
      <p class="muted">{t('maison.chauffage.courbes.chargement')}</p>
    {:else if view === 'all'}
      <div class="room all" class:busy={loading}>
        <p class="when">{hover != null ? when(hover) : t('maison.chauffage.courbes.maintenant')}</p>
        <HeatingPlot
          {from}
          {to}
          {axis}
          {range}
          {hover}
          onhover={(ts) => (hover = ts)}
          label={t('maison.chauffage.courbes.toutes_graphique')}
          lines={[...outdoorLine, ...curves.map((c) => ({ id: c.id, points: c.temp, color: c.color }))]}
        />
        <ul class="legend">
          {#each curves as c (c.id)}
            {@const v = hover != null ? valueAt(c.temp, hover) : c.last?.[1]}
            <li><i style:background={c.color}></i><span>{c.name}</span><b class="num">{v != null ? `${deg(Math.round(v * 10) / 10)}°` : '—'}</b></li>
          {/each}
          {#if outdoor.length}
            {@const v = hover != null ? valueAt(outdoor, hover) : outdoor.at(-1)?.[1]}
            <li><i class="outdoor"></i><span>{t('maison.chauffage.courbes.exterieur')}</span><b class="num">{v != null ? `${deg(Math.round(v * 10) / 10)}°` : '—'}</b></li>
          {/if}
        </ul>
      </div>
    {:else}
      <ul class="keys" aria-hidden="true">
        <li><i class="key temp"></i>{t('maison.chauffage.courbes.releve')}</li>
        <li><i class="key setpoint"></i>{t('maison.chauffage.courbes.consigne_legende')}</li>
        <li><i class="key heat"></i>{t('maison.chauffage.courbes.chauffe_legende')}</li>
        {#if outdoor.length}<li><i class="key outdoor"></i>{t('maison.chauffage.courbes.exterieur')}</li>{/if}
      </ul>
      <div class="rooms" class:busy={loading}>
        {#each curves as c (c.id)}
          {@const now = hover != null ? at(c, hover) : null}
          <article class="room">
            <header>
              <h3>{c.name}</h3>
              {#if now}
                <span class="when">{when(hover)}</span>
              {:else if c.last}
                <span class="when">{t('maison.chauffage.courbes.releve_a', { time: ago(c.last[0]) })}</span>
              {/if}
            </header>
            <div class="readout" aria-live="polite">
              {#if now}
                <strong class="num">{now.temp != null ? `${deg(Math.round(now.temp * 10) / 10)}°` : '—'}</strong>
                {#if now.aim != null}<span>{t('maison.chauffage.courbes.consigne', { temp: deg(now.aim) })}</span>{/if}
                <span class:hot={now.share > 0}>{heatText(now.share)}</span>
                {#if outdoor.length && valueAt(outdoor, hover) != null}<span>{t('maison.chauffage.courbes.dehors', { temp: deg(Math.round(valueAt(outdoor, hover) * 10) / 10) })}</span>{/if}
              {:else if c.last}
                <strong class="num">{deg(Math.round(c.last[1] * 10) / 10)}°</strong>
                {#if c.aim != null}<span>{t('maison.chauffage.courbes.consigne', { temp: deg(c.aim) })}</span>{/if}
                <span class:hot={c.heating}>{heatText(c.heating ? 1 : 0)}</span>
              {:else}
                <span class="muted">{t('maison.chauffage.courbes.vide')}</span>
              {/if}
            </div>
            <HeatingPlot
              {from}
              {to}
              {axis}
              {range}
              {hover}
              onhover={(ts) => (hover = ts)}
              label={t('maison.chauffage.courbes.graphique', { room: c.name })}
              lines={[...outdoorLine, { id: c.id, points: c.temp }]}
              steps={c.set}
              heat={c.heat}
            />
          </article>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .curves {
    display: grid;
    gap: var(--sp-3);
    min-width: 0;
  }

  .head,
  .tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 12px;
  }

  h2 {
    margin: 0;
    font-size: 1.05rem;
  }

  h3 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 650;
  }

  p {
    margin: 0;
  }

  .segmented {
    display: inline-flex;
    padding: 3px;
    border-radius: 999px;
    background: var(--surface-2);
    max-width: 100%;
  }

  .segmented button,
  .chip {
    min-height: 40px;
    padding: 6px 14px;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--ink-2);
    font-weight: 650;
    font-size: 0.9rem;
    white-space: nowrap;
  }

  .segmented button.on {
    background: var(--surface);
    color: var(--ink);
    box-shadow: 0 1px 2px rgb(0 0 0 / 12%);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    background: var(--surface-2);
    border: 2px solid transparent;
  }

  .chip.on {
    border-color: var(--ink-3);
    color: var(--ink);
  }

  .keys {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    font-size: 0.85rem;
    color: var(--ink-2);
  }

  .keys li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .key {
    display: inline-block;
    width: 16px;
    height: 0;
    border-top: 2px solid var(--cool);
  }

  .key.setpoint {
    border-top: 1.5px solid var(--warm);
  }

  .key.heat {
    height: 6px;
    border: 0;
    border-radius: 2px;
    background: var(--warm);
  }

  .key.outdoor {
    border-top: 1.5px dashed var(--ink-3);
  }

  .rooms {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 340px), 1fr));
    gap: var(--sp-3);
  }

  .busy {
    opacity: 0.7;
    transition: opacity 0.2s;
  }

  .room {
    display: grid;
    gap: 6px;
    min-width: 0;
    padding: 12px 14px 8px;
    border-radius: var(--r-sm);
    background: var(--surface-2);
  }

  .room header {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 2px 10px;
  }

  .when {
    color: var(--ink-3);
    font-size: 0.8rem;
    font-weight: 600;
  }

  .readout {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 12px;
    min-height: 30px;
    font-size: 0.9rem;
    color: var(--ink-2);
  }

  .readout strong {
    font-size: 1.4rem;
    font-weight: 650;
    letter-spacing: -0.02em;
    color: var(--ink);
  }

  .hot {
    color: var(--warm-ink);
    font-weight: 650;
  }

  .room :global(.halo) {
    stroke: var(--surface-2);
  }

  .legend {
    list-style: none;
    margin: 6px 0 4px;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 180px), 1fr));
    gap: 6px 16px;
  }

  .legend li {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--sp-2);
    font-size: 0.9rem;
  }

  .legend span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .legend i {
    width: 10px;
    height: 10px;
    border-radius: 4px;
  }

  .legend i.outdoor {
    height: 0;
    border-radius: 0;
    border-top: 2px dashed var(--ink-3);
  }

  button:focus-visible {
    outline: 2px solid currentColor;
    outline-offset: 2px;
  }
</style>
