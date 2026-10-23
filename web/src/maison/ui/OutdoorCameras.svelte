<script>
  // What goes on outside: the outdoor cameras (an image on touch for the
  // ones on battery), under each its last ring and its last motion, and
  // beside them what they saw since yesterday, read in the history, with
  // the photo Moli kept when it kept one. A tap goes to the cameras.
  import { onMount, untrack } from 'svelte';
  import { hub, home, value, num, dayAndTime, clock } from '../lib/home.svelte.js';
  import { t } from '../../lib/i18n.svelte.js';
  import Icon from './Icon.svelte';
  import CameraTile from './CameraTile.svelte';

  /** `cameras`: `[{ id, name, refresh, battery }]`, the outdoor ones. */
  let { cameras = [] } = $props();

  const KINDS = [
    ['doorbell', 'salon.evenement.sonnette', 'bell'],
    ['person', 'salon.evenement.personne', 'account'],
    ['vehicle', 'salon.evenement.vehicule', 'car'],
    ['animal', 'salon.evenement.animal', 'dog'],
    ['package', 'salon.evenement.colis', 'package'],
    ['motion', 'salon.evenement.mouvement', 'motion'],
  ];
  const on = (v) => v === true || (typeof v === 'number' && v > 0);

  /** `{ [camera]: { [key]: [ts…] } }`: the rising edges of the last day. */
  let seen = $state({});
  /** `{ [camera]: [{ ts, key, name }] }`: the photos kept. */
  let photos = $state({});

  async function edges(id, key) {
    try {
      const res = await fetch(`/api/history?point=${encodeURIComponent(`${id}/${key}`)}&hours=24&points=2000`);
      if (!res.ok) return [];
      const s = await res.json();
      const out = [];
      let prev = s?.before ? on(s.before[1]) : false;
      for (const [ts, v] of s?.raw ?? []) {
        if (on(v) && !prev) out.push(ts);
        prev = on(v);
      }
      return out;
    } catch {
      return [];
    }
  }

  async function read() {
    for (const cam of cameras) {
      const d = hub.devices[cam.id];
      if (!d) continue;
      const keys = KINDS.map(([k]) => k).filter((k) => d.points.some((p) => p.key === k));
      Promise.all(keys.map(async (k) => [k, await edges(cam.id, k)])).then((pairs) => {
        seen = { ...seen, [cam.id]: Object.fromEntries(pairs) };
      });
      fetch(`/api/captures?device=${encodeURIComponent(cam.id)}&days=1`)
        .then((r) => (r.ok ? r.json() : []))
        .then((list) => {
          photos = { ...photos, [cam.id]: Array.isArray(list) ? list : [] };
        })
        .catch(() => {});
    }
  }

  onMount(() => {
    const timer = setInterval(read, 120_000);
    return () => clearInterval(timer);
  });
  // Read again when the cameras change, and at a ring or a motion as it
  // happens (not at every image refresh).
  const ids = $derived(cameras.map((c) => c.id).join());
  const live = $derived(cameras.some((c) => KINDS.some(([k]) => on(value(c.id, k)))));
  $effect(() => {
    void ids;
    void live;
    untrack(read);
  });

  const last = (id, key) => {
    const list = seen[id]?.[key];
    return list?.length ? list.at(-1) : null;
  };
  const tells = (id, key) => hub.devices[id]?.points.some((p) => p.key === key);
  const battery = (id) => {
    const b = value(id, 'battery');
    return typeof b === 'number' ? b : null;
  };

  /** Everything seen since yesterday, newest first, with its photo. */
  const passages = $derived.by(() => {
    const out = [];
    for (const cam of cameras) {
      for (const [key, label, icon] of KINDS) {
        for (const ts of seen[cam.id]?.[key] ?? []) {
          const photo = (photos[cam.id] ?? []).find((p) => p.key === key && Math.abs(p.ts - ts) < 2000);
          out.push({ id: `${cam.id}/${key}/${ts}`, cam: cam.name, ts, key, label, icon, src: photo ? `/api/captures/${encodeURIComponent(cam.id)}/${encodeURIComponent(photo.name)}` : null });
        }
      }
    }
    return out.sort((a, b) => b.ts - a.ts).slice(0, 8);
  });
  const telling = $derived(cameras.some((c) => KINDS.some(([k]) => tells(c.id, k))));
  const open = () => (location.hash = '#/cameras');
