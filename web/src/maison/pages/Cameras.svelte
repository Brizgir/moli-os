<script>
  import { untrack } from 'svelte';
  import { home, hub, roomOf, hidden, protectedRoom, nameOf, clock, relative, ringing, camerasAsDoorbell, lastSeen, dayAndTime, value, num } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from '../ui/Icon.svelte';
  import CameraTile from '../ui/CameraTile.svelte';
  import SecurityPanel from '../ui/SecurityPanel.svelte';
  import RingModes from '../ui/RingModes.svelte';

  // The layout's cameras first (their names, pace, battery), then any
  // other; one on battery (a `battery` point: Ring) is asked on touch only.
  const cameras = $derived.by(() => {
    const listed = home.config?.favorites?.cameras ?? [];
    const known = new Set(listed.map((c) => c.id));
    const others = Object.values(hub.devices)
      .filter((d) => d.camera && !known.has(d.id) && !hidden(d.id))
      .map((d) => ({ id: d.id, name: nameOf(d.id), refresh: 15, battery: d.points.some((p) => p.key === 'battery') }));
    return [...listed, ...others].map((c) => ({ ...c, locked: protectedRoom(roomOf(hub.devices[c.id]) ?? '') }));
  });

  // A doorbell and cameras that tell their events without an image (Ring):
  // the last press and motion of each, from the history. Cameras with
  // an image tell theirs under it.
  const rings = $derived(Object.values(hub.devices).filter((d) => !hidden(d.id) && ringing(d)));
  const telling = (d) => d?.points.some((p) => p.key === 'doorbell' || p.key === 'motion');
  let ringSeen = $state({});
  const ringIds = $derived([...rings.map((d) => d.id), ...cameras.filter((c) => telling(hub.devices[c.id])).map((c) => c.id)].join());
  $effect(() => {
    void ringIds;
    untrack(readRings);
  });
  function readRings() {
    const devices = ringIds ? ringIds.split(',').map((id) => hub.devices[id]).filter(Boolean) : [];
    for (const d of devices) {
      const keys = ['doorbell', 'motion'].filter((k) => d.points.some((p) => p.key === k));
      Promise.all(keys.map((k) => lastSeen(d.id, [k], 24 * 7).then((ts) => [k, ts]))).then((pairs) => {
        ringSeen = { ...ringSeen, [d.id]: Object.fromEntries(pairs) };
      });
    }
  }

  let open = $state(null);
  const current = $derived(cameras.find((c) => c.id === open));

  // What the camera saw today, from the history (rising edges only).
  const KINDS = [
    ['doorbell', 'salon.evenement.sonnette', 'bell'],
    ['person', 'salon.evenement.personne', 'account'],
    ['vehicle', 'salon.evenement.vehicule', 'car'],
    ['animal', 'salon.evenement.animal', 'dog'],
    ['package', 'salon.evenement.colis', 'package'],
    ['motion', 'salon.evenement.mouvement', 'motion'],
  ];
  const kindOf = (key) => KINDS.find(([k]) => k === key);
  let seen = $state([]);
  let seenLoading = $state(false);
  // The photos Moli kept of its events (`[server] captures_days`), and the
  // one shown big instead of the live image.
  let photos = $state([]);
  let shown = $state(null);
  const photoUrl = (id, p) => `/api/captures/${encodeURIComponent(id)}/${encodeURIComponent(p.name)}`;
  // An event's photo: the one kept for it (named after its time).
  const photoOf = (s) => photos.find((p) => p.key === s.key && Math.abs(p.ts - s.ts) < 2000);

  $effect(() => {
    const id = open;
    seen = [];
    photos = [];
    shown = null;
    if (!id) return;
    fetch(`/api/captures?device=${encodeURIComponent(id)}&days=7`)
      .then((r) => (r.ok ? r.json() : []))
      .then((list) => {
        if (open === id) photos = list;
      })
      .catch(() => {});
    const d = hub.devices[id];
    const keys = KINDS.filter(([k]) => d?.points.some((p) => p.key === k));
    seenLoading = true;
    Promise.all(
      keys.map(([k, label, icon]) =>
        fetch(`/api/history?point=${encodeURIComponent(`${id}/${k}`)}&hours=24&points=2000`)
          .then((r) => (r.ok ? r.json() : null))
          .then((s) => {
            const out = [];
            const on = (v) => v === true || (typeof v === 'number' && v > 0);
            let prev = s?.before ? on(s.before[1]) : false;
            for (const [ts, v] of s?.raw ?? []) {
              if (on(v) && !prev) out.push({ ts, key: k, label, icon });
              prev = on(v);
            }
            return out;
          })
          .catch(() => []),
      ),
    ).then((lists) => {
      if (open !== id) return;
      seen = lists.flat().sort((a, b) => b.ts - a.ts).slice(0, 12);
      seenLoading = false;
    });
  });

  function onkey(e) {
    if (e.key !== 'Escape') return;
    if (shown) shown = null;
    else open = null;
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="cameras">
  <header>
    <h1 class="page-title">{camerasAsDoorbell() ? t('commun.nav.sonnette_cameras') : t('salon.cameras.titre')}</h1>
    <p class="page-sub">{t('salon.cameras.sous_titre')}</p>
  </header>

  <SecurityPanel />

  <RingModes />

  <div class="wall">
    {#each cameras as cam (cam.id)}
      <div class="slot">
        <CameraTile id={cam.id} name={cam.name} refresh={cam.refresh} battery={cam.battery} onopen={(id) => (open = id)} />
        {#if cam.locked}<span class="chip cool lock"><Icon name="lock" size={14} />{t('salon.cameras.protegee')}</span>{/if}
        {#if telling(hub.devices[cam.id])}
          {@const d = hub.devices[cam.id]}
          {@const seenOf = ringSeen[cam.id] ?? {}}
          <p class="told">
            {#if d.points.some((p) => p.key === 'doorbell')}
              <b>{seenOf.doorbell ? t('maison.accueil.dernier_appui', { when: dayAndTime(seenOf.doorbell, home.now) }) : t('maison.accueil.aucun_appui')}</b>
            {/if}
            {#if d.points.some((p) => p.key === 'motion')}
              <span class="muted">{seenOf.motion ? t('salon.cameras.mouvement', { when: dayAndTime(seenOf.motion, home.now) }) : t('maison.accueil.aucun_mouvement')}</span>
            {/if}
          </p>
        {/if}
      </div>
    {:else}
      {#if !rings.length}<p class="muted">{t('salon.cameras.aucune')}</p>{/if}
    {/each}
  </div>

  {#if rings.length}
    <section class="rings">
      {#each rings as d (d.id)}
        {@const seenOf = ringSeen[d.id] ?? {}}
        <div class="card ring">
          <div class="card-head">
            <h2><Icon name={d.points.some((p) => p.key === 'doorbell') ? 'bell' : 'cctv'} size={18} />{nameOf(d.id)}</h2>
            {#if typeof value(d.id, 'battery') === 'number'}<span class="chip">{t('commun.pourcent', { n: num(value(d.id, 'battery')) })}</span>{/if}
          </div>
          <div class="picture">{t('maison.accueil.sans_image')}</div>
          {#if d.points.some((p) => p.key === 'doorbell')}
            <p><b>{seenOf.doorbell ? t('maison.accueil.dernier_appui', { when: dayAndTime(seenOf.doorbell, home.now) }) : t('maison.accueil.aucun_appui')}</b></p>
          {/if}
          <p class="muted">{seenOf.motion ? t('salon.cameras.mouvement', { when: dayAndTime(seenOf.motion, home.now) }) : t('maison.accueil.aucun_mouvement')}</p>
        </div>
      {/each}
    </section>
  {/if}
</div>

{#if current}
  <div class="viewer" role="dialog" aria-modal="true" aria-label={t('salon.cameras.camera', { nom: current.name })}>
    <button class="backdrop" onclick={() => (open = null)} aria-label={t('commun.fermer')}></button>
    <div class="stage">
      <div class="bar">
        <h2>{current.name}</h2>
        <button class="close" onclick={() => (open = null)} aria-label={t('commun.fermer')}><Icon name="close" size={22} /></button>
      </div>
      {#if shown}
        {@const kind = kindOf(shown.key)}
        <figure class="photo">
          <img src={photoUrl(current.id, shown)} alt={t('salon.cameras.photo', { quoi: kind ? t(kind[1]) : shown.key, quand: dayAndTime(shown.ts, home.now) })} />
          <figcaption>
            <span>{#if kind}<Icon name={kind[2]} size={16} />{t(kind[1])} · {/if}{dayAndTime(shown.ts, home.now)}</span>
            <button class="live" onclick={() => (shown = null)}>{t('salon.cameras.direct')}</button>
          </figcaption>
        </figure>
      {:else}
        {#key current.id}
          <CameraTile id={current.id} name={current.name} refresh={current.battery ? 0 : 2} battery={current.battery} big />
        {/key}
      {/if}
      <div class="seen">
        <h3>{t('salon.cameras.aujourdhui')}</h3>
        {#if seenLoading}
          <p class="muted">…</p>
        {:else if seen.length}
          <ul>
            {#each seen as s (s.ts + s.label)}
              {@const photo = photoOf(s)}
              <li>
                {#if photo}
                  <button class="event" onclick={() => (shown = photo)} aria-label={t('salon.cameras.voir_photo')}>
                    <img src={photoUrl(current.id, photo)} alt="" loading="lazy" />
                    <Icon name={s.icon} size={16} /><b>{t(s.label)}</b><span class="muted">{clock(s.ts)} · {relative(s.ts, home.now)}</span>
                  </button>
                {:else}
                  <Icon name={s.icon} size={16} /><b>{t(s.label)}</b><span class="muted">{clock(s.ts)} · {relative(s.ts, home.now)}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {:else}
          <p class="muted">{t('salon.cameras.rien')}</p>
        {/if}
      </div>
      {#if photos.length}
        <div class="seen">
          <h3>{t('salon.cameras.photos')}</h3>
          <div class="photos">
            {#each photos as p (p.name)}
              {@const kind = kindOf(p.key)}
              <button class="thumb" class:on={shown?.name === p.name} onclick={() => (shown = p)}>
                <img src={photoUrl(current.id, p)} alt={t('salon.cameras.photo', { quoi: kind ? t(kind[1]) : p.key, quand: dayAndTime(p.ts, home.now) })} loading="lazy" />
                <span>{#if kind}<Icon name={kind[2]} size={14} />{/if}{dayAndTime(p.ts, home.now)}</span>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .cameras {
    display: grid;
    gap: 22px;
  }

  .rings {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 320px), 1fr));
    gap: 20px;
  }

  .ring {
    display: grid;
    gap: 10px;
  }

  .ring p {
    margin: 0;
    font-size: 14px;
  }

  .picture {
    aspect-ratio: 16 / 10;
    border-radius: 18px;
    background: var(--surface-2);
    display: grid;
    place-items: center;
    padding: 12px;
    color: var(--ink-3);
    font-size: 14px;
    text-align: center;
  }

  .wall {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 420px), 1fr));
    gap: 20px;
  }

  .slot {
    position: relative;
    display: grid;
    gap: 8px;
  }

  .lock {
    justify-self: start;
  }

  .viewer {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 24px;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    border: 0;
    background: rgb(6 9 14 / 82%);
    backdrop-filter: blur(8px);
  }

  .stage {
    position: relative;
    width: min(1200px, 100%);
    max-height: calc(100dvh - 48px);
    overflow: auto;
    display: grid;
    gap: 14px;
    color: #eef1f7;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .bar h2 {
    font-size: 22px;
    font-weight: 700;
  }

  .close {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    border: 0;
    background: rgb(255 255 255 / 12%);
    color: #fff;
    display: grid;
    place-items: center;
  }

  .seen h3 {
    font-size: 13px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #a9b1c2;
    margin-bottom: 8px;
  }

  .seen ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .seen li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-radius: 999px;
    background: rgb(255 255 255 / 9%);
    font-size: 14px;
  }

  .seen .muted {
    color: #a9b1c2;
  }

  .told {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    font-size: 14px;
  }

  .event {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 0;
    padding: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .event img {
    width: 40px;
    height: 28px;
    object-fit: cover;
    border-radius: 6px;
  }

  .photo {
    margin: 0;
    display: grid;
    gap: 10px;
  }

  .photo img {
    width: 100%;
    max-height: 70dvh;
    object-fit: contain;
    border-radius: 18px;
    background: #000;
  }

  .photo figcaption {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 14px;
  }

  .photo figcaption span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .live {
    border: 0;
    border-radius: 999px;
    padding: 8px 16px;
    background: rgb(255 255 255 / 14%);
    color: #fff;
    font: inherit;
  }

  .photos {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 10px;
  }

  .thumb {
    display: grid;
    gap: 6px;
    border: 2px solid transparent;
    border-radius: 12px;
    padding: 0;
    background: none;
    color: #eef1f7;
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }

  .thumb.on {
    border-color: #fff;
  }

  .thumb img {
    width: 100%;
    aspect-ratio: 16 / 10;
    object-fit: cover;
    border-radius: 10px;
    background: #000;
  }

  .thumb span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 4px 4px;
  }
</style>