</script>

{#if cameras.length}
  <section class="card">
    <div class="card-head">
      <h2><Icon name="cctv" size={18} />{t('maison.dehors.en_ce_moment')}</h2>
      <a class="more" href="#/cameras" aria-label={t('maison.dehors.toutes_cameras')}><Icon name="arrow-top-right" size={18} /></a>
    </div>
    <div class="body" class:with-passages={telling}>
      <div class="cams">
        {#each cameras as cam (cam.id)}
          <div class="slot">
            <CameraTile id={cam.id} name={cam.name} refresh={cam.refresh} battery={cam.battery} onopen={open} />
            {#if tells(cam.id, 'doorbell') || tells(cam.id, 'motion')}
              <p class="told">
                {#if tells(cam.id, 'doorbell')}
                  <b>{last(cam.id, 'doorbell') ? t('maison.accueil.dernier_appui', { when: dayAndTime(last(cam.id, 'doorbell'), home.now) }) : t('maison.accueil.aucun_appui')}</b>
                {/if}
                {#if tells(cam.id, 'motion')}
                  <span class="muted" class:strong={!tells(cam.id, 'doorbell')}>{last(cam.id, 'motion') ? t('salon.cameras.mouvement', { when: dayAndTime(last(cam.id, 'motion'), home.now) }) : t('maison.accueil.aucun_mouvement')}</span>
                {/if}
                {#if battery(cam.id) != null}<span class="muted">· {t('maison.accueil.batterie', { pct: num(battery(cam.id)) })}</span>{/if}
              </p>
            {/if}
          </div>
        {/each}
      </div>
      {#if telling}
        <div class="passages">
          <div class="head"><h3>{t('maison.dehors.passages')}</h3>{#if passages.length}<span class="chip num">{passages.length}</span>{/if}</div>
          {#if passages.length}
            <ol>
              {#each passages as p (p.id)}
                <li>
                  <button onclick={open}>
                    <span class="thumb">
                      {#if p.src}<img src={p.src} alt="" loading="lazy" />{:else}<Icon name={p.icon} size={18} />{/if}
                    </span>
                    <span class="what"><b>{t(p.label)}</b><small>{p.cam}</small></span>
                    <time class="num">{clock(p.ts)}</time>
                  </button>
                </li>
              {/each}
            </ol>
          {:else}
            <p class="muted small">{t('maison.dehors.aucun_passage')}</p>
          {/if}
          <p class="muted small">{t('maison.dehors.passages_note')}</p>
        </div>
      {/if}
    </div>
  </section>
{/if}

<style>
  .body {
    display: grid;
    gap: var(--sp-5);
    align-items: start;
  }

  .body.with-passages {
    grid-template-columns: minmax(0, 2fr) minmax(0, 1fr);
  }

  .cams {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr));
    gap: var(--sp-4);
  }

  .slot {
    display: grid;
    gap: var(--sp-2);
  }

  .told {
    font-size: 13.5px;
  }

  .told b {
    font-weight: 650;
    display: block;
  }

  .told .strong {
    color: var(--ink);
    font-weight: 650;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 10px;
  }

  h3 {
    font-size: 15px;
    font-weight: 700;
  }

  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--sp-1);
  }

  li button {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: grid;
    grid-template-columns: 72px 1fr auto;
    align-items: center;
    gap: var(--sp-3);
    padding: 6px;
    border-radius: var(--r-sm);
    cursor: pointer;
  }

  li button:hover {
    background: var(--surface-2);
  }

  li button:focus-visible {
    outline: 3px solid var(--cool);
    outline-offset: 2px;
  }

  .thumb {
    width: 72px;
    aspect-ratio: 16 / 9;
    border-radius: 8px;
    background: #1b2231;
    color: #8f98ab;
    display: grid;
    place-items: center;
    overflow: hidden;
  }

  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .what b {
    display: block;
    font-weight: 650;
    font-size: 14.5px;
  }

  .what small {
    color: var(--ink-3);
    font-size: 12.5px;
  }

  time {
    font-size: var(--fs-sm);
    color: var(--ink-3);
  }

  .small {
    font-size: 12.5px;
    margin-top: 10px;
  }

  @media (max-width: 767.98px) {
    .body.with-passages {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
